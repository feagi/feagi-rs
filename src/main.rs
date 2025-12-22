//! # FEAGI - Framework for Evolutionary Artificial General Intelligence
//!
//! Full-featured FEAGI server application with REST API, ZMQ streams, and neural processing.
//!
//! ## Features
//! - REST API (HTTP) for brain management and control
//! - ZMQ streams for sensory input and motor output
//! - Real-time neural processing with burst engine
//! - Genome loading and neuroembryogenesis
//! - Agent registration and management
//! - Brain visualization support
//! - Configuration-driven (no hardcoded values)
//!
//! ## License
//! Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, warn, error};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use parking_lot::RwLock;

use feagi_config::{load_config, validate_config, FeagiConfig};
use feagi_brain_development::ConnectomeManager;
use feagi_npu_burst_engine::BurstLoopRunner;
use feagi_npu_burst_engine::backend::GpuConfig;
use feagi_services::*;
use feagi_services::traits::agent_service::AgentService;
use feagi_services::impls::SystemServiceImpl;
use feagi_services::impls::AgentServiceImpl;
use feagi_services::types::LoadGenomeParams;
use feagi_api::transports::http::server::{create_http_server, ApiState};
use feagi_observability::{parse_debug_flags, init_logging_default};
use feagi_io::IOSystem;

/// FEAGI Server - Full-featured neural processing and brain management
#[derive(Parser, Debug)]
#[command(name = "feagi", version, author, about, long_about = None)]
struct Args {
    /// Path to feagi_configuration.toml (searches automatically if not provided)
    #[arg(short = 'f', long)]
    config: Option<PathBuf>,

    /// Path to genome file to load on startup (optional)
    #[arg(short = 'g', long)]
    genome: Option<PathBuf>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Override API port from config
    #[arg(long)]
    api_port: Option<u16>,

    /// Override burst frequency (Hz)
    #[arg(long)]
    burst_hz: Option<u64>,
    
    /// Enable debug logging for specific crates
    /// Example: --debug feagi-api --debug feagi-burst-engine
    /// Or use: --debug-feagi-api --debug-feagi-burst-engine
    /// Use --debug-all to enable debug for all crates
    #[arg(long, action = clap::ArgAction::Append)]
    debug: Vec<String>,
    
    /// Enable debug logging for all crates
    #[arg(long)]
    debug_all: bool,
    
    // Note: --debug-{crate-name} flags are parsed automatically via parse_debug_flags()
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Initialize observability with per-crate debug flags
    // This automatically parses --debug-{crate-name} flags from command line
    // and also checks FEAGI_DEBUG environment variable
    let mut debug_flags = parse_debug_flags();
    
    // Apply --debug-all flag
    if args.debug_all {
        for crate_name in feagi_observability::KNOWN_CRATES {
            debug_flags.enabled_crates.insert(crate_name.to_string(), true);
        }
    }
    
    // Apply --debug {crate-name} values
    for crate_name in &args.debug {
        debug_flags.enabled_crates.insert(crate_name.clone(), true);
    }
    
    // Apply verbose mode (enable debug for all crates)
    if args.verbose {
        for crate_name in feagi_observability::KNOWN_CRATES {
            debug_flags.enabled_crates.insert(crate_name.to_string(), true);
        }
    }
    
    // Initialize logging with file output
    let _log_guard = init_logging_default(&debug_flags)
        .context("Failed to initialize logging")?;
    
    // Log enabled debug crates if any
    if debug_flags.any_enabled() {
        let enabled_crates: Vec<String> = debug_flags.enabled_crates().into_iter().cloned().collect();
        info!("Debug logging enabled for: {}", enabled_crates.join(", "));
    }
    
    info!("Logs are being saved to: {}", _log_guard.log_dir().display());

    // Print banner
    print_banner();

    // Load configuration (REQUIRED - no hardcoded fallbacks)
    info!("Loading FEAGI configuration...");
    let config = load_config(args.config.as_deref(), None)
        .context("Failed to load configuration. Ensure feagi_configuration.toml exists.")?;
    validate_config(&config)
        .context("Configuration validation failed")?;
    
