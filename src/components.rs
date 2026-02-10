//! Internal FEAGI component initialization
//!
//! This module contains the logic for initializing FEAGI's core components.
//! It is internal to the library and should not be used directly by embedders.

use anyhow::{Context, Result};
use parking_lot::RwLock;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::network_provider::FeagiNetworkConnectionInfoProvider;
use feagi_api::endpoints::network::NetworkConnectionInfoProvider;
use feagi_api::transports::http::server::{create_http_server, ApiState};
use feagi_brain_development::ConnectomeManager;
use feagi_config::FeagiConfig;
use feagi_io::SensoryIntakeQueue;
use feagi_npu_burst_engine::backend::GpuConfig;
use feagi_npu_burst_engine::{BurstLoopRunner, DynamicNPU, RustNPU, SensoryIntake, TracingMutex};
use feagi_services::impls::{AgentServiceImpl, SystemServiceImpl};
use feagi_services::traits::agent_service::AgentService;
use feagi_services::*;

// New architecture imports
use feagi_agent::server::auth::DummyAuth;
use feagi_agent::server::FeagiAgentHandler;

#[cfg(feature = "zmq-transport")]
use feagi_io::protocol_implementations::zmq::{
    FeagiZmqServerPublisherProperties, FeagiZmqServerPullerProperties,
    FeagiZmqServerRouterProperties,
};
use feagi_io::protocol_implementations::websocket::websocket_std::{
    FeagiWebSocketServerPublisherProperties, FeagiWebSocketServerPullerProperties,
    FeagiWebSocketServerRouterProperties,
};

/// Core FEAGI components
///
/// All components are wrapped in Arc/Mutex for thread-safe access.
pub struct FeagiComponents {
    pub npu: Arc<feagi_npu_burst_engine::TracingMutex<DynamicNPU>>,
    pub connectome_manager: Arc<RwLock<ConnectomeManager>>,
    pub runtime_service: Arc<RuntimeServiceImpl>,
    pub burst_runner: Arc<RwLock<BurstLoopRunner>>,
    pub agent_handler: Arc<Mutex<FeagiAgentHandler>>,
    /// Transport-agnostic sensory queue (feagi-io); feed from polling loop when agents send sensory
    pub sensory_intake_queue: Arc<SensoryIntakeQueue>,
}

