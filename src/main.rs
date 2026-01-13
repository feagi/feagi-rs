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
use feagi_npu_burst_engine::{BurstLoopRunner, TracingMutex};
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

    /// Override visualization transport policy (authoritative, overrides TOML).
    ///
    /// Allowed values:
    /// - auto: honor agent request (chosen_transport / shm_path)
    /// - websocket: disable SHM path allocation/advertising for visualization
    /// - shm: enable SHM path allocation/advertising for visualization
    #[arg(long, value_parser = ["auto", "websocket", "shm"])]
    viz_transport: Option<String>,

    /// Override NPU quantization precision (bypasses genome peek).
    ///
    /// Supported values:
    /// - fp32
    /// - int8
    ///
    /// Example:
    ///   --precision fp32
    #[arg(long, value_parser = ["fp32", "int8"])]
    precision: Option<String>,
    
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

    /// Enable NPU trace logging (synapse + dynamics) via a single switch.
    /// This enables the `feagi-npu-trace` tracing target at DEBUG level and turns on the
    /// internal NPU trace emitters (power excluded).
    ///
    /// Optional filters:
    /// - --npu-trace-src <NEURON_ID>
    /// - --npu-trace-dst <NEURON_ID>
    /// - --npu-trace-neuron <NEURON_ID>
    #[arg(long)]
    npu_trace: bool,

    /// Enable only synapse contribution traces (power excluded).
    #[arg(long)]
    npu_trace_synapse: bool,

    /// Enable only neural dynamics traces (power excluded).
    #[arg(long)]
    npu_trace_dynamics: bool,

    /// Filter synapse traces to a single source neuron id.
    #[arg(long)]
    npu_trace_src: Option<u32>,

    /// Filter synapse traces to a single destination neuron id.
    #[arg(long)]
    npu_trace_dst: Option<u32>,

    /// Filter dynamics traces to a single neuron id.
    #[arg(long)]
    npu_trace_neuron: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Configure NPU tracing BEFORE logging initialization (trace config is cached via OnceLock)
    let enable_any_trace = args.npu_trace || args.npu_trace_synapse || args.npu_trace_dynamics;
    if enable_any_trace {
        // Gate emitters
        if args.npu_trace || args.npu_trace_synapse {
            std::env::set_var("FEAGI_NPU_TRACE_SYNAPSE", "1");
        }
        if args.npu_trace || args.npu_trace_dynamics {
            std::env::set_var("FEAGI_NPU_TRACE_DYNAMICS", "1");
        }
        if let Some(src) = args.npu_trace_src {
            std::env::set_var("FEAGI_NPU_TRACE_SRC", src.to_string());
        }
        if let Some(dst) = args.npu_trace_dst {
            std::env::set_var("FEAGI_NPU_TRACE_DST", dst.to_string());
        }
        if let Some(n) = args.npu_trace_neuron {
            std::env::set_var("FEAGI_NPU_TRACE_NEURON", n.to_string());
        }
    }

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
    
    // If NPU tracing was requested, ensure the trace target is visible.
    // Note: this is a tracing target name, but EnvFilter can match it the same way as a crate/module path.
    if enable_any_trace {
        debug_flags
            .enabled_crates
            .insert("feagi-npu-trace".to_string(), true);
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
    let mut config = load_config(args.config.as_deref(), None)
        .context("Failed to load configuration. Ensure feagi_configuration.toml exists.")?;

    // Apply CLI overrides that must be validated as part of configuration correctness.
    if let Some(ref policy) = args.viz_transport {
        config.visualization.transport = policy.clone();
        info!(
            "Applied CLI override: visualization.transport = '{}'",
            config.visualization.transport
        );
    }

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
    npu: Arc<feagi_npu_burst_engine::TracingMutex<feagi_npu_burst_engine::DynamicNPU>>,
    connectome_manager: Arc<RwLock<ConnectomeManager>>,
    runtime_service: Arc<RuntimeServiceImpl>,
    burst_runner: Arc<RwLock<BurstLoopRunner>>,
    pns: Arc<IOSystem>,
    #[cfg(feature = "plasticity")]
    plasticity_executor: Option<Arc<std::sync::Mutex<feagi_npu_plasticity::AsyncPlasticityExecutor>>>,
    #[cfg(feature = "plasticity")]
    memory_stats_cache: Option<feagi_npu_plasticity::MemoryStatsCache>,
    #[cfg(not(feature = "plasticity"))]
    plasticity_executor: Option<()>,
    #[cfg(not(feature = "plasticity"))]
    memory_stats_cache: Option<()>,
}

