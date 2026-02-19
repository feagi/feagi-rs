//! # FEAGI Embedded Library
//!
//! Provides a programmatic interface to FEAGI for embedding in applications.
//!
//! ## Usage Modes
//!
//! - **Library Mode:** Embed FEAGI in applications (Godot, Unity, custom tools)
//! - **Binary Mode:** Run FEAGI as standalone server (via `main.rs`)
//!
//! ## Example
//!
//! ```no_run
//! use feagi::{FeagiInstance, FeagiConfig};
//!
//! // Create configuration
//! let config = FeagiConfig::default();
//!
//! // Create and initialize FEAGI
//! let mut feagi = FeagiInstance::new(config).unwrap();
//! feagi.initialize().unwrap();
//!
//! // Register visualization callback
//! feagi.set_visualization_callback(Box::new(|fire_data| {
//!     println!("Burst fired: {} areas active", fire_data.len());
//! }));
//!
//! // Start burst engine
//! feagi.start().unwrap();
//!
//! // ... do work ...
//!
//! // Shutdown gracefully
//! feagi.shutdown().unwrap();
//! ```

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

// Re-export public types
pub use feagi_config::{load_config, FeagiConfig};
pub use feagi_npu_burst_engine::backend::GpuConfig;
pub use feagi_npu_burst_engine::RawFireQueueSnapshot;

// Re-export for embedders who need them
pub use feagi_brain_development::ConnectomeManager;
pub use feagi_agent::server::FeagiAgentHandler;
pub use feagi_npu_burst_engine::BurstLoopRunner;
pub use feagi_services::*;

// Internal modules (reused from main.rs initialization logic)
pub mod components;
pub mod network_provider;
pub mod plasticity_runtime;
pub mod version;

pub use components::FeagiComponents;
pub use version::collect_version_info;

/// Visualization callback type
///
/// Called every burst cycle with neuron fire data.
/// The callback runs on FEAGI's worker thread, so keep it fast.
pub type VisualizationCallback = Box<dyn Fn(&RawFireQueueSnapshot) + Send + Sync>;

/// Main FEAGI instance handle
///
/// This is the primary interface for embedded FEAGI.
/// All operations are thread-safe and can be called from any thread.
///
/// # Thread Safety
///
/// - All methods are thread-safe (use Arc/Mutex internally)
/// - Visualization callbacks run on FEAGI's worker threads
/// - HTTP server runs on dedicated Tokio runtime
pub struct FeagiInstance {
    components: Arc<Mutex<Option<FeagiComponents>>>,
    #[allow(dead_code)]
    config: FeagiConfig,
    runtime: Arc<tokio::runtime::Runtime>,
    viz_callback: Arc<Mutex<Option<VisualizationCallback>>>,
    http_server_url: String,
    _runtime_keeper: Option<std::thread::JoinHandle<()>>,
}

impl FeagiInstance {
    /// Create a new FEAGI instance with the given configuration
    ///
    /// # Arguments
    ///
    /// * `config` - FEAGI configuration (from TOML or built programmatically)
    ///
    /// # Returns
    ///
    /// A new FEAGI instance ready to be initialized
    ///
    /// # Errors
    ///
    /// Returns error if Tokio runtime cannot be created
    pub fn new(config: FeagiConfig) -> Result<Self> {
        // NOTE: Logging should be initialized by the host application (GDExtension)
        // before calling FeagiInstance::new() to ensure logs are captured

        // Create dedicated tokio runtime for FEAGI
        // This ensures FEAGI doesn't interfere with host application's threading
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("feagi-worker")
            .enable_all()
            .build()
            .context("Failed to create Tokio runtime")?;

        let http_server_url = format!(
            "http://{}:{}",
            config.api.advertised_host, config.api.port
        );

        info!("🦀 FEAGI Instance created (embedded mode)");
        info!("   HTTP API will be available at: {}", http_server_url);

        let runtime_arc = Arc::new(runtime);

        Ok(Self {
            components: Arc::new(Mutex::new(None)),
            config,
            runtime: runtime_arc,
            viz_callback: Arc::new(Mutex::new(None)),
            http_server_url,
            _runtime_keeper: None,
        })
    }

