use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tracing::{info, warn};

use feagi_agent::server::auth::DummyAuth;
use feagi_agent::server::FeagiAgentHandler;
use feagi_config::FeagiConfig;
use feagi_io::core::protocol_implementations::websocket::{
    FeagiWebSocketServerPublisherProperties, FeagiWebSocketServerPullerProperties,
    FeagiWebSocketServerRouterProperties,
};
use feagi_io::core::protocol_implementations::zmq::{
    FeagiZmqServerPublisherProperties, FeagiZmqServerPullerProperties,
    FeagiZmqServerRouterProperties,
};
use feagi_npu_burst_engine::{
    DynamicNPU, MotorPublisher, RawFireQueueSnapshot, TracingMutex, VisualizationPublisher,
};
use feagi_serialization::{FeagiByteContainer, FeagiByteStructureType};
use feagi_structures::genomic::cortical_area::CorticalID;
use feagi_structures::neuron_voxels::xyzp::{
    CorticalMappedXYZPNeuronVoxels, NeuronVoxelXYZPArrays,
};

/// Runtime loop that polls agent transport servers and injects sensory data.
pub struct AgentHandlerRuntime {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl AgentHandlerRuntime {
    /// Start the agent handler poll loop on a dedicated thread.
    pub fn start(
        handler: Arc<Mutex<FeagiAgentHandler>>,
        npu: Arc<TracingMutex<DynamicNPU>>,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_flag = Arc::clone(&running);

        let handle = match std::thread::Builder::new()
            .name("feagi-agent-io".to_string())
            .spawn(move || {
                while running_flag.load(Ordering::Relaxed) {
                    let sensory_fbc = {
                        let mut guard = handler.lock();
                        if let Err(e) = guard.poll_registration_handlers() {
                            warn!("Agent registration poll failed: {}", e);
                        }
                        guard.poll_sensory_handlers().cloned()
                    };

                    if let Some(fbc) = sensory_fbc {
                        if let Err(e) = inject_sensory_from_fbc(&npu, &fbc) {
                            warn!("Sensory injection failed: {}", e);
                        }
                    }

                    std::thread::yield_now();
                }
            }) {
            Ok(handle) => Some(handle),
            Err(e) => {
                warn!("Failed to spawn agent handler thread: {}", e);
                None
            }
        };

        Self { running, handle }
    }