/// Initialize all core FEAGI components
///
/// This function is adapted from main.rs initialization logic.
pub async fn initialize_components(config: &FeagiConfig) -> Result<FeagiComponents> {
    use feagi_npu_burst_engine::backend::CPUBackend;
    use feagi_npu_neural::types::FeagiError;
    use feagi_npu_runtime::StdRuntime;

    info!("  Initializing NPU...");

    // GPU config is available but not yet used in NPU initialization
    let _gpu_config = GpuConfig {
        use_gpu: config.resources.use_gpu,
        hybrid_enabled: config.neural.hybrid.enabled,
        gpu_threshold: config.neural.hybrid.gpu_threshold,
        gpu_memory_fraction: config.resources.gpu_memory_fraction,
    };

    // Default to INT8 quantization for embedded mode (memory efficient)
    let runtime = StdRuntime;
    let backend = CPUBackend::new();

    let npu_result = RustNPU::new(
        runtime,
        backend,
        config.connectome.neuron_space,
        config.connectome.synapse_space,
        10, // fire_ledger_window
    );

    // Wrap NPU in TracingMutex (or Mutex if tracing disabled) to automatically log all lock acquisitions
    // When npu-lock-tracing feature is disabled, TracingMutex is a type alias for std::sync::Mutex (zero overhead)
    let npu = Arc::new(TracingMutex::new(
        DynamicNPU::INT8(
            npu_result.map_err(|e: FeagiError| anyhow::anyhow!("Failed to create NPU: {}", e))?,
        ),
        "NPU",
    ));

    info!("    ✓ NPU initialized with INT8 quantization");

    // Initialize ConnectomeManager
    info!("  Initializing ConnectomeManager...");
    let manager = ConnectomeManager::instance();
    manager.write().set_npu(Arc::clone(&npu));
    info!("    ✓ ConnectomeManager initialized");

    // Initialize Agent Handler (new architecture)
    info!("  Creating Agent Handler (new architecture)...");

    let auth_backend = Box::new(DummyAuth {});
    let mut agent_handler = FeagiAgentHandler::new(auth_backend);

    // Add ZMQ servers if enabled
    #[cfg(feature = "zmq-transport")]
    {
        info!("    Adding ZMQ transport servers...");
        
        // Registration router (command/control) - single shared router
        let registration_addr = format!(
            "tcp://{}:{}",
            config.agent.host, config.agent.registration_port
        );
        let router_props = Box::new(
            FeagiZmqServerRouterProperties::new(&registration_addr)
                .context("Failed to create ZMQ router properties")?
        );
        agent_handler
            .add_and_start_command_control_server(router_props)
            .context("Failed to add ZMQ registration server")?;
        info!("      ✓ ZMQ registration router: {}", registration_addr);

        // Multiple agent slots: handler consumes one puller + two publishers per ZMQ agent.
        // Slot 0 uses config ports; slots 1..N use offset ranges to avoid overlap.
        const ZMQ_AGENT_SLOTS: u16 = 8;
        const ZMQ_SENSORY_OFFSET: u16 = 5566;  // slots 1.. use 5566, 5567, ...
        const ZMQ_MOTOR_OFFSET: u16 = 5574;    // slots 1.. use 5574, 5575, ...
        const ZMQ_VIZ_OFFSET: u16 = 5582;      // slots 1.. use 5582, 5583, ...

        for slot in 0..ZMQ_AGENT_SLOTS {
            let (sensory_port, motor_port, viz_port) = if slot == 0 {
                (
                    config.ports.zmq_sensory_port,
                    config.ports.zmq_motor_port,
                    config.ports.zmq_visualization_port,
                )
            } else {
                let i = slot;
                (
                    ZMQ_SENSORY_OFFSET + i - 1,
                    ZMQ_MOTOR_OFFSET + i - 1,
                    ZMQ_VIZ_OFFSET + i - 1,
                )
            };

            let sensory_addr = format!("tcp://{}:{}", config.zmq.host, sensory_port);
            let sensory_props = Box::new(
                FeagiZmqServerPullerProperties::new(&sensory_addr)
                    .context("Failed to create ZMQ sensory puller properties")?
            );
            agent_handler.add_puller_server(sensory_props);

            let motor_addr = format!("tcp://{}:{}", config.zmq.host, motor_port);
            let motor_props = Box::new(
                FeagiZmqServerPublisherProperties::new(&motor_addr)
                    .context("Failed to create ZMQ motor publisher properties")?
            );
            agent_handler.add_publisher_server(motor_props);

            let viz_addr = format!("tcp://{}:{}", config.zmq.host, viz_port);
            let viz_props = Box::new(
                FeagiZmqServerPublisherProperties::new(&viz_addr)
                    .context("Failed to create ZMQ visualization publisher properties")?
            );
            agent_handler.add_publisher_server(viz_props);
        }
        info!(
            "      ✓ ZMQ sensory/motor/viz: {} slots (ports 0: {}/{}/{}, 1..: {}-{}/{}-{}/{}-{})",
            ZMQ_AGENT_SLOTS,
            config.ports.zmq_sensory_port,
            config.ports.zmq_motor_port,
            config.ports.zmq_visualization_port,
            ZMQ_SENSORY_OFFSET,
            ZMQ_SENSORY_OFFSET + ZMQ_AGENT_SLOTS - 2,
            ZMQ_MOTOR_OFFSET,
            ZMQ_MOTOR_OFFSET + ZMQ_AGENT_SLOTS - 2,
            ZMQ_VIZ_OFFSET,
            ZMQ_VIZ_OFFSET + ZMQ_AGENT_SLOTS - 2
        );
    }

    // Add WebSocket servers if enabled
    if config.websocket.enabled {
        info!("    Adding WebSocket transport servers...");

        // Registration router
        let ws_registration_addr = format!(
            "{}:{}",
            config.websocket.host, config.websocket.registration_port
        );
        let ws_router_props = Box::new(
            FeagiWebSocketServerRouterProperties::new(&ws_registration_addr)
                .context("Failed to create WebSocket router properties")?
        );
        agent_handler
            .add_and_start_command_control_server(ws_router_props)
            .context("Failed to add WebSocket registration server")?;
        info!("      ✓ WebSocket registration router: {}", ws_registration_addr);

        // Sensory puller
        let ws_sensory_addr = format!(
            "{}:{}",
            config.websocket.host, config.websocket.sensory_port
        );
        let ws_sensory_props = Box::new(
            FeagiWebSocketServerPullerProperties::new(&ws_sensory_addr)
                .context("Failed to create WebSocket sensory puller properties")?
        );
        agent_handler.add_puller_server(ws_sensory_props);
        info!("      ✓ WebSocket sensory puller: {}", ws_sensory_addr);

        // Motor publisher
        let ws_motor_addr = format!(
            "{}:{}",
            config.websocket.host, config.websocket.motor_port
        );
        let ws_motor_props = Box::new(
            FeagiWebSocketServerPublisherProperties::new(&ws_motor_addr)
                .context("Failed to create WebSocket motor publisher properties")?
        );
        agent_handler.add_publisher_server(ws_motor_props);
        info!("      ✓ WebSocket motor publisher: {}", ws_motor_addr);

        // Visualization publisher
        let ws_viz_addr = format!(
            "{}:{}",
            config.websocket.host, config.websocket.visualization_port
        );
        let ws_viz_props = Box::new(
            FeagiWebSocketServerPublisherProperties::new(&ws_viz_addr)
                .context("Failed to create WebSocket visualization publisher properties")?
        );
        agent_handler.add_and_start_broadcast_publisher(ws_viz_props)
            .context("Failed to start WebSocket visualization publisher")?;
        info!("      ✓ WebSocket visualization publisher: {} (broadcast mode)", ws_viz_addr);
    }

    let agent_handler = Arc::new(Mutex::new(agent_handler));
    info!("    ✓ Agent Handler created with transports");

    // Initialize BurstLoopRunner
    info!("  Initializing BurstLoopRunner...");
    let burst_timestep = config.neural.burst_engine_timestep;

    // Create agent-handler-backed publishers
    struct AgentHandlerVisualizationPublisher {
        #[allow(dead_code)] // TODO: Use when encoding/sending is implemented
        handler: Arc<Mutex<FeagiAgentHandler>>,
    }
    
    impl feagi_npu_burst_engine::VisualizationPublisher for AgentHandlerVisualizationPublisher {
        fn publish_raw_fire_queue_for_agent(
            &self,
            agent_id: &str,
            fire_data: feagi_npu_burst_engine::RawFireQueueSnapshot,
        ) -> Result<(), String> {
            if fire_data.is_empty() {
                return Ok(());
            }

            let mut handler_guard = self.handler.lock().unwrap();
            
            // Convert RawFireQueueSnapshot to CorticalMappedXYZPNeuronVoxels
            use feagi_structures::neuron_voxels::xyzp::{CorticalMappedXYZPNeuronVoxels, NeuronVoxelXYZPArrays};
            use feagi_structures::genomic::cortical_area::CorticalID;
            use feagi_serialization::FeagiByteContainer;
            
            let mut cortical_mapped = CorticalMappedXYZPNeuronVoxels::new();
            
            for (_area_idx, fire_queue_data) in fire_data {
                // Parse cortical_id from base64 string
                if let Ok(cortical_id) = CorticalID::try_from_base_64(&fire_queue_data.cortical_id) {
                    if let Ok(neuron_voxels) = NeuronVoxelXYZPArrays::new_from_vectors(
                        fire_queue_data.coords_x,
                        fire_queue_data.coords_y,
                        fire_queue_data.coords_z,
                        fire_queue_data.potentials,
                    ) {
                        cortical_mapped.insert(cortical_id, neuron_voxels);
                    }
                }
            }

            // Serialize to raw bytes (Type 11 format)
            use feagi_serialization::FeagiSerializable;
            let num_bytes = cortical_mapped.get_number_of_bytes_needed();
            let mut raw_bytes = vec![0u8; num_bytes];
            cortical_mapped.try_serialize_struct_to_byte_slice(&mut raw_bytes)
                .map_err(|e| format!("Failed to serialize visualization: {:?}", e))?;

            // Try to find SessionID for embodiment agents
            if let Some(session_id) = handler_guard.find_session_by_agent_id(agent_id) {
                // Embodiment agent - properly wrap in FeagiByteContainer
                let mut container = FeagiByteContainer::new_empty();
                let _ = container.set_session_id(session_id);
                container.overwrite_byte_data_with_single_struct_data(&cortical_mapped, 0)
                    .map_err(|e| format!("Failed to wrap visualization: {:?}", e))?;
                handler_guard.send_visualization_data(session_id, &container)
                    .map_err(|e| format!("Failed to send visualization: {:?}", e))?;
            } else {
                // Visualization-only agent - send raw Type 11 data (BV expects unwrapped format)
                tracing::trace!("[VIZ-PUBLISHER] Broadcasting {} raw bytes to visualization-only clients (agent_id={})", raw_bytes.len(), agent_id);
                handler_guard.broadcast_raw_visualization_data(&raw_bytes)
                    .map_err(|e| format!("Failed to broadcast visualization: {:?}", e))?;
            }

            Ok(())
        }
    }

    struct AgentHandlerMotorPublisher {
        #[allow(dead_code)] // TODO: Use when SessionID lookup is implemented
        handler: Arc<Mutex<FeagiAgentHandler>>,
    }
    
    impl feagi_npu_burst_engine::MotorPublisher for AgentHandlerMotorPublisher {
        fn publish_motor(&self, agent_id: &str, data: &[u8]) -> Result<(), String> {
            if data.is_empty() {
                return Ok(());
            }

            let mut handler_guard = self.handler.lock().unwrap();
            
            // Find SessionID for this agent
            let session_id = match handler_guard.find_session_by_agent_id(agent_id) {
                Some(sid) => sid,
                None => {
                    // Agent not connected, skip silently
                    return Ok(());
                }
            };

            // Motor data is already encoded as FeagiByteContainer bytes
            use feagi_serialization::FeagiByteContainer;
            // Create container by copying existing bytes
            let mut container = FeagiByteContainer::new_empty();
            container.try_write_data_by_copy_and_verify(data)
                .map_err(|e| format!("Failed to parse motor data: {:?}", e))?;

            // Send via handler
            handler_guard.send_motor_data(session_id, &container)
                .map_err(|e| format!("Failed to send motor data: {:?}", e))?;

            Ok(())
        }
    }

    let viz_publisher = Arc::new(Mutex::new(AgentHandlerVisualizationPublisher {
        handler: Arc::clone(&agent_handler),
    }));
    let motor_publisher = Arc::new(Mutex::new(AgentHandlerMotorPublisher {
        handler: Arc::clone(&agent_handler),
    }));

    let burst_hz = 1.0 / burst_timestep;

    let burst_runner = Arc::new(RwLock::new(BurstLoopRunner::new(
        Arc::clone(&npu),
        Some(viz_publisher),
        Some(motor_publisher),
        burst_hz,
    )));
    info!("    ✓ BurstLoopRunner initialized ({:.0}Hz)", burst_hz);

    // Create runtime service
    let runtime_service = Arc::new(RuntimeServiceImpl::new(Arc::clone(&burst_runner)));
    info!("    ✓ Runtime service created");

    // Transport-agnostic sensory intake (feagi-io): burst loop consumes from queue; caller feeds it
    let sensory_intake_queue = Arc::new(SensoryIntakeQueue::new());
    struct SensoryIntakeAdapter {
        queue: Arc<SensoryIntakeQueue>,
    }
    impl SensoryIntake for SensoryIntakeAdapter {
        fn poll_sensory_data(&mut self) -> Result<Option<Vec<u8>>, String> {
            Ok(self.queue.poll_next())
        }
    }
    burst_runner.write().set_sensory_intake(Arc::new(Mutex::new(SensoryIntakeAdapter {
        queue: Arc::clone(&sensory_intake_queue),
    })) as Arc<Mutex<dyn SensoryIntake>>);
    info!("    ✓ Sensory intake (feagi-io) wired to BurstLoopRunner");

    Ok(FeagiComponents {
        npu,
        connectome_manager: manager,
        runtime_service,
        burst_runner,
        agent_handler,
        sensory_intake_queue,
    })
}