    info!("✓ Configuration loaded and validated");
    log_config_summary(&config);

    // Initialize core components
    info!("Initializing FEAGI core components...");
    let components = initialize_components(&config, &args).await?;
    info!("✓ Core components initialized");

    // Start services
    info!("Starting FEAGI services...");
    start_services(components, &config, &args).await?;

    Ok(())
}

/// Core FEAGI components
struct FeagiComponents {
    #[allow(dead_code)]  // In development - will be exposed via additional services
    npu: Arc<Mutex<feagi_npu_burst_engine::DynamicNPU>>,
    connectome_manager: Arc<RwLock<ConnectomeManager>>,
    runtime_service: Arc<RuntimeServiceImpl>,
    burst_runner: Arc<RwLock<BurstLoopRunner>>,
    pns: Arc<IOSystem>,
}

/// Initialize all core FEAGI components
async fn initialize_components(config: &FeagiConfig, args: &Args) -> Result<FeagiComponents> {
    // Peek at genome to determine quantization precision (if genome provided)
    let precision = if let Some(genome_path) = &args.genome {
        match feagi_evolutionary::peek_quantization_precision(genome_path) {
            Ok(p) => {
                info!("  Genome specifies quantization precision: {}", p);
                p
            },
            Err(e) => {
                warn!("  Failed to peek genome precision ({}), defaulting to int8", e);
                "int8".to_string()
            }
        }
    } else {
        info!("  No genome provided at startup, defaulting to int8 quantization");
        "int8".to_string()
    };
    
    // Initialize NPU with appropriate precision
    info!("  Initializing NPU with {} quantization...", precision.to_uppercase());
    
    // GPU config is available but not yet used in NPU initialization
    let _gpu_config = GpuConfig {
        use_gpu: config.resources.use_gpu,
        hybrid_enabled: config.neural.hybrid.enabled,
        gpu_threshold: config.neural.hybrid.gpu_threshold,
        gpu_memory_fraction: config.resources.gpu_memory_fraction,
    };
    
    // Create NPU based on quantization precision
    use feagi_npu_runtime::StdRuntime;
    use feagi_npu_burst_engine::backend::CPUBackend;
    
    let runtime = StdRuntime;
    let backend = CPUBackend::new();
    
    let npu = Arc::new(Mutex::new(match precision.as_str() {
        "fp32" | "f32" => {
            info!("    Creating FP32 NPU (32-bit floating point, highest precision)");
            feagi_npu_burst_engine::DynamicNPU::F32(feagi_npu_burst_engine::RustNPU::new(
                runtime,
                backend,
                config.connectome.neuron_space,
                config.connectome.synapse_space,
                10, // fire_ledger_window
            )?)
        },
        "int8" => {
            info!("    Creating INT8 NPU (8-bit integer, 42% memory reduction)");
            feagi_npu_burst_engine::DynamicNPU::INT8(feagi_npu_burst_engine::RustNPU::new(
                runtime,
                backend,
                config.connectome.neuron_space,
                config.connectome.synapse_space,
                10, // fire_ledger_window
            )?)
        },
        _ => {
            warn!("    Unknown precision '{}', defaulting to INT8", precision);
            feagi_npu_burst_engine::DynamicNPU::INT8(feagi_npu_burst_engine::RustNPU::new(
                runtime,
                backend,
                config.connectome.neuron_space,
                config.connectome.synapse_space,
                10, // fire_ledger_window
            )?)
        }
    }));
    
    info!("    ✓ NPU initialized with {} precision (capacity: {} neurons, {} synapses)",
          match &*npu.lock().unwrap() {
            feagi_npu_burst_engine::DynamicNPU::F32(_) => "fp32",
            feagi_npu_burst_engine::DynamicNPU::INT8(_) => "int8",
        },
          config.connectome.neuron_space,
          config.connectome.synapse_space);

    // Initialize ConnectomeManager
    info!("  Initializing ConnectomeManager...");
    let manager = ConnectomeManager::instance();  // Already returns Arc<RwLock<>>
    manager.write().set_npu(Arc::clone(&npu));
    info!("    ✓ ConnectomeManager initialized and connected to NPU");

    // NOTE: Genome loading is deferred until after PNS is created and wired
    // This allows dynamic stream gating to work properly

    // Initialize PNS (Peripheral Nervous System - handles agent I/O)
    // MUST be created BEFORE BurstLoopRunner to provide visualization publisher
    info!("  Creating PNS (Agent Management)...");
    
    // Build PNS config from FEAGI config (NO HARDCODED DEFAULTS!)
    use feagi_io::IOConfig;
    
    let mut io_config = IOConfig::default();
    // Override with actual config values
    io_config.zmq_rest_address = format!("tcp://{}:{}", config.agent.host, config.agent.registration_port);
    io_config.zmq_motor_address = format!("tcp://{}:{}", config.zmq.host, config.ports.zmq_motor_port);
    io_config.zmq_viz_address = format!("tcp://{}:{}", config.zmq.host, config.ports.zmq_visualization_port);
    io_config.zmq_sensory_address = format!("tcp://{}:{}", config.zmq.host, config.ports.zmq_sensory_port);
    
    // Load WebSocket configuration from TOML
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
    info!("    ✓ WebSocket config loaded: enabled={}, ports={}/{}/{}/{}", 
        io_config.websocket.enabled,
        io_config.websocket.sensory_port,
        io_config.websocket.motor_port,
        io_config.websocket.visualization_port,
        io_config.websocket.registration_port
    );
    
    let pns = Arc::new(IOSystem::with_config(io_config)
        .context("Failed to create PNS")?);
    
    // Wire dynamic gating callbacks (must be done after Arc wrapping)
    IOSystem::wire_dynamic_gating_callbacks(&pns);
    
    info!("    ✓ PNS created");
    
    // Initialize BurstLoopRunner with PNS-backed publishers
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
    
    // Calculate burst frequency from timestep (seconds → Hz)
    // timestep is in seconds, so frequency = 1 / timestep_seconds
    let burst_hz = 1.0 / burst_timestep;
    
    let burst_runner = Arc::new(RwLock::new(BurstLoopRunner::new(
        Arc::clone(&npu),
        Some(viz_publisher),
        Some(motor_publisher),
        burst_hz,
    )));
    info!("    ✓ BurstLoopRunner initialized ({:.0}Hz, {}s timestep, PNS-backed viz+motor)", burst_hz, burst_timestep);

    // Create runtime service (wraps BurstLoopRunner)
    let runtime_service = Arc::new(RuntimeServiceImpl::new(Arc::clone(&burst_runner)));
    info!("    ✓ Runtime service created");

    // Wire up bidirectional connections between PNS and BurstLoopRunner
    info!("  Wiring PNS ↔ BurstLoopRunner connections...");
    
    // PNS needs sensory manager from BurstLoopRunner (for sensory injection)
    let sensory_mgr = burst_runner.read().sensory_manager.clone();
    pns.set_sensory_agent_manager(sensory_mgr);
    
    // PNS needs burst_runner reference (for motor subscription tracking)
    pns.set_burst_runner(Arc::clone(&burst_runner));
    
    // Wire NPU to PNS for dynamic stream gating
    pns.set_npu_for_gating(Arc::clone(&npu));
    
    info!("    ✓ PNS ↔ BurstLoopRunner connections established");
    info!("      - Sensory: PNS → BurstLoopRunner (injection)");
    info!("      - Motor: BurstLoopRunner → PNS (publishing)");
    info!("      - Dynamic gating: NPU genome state → PNS stream control");

    Ok(FeagiComponents {
        npu,
        connectome_manager: manager,
        runtime_service,
        burst_runner,
        pns,
    })
}