    /// Stop the agent handler poll loop and wait for the thread to exit.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Receiver for device_registrations from ZMQ/WS registrations (for auto IPU/OPU creation).
pub type RegistrationDeviceRegistrationsRx = tokio::sync::mpsc::Receiver<serde_json::Value>;

/// Build and configure a FeagiAgentHandler based on FEAGI config.
/// Returns the handler and a receiver for device_registrations sent by the registration hook
/// (ZMQ/WS path); the host should spawn a task that receives and runs auto_create.
pub fn build_agent_handler(
    config: &FeagiConfig,
) -> Result<(FeagiAgentHandler, RegistrationDeviceRegistrationsRx)> {
    let mut handler =
        FeagiAgentHandler::new_with_config(Box::new(DummyAuth {}), config.clone());

    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let hook: Arc<
        dyn Fn(
                feagi_serialization::SessionID,
                feagi_agent::registration::AgentDescriptor,
                Option<serde_json::Value>,
            ) + Send
            + Sync,
    > = Arc::new(move |_session_id, _descriptor, device_registrations| {
        if let Some(dr) = device_registrations {
            let _ = tx.try_send(dr);
        }
    });
    handler.set_registration_hook(hook);

    let available_transports: Vec<String> = config
        .transports
        .available
        .iter()
        .map(|transport| transport.to_lowercase())
        .collect();

    if available_transports.iter().any(|transport| transport == "zmq") {
        let sensory_address = format_tcp_endpoint(&config.zmq.host, config.ports.zmq_sensory_port);
        let motor_address = format_tcp_endpoint(&config.zmq.host, config.ports.zmq_motor_port);
        let visualization_address = format_tcp_endpoint(
            &config.zmq.host,
            config.ports.zmq_visualization_port,
        );
        let registration_address =
            format_tcp_endpoint(&config.agent.host, config.agent.registration_port);

        let sensory = FeagiZmqServerPullerProperties::new(&sensory_address)
            .context("Failed to create ZMQ sensory puller properties")?;
        handler.add_puller_server(Box::new(sensory));

        let motor = FeagiZmqServerPublisherProperties::new(&motor_address)
            .context("Failed to create ZMQ motor publisher properties")?;
        let visualization = FeagiZmqServerPublisherProperties::new(&visualization_address)
            .context("Failed to create ZMQ visualization publisher properties")?;
        handler.add_publisher_server(Box::new(motor));
        handler.add_publisher_server(Box::new(visualization));

        let registration = FeagiZmqServerRouterProperties::new(&registration_address)
            .context("Failed to create ZMQ registration router properties")?;
        handler
            .add_and_start_registration_server(Box::new(registration))
            .context("Failed to start ZMQ registration server")?;
    }

    if available_transports
        .iter()
        .any(|transport| transport == "websocket" || transport == "ws")
    {
        let sensory_address =
            format_ws_address(&config.websocket.host, config.websocket.sensory_port);
        let motor_address =
            format_ws_address(&config.websocket.host, config.websocket.motor_port);
        let visualization_address = format_ws_address(
            &config.websocket.host,
            config.websocket.visualization_port,
        );
        let registration_address =
            format_ws_address(&config.websocket.host, config.websocket.registration_port);

        let sensory = FeagiWebSocketServerPullerProperties::new(&sensory_address)
            .context("Failed to create WebSocket sensory puller properties")?;
        handler.add_puller_server(Box::new(sensory));

        let motor = FeagiWebSocketServerPublisherProperties::new(&motor_address)
            .context("Failed to create WebSocket motor publisher properties")?;
        let visualization = FeagiWebSocketServerPublisherProperties::new(&visualization_address)
            .context("Failed to create WebSocket visualization publisher properties")?;
        handler.add_publisher_server(Box::new(motor));
        handler.add_publisher_server(Box::new(visualization));

        let registration = FeagiWebSocketServerRouterProperties::new(&registration_address)
            .context("Failed to create WebSocket registration router properties")?;
        handler
            .add_and_start_registration_server(Box::new(registration))
            .context("Failed to start WebSocket registration server")?;
    }

    info!("    ✓ Agent handler configured");
    Ok((handler, rx))
}

/// Publisher that forwards visualization data to FeagiAgentHandler transports.
pub struct HandlerVisualizationPublisher {
    handler: Arc<Mutex<FeagiAgentHandler>>,
}

impl HandlerVisualizationPublisher {
    /// Create a new visualization publisher backed by FeagiAgentHandler.
    pub fn new(handler: Arc<Mutex<FeagiAgentHandler>>) -> Self {
        Self { handler }
    }
}

impl VisualizationPublisher for HandlerVisualizationPublisher {
    fn publish_raw_fire_queue_for_agent(
        &self,
        _agent_id: &str,
        fire_data: RawFireQueueSnapshot,
    ) -> Result<(), String> {
        let fbc = encode_fire_queue_to_fbc(fire_data)?;
        let mut handler = self.handler.lock();
        handler
            .poll_visualization_handlers(Some(&fbc))
            .map_err(|e| format!("Failed to publish visualization data: {}", e))?;
        Ok(())
    }
}

/// Publisher that forwards motor bytes to FeagiAgentHandler transports.
pub struct HandlerMotorPublisher {
    handler: Arc<Mutex<FeagiAgentHandler>>,
}

impl HandlerMotorPublisher {
    /// Create a new motor publisher backed by FeagiAgentHandler.
    pub fn new(handler: Arc<Mutex<FeagiAgentHandler>>) -> Self {
        Self { handler }
    }
}

impl MotorPublisher for HandlerMotorPublisher {
    fn publish_motor(&self, _agent_id: &str, data: &[u8]) -> Result<(), String> {
        let fbc = fbc_from_bytes(data)?;
        let mut handler = self.handler.lock();
        handler
            .poll_motor_handlers(Some(&fbc))
            .map_err(|e| format!("Failed to publish motor data: {}", e))?;
        Ok(())
    }
}

fn encode_fire_queue_to_fbc(
    fire_data: RawFireQueueSnapshot,
) -> Result<FeagiByteContainer, String> {
    let mut cortical_mapped = CorticalMappedXYZPNeuronVoxels::new_with_capacity(fire_data.len());

    for (_area_id, area_data) in fire_data {
        if area_data.coords_x.is_empty()
            || area_data.coords_y.is_empty()
            || area_data.coords_z.is_empty()
            || area_data.potentials.is_empty()
        {
            return Err("Visualization fire queue contains empty vectors".to_string());
        }

        if area_data.coords_x.len() != area_data.coords_y.len()
            || area_data.coords_x.len() != area_data.coords_z.len()
            || area_data.coords_x.len() != area_data.potentials.len()
        {
            return Err("Visualization fire queue vector length mismatch".to_string());
        }

        let cortical_id = CorticalID::try_from_base_64(&area_data.cortical_id)
            .map_err(|e| format!("Invalid cortical_id '{}': {}", area_data.cortical_id, e))?;

        let arrays = NeuronVoxelXYZPArrays::new_from_vectors(
            area_data.coords_x,
            area_data.coords_y,
            area_data.coords_z,
            area_data.potentials,
        )
        .map_err(|e| format!("Failed to build XYZP arrays: {:?}", e))?;

        cortical_mapped.mappings.insert(cortical_id, arrays);
    }

    let mut byte_container = FeagiByteContainer::new_empty();
    byte_container
        .overwrite_byte_data_with_single_struct_data(&cortical_mapped, 0)
        .map_err(|e| format!("Failed to serialize visualization FBC: {:?}", e))?;

    Ok(byte_container)
}

fn fbc_from_bytes(data: &[u8]) -> Result<FeagiByteContainer, String> {
    let mut byte_container = FeagiByteContainer::new_empty();
    let mut data_vec = data.to_vec();
    byte_container
        .try_write_data_to_container_and_verify(&mut |bytes| {
            std::mem::swap(bytes, &mut data_vec);
            Ok(())
        })
        .map_err(|e| format!("Failed to load FeagiByteContainer bytes: {:?}", e))?;
    Ok(byte_container)
}

fn inject_sensory_from_fbc(
    npu: &Arc<TracingMutex<DynamicNPU>>,
    fbc: &FeagiByteContainer,
) -> Result<(), String> {
    let boxed = fbc
        .try_create_struct_from_first_found_struct_of_type(
            FeagiByteStructureType::NeuronCategoricalXYZP,
        )
        .map_err(|e| format!("Failed to read sensory FBC: {:?}", e))?
        .ok_or_else(|| "Sensory FBC missing NeuronCategoricalXYZP payload".to_string())?;

    let mapped = CorticalMappedXYZPNeuronVoxels::try_from(boxed)
        .map_err(|e| format!("Failed to decode sensory XYZP payload: {:?}", e))?;

    let mut npu_lock = npu.lock().map_err(|_| "Failed to lock NPU".to_string())?;
    for (cortical_id, arrays) in mapped.mappings.iter() {
        let (x, y, z, p) = arrays.borrow_xyzp_vectors();
        let _ = npu_lock.inject_sensory_xyzp_arrays_by_id(cortical_id, x, y, z, p);
    }

    Ok(())
}

fn format_tcp_endpoint(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("tcp://[{host}]:{port}")
    } else {
        format!("tcp://{host}:{port}")
    }
}

fn format_ws_address(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
