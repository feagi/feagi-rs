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
use feagi_bdu::ConnectomeManager;
use feagi_burst_engine::{RustNPU, BurstLoopRunner};
use feagi_services::*;
use feagi_services::traits::agent_service::AgentService;
use feagi_services::impls::AgentServiceImpl;
use feagi_api::transports::http::server::{create_http_server, ApiState};
use feagi_observability::{parse_debug_flags, init_logging_default};
use feagi_pns::PNS;

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
    npu: Arc<Mutex<RustNPU>>,
    connectome_manager: Arc<RwLock<ConnectomeManager>>,
    runtime_service: Arc<RuntimeServiceImpl>,
    burst_runner: Arc<RwLock<BurstLoopRunner>>,
    pns: Arc<PNS>,
}

/// Initialize all core FEAGI components
async fn initialize_components(config: &FeagiConfig, args: &Args) -> Result<FeagiComponents> {
    // Initialize NPU
    info!("  Initializing NPU...");
    let npu = Arc::new(Mutex::new(RustNPU::new(
        config.connectome.min_neuron_space,
        config.connectome.min_synapse_space,
        10, // cortical_area_count - will be resized as needed
    )));
    info!("    ✓ NPU initialized (capacity: {} neurons, {} synapses)",
          config.connectome.min_neuron_space,
          config.connectome.min_synapse_space);

    // Initialize ConnectomeManager
    info!("  Initializing ConnectomeManager...");
    let manager = ConnectomeManager::instance();  // Already returns Arc<RwLock<>>
    manager.write().set_npu(Arc::clone(&npu));
    info!("    ✓ ConnectomeManager initialized and connected to NPU");

    // Load genome if provided
    if let Some(genome_path) = &args.genome {
        info!("  Loading genome from: {}", genome_path.display());
        load_genome(&manager, genome_path).await?;
        info!("    ✓ Genome loaded and brain developed");
    } else {
        info!("  No genome specified, starting with empty connectome");
    }

    // Initialize BurstLoopRunner (for internal use by runtime service)
    info!("  Initializing BurstLoopRunner...");
    let burst_timestep = config.neural.burst_engine_timestep;
    
    // Create a no-op visualization publisher
    struct NoOpPublisher;
    impl feagi_burst_engine::VisualizationPublisher for NoOpPublisher {
        fn publish_visualization(&self, _data: &[u8]) -> Result<(), String> {
            Ok(()) // No-op: don't publish anything
        }
    }
    
    let no_op_publisher = Arc::new(Mutex::new(NoOpPublisher));
    let burst_runner = Arc::new(RwLock::new(BurstLoopRunner::new(
        Arc::clone(&npu),
        Some(no_op_publisher),
        burst_timestep,
    )));
    let burst_hz = (1000.0 / burst_timestep) as u64;
    info!("    ✓ BurstLoopRunner initialized ({}Hz, {}ms timestep)", burst_hz, burst_timestep);

    // Create runtime service (wraps BurstLoopRunner)
    let runtime_service = Arc::new(RuntimeServiceImpl::new(Arc::clone(&burst_runner)));
    info!("    ✓ Runtime service created");
    
    // Initialize PNS (Peripheral Nervous System - handles agent I/O)
    info!("  Creating PNS (Agent Management)...");
    let pns = Arc::new(PNS::new()
        .context("Failed to create PNS")?);
    info!("    ✓ PNS created");

    Ok(FeagiComponents {
        npu,
        connectome_manager: manager,
        runtime_service,
        burst_runner,
        pns,
    })
}

/// Load and develop a genome
async fn load_genome(
    manager: &Arc<RwLock<ConnectomeManager>>,
    genome_path: &PathBuf,
) -> Result<()> {
    use feagi_evo::{load_genome_from_file, validate_genome};
    
    // Load genome from file
    let genome = load_genome_from_file(genome_path)
        .context("Failed to load genome file")?;
    
    // Validate genome
    let validation = validate_genome(&genome);
    if !validation.errors.is_empty() {
        error!("Genome validation errors:");
        for error in &validation.errors {
            error!("  - {}", error);
        }
        return Err(anyhow::anyhow!("Genome validation failed"));
    }
    
    if !validation.warnings.is_empty() {
        warn!("Genome validation warnings:");
        for warning in &validation.warnings {
            warn!("  - {}", warning);
        }
    }
    
    // Load genome into connectome (includes neuroembryogenesis)
    manager.write().load_from_genome(genome)
        .context("Failed to load genome into connectome")?;
    
    Ok(())
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

    // Create remaining services
    info!("  Creating service layer...");
    let genome_service = Arc::new(GenomeServiceImpl::new(
        Arc::clone(&components.connectome_manager)
    ));
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
    
    // Get agent registry from PNS for agent service
    let agent_registry = components.pns.get_agent_registry();
    let agent_service = Arc::new(AgentServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        agent_registry,
    ));
    info!("    ✓ Services created");

    // Create API state (runtime_service already created in components)
    // Create snapshot service
    let snapshot_dir = std::path::PathBuf::from("./snapshots");
    let snapshot_service = Arc::new(feagi_services::SnapshotServiceImpl::new(snapshot_dir));
    
    let api_state = ApiState {
        agent_service: Some(agent_service as Arc<dyn AgentService + Send + Sync>),
        genome_service: genome_service as Arc<dyn GenomeService + Send + Sync>,
        connectome_service: connectome_service as Arc<dyn ConnectomeService + Send + Sync>,
        analytics_service: analytics_service as Arc<dyn AnalyticsService + Send + Sync>,
        runtime_service: components.runtime_service.clone() as Arc<dyn RuntimeService + Send + Sync>,
        neuron_service: neuron_service as Arc<dyn NeuronService + Send + Sync>,
        snapshot_service: Some(snapshot_service as Arc<dyn feagi_services::SnapshotService + Send + Sync>),
    };

    // Start HTTP API server
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

    // TODO: Start ZMQ streams (PNS)
    // This will be wired up once feagi-pns integration is complete
    info!("  ZMQ streams: Not yet implemented");

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
    info!("    - Max neurons: {}", config.connectome.min_neuron_space);
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

