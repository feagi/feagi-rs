//! Internal FEAGI component initialization
//!
//! This module contains the logic for initializing FEAGI's core components.
//! It is internal to the library and should not be used directly by embedders.
//!
//! The neural half of this module was removed with the old NPU. What remains is the transport and
//! API surface: the agent handler with its ZMQ/WebSocket servers, the sensory intake queue, and
//! the HTTP API backed by [`crate::stub_services`], except for analytics, which
//! [`crate::brain_development`] serves from the BDU so the health endpoint reports real figures.
//! Re-attaching a burst engine means giving [`FeagiComponents`] an NPU handle again and replacing
//! the remaining stub services.

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::brain_development::{BduAnalyticsService, DevelopedBrain};
use crate::network_provider::FeagiNetworkConnectionInfoProvider;
use crate::stub_services::{
    StubAgentService, StubConnectomeService, StubGenomeService, StubNeuronService,
    StubRuntimeService, StubSnapshotService, StubSystemService,
};
use feagi_api::endpoints::network::NetworkConnectionInfoProvider;
use feagi_api::transports::http::server::{create_http_server, ApiState};
use feagi_config::FeagiConfig;
use feagi_io::SensoryIntakeQueue;
use feagi_npu::standard::npu::npu_target_frequency::NPUTargetFrequency;
use feagi_npu::standard::wnpu::wnpu::WrappedNeuronProcessingUnit;
use feagi_services::traits::agent_service::AgentService;
use feagi_services::traits::SystemService as SystemServiceTrait;
use feagi_services::{
    AnalyticsService, ConnectomeService, GenomeService, NeuronService, RuntimeService,
    SnapshotService,
};

// New architecture imports
use feagi_agent::server::auth::DummyAuth;
use feagi_agent::server::FeagiAgentHandler;

use feagi_io::protocol_implementations::websocket::websocket_std::{
    FeagiWebSocketServerPublisherProperties, FeagiWebSocketServerPullerProperties,
    FeagiWebSocketServerRouterProperties,
};
#[cfg(feature = "zmq-transport")]
use feagi_io::protocol_implementations::zmq::{
    FeagiZmqServerPublisherProperties, FeagiZmqServerPullerProperties,
    FeagiZmqServerRouterProperties,
};

/// Core FEAGI components
///
/// All components are wrapped in Arc/Mutex for thread-safe access.
pub struct FeagiComponents {
    /// Wrapped NPU shared with every stub service adapter. Held behind a `parking_lot::Mutex`
    /// because WNPU's mutation surface takes `&mut self`; every call site locks briefly and never
    /// crosses an `await`, so this remains `Send` for `#[async_trait]`.
    pub neuron_processing_unit: Arc<parking_lot::Mutex<WrappedNeuronProcessingUnit>>,
    pub runtime_service: Arc<StubRuntimeService>,
    /// What the BDU has developed; the source of the health endpoint's brain figures.
    ///
    /// Nothing develops a genome on this path yet, so it stays empty and health reports an
    /// undeveloped brain.
    pub developed_brain: Arc<DevelopedBrain>,
    pub agent_handler: Arc<Mutex<FeagiAgentHandler>>,
    /// Transport-agnostic sensory queue (feagi-io); feed from polling loop when agents send sensory
    ///
    /// The wrapped NPU's agent-subscription surface is still `todo!()`, so nothing drains this
    /// queue in practice yet. The queue is depth-bounded and simply discards the oldest payloads
    /// until WNPU's sensory input pipeline is wired.
    pub sensory_intake_queue: Arc<SensoryIntakeQueue>,
}

