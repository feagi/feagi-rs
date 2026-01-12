//! Internal FEAGI component initialization
//! 
//! This module contains the logic for initializing FEAGI's core components.
//! It is internal to the library and should not be used directly by embedders.

use std::sync::{Arc, Mutex};
use parking_lot::RwLock;
use anyhow::{Context, Result};
use tracing::{info, warn, error};

use feagi_config::FeagiConfig;
use feagi_brain_development::ConnectomeManager;
use feagi_npu_burst_engine::{BurstLoopRunner, DynamicNPU, RustNPU, TracingMutex};
use feagi_npu_burst_engine::backend::GpuConfig;
use feagi_services::*;
use feagi_services::traits::agent_service::AgentService;
use feagi_services::impls::{AgentServiceImpl, SystemServiceImpl};
use feagi_api::transports::http::server::{create_http_server, ApiState};
use feagi_io::IOSystem;

/// Core FEAGI components
/// 
/// All components are wrapped in Arc/Mutex for thread-safe access.
pub struct FeagiComponents {
    pub npu: Arc<feagi_npu_burst_engine::TracingMutex<DynamicNPU>>,
    pub connectome_manager: Arc<RwLock<ConnectomeManager>>,
    pub runtime_service: Arc<RuntimeServiceImpl>,
    pub burst_runner: Arc<RwLock<BurstLoopRunner>>,
    pub pns: Arc<IOSystem>,
}