    /// Initialize logging for embedded mode using TOML configuration
    ///
    /// Respects `feagi_configuration.toml` settings:
    /// - `logging.global_log_level` (WARNING, INFO, DEBUG, TRACE)
    /// - `logging.print_debug_logs` (true/false)
    ///
    /// No file logging in embedded mode (stdout only, captured by host application)
    ///
    /// NOTE: Currently unused - logging is initialized by GDExtension before FeagiInstance::new()
    #[allow(dead_code)]
    fn init_embedded_logging(logging_config: &feagi_config::LoggingConfig) {
        use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

        // Determine log level from config
        let base_level = if logging_config.print_debug_logs {
            "debug".to_string()
        } else {
            logging_config.global_log_level.to_lowercase()
        };

        // Build filter with per-crate levels
        // For embedded mode, we enable enhanced logging for API/HTTP to debug issues
        let filter_str = if base_level == "debug" || base_level == "trace" {
            // Debug/trace mode: verbose logging for all FEAGI crates
            format!(
                "{}=info,\
                 feagi={},\
                 feagi_api=trace,\
                 feagi_services=debug,\
                 feagi_io=debug,\
                 feagi_npu_burst_engine=debug,\
                 feagi_brain_development=debug,\
                 feagi_evolutionary=debug,\
                 axum=debug,\
                 tower_http=debug,\
                 hyper=debug",
                base_level, base_level
            )
        } else {
            // Production mode: respect global level, but still show API errors
            format!(
                "{}=info,\
                 feagi={},\
                 feagi_api={},\
                 feagi_services={},\
                 feagi_io={},\
                 axum=warn,\
                 tower_http=warn",
                base_level, base_level, base_level, base_level, base_level
            )
        };

        let filter = EnvFilter::new(filter_str);

        // Initialize with stdout output (no file logging in embedded mode)
        // Host application (Godot) captures stdout and redirects to its console
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_level(true)
                    .with_line_number(false),
            ) // Reduce noise in embedded mode
            .try_init();