/// Initialize all core FEAGI components
///
/// This function is adapted from main.rs initialization logic.
pub async fn initialize_components(config: &FeagiConfig) -> Result<FeagiComponents> {
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
            config.agent.bind_host, config.agent.registration_port
        );
        let registration_adv_addr = format!(
            "tcp://{}:{}",
            config.agent.advertised_host, config.agent.registration_port
        );
        let router_props = Box::new(
            FeagiZmqServerRouterProperties::new(&registration_addr, &registration_adv_addr)
                .context("Failed to create ZMQ router properties")?,
        );
        agent_handler
            .add_and_start_command_control_server(router_props)
            .context("Failed to add ZMQ registration server")?;
        info!("      ✓ ZMQ registration router: {}", registration_addr);

        // Multiple agent slots: handler consumes one puller + two publishers per ZMQ agent.
        // Slot 0 uses config ports; slots 1..N use offset ranges to avoid overlap.
        const ZMQ_AGENT_SLOTS: u16 = 8;
        const ZMQ_SENSORY_OFFSET: u16 = 5566; // slots 1.. use 5566, 5567, ...
        const ZMQ_MOTOR_OFFSET: u16 = 5574; // slots 1.. use 5574, 5575, ...
        const ZMQ_VIZ_OFFSET: u16 = 5582; // slots 1.. use 5582, 5583, ...

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

            let sensory_addr = format!("tcp://{}:{}", config.zmq.bind_host, sensory_port);
            let sensory_adv_addr = format!("tcp://{}:{}", config.zmq.advertised_host, sensory_port);
            let sensory_props = Box::new(
                FeagiZmqServerPullerProperties::new(&sensory_addr, &sensory_adv_addr)
                    .context("Failed to create ZMQ sensory puller properties")?,
            );
            agent_handler.add_puller_server(sensory_props);

            let motor_addr = format!("tcp://{}:{}", config.zmq.bind_host, motor_port);
            let motor_adv_addr = format!("tcp://{}:{}", config.zmq.advertised_host, motor_port);
            let motor_props = Box::new(
                FeagiZmqServerPublisherProperties::new(&motor_addr, &motor_adv_addr)
                    .context("Failed to create ZMQ motor publisher properties")?,
            );
            agent_handler.add_publisher_server(motor_props);

            let viz_addr = format!("tcp://{}:{}", config.zmq.bind_host, viz_port);
            let viz_adv_addr = format!("tcp://{}:{}", config.zmq.advertised_host, viz_port);
            let viz_props = Box::new(
                FeagiZmqServerPublisherProperties::new(&viz_addr, &viz_adv_addr)
                    .context("Failed to create ZMQ visualization publisher properties")?,
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
            config.websocket.bind_host, config.websocket.registration_port
        );
        let ws_registration_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.registration_port
        );
        let ws_router_props = Box::new(
            FeagiWebSocketServerRouterProperties::new_with_remote(
                &ws_registration_addr,
                &ws_registration_adv_addr,
            )
            .context("Failed to create WebSocket router properties")?,
        );
        agent_handler
            .add_and_start_command_control_server(ws_router_props)
            .context("Failed to add WebSocket registration server")?;
        info!(
            "      ✓ WebSocket registration router: {}",
            ws_registration_addr
        );

        // Sensory puller
        let ws_sensory_addr = format!(
            "{}:{}",
            config.websocket.bind_host, config.websocket.sensory_port
        );
        let ws_sensory_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.sensory_port
        );
        let ws_sensory_props = Box::new(
            FeagiWebSocketServerPullerProperties::new_with_remote(
                &ws_sensory_addr,
                &ws_sensory_adv_addr,
            )
            .context("Failed to create WebSocket sensory puller properties")?,
        );
        agent_handler.add_puller_server(ws_sensory_props);
        info!("      ✓ WebSocket sensory puller: {}", ws_sensory_addr);

        // Motor publisher
        let ws_motor_addr = format!(
            "{}:{}",
            config.websocket.bind_host, config.websocket.motor_port
        );
        let ws_motor_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.motor_port
        );
        let ws_motor_props = Box::new(
            FeagiWebSocketServerPublisherProperties::new(&ws_motor_addr, &ws_motor_adv_addr)
                .context("Failed to create WebSocket motor publisher properties")?,
        );
        agent_handler.add_publisher_server(ws_motor_props);
        info!("      ✓ WebSocket motor publisher: {}", ws_motor_addr);

        // Visualization publisher
        let ws_viz_addr = format!(
            "{}:{}",
            config.websocket.bind_host, config.websocket.visualization_port
        );
        let ws_viz_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.visualization_port
        );
        let ws_viz_props = Box::new(
            FeagiWebSocketServerPublisherProperties::new(&ws_viz_addr, &ws_viz_adv_addr)
                .context("Failed to create WebSocket visualization publisher properties")?,
        );
        agent_handler.add_publisher_server(ws_viz_props);
        info!("      ✓ WebSocket visualization publisher: {}", ws_viz_addr);
    }

    let agent_handler = Arc::new(Mutex::new(agent_handler));
    info!("    ✓ Agent Handler created with transports");

    let burst_hz = 1.0 / config.neural.burst_engine_timestep;

    // Wrapped NPU: constructed empty and shared so every service adapter can forward calls into
    // it. Nothing drives it yet on this path (embedding), but every route now has a live handle.
    let wnpu = WrappedNeuronProcessingUnit::new(
        feagi_data::quantization_levels::feagi_index_quantization::FeagiIndexQuantizationLevel::Genomic,
        vec![],
    )
    .context("Failed to construct wrapped NPU")?;
    let wnpu = Arc::new(parking_lot::Mutex::new(wnpu));
    if let Err(e) = wnpu
        .lock()
        .run_at_frequency(NPUTargetFrequency::new_from_frequency(burst_hz))
    {
        warn!(
            "    ⚠ Wrapped NPU rejected initial run_at_frequency({} Hz): {}",
            burst_hz, e
        );
    }

    let runtime_service = Arc::new(StubRuntimeService::new(burst_hz, Arc::clone(&wnpu)));
    info!(
        "    ✓ Runtime service created and wired to the wrapped NPU (target {:.0}Hz)",
        burst_hz
    );

    let developed_brain = Arc::new(DevelopedBrain::new(&config.connectome));

    let sensory_intake_queue = Arc::new(SensoryIntakeQueue::new());
    info!("    ⚠ Sensory intake queue created without a consumer (awaiting WNPU subscription surface)");

    Ok(FeagiComponents {
        neuron_processing_unit: wnpu,
        runtime_service,
        developed_brain,
        agent_handler,
        sensory_intake_queue,
    })
}