/// Load genome and notify PNS for dynamic gating
/// Returns the genome's simulation_timestep (in seconds) if available
async fn load_genome_with_pns(
    genome_service: &Arc<GenomeServiceImpl>,
    pns: &Arc<IOSystem>,
    genome_path: &PathBuf,
) -> Result<Option<f64>> {
    info!("    [GENOME-LOAD] Step 1: Reading genome file...");
    
    // Read genome file to JSON string
    let json_str = std::fs::read_to_string(genome_path)
        .context("Failed to read genome file")?;
    
    info!("    [GENOME-LOAD] Step 2: Loading genome via GenomeService...");
    
    // Use GenomeService::load_genome which properly stores RuntimeGenome
    let genome_info = genome_service.load_genome(LoadGenomeParams {
        json_str,
    }).await
        .map_err(|e| anyhow::anyhow!("Failed to load genome: {}", e))?;
    
    let simulation_timestep = genome_info.simulation_timestep;
    info!("    [GENOME-LOAD] Genome loaded: {} cortical areas, {}s timestep ({:.0}Hz)", 
          genome_info.cortical_area_count, simulation_timestep, 1.0 / simulation_timestep);
    
    info!("    [GENOME-LOAD] Step 3: Notifying PNS (triggers dynamic stream evaluation)...");
    // Notify PNS that genome is loaded (triggers stream evaluation)
    pns.on_genome_loaded();
    info!("    [GENOME-LOAD] Step 4: PNS notified, dynamic evaluation complete");
    
    Ok(Some(simulation_timestep))
}