/// Initialize all core FEAGI components
async fn initialize_components(config: &FeagiConfig, args: &Args) -> Result<FeagiComponents> {
    // Determine quantization precision:
    // - If CLI override is present, use it.
    // - Else, peek genome if provided.
    // - Else, use int8 (existing behavior).
    let precision = if let Some(p) = &args.precision {
        info!("  Precision override from CLI: {}", p);
        p.clone()
    } else if let Some(genome_path) = &args.genome {
        match feagi_evolutionary::peek_quantization_precision(genome_path) {
            Ok(p) => {
                info!("  Genome specifies quantization precision: {}", p);
                p
            }
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
    
    // Wrap NPU in TracingMutex (or Mutex if tracing disabled) to automatically log all lock acquisitions
    // When npu-lock-tracing feature is disabled, TracingMutex is a type alias for std::sync::Mutex (zero overhead)
    let npu = Arc::new(TracingMutex::new(match precision.as_str() {
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
    }, "NPU"));
    
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

    // Initialize plasticity executor (if plasticity feature enabled)
    #[cfg(feature = "plasticity")]
    let (plasticity_executor, memory_stats_cache) = {
        use feagi_npu_plasticity::{create_memory_stats_cache, PlasticityConfig, AsyncPlasticityExecutor, PlasticityExecutor};
        use std::sync::Mutex;

        info!("╔═══════════════════════════════════════════════════════════════╗");
        info!("║  PLASTICITY SUBSYSTEM INITIALIZATION                          ║");
        info!("╚═══════════════════════════════════════════════════════════════╝");
        info!("  📊 Creating memory stats cache...");
        let cache = create_memory_stats_cache();
        let config = PlasticityConfig::default();
        
        info!("  🧠 Creating AsyncPlasticityExecutor with NPU reference...");
        // Create executor with NPU reference (for querying CPU-resident FireLedger)
        let executor = Arc::new(Mutex::new(
            AsyncPlasticityExecutor::new(config, cache.clone(), Arc::clone(&npu))
        ));
        
        info!("  🚀 Starting PlasticityService background thread...");
        // Start the plasticity service thread
        {
            let mut exec = executor.lock().unwrap();
            PlasticityExecutor::start(&mut *exec);
        }
        
        info!("  🔗 Wiring PlasticityExecutor into ConnectomeManager...");
        // Wire plasticity executor into ConnectomeManager for automatic memory area registration
        ConnectomeManager::instance().write().set_plasticity_executor(Arc::clone(&executor));
        
        info!("  🔗 Wiring PlasticityExecutor into BurstLoopRunner...");
        // Wire plasticity executor notification callback into BurstLoopRunner
        // This avoids circular dependency: burst-engine depends on plasticity types,
        // but feagi-rs can safely depend on both and wire them via callback
        let executor_for_callback = Arc::clone(&executor);
        burst_runner.write().set_plasticity_notify_callback(move |timestep: u64| {
            if let Ok(exec) = executor_for_callback.lock() {
                use feagi_npu_plasticity::PlasticityExecutor;
                exec.notify_burst(timestep);
            }
        });
        
        info!("╔═══════════════════════════════════════════════════════════════╗");
        info!("║  ✅ PLASTICITY SUBSYSTEM READY                                ║");
        info!("║     • Memory neuron pattern detection: ENABLED                ║");
        info!("║     • STDP synaptic plasticity: ENABLED                       ║");
        info!("║     • Background processing thread: ACTIVE                    ║");
        info!("╚═══════════════════════════════════════════════════════════════╝");
        (Some(executor), Some(cache))
    };

    #[cfg(not(feature = "plasticity"))]
    let (plasticity_executor, memory_stats_cache): (Option<()>, Option<()>) = {
        info!("  ℹ️  Plasticity feature disabled (compiled without --features plasticity)");
        (None, None)
    };

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
        plasticity_executor,
        memory_stats_cache,
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
    
    // Create GenomeServiceImpl and get reference to current_genome for sharing with ConnectomeService
    let mut genome_service_impl = GenomeServiceImpl::new_with_parameter_queue(
        Arc::clone(&components.connectome_manager),
        parameter_queue,
    );
    // Wire burst runner for cache refresh
    genome_service_impl.set_burst_runner(Arc::clone(&components.burst_runner));
    let genome_service_impl = Arc::new(genome_service_impl);
    let current_genome = genome_service_impl.get_current_genome_arc();
    let genome_service = genome_service_impl;
    info!("    ✓ Genome service created (with RuntimeGenome storage)");
    
    // Now create remaining services (share current_genome with ConnectomeService for mapping persistence)
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
        // Visualization transport is driven by feagi_configuration.toml (authoritative).
        // This controls whether FEAGI allocates/advertises SHM visualization paths during registration.
        {
            use feagi_io::core::registration::VisualizationShmPolicy;
            let policy = match config.visualization.transport.as_str() {
                "auto" => VisualizationShmPolicy::Auto,
                "websocket" => VisualizationShmPolicy::ForceWebSocket,
                "shm" => VisualizationShmPolicy::ForceShm,
                other => {
                    // Validation should have rejected this already, but keep behavior deterministic.
                    return Err(anyhow::anyhow!(
                        "Invalid config value visualization.transport='{}' (expected auto/websocket/shm)",
                        other
                    ));
                }
            };
            handler.set_visualization_shm_policy(policy);
        }
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
        genome_service: genome_service.clone() as Arc<dyn GenomeService + Send + Sync>,
        connectome_service: connectome_service as Arc<dyn ConnectomeService + Send + Sync>,
        analytics_service: analytics_service as Arc<dyn AnalyticsService + Send + Sync>,
        runtime_service: components.runtime_service.clone() as Arc<dyn RuntimeService + Send + Sync>,
        neuron_service: neuron_service as Arc<dyn NeuronService + Send + Sync>,
        system_service: system_service as Arc<dyn feagi_services::traits::SystemService + Send + Sync>,
        snapshot_service: Some(snapshot_service as Arc<dyn feagi_services::SnapshotService + Send + Sync>),
        feagi_session_timestamp,
        memory_stats_cache: components.memory_stats_cache.clone(),
        agent_connectors: ApiState::init_agent_connectors(),
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

    // IMPORTANT:
    // Do NOT start the burst engine before genome load completes.
    //
    // Rationale:
    // - Genome load performs neuroembryogenesis/synaptogenesis and mutates ConnectomeManager + NPU.
    // - Running bursts concurrently with connectome mutation is a correctness and determinism risk.
    //
    // The burst engine, plasticity executor, and NPU↔sensory wiring are started AFTER genome load below.

    // Load genome AFTER the HTTP API is online.
    //
    // Rationale:
    // - NIFTI-scale genomes can take a long time to load (neuroembryogenesis/synaptogenesis).
    // - BV's startup health probe requires the API server to be listening.
    // - Starting the API first improves observability and avoids "API never came online" false negatives.
    //
    // Determinism:
    // - If --genome is provided and loading fails, FEAGI exits (same behavior as before).
    if let Some(genome_path) = &args.genome {
        info!("  Loading genome from: {} (API is already online)", genome_path.display());
        match load_genome_with_pns(&genome_service, &components.pns, genome_path).await {
            Ok(Some(genome_timestep)) => {
                info!("    ✓ Genome loaded via GenomeService (RuntimeGenome stored)");
                info!("    ✓ Dynamic stream evaluation triggered");

                // Update burst frequency to match genome's simulation_timestep
                let new_freq = 1.0 / genome_timestep;
                info!(
                    "    ✓ Updating burst frequency from genome: {}Hz ({}s timestep)",
                    new_freq, genome_timestep
                );
                components.burst_runner.write().set_frequency(new_freq);
                info!("    ✓ Burst frequency updated successfully");
            }
            Ok(None) => {
                info!("    ✓ Genome loaded (using config burst frequency)");
                info!("    ✓ Dynamic stream evaluation triggered");
            }
            Err(e) => {
                error!("    ✗ Failed to load genome: {}", e);
                error!("    ✗ --genome was provided, so FEAGI will exit");
                // Best-effort graceful shutdown of the API server task before returning.
                let _ = shutdown_tx_api.send(());
                let _ = api_handle.await;
                return Err(e);
            }
        }
    } else {
        info!("  No genome specified, starting with empty connectome");
        info!("    ⚠️  Data streams will not start until genome is loaded");
    }

    // Start burst engine via service layer (safe after genome load / connectome reset completes)
    info!("  Starting burst engine...");
    components
        .runtime_service
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to start burst engine: {}", e))?;
    info!("    ✓ Burst engine running");

    // Start plasticity executor and command processing loop (if enabled)
    #[cfg(feature = "plasticity")]
    if let Some(ref plasticity_exec) = components.plasticity_executor {
        use feagi_npu_plasticity::PlasticityExecutor;

        info!("  Starting plasticity executor...");
        plasticity_exec.lock().unwrap().start();
        info!("    ✓ Plasticity executor running");

        // Spawn plasticity command processing loop
        // This loop reads commands from the PlasticityService and executes them on the NPU
        let npu_for_plasticity = Arc::clone(&components.npu);
        let plasticity_for_loop = Arc::clone(plasticity_exec);
        let burst_runner_for_plasticity = Arc::clone(&components.burst_runner);

        std::thread::Builder::new()
            .name("feagi-plasticity-cmd-processor".to_string())
            .spawn(move || {
                use feagi_npu_plasticity::{PlasticityCommand, PlasticityExecutor};
                use tracing::{debug, info, warn};

                info!("[PLASTICITY-CMD] Command processor thread started");

                // BurstLoopRunner already notifies plasticity exactly once per completed burst
                // via `set_plasticity_notify_callback()`. This thread must NOT call `notify_burst()`
                // again, otherwise neurons get aged/pruned multiple times per burst and indices
                // get rapidly reused (appearing as "memory count barely increases").
                let mut last_seen_burst: u64 = 0;

                loop {
                    // Wait for burst to complete (check every 10ms)
                    std::thread::sleep(std::time::Duration::from_millis(10));

                    // Track burst progress (draining can happen multiple times per burst; notify must not)
                    let current_burst = burst_runner_for_plasticity.read().get_burst_count();
                    if current_burst != last_seen_burst {
                        last_seen_burst = current_burst;
                    }

                    // Drain and process commands
                    let commands = plasticity_for_loop.lock().unwrap().drain_commands();

                    if !commands.is_empty() {
                        debug!("[PLASTICITY-CMD] 🎯 Processing {} commands", commands.len());
                        let mut npu_lock = npu_for_plasticity.lock().unwrap();

                        for cmd in commands {
                            match cmd {
                                PlasticityCommand::RegisterMemoryNeuron {
                                    neuron_id,
                                    area_idx: _,
                                    threshold: _,
                                    membrane_potential: _,
                                } => {
                                    debug!(
                                        "[PLASTICITY-CMD] 📝 Registering memory neuron: neuron_id={}",
                                        neuron_id
                                    );
                                    // Memory neurons are already created by the plasticity service
                                    // This command serves as a notification
                                }
                                PlasticityCommand::InjectMemoryNeuronToFCL {
                                    neuron_id,
                                    area_idx,
                                    membrane_potential,
                                    pattern_hash,
                                    is_reactivation: _,
                                } => {
                                    debug!(
                                        "[PLASTICITY-CMD] 💉 Injecting memory neuron to FCL: neuron_id={}, area_idx={}, potential={}, pattern_hash={}",
                                        neuron_id, area_idx, membrane_potential, pattern_hash
                                    );

                                    // Get cortical ID from ConnectomeManager (required for propagation engine mapping).
                                    let cortical_id_opt = {
                                        let instance = ConnectomeManager::instance();
                                        let cm = instance.read();
                                        cm.get_cortical_id(area_idx).cloned()
                                    };

                                    if let Some(cortical_id) = cortical_id_opt {
                                        // Register mapping so synaptic propagation can resolve the cortical area for this ID.
                                        npu_lock.register_dynamic_neuron_mapping(neuron_id, cortical_id);

                                        // Stage injection to next burst’s FCL using the *actual* memory neuron ID.
                                        npu_lock.inject_memory_neuron_to_fcl(
                                            neuron_id,
                                            area_idx,
                                            membrane_potential,
                                        );

                                        debug!(
                                            "[PLASTICITY-CMD] ✅ Memory neuron staged to FCL (id={}, area_idx={}, pattern={}, will fire next burst)",
                                            neuron_id, area_idx, pattern_hash
                                        );
                                    } else {
                                        warn!(
                                            "[PLASTICITY-CMD] ⚠️ Could not find cortical ID for area_idx={}",
                                            area_idx
                                        );
                                    }
                                }
                                PlasticityCommand::UpdateWeightsDelta {
                                    synapse_indices: _,
                                    deltas: _,
                                } => {
                                    // TODO: Implement STDP weight updates
                                    warn!("[PLASTICITY-CMD] STDP weight updates not yet implemented");
                                }
                                PlasticityCommand::UpdateStateCounters { .. } => {
                                    // Stats tracking only, no NPU action needed
                                }
                            }
                        }
                    }
                }
            })
            .expect("Failed to spawn plasticity command processor thread");

        info!("    ✓ Plasticity command processor running");
    }

    // Connect NPU to sensory stream for data injection
    info!("  Connecting NPU to PNS sensory stream...");
    components
        .pns
        .connect_npu_to_sensory_stream(Arc::clone(&components.npu));
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

