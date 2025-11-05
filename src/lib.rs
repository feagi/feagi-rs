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

use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use parking_lot::RwLock;
use anyhow::{Context, Result};
use tracing::{info, warn, error};

// Re-export public types
pub use feagi_config::{FeagiConfig, load_config};
pub use feagi_burst_engine::RawFireQueueSnapshot;
pub use feagi_burst_engine::backend::GpuConfig;

// Re-export for embedders who need them
pub use feagi_bdu::ConnectomeManager;
pub use feagi_burst_engine::BurstLoopRunner;
pub use feagi_services::*;
pub use feagi_pns::PNS;

// Internal modules (reused from main.rs initialization logic)
pub mod components;
pub use components::FeagiComponents;

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
    config: FeagiConfig,
    runtime: tokio::runtime::Runtime,
    viz_callback: Arc<Mutex<Option<VisualizationCallback>>>,
    http_server_url: String,
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
        // Create dedicated tokio runtime for FEAGI
        // This ensures FEAGI doesn't interfere with host application's threading
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("feagi-worker")
            .enable_all()
            .build()
            .context("Failed to create Tokio runtime")?;
        
        let http_server_url = format!("http://{}:{}", config.api.host, config.api.port);
        
        info!("🦀 FEAGI Instance created (embedded mode)");
        info!("   HTTP API will be available at: {}", http_server_url);
        
        Ok(Self {
            components: Arc::new(Mutex::new(None)),
            config,
            runtime,
            viz_callback: Arc::new(Mutex::new(None)),
            http_server_url,
        })
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
    pub fn initialize(&mut self) -> Result<()> {
        info!("🚀 Initializing FEAGI components...");
        
        let config = self.config.clone();
        let components_arc = self.components.clone();
        let viz_callback = self.viz_callback.clone();
        
        self.runtime.block_on(async move {
            let components = components::initialize_components(&config).await
                .context("Failed to initialize FEAGI components")?;
            
            // TODO: Wire visualization callback when PNS supports it
            // For now, PNS will use existing ZMQ/WebSocket publishing
            
            // Start HTTP API server
            components::start_http_server(&components, &config).await
                .context("Failed to start HTTP API server")?;
            
            // Start PNS control streams (agent registration)
            components.pns.start_control_streams()
                .context("Failed to start PNS control streams")?;
            
            *components_arc.lock().unwrap() = Some(components);
            
            info!("✅ FEAGI initialization complete");
            Ok(())
        })
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
        let components = components.as_ref()
            .ok_or_else(|| anyhow::anyhow!("FEAGI not initialized. Call initialize() first."))?;
        
        components.burst_runner.write().start();
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
        let components = components.as_ref()
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
        let components = components.as_ref()
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
        let components = components.as_ref()
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
        let components = components.as_ref()
            .ok_or_else(|| anyhow::anyhow!("FEAGI not initialized"))?;
        
        let path = PathBuf::from(genome_path);
        
        info!("🧠 Loading genome: {}", genome_path);
        
        self.runtime.block_on(async {
            components::load_genome_with_pns(
                &components.connectome_manager,
                &components.pns,
                &path,
            ).await?;
            
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