/// Start all FEAGI services (API, ZMQ, Burst Engine)
async fn start_services(
    components: FeagiComponents,
    config: &FeagiConfig,
    args: &Args,
) -> Result<()> {
    // Setup signal handler for graceful shutdown using tokio's built-in signal handling
    // This is the recommended way and handles all synchronization correctly
    // Create a shutdown flag
    let shutdown_flag = Arc::new(AtomicBool::new(true));  // Start as true (running)
    
    // Spawn a task to watch for Ctrl+C signal using tokio's async signal handling
    let shutdown_flag_for_task = shutdown_flag.clone();
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                eprintln!("[SHUTDOWN-TASK] Ctrl+C received - setting shutdown flag");
                shutdown_flag_for_task.store(false, Ordering::SeqCst);
                eprintln!("[SHUTDOWN-TASK] Shutdown flag set to false");
            }
            Err(e) => {
                eprintln!("[SHUTDOWN-TASK] Error receiving Ctrl+C signal: {}", e);
            }
        }
    });

    // Create genome service FIRST (needed for genome loading at startup)
    info!("  Creating service layer...");
    
    // Get parameter queue from burst runner for async parameter updates
    let parameter_queue = components.burst_runner.read().parameter_queue.clone();
    
    let genome_service = Arc::new(GenomeServiceImpl::new_with_parameter_queue(
        Arc::clone(&components.connectome_manager),
        parameter_queue,
    ));
    info!("    ✓ Genome service created (with RuntimeGenome storage)");
    
    // Load genome BEFORE starting other services (so RuntimeGenome is stored)
    // This must happen after burst engine is started but before API server
    if let Some(genome_path) = &args.genome {
        info!("  Loading genome from: {}", genome_path.display());
        match load_genome_with_pns(&genome_service, &components.pns, genome_path).await {
            Ok(Some(genome_timestep)) => {
                info!("    ✓ Genome loaded via GenomeService (RuntimeGenome stored)");
                info!("    ✓ Dynamic stream evaluation triggered");
                
                // Update burst frequency to match genome's simulation_timestep
                let new_freq = 1.0 / genome_timestep;
                info!("    ✓ Updating burst frequency from genome: {}Hz ({}s timestep)", 
                      new_freq, genome_timestep);
                components.burst_runner.write().set_frequency(new_freq);
                info!("    ✓ Burst frequency updated successfully");
            }
            Ok(None) => {
                info!("    ✓ Genome loaded (using config burst frequency)");
                info!("    ✓ Dynamic stream evaluation triggered");
            }
            Err(e) => {
                error!("    ✗ Failed to load genome: {}", e);
                error!("    ✗ FEAGI will continue without genome (no data streams will start)");
                error!("    ✗ You can load a genome later via the REST API");
                // Don't fail startup - allow FEAGI to run without genome
            }
        }
    } else {
        info!("  No genome specified, starting with empty connectome");
        info!("    ⚠️  Data streams will not start until genome is loaded");
    }
    
    // Now create remaining services
    let connectome_service = Arc::new(ConnectomeServiceImpl::new(
        Arc::clone(&components.connectome_manager)
    ));
    let analytics_service = Arc::new(AnalyticsServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        Some(Arc::clone(&components.burst_runner)),
    ));
    let neuron_service = Arc::new(NeuronServiceImpl::new(
        Arc::clone(&components.connectome_manager)
    ));
    
    // Collect version information for all crates in this binary
    let version_info = feagi::collect_version_info();
    
    let system_service = Arc::new(SystemServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        Some(Arc::clone(&components.burst_runner)),
        version_info,
    ));
    
    // Get agent registry from PNS for agent service
    let agent_registry = components.pns.get_agent_registry();
    let registration_handler = components.pns.get_registration_handler();
    
    // Wire GenomeService and ConnectomeService to RegistrationHandler (required for auto-creation feature)
    // NOTE: This wiring is REQUIRED for the auto-creation of missing IPU/OPU cortical areas feature.
    // All FEAGI embedders must perform this wiring after creating services.
    {
        let mut handler = registration_handler.lock();
        handler.set_genome_service(Arc::clone(&genome_service) as Arc<dyn feagi_services::traits::GenomeService + Send + Sync>);
        handler.set_connectome_service(Arc::clone(&connectome_service) as Arc<dyn feagi_services::traits::ConnectomeService + Send + Sync>);
        handler.set_auto_create_missing_areas(config.agent.auto_create_missing_cortical_areas);
    }
    info!("    ✓ RegistrationHandler services wired (GenomeService, ConnectomeService, auto-create: {})", config.agent.auto_create_missing_cortical_areas);
    
    let mut agent_service_impl = AgentServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        agent_registry,
    );
    
    // Wire registration handler for full transport negotiation
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
    info!("    ✓ Services created (agent service with transport negotiation)");

    // Create API state (runtime_service already created in components)
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
    };

    // Start PNS control streams FIRST (this wires the dynamic gating callbacks)
    info!("  Starting PNS control streams (agent registration)...");
    components.pns.start_control_streams()
        .context("Failed to start PNS control streams")?;
    info!("    ✓ PNS control streams started (agent registration ready)");

    // Start HTTP API server (before genome load in case it hangs)
    let api_port = args.api_port.unwrap_or(config.api.port);
    let api_host = config.api.host.clone();
    
    info!("  Starting HTTP API server on {}:{}...", api_host, api_port);
    let app = create_http_server(api_state);
    let addr = format!("{}:{}", api_host, api_port);
    
    info!("  API routes registered, binding to {}...", addr);
    
    // Spawn API server in background with graceful shutdown support
    let (shutdown_tx_api, shutdown_rx_api) = tokio::sync::oneshot::channel::<()>();
    let api_handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("Failed to bind API server");
        
        info!("    ✓ HTTP API server listening on {}", addr);
        info!("    📡 Swagger UI available at http://{}/swagger-ui/", addr);
        
        // Use graceful shutdown - when shutdown_rx_api is triggered, server will stop accepting new connections
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                shutdown_rx_api.await.ok();
            })
            .await
            .expect("API server error");
    });

    // Start burst engine via service layer
    info!("  Starting burst engine...");
    components.runtime_service.start().await
        .map_err(|e| anyhow::anyhow!("Failed to start burst engine: {}", e))?;
    info!("    ✓ Burst engine running");

    // Connect NPU to sensory stream for data injection
    info!("  Connecting NPU to PNS sensory stream...");
    components.pns.connect_npu_to_sensory_stream(Arc::clone(&components.npu));
    info!("    ✓ NPU connected to sensory stream");

    // Data streams start DYNAMICALLY based on:
    // 1. Genome loaded (NPU has neurons)
    // 2. At least one agent with matching capability registered
    info!("  ⏸️  Data streams will start automatically when conditions are met:");
    info!("      - Sensory: genome loaded + sensory agent registered");
    info!("      - Motor: genome loaded + motor agent registered");
    info!("      - Visualization: genome loaded + viz agent registered");

    info!("");
    info!("🚀 FEAGI server is running!");
    info!("   REST API: http://{}:{}", config.api.host, api_port);
    info!("   Press Ctrl+C to stop");
    info!("");

    // Wait for shutdown signal using tokio's signal handling (recommended approach)
    info!("Waiting for shutdown signal...");
    
    // Wait loop - check flag with SeqCst ordering
    // Note: We don't log periodically here to avoid noise - only log when shutdown actually happens
    loop {
        let flag_value = shutdown_flag.load(Ordering::SeqCst);
        
        if !flag_value {
            info!("✓ Shutdown signal detected! Initiating graceful shutdown...");
            break;
        }
        
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Graceful shutdown
    info!("Shutting down FEAGI...");
    
    info!("  Stopping burst engine...");
    let stop_result = components.runtime_service.stop().await;
    match stop_result {
        Ok(_) => info!("    ✓ Burst engine stopped"),
        Err(e) => {
            error!("    ✗ Failed to stop burst engine: {}", e);
            return Err(anyhow::anyhow!("Failed to stop burst engine: {}", e));
        }
    }
    
    info!("  Stopping PNS (agent I/O)...");
    match components.pns.stop() {
        Ok(_) => info!("    ✓ PNS stopped"),
        Err(e) => {
            error!("    ✗ Failed to stop PNS: {}", e);
        }
    }

    info!("  Stopping API server...");
    // Trigger graceful shutdown for axum server
    let _ = shutdown_tx_api.send(());
    
    // Wait for server to finish, with a timeout
    match tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        api_handle
    ).await {
        Ok(Ok(_)) => {
            info!("    ✓ API server stopped cleanly");
        }
        Ok(Err(e)) => {
            warn!("    ⚠️ API server error during shutdown: {}", e);
        }
        Err(_) => {
            warn!("    ⚠️ API server shutdown timed out after 5 seconds, proceeding anyway");
        }
    }

    info!("✅ FEAGI shutdown complete");
    info!("Exiting process...");
    // Force exit to ensure process terminates (kills all threads immediately)
    std::process::exit(0);
}