/// Start HTTP API server
///
/// Spawns Axum server on Tokio runtime (non-blocking).
pub async fn start_http_server(components: &FeagiComponents, config: &FeagiConfig) -> Result<()> {
    info!("  Creating service layer...");

    // Get parameter queue from burst runner
    let parameter_queue = components.burst_runner.read().parameter_queue.clone();

    // Create GenomeServiceImpl and get reference to current_genome for sharing
    let mut genome_service_impl = GenomeServiceImpl::new_with_parameter_queue(
        Arc::clone(&components.connectome_manager),
        parameter_queue,
    );
    // Wire burst runner for cache refresh
    genome_service_impl.set_burst_runner(Arc::clone(&components.burst_runner));
    let genome_service_impl = Arc::new(genome_service_impl);
    let current_genome = genome_service_impl.get_current_genome_arc();
    let genome_service = genome_service_impl;

    let mut connectome_service_impl = ConnectomeServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        current_genome.clone(),
    );
    // Wire burst runner for cache refresh
    connectome_service_impl.set_burst_runner(Arc::clone(&components.burst_runner));
    let connectome_service = Arc::new(connectome_service_impl);
    let analytics_service = Arc::new(AnalyticsServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        Some(Arc::clone(&components.burst_runner)),
    ));
    let neuron_service = Arc::new(NeuronServiceImpl::new(Arc::clone(
        &components.connectome_manager,
    )));

    // Collect version information for all crates in this binary
    let version_info = crate::version::collect_version_info();

    let system_service = Arc::new(SystemServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        Some(Arc::clone(&components.burst_runner)),
        version_info,
    ));

    // TODO: Create agent service with agent registry from handler
    // For now, create minimal agent service with empty registry
    use parking_lot::RwLock as PRwLock;
    // Create empty agent registry with default settings
    let empty_registry = Arc::new(PRwLock::new(feagi_services::AgentRegistry::new(100, 60000)));
    let agent_service_impl = AgentServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        empty_registry
    );
    let agent_service = Arc::new(agent_service_impl);
    
    info!("    ✓ Services created");

    // Create snapshot service
    let snapshot_dir = std::path::PathBuf::from("./snapshots");
    let snapshot_service = Arc::new(feagi_services::SnapshotServiceImpl::new(snapshot_dir));

    // Get FEAGI session timestamp (when this instance started)
    let feagi_session_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    info!("    ✓ FEAGI session timestamp: {}", feagi_session_timestamp);

    let api_port = config.api.port;
    let api_host = config.api.host.clone();
    let network_provider = Arc::new(FeagiNetworkConnectionInfoProvider {
        api_host: api_host.clone(),
        api_port,
        agent_handler: Arc::clone(&components.agent_handler),
        viz_transport_policy: config.visualization.transport.clone(),
    }) as Arc<dyn NetworkConnectionInfoProvider>;

    let api_state = ApiState {
        network_connection_info_provider: Some(network_provider),
        agent_service: Some(agent_service as Arc<dyn AgentService + Send + Sync>),
        genome_service: genome_service as Arc<dyn GenomeService + Send + Sync>,
        connectome_service: connectome_service as Arc<dyn ConnectomeService + Send + Sync>,
        analytics_service: analytics_service as Arc<dyn AnalyticsService + Send + Sync>,
        runtime_service: components.runtime_service.clone()
            as Arc<dyn RuntimeService + Send + Sync>,
        neuron_service: neuron_service as Arc<dyn NeuronService + Send + Sync>,
        system_service: system_service
            as Arc<dyn feagi_services::traits::SystemService + Send + Sync>,
        snapshot_service: Some(
            snapshot_service as Arc<dyn feagi_services::SnapshotService + Send + Sync>,
        ),
        feagi_session_timestamp,
        #[cfg(feature = "plasticity")]
        memory_stats_cache: None, // Will be initialized with plasticity manager in main.rs
        #[cfg(not(feature = "plasticity"))]
        memory_stats_cache: None,
        amalgamation_state: ApiState::init_amalgamation_state(),
        #[cfg(feature = "feagi-agent")]
        agent_handler: Some(Arc::clone(&components.agent_handler)),
        #[cfg(not(feature = "feagi-agent"))]
        agent_handler: None,
    };

    // Start HTTP API server
    info!("  Starting HTTP API server on {}:{}...", api_host, api_port);
    let app = create_http_server(api_state);
    let addr = format!("{}:{}", api_host, api_port);

    info!("  Spawning HTTP server task...");
    tokio::spawn(async move {
        info!("  📡 HTTP server task started - binding to {}...", addr);

        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                info!("  ✅ HTTP listener bound successfully to {}", addr);
                l
            }
            Err(e) => {
                error!("  ❌ Failed to bind HTTP listener to {}: {}", addr, e);
                panic!("Failed to bind API server: {}", e);
            }
        };

        info!("  📡 Swagger UI available at http://{}/swagger-ui/", addr);
        info!("  🌐 HTTP server accepting connections on {}...", addr);

        match axum::serve(listener, app).await {
            Ok(_) => info!("  ✅ HTTP server serve() completed gracefully"),
            Err(e) => error!("  ❌ HTTP server serve() error: {}", e),
        }

        error!("  ⚠️ HTTP server task ending (should never happen!)");
    });

    // CRITICAL: Give the spawned server task time to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    info!("  ✅ HTTP server task spawned successfully");

    Ok(())
}