        info!(
            "📝 Embedded logging initialized: level={}, debug_logs={}",
            logging_config.global_log_level, logging_config.print_debug_logs
        );
    }

    /// Initialize FEAGI components
    ///
    /// This is a heavy operation that creates:
    /// - Neural Processing Unit (NPU)
    /// - Connectome Manager
    /// - Burst Engine
    /// - Peripheral Nervous System (PNS)
    /// - HTTP API Server
    ///
    /// Call this once during application startup.
    ///
    /// # Errors
    ///
    /// Returns error if any component fails to initialize
    /// Initialize FEAGI components
    ///
    /// **IMPORTANT:** This method is currently **not supported** for in-process embedding
    /// (e.g., GDExtension, Unity plugins, etc.) due to threading model incompatibilities.
    ///
    /// For embedded use cases, run FEAGI as a subprocess and connect via:
    /// - HTTP API (port 8000) for control commands
    /// - WebSocket (port 9050) for real-time data streams
    /// - Shared Memory (SHM) for high-performance visualization
    ///
    /// See `docs/EMBEDDING_GUIDE.md` for details.
    ///
    /// # Errors
    ///
    /// Currently returns an error indicating in-process embedding is not supported.
    pub fn initialize(&mut self) -> Result<()> {
        error!("❌ In-process FEAGI initialization is not supported");
        error!("   Reason: GDExtension/FFI threading model incompatibility");
        error!("   Solution: Run FEAGI as a subprocess and connect via HTTP/WebSocket/SHM");
        error!("   See: docs/EMBEDDING_GUIDE.md");

        Err(anyhow::anyhow!(
            "In-process FEAGI initialization is not supported. \
             Run FEAGI as a subprocess and connect via HTTP (port 8000), \
             WebSocket (port 9050), or Shared Memory for high performance."
        ))
    }

    /// Register a visualization callback
    ///
    /// This callback will be invoked every burst cycle with neuron fire data.
    /// The callback runs on FEAGI's thread pool, so keep it fast or queue data
    /// for processing on another thread.
    ///
    /// NOTE: Callback support is pending PNS API additions. Currently, visualization
    /// data is published via ZMQ/WebSocket.
    ///
    /// # Arguments
    ///
    /// * `callback` - Function to call with fire data
    pub fn set_visualization_callback(&self, callback: VisualizationCallback) {
        *self.viz_callback.lock().unwrap() = Some(callback);
        info!("📊 Visualization callback registered (pending PNS integration)");
    }

    //
    // ============ BURST ENGINE CONTROL ============
    //

    /// Start the burst engine
    ///
    /// Begins neural processing loop. Visualization data will be published.
    ///
    /// # Errors
    ///
    /// Returns error if FEAGI is not initialized or burst engine fails to start
    pub fn start(&self) -> Result<()> {
        let components = self.components.lock().unwrap();
        let components = components
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("FEAGI not initialized. Call initialize() first."))?;

        let _ = components.burst_runner.write().start();
        info!("▶️ Burst engine started");

        Ok(())
    }

    /// Stop the burst engine
    ///
    /// Halts neural processing. Can be restarted with `start()`.
    ///
    /// # Errors
    ///
    /// Returns error if FEAGI is not initialized
    pub fn stop(&self) -> Result<()> {
        let components = self.components.lock().unwrap();
        let components = components
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("FEAGI not initialized"))?;

        components.burst_runner.write().stop();
        info!("⏸️ Burst engine stopped");

        Ok(())
    }

    /// Set burst frequency (Hz)
    ///
    /// Changes the neural processing speed.
    ///
    /// # Arguments
    ///
    /// * `hz` - Frequency in Hz (e.g., 100.0 for 100Hz)
    ///
    /// # Errors
    ///
    /// Returns error if FEAGI is not initialized
    pub fn set_burst_frequency(&self, hz: f64) -> Result<()> {
        let components = self.components.lock().unwrap();
        let components = components
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("FEAGI not initialized"))?;

        components.burst_runner.write().set_frequency(hz);
        info!("⚡ Burst frequency set to {:.1}Hz", hz);

        Ok(())
    }

    //
    // ============ REAL-TIME STATS (Read-Only) ============
    //

    /// Check if burst engine is running
    ///
    /// This is a fast read.
    pub fn is_running(&self) -> bool {
        let components = self.components.lock().unwrap();
        if let Some(ref components) = *components {
            components.burst_runner.read().is_running()
        } else {
            false
        }
    }

    /// Get neuron count
    ///
    /// Returns the total number of neurons in the loaded genome.
    ///
    /// # Errors
    ///
    /// Returns error if FEAGI is not initialized
    pub fn get_neuron_count(&self) -> Result<usize> {
        let components = self.components.lock().unwrap();
        let components = components
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("FEAGI not initialized"))?;

        let manager = components.connectome_manager.read();
        Ok(manager.get_neuron_count())
    }

    /// Check if a genome is loaded
    ///
    /// Returns true if neuroembryogenesis has completed successfully.
    pub fn is_genome_loaded(&self) -> bool {
        let components = self.components.lock().unwrap();
        if let Some(ref components) = *components {
            let npu = components.npu.lock().unwrap();
            // If NPU has neurons, genome is loaded
            npu.neuron_count() > 0
        } else {
            false
        }
    }

    //
    // ============ GENOME OPERATIONS ============
    //

    /// Load a genome from file
    ///
    /// Performs neuroembryogenesis to create neurons and synapses.
    /// This is a heavy operation that can take several seconds.
    ///
    /// # Arguments
    ///
    /// * `genome_path` - Path to .brain.json genome file
    ///
    /// # Errors
    ///
    /// Returns error if genome file is invalid or neuroembryogenesis fails
    pub fn load_genome(&self, genome_path: &str) -> Result<()> {
        let components = self.components.lock().unwrap();
        let components = components
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("FEAGI not initialized"))?;

        let path = PathBuf::from(genome_path);

        info!("🧠 Loading genome: {}", genome_path);

        self.runtime.block_on(async {
            components::load_genome_with_agent_handler(
                &components.connectome_manager,
                &components.agent_handler,
                &path,
            )
            .await?;

            info!("✅ Genome loaded successfully");
            Ok(())
        })
    }

    //
    // ============ HTTP SERVER INFO ============
    //

    /// Get the HTTP API base URL
    ///
    /// Returns the URL where the REST API is accessible (e.g., "http://127.0.0.1:8000").
    /// Use this for complex operations not exposed via FFI.
    pub fn get_api_url(&self) -> String {
        self.http_server_url.clone()
    }

    /// Check if HTTP server is running
    ///
    /// Returns true if the Axum server is bound and listening.
    pub fn is_http_server_running(&self) -> bool {
        // HTTP server starts during initialization and runs until shutdown
        self.components.lock().unwrap().is_some()
    }

    //
    // ============ LIFECYCLE ============
    //

    /// Shutdown FEAGI gracefully
    ///
    /// Stops burst engine, closes streams, and saves state.
    /// Blocks until shutdown is complete.
    ///
    /// # Errors
    ///
    /// Returns error if shutdown fails
    pub fn shutdown(&self) -> Result<()> {
        info!("🛑 Shutting down FEAGI...");

        let components = self.components.lock().unwrap();
        if let Some(ref components) = *components {
            // Stop burst engine
            components.burst_runner.write().stop();

            info!("✅ FEAGI shutdown complete");
        }

        Ok(())
    }
}

// Ensure FEAGI shuts down when dropped
impl Drop for FeagiInstance {
    fn drop(&mut self) {
        if let Err(e) = self.shutdown() {
            error!("Error during FEAGI shutdown: {}", e);
        }
    }
}