/// Start HTTP API server
///
/// Spawns Axum server on Tokio runtime (non-blocking).
pub async fn start_http_server(components: &FeagiComponents, config: &FeagiConfig) -> Result<()> {
    info!("  Creating service layer (analytics from BDU, rest wired to wrapped NPU)...");

    let wnpu_handle = Arc::clone(&components.neuron_processing_unit);
    let genome_service = Arc::new(StubGenomeService::new(Arc::clone(&wnpu_handle)));
    let connectome_service = Arc::new(StubConnectomeService::new(Arc::clone(&wnpu_handle)));
    // Analytics is not a stub: `/v1/system/health_check` is served from the BDU's development
    // report plus the runtime service's burst state. See [`crate::brain_development`].
    let analytics_service = Arc::new(BduAnalyticsService::new(
        Arc::clone(&components.developed_brain),
        components.runtime_service.clone() as Arc<dyn RuntimeService + Send + Sync>,
    ));
    let neuron_service = Arc::new(StubNeuronService::new(Arc::clone(&wnpu_handle)));
    let snapshot_service = Arc::new(StubSnapshotService::new(Arc::clone(&wnpu_handle)));
    let agent_service = Arc::new(StubAgentService::new(Arc::clone(&wnpu_handle)));
    let system_service = Arc::new(StubSystemService::new(
        crate::version::collect_version_info(),
        Arc::clone(&wnpu_handle),
    ));

    info!("    ✓ Services created (forwarded to wrapped NPU placeholders)");

    // Get FEAGI session timestamp (when this instance started)
    let feagi_session_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    info!("    ✓ FEAGI session timestamp: {}", feagi_session_timestamp);

    let api_port = config.api.port;
    let api_bind_host = config.api.bind_host.clone();
    let api_advertised_host = config.api.advertised_host.clone();
    let network_provider = Arc::new(FeagiNetworkConnectionInfoProvider {
        api_advertised_host: api_advertised_host.clone(),
        api_port,
        agent_handler: Arc::clone(&components.agent_handler),
        viz_transport_policy: config.visualization.transport.clone(),
        // Registration endpoint is from agent config; data endpoints are from ZMQ ports config.
        zmq_enabled: true,
        zmq_registration_advertised_host: config.agent.advertised_host.clone(),
        zmq_advertised_host: config.zmq.advertised_host.clone(),
        zmq_registration_port: config.agent.registration_port,
        zmq_sensory_port: config.ports.zmq_sensory_port,
        zmq_motor_port: config.ports.zmq_motor_port,
        zmq_visualization_port: config.ports.zmq_visualization_port,
        zmq_api_control_port: config.ports.zmq_rest_port,
        websocket_enabled: config.websocket.enabled,
        websocket_advertised_host: config.websocket.advertised_host.clone(),
        websocket_registration_port: config.websocket.registration_port,
        websocket_sensory_port: config.websocket.sensory_port,
        websocket_motor_port: config.websocket.motor_port,
        websocket_visualization_port: config.websocket.visualization_port,
        websocket_rest_api_port: config.websocket.rest_api_port,
    }) as Arc<dyn NetworkConnectionInfoProvider>;
    let (genome_transition_lock, genome_transition_in_progress) =
        ApiState::init_genome_transition_controls();

    let filesystem_data_root = ApiState::filesystem_data_root_from_config(&config.system.data_dir);

    let api_state = ApiState {
        network_connection_info_provider: Some(network_provider),
        agent_service: Some(agent_service as Arc<dyn AgentService + Send + Sync>),
        genome_service: genome_service as Arc<dyn GenomeService + Send + Sync>,
        connectome_service: connectome_service as Arc<dyn ConnectomeService + Send + Sync>,
        analytics_service: analytics_service as Arc<dyn AnalyticsService + Send + Sync>,
        runtime_service: components.runtime_service.clone()
            as Arc<dyn RuntimeService + Send + Sync>,
        neuron_service: neuron_service as Arc<dyn NeuronService + Send + Sync>,
        system_service: system_service as Arc<dyn SystemServiceTrait + Send + Sync>,
        snapshot_service: Some(snapshot_service as Arc<dyn SnapshotService + Send + Sync>),
        feagi_session_timestamp,
        filesystem_data_root,
        // Memory area stats came from the plasticity executor, which went with the old NPU.
        memory_stats_cache: None,
        amalgamation_state: ApiState::init_amalgamation_state(),
        genome_transition_lock,
        genome_transition_in_progress,
        #[cfg(feature = "feagi-agent")]
        agent_handler: Some(Arc::clone(&components.agent_handler)),
        #[cfg(not(feature = "feagi-agent"))]
        agent_handler: None,
    };

    // Start HTTP API server
    info!(
        "  Starting HTTP API server on {}:{} (advertised as {}:{})...",
        api_bind_host, api_port, api_advertised_host, api_port
    );
    let app = create_http_server(api_state);
    let addr = format!("{}:{}", api_bind_host, api_port);

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

/// Load a genome and notify the agent handler.
///
/// Genome loading drove neuroembryogenesis against `ConnectomeManager` and the old NPU. Both are
/// gone, so there is nowhere to develop a connectome into.
pub async fn load_genome_with_agent_handler(
    _agent_handler: &Arc<Mutex<FeagiAgentHandler>>,
    genome_path: &std::path::Path,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "Genome loading is unavailable: neuroembryogenesis needs an NPU, and the old one was \
         removed pending integration of feagi_npu::wnpu (requested: {})",
        genome_path.display()
    ))
}