/// Load genome and notify agent handler
pub async fn load_genome_with_agent_handler(
    manager: &Arc<RwLock<ConnectomeManager>>,
    _agent_handler: &Arc<Mutex<FeagiAgentHandler>>,
    genome_path: &std::path::Path,
) -> Result<()> {
    use feagi_evolutionary::{load_genome_from_file, validate_genome};

    info!("    [GENOME-LOAD] Step 1: Loading genome file...");

    // Load genome from file
    let genome = load_genome_from_file(genome_path).context("Failed to load genome file")?;

    // Validate genome
    let validation = validate_genome(&genome);
    if !validation.errors.is_empty() {
        warn!("Genome validation errors:");
        for error in &validation.errors {
            warn!("  - {}", error);
        }
        return Err(anyhow::anyhow!("Genome validation failed"));
    }

    if !validation.warnings.is_empty() {
        warn!("Genome validation warnings:");
        for warning in &validation.warnings {
            warn!("  - {}", warning);
        }
    }

    let simulation_timestep = genome.physiology.simulation_timestep;
    info!(
        "    [GENOME-LOAD] Genome specifies simulation_timestep: {}s ({:.0}Hz)",
        simulation_timestep,
        1.0 / simulation_timestep
    );

    info!("    [GENOME-LOAD] Step 2: Performing neuroembryogenesis...");

    // Load genome into connectome (includes neuroembryogenesis)
    let result = {
        // Acquire write lock only for prepare/resize operations
        let mut mgr = manager.write();
        mgr.prepare_for_new_genome()
            .context("Failed to prepare for new genome")?;

        // Resize if needed
        mgr.resize_for_genome(&genome)
            .context("Failed to resize for genome")?;

        // Release write lock before long-running neuroembryogenesis
        drop(mgr);

        // Now develop genome (will acquire its own fine-grained locks)
        use feagi_brain_development::neuroembryogenesis::Neuroembryogenesis;
        let mut neuro = Neuroembryogenesis::new(manager.clone());
        neuro
            .develop_from_genome(&genome)
            .context("Failed to develop brain from genome")?;

        neuro.get_progress()
    };

    info!(
        "    [GENOME-LOAD] Neuroembryogenesis complete: {} neurons, {} synapses",
        result.neurons_created, result.synapses_created
    );

    // TODO: Notify agent handler of genome load for dynamic evaluation
    info!("    [GENOME-LOAD] ⚠ Agent handler notification not yet implemented");

    Ok(())
}