/// Initialize all core FEAGI components
/// 
/// This function is adapted from main.rs initialization logic.
pub async fn initialize_components(config: &FeagiConfig) -> Result<FeagiComponents> {
    use feagi_npu_neural::types::FeagiError;
    use feagi_npu_runtime::StdRuntime;
    use feagi_npu_burst_engine::backend::CPUBackend;
    
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
            npu_result.map_err(|e: FeagiError| anyhow::anyhow!("Failed to create NPU: {}", e))?
        ),
        "NPU"
    ));
    
    info!("    ✓ NPU initialized with INT8 quantization");

    // Initialize ConnectomeManager
    info!("  Initializing ConnectomeManager...");
    let manager = ConnectomeManager::instance();
    manager.write().set_npu(Arc::clone(&npu));
    info!("    ✓ ConnectomeManager initialized");

    // Initialize PNS
    info!("  Creating PNS (Agent Management)...");
    
    use feagi_io::IOConfig;
    
    let mut io_config = IOConfig::default();
    io_config.zmq_rest_address = format!("tcp://{}:{}", config.agent.host, config.agent.registration_port);
    io_config.zmq_motor_address = format!("tcp://{}:{}", config.zmq.host, config.ports.zmq_motor_port);
    io_config.zmq_viz_address = format!("tcp://{}:{}", config.zmq.host, config.ports.zmq_visualization_port);
    io_config.zmq_sensory_address = format!("tcp://{}:{}", config.zmq.host, config.ports.zmq_sensory_port);
    
    // Load WebSocket configuration
    io_config.websocket.enabled = config.websocket.enabled;
    io_config.websocket.host = config.websocket.host.clone();
    io_config.websocket.sensory_port = config.websocket.sensory_port;
    io_config.websocket.motor_port = config.websocket.motor_port;
    io_config.websocket.visualization_port = config.websocket.visualization_port;
    io_config.websocket.registration_port = config.websocket.registration_port;
    io_config.websocket.rest_api_port = config.websocket.rest_api_port;
    io_config.websocket.connection_timeout_ms = config.websocket.connection_timeout_ms;
    io_config.websocket.ping_interval_ms = config.websocket.ping_interval_ms;
    io_config.websocket.ping_timeout_ms = config.websocket.ping_timeout_ms;
    io_config.websocket.close_timeout_ms = config.websocket.close_timeout_ms;
    io_config.websocket.max_message_size = config.websocket.max_message_size;
    io_config.websocket.max_connections = config.websocket.max_connections;
    
    let pns = Arc::new(IOSystem::with_config(io_config)
        .context("Failed to create PNS")?);
    
    // Wire dynamic gating callbacks
    IOSystem::wire_dynamic_gating_callbacks(&pns);
    
    info!("    ✓ PNS created");
    
    // Initialize BurstLoopRunner
    info!("  Initializing BurstLoopRunner...");
    let burst_timestep = config.neural.burst_engine_timestep;
    
    // Create PNS-backed visualization publisher
    struct PnsVisualizationPublisher {
        pns: Arc<IOSystem>,
    }
    
    impl feagi_npu_burst_engine::VisualizationPublisher for PnsVisualizationPublisher {
        fn publish_raw_fire_queue(&self, fire_data: feagi_npu_burst_engine::RawFireQueueSnapshot) -> Result<(), String> {
            self.pns.publish_raw_fire_queue(fire_data)
                .map_err(|e| format!("PNS viz publish failed: {}", e))
        }
    }
    
    // Create PNS-backed motor publisher
    struct PnsMotorPublisher {
        pns: Arc<IOSystem>,
    }
    
    impl feagi_npu_burst_engine::MotorPublisher for PnsMotorPublisher {
        fn publish_motor(&self, agent_id: &str, data: &[u8]) -> Result<(), String> {
            self.pns.publish_motor(agent_id, data)
                .map_err(|e| format!("PNS motor publish failed: {}", e))
        }
    }
    
    let viz_publisher = Arc::new(Mutex::new(PnsVisualizationPublisher {
        pns: Arc::clone(&pns),
    }));
    
    let motor_publisher = Arc::new(Mutex::new(PnsMotorPublisher {
        pns: Arc::clone(&pns),
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

    // Wire up bidirectional connections
    info!("  Wiring PNS ↔ BurstLoopRunner connections...");
    
    let sensory_mgr = burst_runner.read().sensory_manager.clone();
    pns.set_sensory_agent_manager(sensory_mgr);
    pns.set_burst_runner(Arc::clone(&burst_runner));
    pns.set_npu_for_gating(Arc::clone(&npu));
    
    info!("    ✓ PNS ↔ BurstLoopRunner connections established");

    Ok(FeagiComponents {
        npu,
        connectome_manager: manager,
        runtime_service,
        burst_runner,
        pns,
    })
}

/// Wire GenomeService and ConnectomeService to RegistrationHandler
/// 
/// This is REQUIRED for the auto-creation of missing IPU/OPU cortical areas feature.
/// All FEAGI embedders must call this function after creating services and before
/// starting the HTTP API server.
/// 
/// # Arguments
/// * `registration_handler` - The registration handler from PNS
/// * `genome_service` - The genome service instance
/// * `connectome_service` - The connectome service instance  
/// * `auto_create_enabled` - Whether to auto-create missing cortical areas (from config)
pub fn wire_registration_handler_services(
    registration_handler: &Arc<parking_lot::Mutex<feagi_io::RegistrationHandler>>,
    genome_service: &Arc<dyn feagi_services::traits::GenomeService + Send + Sync>,
    connectome_service: &Arc<dyn feagi_services::traits::ConnectomeService + Send + Sync>,
    auto_create_enabled: bool,
) {
    let mut handler = registration_handler.lock();
    handler.set_genome_service(Arc::clone(genome_service) as Arc<dyn feagi_services::traits::GenomeService + Send + Sync>);
    handler.set_connectome_service(Arc::clone(connectome_service) as Arc<dyn feagi_services::traits::ConnectomeService + Send + Sync>);
    handler.set_auto_create_missing_areas(auto_create_enabled);
    info!("    ✓ RegistrationHandler services wired (GenomeService, ConnectomeService, auto-create: {})", auto_create_enabled);
}

/// Start HTTP API server
/// 
/// Spawns Axum server on Tokio runtime (non-blocking).
pub async fn start_http_server(
    components: &FeagiComponents,
    config: &FeagiConfig,
) -> Result<()> {
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
    let neuron_service = Arc::new(NeuronServiceImpl::new(
        Arc::clone(&components.connectome_manager)
    ));
    
    // Collect version information for all crates in this binary
    let version_info = crate::version::collect_version_info();
    
    let system_service = Arc::new(SystemServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        Some(Arc::clone(&components.burst_runner)),
        version_info,
    ));
    
    let agent_registry = components.pns.get_agent_registry();
    let registration_handler = components.pns.get_registration_handler();
    
    // Wire services to RegistrationHandler (required for auto-creation of missing cortical areas)
    wire_registration_handler_services(
        &registration_handler,
        &(genome_service.clone() as Arc<dyn feagi_services::traits::GenomeService + Send + Sync>),
        &(connectome_service.clone() as Arc<dyn feagi_services::traits::ConnectomeService + Send + Sync>),
        config.agent.auto_create_missing_cortical_areas,
    );
    
    let mut agent_service_impl = AgentServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        agent_registry,
    );
    
    // Convert Arc<Mutex<RegistrationHandler>> to Arc<dyn RegistrationHandlerTrait>
    use feagi_services::traits::registration_handler::RegistrationHandlerTrait;
    
    // Wrapper to convert Arc<Mutex<RegistrationHandler>> to trait object
    struct RegistrationHandlerWrapper(Arc<parking_lot::Mutex<feagi_io::RegistrationHandler>>);
    impl RegistrationHandlerTrait for RegistrationHandlerWrapper {
        fn process_registration(&self, request: feagi_services::types::registration::RegistrationRequest) -> Result<feagi_services::types::registration::RegistrationResponse, String> {
            self.0.lock().process_registration(request)
        }
    }
    
    let handler_trait: Arc<dyn RegistrationHandlerTrait> = Arc::new(RegistrationHandlerWrapper(registration_handler));
    agent_service_impl.set_registration_handler(handler_trait);
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
    
    let api_state = ApiState {
        agent_service: Some(agent_service as Arc<dyn AgentService + Send + Sync>),
        genome_service: genome_service as Arc<dyn GenomeService + Send + Sync>,
        connectome_service: connectome_service as Arc<dyn ConnectomeService + Send + Sync>,
        analytics_service: analytics_service as Arc<dyn AnalyticsService + Send + Sync>,
        runtime_service: components.runtime_service.clone() as Arc<dyn RuntimeService + Send + Sync>,
        neuron_service: neuron_service as Arc<dyn NeuronService + Send + Sync>,
        system_service: system_service as Arc<dyn feagi_services::traits::SystemService + Send + Sync>,
        snapshot_service: Some(snapshot_service as Arc<dyn feagi_services::SnapshotService + Send + Sync>),
        feagi_session_timestamp,
        #[cfg(feature = "plasticity")]
        memory_stats_cache: None, // Will be initialized with plasticity manager in main.rs
        #[cfg(not(feature = "plasticity"))]
        memory_stats_cache: None,
        agent_connectors: ApiState::init_agent_connectors(),
    };

    // Start HTTP API server
    let api_port = config.api.port;
    let api_host = config.api.host.clone();
    
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
            },
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

