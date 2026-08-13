//! # FEAGI Embedded Library
//!
//! Provides a programmatic interface to FEAGI for embedding in applications.
//!
//! ## Usage Modes
//!
//! - **Library Mode:** Embed FEAGI in applications (Godot, Unity, custom tools)
//! - **Binary Mode:** Run FEAGI as standalone server (via `main.rs`)
//!
//! ## Neural processing status
//!
//! The old NPU (`feagi-npu-burst-engine` and its `ConnectomeManager`) has been removed ahead of
//! integrating the rewritten NPU via `feagi_npu::wnpu::WrappedNeuronProcessingUnit`. Transport and
//! API surfaces are intact, but every method on [`FeagiInstance`] that drives neural state returns
//! an error until that integration lands.
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
//!
//! // Neural operations are unavailable until the new NPU is wired in.
//! assert!(feagi.initialize().is_err());
//!
//! // Shutdown gracefully
//! feagi.shutdown().unwrap();
//! ```

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use tracing::{error, info};

// Re-export public types
pub use feagi_config::{load_config, FeagiConfig};

// Re-export for embedders who need them
pub use feagi_agent::server::FeagiAgentHandler;
pub use feagi_services::*;

// Internal modules (reused from main.rs initialization logic)
pub mod components;
pub mod network_provider;
pub mod stub_services;
pub mod version;

pub use components::FeagiComponents;
pub use version::collect_version_info;

/// Error returned by every [`FeagiInstance`] operation that needs a neural backend.
fn npu_unavailable(operation: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{operation} is unavailable: the old NPU was removed and the new one \
         (feagi_npu::wnpu) is not integrated yet"
    )
}

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
    /// Retained for the NPU integration, which needs a runtime to drive async component setup.
    #[allow(dead_code)]
    runtime: Arc<tokio::runtime::Runtime>,
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

        let http_server_url = format!("http://{}:{}", config.api.advertised_host, config.api.port);

        info!("🦀 FEAGI Instance created (embedded mode)");
        info!("   HTTP API will be available at: {}", http_server_url);

        let runtime_arc = Arc::new(runtime);

        Ok(Self {
            components: Arc::new(Mutex::new(None)),
            config,
            runtime: runtime_arc,
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

    //
    // ============ BURST ENGINE CONTROL ============
    //

    /// Start the burst engine
    ///
    /// # Errors
    ///
    /// Always errors: there is no burst engine until the new NPU is integrated.
    pub fn start(&self) -> Result<()> {
        Err(npu_unavailable("Starting the burst engine"))
    }

    /// Stop the burst engine
    ///
    /// # Errors
    ///
    /// Always errors: there is no burst engine until the new NPU is integrated.
    pub fn stop(&self) -> Result<()> {
        Err(npu_unavailable("Stopping the burst engine"))
    }

    /// Set burst frequency (Hz)
    ///
    /// # Errors
    ///
    /// Always errors: there is no burst engine until the new NPU is integrated.
    pub fn set_burst_frequency(&self, _hz: f64) -> Result<()> {
        Err(npu_unavailable("Setting the burst frequency"))
    }

    //
    // ============ REAL-TIME STATS (Read-Only) ============
    //

    /// Check if burst engine is running
    ///
    /// Always `false`: there is no burst engine until the new NPU is integrated.
    pub fn is_running(&self) -> bool {
        false
    }

    /// Get neuron count
    ///
    /// # Errors
    ///
    /// Always errors: neuron storage lived in the removed NPU.
    pub fn get_neuron_count(&self) -> Result<usize> {
        Err(npu_unavailable("Reading the neuron count"))
    }

    /// Check if a genome is loaded
    ///
    /// Always `false`: genome loading needs an NPU to develop the connectome into.
    pub fn is_genome_loaded(&self) -> bool {
        false
    }

    //
    // ============ GENOME OPERATIONS ============
    //

    /// Load a genome from file
    ///
    /// # Errors
    ///
    /// Always errors: neuroembryogenesis needs an NPU to develop the connectome into.
    pub fn load_genome(&self, genome_path: &str) -> Result<()> {
        Err(npu_unavailable(&format!("Loading genome '{genome_path}'")))
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
    /// Closes streams and releases components. Blocks until shutdown is complete.
    ///
    /// # Errors
    ///
    /// Returns error if shutdown fails
    pub fn shutdown(&self) -> Result<()> {
        info!("🛑 Shutting down FEAGI...");

        if self.components.lock().unwrap().take().is_some() {
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