/// Log configuration summary
fn log_config_summary(config: &FeagiConfig) {
    info!("Configuration Summary:");
    info!("  API: {}:{}", config.api.host, config.api.port);
    info!("  ZMQ Host: {}", config.zmq.host);
    info!("  Ports:");
    info!("    - Sensory: {}", config.ports.zmq_sensory_port);
    info!("    - Motor: {}", config.ports.zmq_motor_port);
    info!("    - Visualization: {}", config.ports.zmq_visualization_port);
    info!("  Neural:");
    info!("    - Burst timestep: {}ms", config.neural.burst_engine_timestep);
    info!("    - Batch size: {}", config.neural.batch_size);
    info!("  Resources:");
    info!("    - GPU enabled: {}", config.resources.use_gpu);
    info!("    - Max neurons: {}", config.connectome.neuron_space);
}

/// Print FEAGI banner
fn print_banner() {
    println!(r#"
╔═══════════════════════════════════════════════════════════════════╗
║                                                                   ║
║   ███████╗███████╗ █████╗  ██████╗ ██╗                          ║
║   ██╔════╝██╔════╝██╔══██╗██╔════╝ ██║                          ║
║   █████╗  █████╗  ███████║██║  ███╗██║                          ║
║   ██╔══╝  ██╔══╝  ██╔══██║██║   ██║██║                          ║
║   ██║     ███████╗██║  ██║╚██████╔╝██║                          ║
║   ╚═╝     ╚══════╝╚═╝  ╚═╝ ╚═════╝ ╚═╝                          ║
║                                                                   ║
║   Framework for Evolutionary Artificial General Intelligence     ║
║   Version 2.0.0 - Apache-2.0 License                            ║
║   Copyright 2016-2025 Neuraville Inc.                           ║
║                                                                   ║
╚═══════════════════════════════════════════════════════════════════╝
"#);
}