/// Load genome and notify PNS for dynamic gating
pub async fn load_genome_with_pns(
    manager: &Arc<RwLock<ConnectomeManager>>,
    pns: &Arc<IOSystem>,
    genome_path: &std::path::Path,
) -> Result<()> {
    use feagi_evolutionary::{load_genome_from_file, validate_genome};
    
    info!("    [GENOME-LOAD] Step 1: Loading genome file...");
    
    // Load genome from file
    let genome = load_genome_from_file(genome_path)
        .context("Failed to load genome file")?;
    
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
    info!("    [GENOME-LOAD] Genome specifies simulation_timestep: {}s ({:.0}Hz)", 
          simulation_timestep, 1.0 / simulation_timestep);
    
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
        neuro.develop_from_genome(&genome)
            .context("Failed to develop brain from genome")?;
        
        neuro.get_progress()
    };
    
    info!("    [GENOME-LOAD] Neuroembryogenesis complete: {} neurons, {} synapses", 
          result.neurons_created, result.synapses_created);
    
    info!("    [GENOME-LOAD] Step 3: Notifying PNS (triggers dynamic stream evaluation)...");
    pns.on_genome_loaded();
    info!("    [GENOME-LOAD] Step 4: PNS notified, dynamic evaluation complete");
    
    Ok(())
}

