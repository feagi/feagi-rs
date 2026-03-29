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
#[cfg(feature = "plasticity")]
use feagi::plasticity_runtime::wire_plasticity_callbacks;
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

use feagi::network_provider::FeagiNetworkConnectionInfoProvider;
use feagi_agent::command_and_control::agent_embodiment_configuration_message::AgentEmbodimentConfigurationMessage;
use feagi_agent::command_and_control::FeagiMessage;
use feagi_api::common::agent_registration::{
    auto_create_cortical_areas_from_device_registrations,
    derive_motor_cortical_ids_from_device_registrations,
    derive_sensory_cortical_ids_from_device_registrations,
};
use feagi_api::endpoints::network::NetworkConnectionInfoProvider;
use feagi_api::transports::http::server::{create_http_server, ApiState};
use feagi_brain_development::models::cortical_area::CorticalAreaExt;
use feagi_brain_development::ConnectomeManager;
use feagi_config::{load_config, validate_config, FeagiConfig};
use feagi_io::{AgentID, SensoryIntakeQueue};
use feagi_npu_burst_engine::backend::GpuConfig;
use feagi_npu_burst_engine::{BurstLoopRunner, SensoryIngressPayload, SensoryIntake, TracingMutex};
use feagi_observability::{init_logging_default, parse_debug_flags};
use feagi_services::impls::AgentServiceImpl;
use feagi_services::impls::SystemServiceImpl;
use feagi_services::traits::agent_service::AgentService;
use feagi_services::types::LoadGenomeParams;
use feagi_services::*;
use feagi_state_manager::StateManager;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Sender};

#[cfg(feature = "plasticity")]
fn build_plasticity_config(config: &FeagiConfig) -> feagi_npu_plasticity::PlasticityConfig {
    use feagi_npu_plasticity::{MemoryNeuronLifecycleConfig, PatternConfig, STDPConfig};

    let stdp_cfg = STDPConfig {
        lookback_steps: config.plasticity.stdp.lookback_steps as u32,
        tau_pre: config.plasticity.stdp.tau_pre as f32,
        tau_post: config.plasticity.stdp.tau_post as f32,
        a_plus: config.plasticity.stdp.a_plus as f32,
        a_minus: config.plasticity.stdp.a_minus as f32,
        // max_pairs_per_synapse is not yet configurable in FeagiConfig.
        max_pairs_per_synapse: STDPConfig::default().max_pairs_per_synapse,
    };

    let pattern_cfg = PatternConfig {
        default_temporal_depth: config.plasticity.memory.default_temporal_depth as u32,
        min_activity_threshold: config.plasticity.memory.min_activation_count,
        max_pattern_cache_size: config.plasticity.memory.pattern_cache_size,
    };

    let lifecycle_cfg = MemoryNeuronLifecycleConfig {
        initial_lifespan: config.plasticity.memory.initial_lifespan,
        lifespan_growth_rate: config.plasticity.memory.lifespan_growth_rate,
        longterm_threshold: config.plasticity.memory.longterm_threshold,
        max_reactivations: config.plasticity.memory.max_reactivations,
    };

    feagi_npu_plasticity::PlasticityConfig {
        queue_capacity: config.plasticity.queue_capacity,
        max_ops_per_burst: config.plasticity.max_ops_per_burst,
        memory_array_capacity: config.plasticity.memory.array_capacity,
        stdp: Some(stdp_cfg),
        pattern_config: pattern_cfg,
        memory_lifecycle_config: lifecycle_cfg,
    }
}

/// Mask for agent_data_hash to keep within JSON-safe integer range (BV expects int).
const AGENT_HASH_SAFE_MASK: u64 = (1u64 << 53) - 1;

/// Row from agent registration snapshot (handler loops / hashing).
type AgentRegistrationRow = (
    AgentID,
    String,
    feagi_agent::AgentDescriptor,
    Vec<feagi_agent::AgentCapabilities>,
    Option<serde_json::Value>,
    Option<(String, f64)>,
);

fn hash_json_value_for_agent(value: &serde_json::Value, hasher: &mut DefaultHasher) {
    match value {
        serde_json::Value::Null => hasher.write_u8(0),
        serde_json::Value::Bool(val) => {
            hasher.write_u8(1);
            hasher.write_u8(*val as u8);
        }
        serde_json::Value::Number(num) => {
            hasher.write_u8(2);
            num.to_string().hash(hasher);
        }
        serde_json::Value::String(text) => {
            hasher.write_u8(3);
            text.hash(hasher);
        }
        serde_json::Value::Array(values) => {
            hasher.write_u8(4);
            for item in values {
                hash_json_value_for_agent(item, hasher);
            }
        }
        serde_json::Value::Object(map) => {
            hasher.write_u8(5);
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                key.hash(hasher);
                if let Some(item) = map.get(key) {
                    hash_json_value_for_agent(item, hasher);
                } else {
                    hasher.write_u8(0);
                }
            }
        }
    }
}

/// Updates StateManager agent_data_hash from FeagiAgentHandler's registered agents.
/// BV polls health check and refreshes agent registry when this hash changes.
fn update_agent_data_hash_from_registration_snapshot(snapshot: &[AgentRegistrationRow]) {
    let mut agent_ids: Vec<&String> = snapshot.iter().map(|(_, id, ..)| id).collect();
    agent_ids.sort();
    let mut hasher = DefaultHasher::new();
    for agent_id in agent_ids {
        agent_id.hash(&mut hasher);
        if let Some((_, _, descriptor, capabilities, device_regs, _)) =
            snapshot.iter().find(|(_, id, ..)| id == agent_id)
        {
            descriptor.hash(&mut hasher);
            for cap in capabilities {
                cap.hash(&mut hasher);
            }
            if let Some(regs) = device_regs {
                hash_json_value_for_agent(regs, &mut hasher);
            } else {
                hasher.write_u8(0);
            }
        }
    }
    let hash_value = hasher.finish() & AGENT_HASH_SAFE_MASK;
    if let Some(state_manager) = StateManager::instance().try_write() {
        state_manager.set_agent_data_hash(hash_value);
    }
}

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
    /// - --npu-trace-chain-upstream / --npu-trace-chain-downstream <NEURON_ID> (CHAIN latency lines)
    /// - --npu-trace-src <NEURON_ID>
    /// - --npu-trace-dst <NEURON_ID>
    /// - --npu-trace-neuron <NEURON_ID>
    /// - --npu-trace-cortical-idx <U32> (runtime cortical_idx; enables dynamics + FCL summary)
    /// - --npu-trace-cortical-id <BASE64> (genome cortical id; enables synapse traces to that area;
    ///   after genome load, cortical_idx is resolved for dynamics/FCL when idx not set explicitly)
    ///
    /// Synapse per-edge lines: set env `FEAGI_NPU_TRACE_SYNAPSE_VERBOSE=1` (default is summary only).
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

    /// Filter dynamics / FCL instrumentation to neurons with this runtime `cortical_idx`.
    /// Implies dynamics tracing if `--npu-trace` / `--npu-trace-dynamics` are not set.
    #[arg(long)]
    npu_trace_cortical_idx: Option<u32>,

    /// Filter synapse traces to edges whose **target** is in this cortical area (base64 id, e.g. from genome).
    /// Implies synapse tracing if `--npu-trace` / `--npu-trace-synapse` are not set.
    #[arg(long)]
    npu_trace_cortical_id: Option<String>,

    /// Log `CHAIN upstream_fired` / `CHAIN downstream_fired` with burst deltas (pair with downstream).
    #[arg(long)]
    npu_trace_chain_upstream: Option<u32>,

    /// Log `CHAIN downstream_fired` with `delta_bursts_since_upstream` (pair with upstream).
    #[arg(long)]
    npu_trace_chain_downstream: Option<u32>,

    /// After load: log every neuron id in the traced cortical area; each burst: `AREA_FIRES` with ids + inter-fire burst deltas. Use with `--npu-trace-cortical-idx` or `--npu-trace-cortical-id` (no coordinates).
    #[arg(long)]
    npu_trace_area_fire_ids: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Configure NPU tracing BEFORE logging initialization (trace config is cached via OnceLock)
    let enable_any_trace = args.npu_trace
        || args.npu_trace_synapse
        || args.npu_trace_dynamics
        || args.npu_trace_cortical_idx.is_some()
        || args.npu_trace_cortical_id.is_some()
        || args.npu_trace_chain_upstream.is_some()
        || args.npu_trace_chain_downstream.is_some()
        || args.npu_trace_area_fire_ids;

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
        if let Some(idx) = args.npu_trace_cortical_idx {
            std::env::set_var("FEAGI_NPU_TRACE_CORTICAL_IDX", idx.to_string());
            if !(args.npu_trace || args.npu_trace_dynamics) {
                std::env::set_var("FEAGI_NPU_TRACE_DYNAMICS", "1");
            }
        }
        if let Some(ref id) = args.npu_trace_cortical_id {
            std::env::set_var("FEAGI_NPU_TRACE_CORTICAL_ID", id.trim());
            if !(args.npu_trace || args.npu_trace_synapse) {
                std::env::set_var("FEAGI_NPU_TRACE_SYNAPSE", "1");
            }
        }
        if let Some(u) = args.npu_trace_chain_upstream {
            std::env::set_var("FEAGI_NPU_TRACE_CHAIN_UPSTREAM", u.to_string());
        }
        if let Some(d) = args.npu_trace_chain_downstream {
            std::env::set_var("FEAGI_NPU_TRACE_CHAIN_DOWNSTREAM", d.to_string());
        }
        if args.npu_trace_area_fire_ids {
            std::env::set_var("FEAGI_NPU_TRACE_AREA_FIRE_IDS", "1");
            if !(args.npu_trace || args.npu_trace_dynamics) {
                std::env::set_var("FEAGI_NPU_TRACE_DYNAMICS", "1");
            }
        }
    }

    // Initialize observability with per-crate debug flags
    // This automatically parses --debug-{crate-name} flags from command line
    // and also checks FEAGI_DEBUG environment variable
    let mut debug_flags = parse_debug_flags();

    // Apply --debug-all flag
    if args.debug_all {
        for crate_name in feagi_observability::KNOWN_CRATES {
            debug_flags
                .enabled_crates
                .insert(crate_name.to_string(), true);
        }
    }

    // Apply --debug {crate-name} values
    for crate_name in &args.debug {
        debug_flags.enabled_crates.insert(crate_name.clone(), true);
    }

    // Apply verbose mode (enable debug for all crates)
    if args.verbose {
        for crate_name in feagi_observability::KNOWN_CRATES {
            debug_flags
                .enabled_crates
                .insert(crate_name.to_string(), true);
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
    let _log_guard = init_logging_default(&debug_flags).context("Failed to initialize logging")?;

    // Log enabled debug crates if any
    if debug_flags.any_enabled() {
        let enabled_crates: Vec<String> =
            debug_flags.enabled_crates().into_iter().cloned().collect();
        info!("Debug logging enabled for: {}", enabled_crates.join(", "));
    }

    info!(
        "Logs are being saved to: {}",
        _log_guard.log_dir().display()
    );

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

    validate_config(&config).context("Configuration validation failed")?;

    info!("✓ Configuration loaded and validated");
    log_config_summary(&config);

    // Create shutdown flag early (needed for polling loop)
    let shutdown_flag = Arc::new(AtomicBool::new(true));

    // Setup signal handler for graceful shutdown
    let shutdown_flag_for_signal = shutdown_flag.clone();
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                eprintln!("[SHUTDOWN] Ctrl+C received");
                shutdown_flag_for_signal.store(false, Ordering::SeqCst);
            }
            Err(e) => {
                eprintln!("[SHUTDOWN] Error receiving Ctrl+C: {}", e);
            }
        }
    });

    // Initialize core components
    info!("Initializing FEAGI core components...");
    let components = initialize_components(&config, &args).await?;
    info!("✓ Core components initialized");

    // Start agent handler polling loop IMMEDIATELY (servers need polling to accept connections)
    let agent_handler_for_loop = Arc::clone(&components.agent_handler);
    let shutdown_flag_for_polling = Arc::clone(&shutdown_flag);
    let runtime_service_for_polling = components.runtime_service.clone();
    let sensory_intake_queue_for_polling = Arc::clone(&components.sensory_intake_queue);
    let connectome_manager_for_polling = Arc::clone(&components.connectome_manager);
    let sensory_drain_budget_per_cycle =
        ((1.0 / config.neural.burst_engine_timestep).ceil() as usize).max(1);

    // Holder for ApiState so polling loop can call auto_create when device_registrations arrive.
    // Set by start_services when genome/connectome services are ready.
    let api_state_holder: Arc<Mutex<Option<Arc<ApiState>>>> = Arc::new(Mutex::new(None));
    let api_state_holder_for_polling = Arc::clone(&api_state_holder);
    let rt_handle = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || {
        use std::collections::{HashMap, HashSet};

        // Shared so spawned registration tasks can update on success (non-blocking design).
        let known_motor_sessions: Arc<Mutex<HashSet<AgentID>>> =
            Arc::new(Mutex::new(HashSet::new()));
        let known_motor_subscriptions: Arc<Mutex<HashMap<AgentID, HashSet<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let known_visualization_sessions: Arc<Mutex<HashSet<AgentID>>> =
            Arc::new(Mutex::new(HashSet::new()));
        // Maps session_id -> viz_agent_id used at registration. Required so we unregister
        // with the same key (e.g. "brain-visualizer" from old BV) that was used to register.
        let session_id_to_viz_agent_id: Arc<Mutex<HashMap<AgentID, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // Track descriptors that already had auto-create applied successfully.
        // This prevents per-cycle re-check spam while still retrying until first success.
        let auto_created_descriptors: Arc<Mutex<HashSet<feagi_agent::AgentDescriptor>>> =
            Arc::new(Mutex::new(HashSet::new()));
        // Log "Deferring motor registration" only once per agent-id until registration recovers.
        // This prevents startup/restart loops from flooding logs every polling cycle.
        let deferred_motor_logged_agents: Arc<Mutex<HashSet<String>>> =
            Arc::new(Mutex::new(HashSet::new()));

        let api_state_holder = api_state_holder_for_polling;

        loop {
            if !shutdown_flag_for_polling.load(Ordering::SeqCst) {
                info!("✓ Agent handler polling loop shutting down");
                break;
            }

            let transition_in_progress = api_state_holder
                .lock()
                .unwrap()
                .as_ref()
                .map(|api| api.genome_transition_in_progress.load(Ordering::SeqCst))
                .unwrap_or(false);
            if transition_in_progress {
                // Strict transition barrier: never carry pre-transition sensory frames
                // into a post-load genome.
                sensory_intake_queue_for_polling.clear();
                std::thread::yield_now();
                continue;
            }

            {
                let mut handler_guard = agent_handler_for_loop.lock().unwrap();
                match handler_guard.poll_command_and_control() {
                    Ok(Some((session_id, message))) => {
                        info!(
                            "📨 Received message from session {:?}: {:?}",
                            session_id, message
                        );
                        // Keep "auto_create before response" ordering for AgentConfiguration,
                        // but release agent_handler lock while running expensive auto_create work.
                        let pre_response_device_regs = if let FeagiMessage::AgentConfiguration(
                            AgentEmbodimentConfigurationMessage::AgentConfigurationDetails(
                                device_def,
                            ),
                        ) = &message
                        {
                            Some(serde_json::to_value(device_def).unwrap_or_else(|_| {
                                tracing::warn!(
                                    target: "feagi-rs",
                                    "Failed to serialize AgentConfigurationDetails to JSON"
                                );
                                serde_json::Value::Object(serde_json::Map::new())
                            }))
                        } else {
                            None
                        };

                        if let Some(device_regs) = pre_response_device_regs.as_ref() {
                            drop(handler_guard);
                            match api_state_holder.lock().unwrap().as_ref() {
                                Some(api) => {
                                    info!(
                                        "[MOTOR-REG] Running auto_create before AgentConfiguration response"
                                    );
                                    rt_handle.block_on(
                                        auto_create_cortical_areas_from_device_registrations(
                                            api.as_ref(),
                                            device_regs,
                                        ),
                                    );
                                }
                                None => {
                                    warn!(
                                        "[MOTOR-REG] ApiState not yet available (genome may still be loading); \
                                         auto_create deferred to Pass 2"
                                    );
                                }
                            }
                            handler_guard = agent_handler_for_loop.lock().unwrap();
                        }
                        match handler_guard.send_message_to_agent(session_id, message, 0) {
                            Ok(()) => {
                                info!("✅ Sent response to session {:?}", session_id);
                            }
                            Err(e) => {
                                error!("❌ Failed to send response to agent: {:?}", e);
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        error!("❌ Error polling command/control: {:?}", e);
                    }
                }

                drop(handler_guard);

                {
                    let mut handler_guard = agent_handler_for_loop.lock().unwrap();
                    if let Err(e) = handler_guard.poll_agent_motors() {
                        error!("❌ Error polling embodiment motors: {:?}", e);
                    }
                }

                // Keep visualization publishers polled so WebSocket clients can complete handshake
                // even before visualization payloads are emitted.
                {
                    let mut handler_guard = agent_handler_for_loop.lock().unwrap();
                    if let Err(e) = handler_guard.poll_agent_visualizers() {
                        error!("❌ Error polling embodiment visualizers: {:?}", e);
                    }
                }

                // Check for new WebSocket agent registrations with visualization capability.
                // Collect a snapshot quickly under lock, then perform heavier processing outside.
                let mut pending_motor: Vec<(AgentID, String, Vec<String>)> = Vec::new();
                let mut pending_viz: Vec<(AgentID, String, f64)> = Vec::new();
                let mut stale_motor: Vec<AgentID> = Vec::new();
                let mut stale_viz: Vec<AgentID> = Vec::new();
                let mut device_regs_to_auto_create: Vec<(
                    feagi_agent::AgentDescriptor,
                    serde_json::Value,
                )> = Vec::new();

                let mut registration_snapshot: Vec<AgentRegistrationRow> = Vec::new();

                {
                    let handler_guard = agent_handler_for_loop.lock().unwrap();
                    let registered_agents = handler_guard.get_all_registered_agents();
                    registration_snapshot.reserve(registered_agents.len());
                    for (session_id, (agent_descriptor, capabilities)) in registered_agents.iter() {
                        let device_regs = handler_guard
                            .get_device_registrations_by_descriptor(agent_descriptor)
                            .or_else(|| {
                                handler_guard.get_device_registrations_by_agent(*session_id)
                            })
                            .cloned();
                        let viz_registration =
                            handler_guard.get_visualization_info_for_agent(*session_id);
                        registration_snapshot.push((
                            *session_id,
                            session_id.to_base64(),
                            agent_descriptor.clone(),
                            capabilities.clone(),
                            device_regs,
                            viz_registration,
                        ));
                    }
                }

                update_agent_data_hash_from_registration_snapshot(&registration_snapshot);

                let current_sessions: HashSet<AgentID> =
                    registration_snapshot.iter().map(|(sid, ..)| *sid).collect();
                let current_descriptors: HashSet<feagi_agent::AgentDescriptor> =
                    registration_snapshot
                        .iter()
                        .map(|(_, _, descriptor, ..)| descriptor.clone())
                        .collect();

                // Prune deferral-log state for disconnected agents.
                {
                    let current_agent_ids: HashSet<String> = registration_snapshot
                        .iter()
                        .map(|(_, agent_id, ..)| agent_id.clone())
                        .collect();
                    let mut logged_guard = deferred_motor_logged_agents.lock().unwrap();
                    logged_guard.retain(|agent_id| current_agent_ids.contains(agent_id));
                }

                // Detect agents that were registered but are no longer present (deregistered).
                {
                    let motor_guard = known_motor_sessions.lock().unwrap();
                    for sid in motor_guard.difference(&current_sessions) {
                        stale_motor.push(*sid);
                    }
                }
                {
                    let viz_guard = known_visualization_sessions.lock().unwrap();
                    for sid in viz_guard.difference(&current_sessions) {
                        stale_viz.push(*sid);
                    }
                }

                // Drop auto-create completion markers for disconnected/replaced descriptors.
                auto_created_descriptors
                    .lock()
                    .unwrap()
                    .retain(|descriptor| current_descriptors.contains(descriptor));

                // Pass 1: Collect auto-create work and new visualization registrations.
                for (
                    session_id,
                    agent_id,
                    agent_descriptor,
                    capabilities,
                    device_regs_opt,
                    viz_registration_opt,
                ) in &registration_snapshot
                {
                    if let Some(device_regs) = device_regs_opt.as_ref() {
                        let already_marked_complete = auto_created_descriptors
                            .lock()
                            .unwrap()
                            .contains(agent_descriptor);
                        let needs_auto_create = if !already_marked_complete {
                            true
                        } else {
                            // Genome reload/reset can remove previously auto-created areas while
                            // the descriptor remains connected. Re-run auto-create when any expected
                            // motor OR sensory IDs are no longer present.
                            let mut expected_ids: HashSet<String> = HashSet::new();
                            let mut derivation_failed = false;
                            match derive_motor_cortical_ids_from_device_registrations(device_regs) {
                                Ok(ids) => expected_ids.extend(ids),
                                Err(e) => {
                                    derivation_failed = true;
                                    debug!(
                                        "[MOTOR-REG] Could not derive motor IDs while checking auto-create completion for descriptor {:?}: {}",
                                        agent_descriptor, e
                                    );
                                }
                            }
                            match derive_sensory_cortical_ids_from_device_registrations(device_regs)
                            {
                                Ok(ids) => expected_ids.extend(ids),
                                Err(e) => {
                                    derivation_failed = true;
                                    debug!(
                                        "[MOTOR-REG] Could not derive sensory IDs while checking auto-create completion for descriptor {:?}: {}",
                                        agent_descriptor, e
                                    );
                                }
                            }

                            if derivation_failed || expected_ids.is_empty() {
                                true
                            } else {
                                let connectome_guard = connectome_manager_for_polling.read();
                                !expected_ids.iter().all(|id_b64| {
                                    feagi_structures::genomic::cortical_area::CorticalID::try_from_base_64(id_b64)
                                        .ok()
                                        .map(|id| connectome_guard.has_cortical_area(&id))
                                        .unwrap_or(false)
                                })
                            }
                        };
                        if needs_auto_create {
                            device_regs_to_auto_create
                                .push((agent_descriptor.clone(), device_regs.clone()));
                        }
                    }

                    if !known_visualization_sessions
                        .lock()
                        .unwrap()
                        .contains(session_id)
                    {
                        let mut viz_registration = viz_registration_opt.clone();
                        if viz_registration.is_none()
                            && capabilities.contains(
                                &feagi_agent::AgentCapabilities::ReceiveNeuronVisualizations,
                            )
                        {
                            viz_registration = Some((agent_id.clone(), 0.0));
                        }
                        if let Some((viz_agent_id, requested_rate_hz)) = viz_registration {
                            pending_viz.push((*session_id, viz_agent_id, requested_rate_hz));
                        }
                    }
                }

                // Pass 2: Derive motor cortical IDs and build pending_motor.
                // Gate motor registration attempts until API/connectome state is ready.
                // During startup/restart, agents can connect before genome/connectome is ready,
                // which causes repeated unresolved motor-ID resolution attempts.
                let motor_registration_ready = api_state_holder
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|api| !api.genome_transition_in_progress.load(Ordering::SeqCst))
                    .unwrap_or(false);
                if motor_registration_ready {
                    for (
                        session_id,
                        agent_id,
                        agent_descriptor,
                        capabilities,
                        device_regs_opt,
                        _viz_registration_opt,
                    ) in &registration_snapshot
                    {
                        if !capabilities.contains(&feagi_agent::AgentCapabilities::ReceiveMotorData)
                        {
                            continue;
                        }

                        let motor_cortical_ids: Vec<String> = if let Some(device_regs) =
                            device_regs_opt
                        {
                            match derive_motor_cortical_ids_from_device_registrations(device_regs) {
                                Ok(ids) => ids.into_iter().collect(),
                                Err(e) => {
                                    warn!(
                                        "⚠️ [WS-REGISTRATION] Failed deriving motor cortical IDs from device registrations: {}",
                                        e
                                    );
                                    Vec::new()
                                }
                            }
                        } else {
                            debug!(
                                "[MOTOR-REG] No device registrations for agent '{}' (descriptor {:?}); using connectome output areas as fallback",
                                agent_id,
                                agent_descriptor
                            );
                            let connectome_guard = connectome_manager_for_polling.read();
                            connectome_guard
                                .get_cortical_area_ids()
                                .iter()
                                .filter_map(|cortical_id| {
                                    connectome_guard
                                        .get_cortical_area(cortical_id)
                                        .filter(|area| area.is_output_area())
                                        .map(|_| cortical_id.as_base_64())
                                })
                                .collect()
                        };

                        if motor_cortical_ids.is_empty() {
                            let should_log = {
                                let mut logged_guard = deferred_motor_logged_agents.lock().unwrap();
                                logged_guard.insert(agent_id.clone())
                            };
                            if should_log {
                                info!(
                                    "[MOTOR-REG] Deferring motor registration for agent '{}' (no motor cortical IDs resolved)",
                                    agent_id
                                );
                            }
                        } else {
                            let desired_set: HashSet<String> =
                                motor_cortical_ids.iter().cloned().collect();
                            let current_set = known_motor_subscriptions
                                .lock()
                                .unwrap()
                                .get(session_id)
                                .cloned()
                                .unwrap_or_default();
                            let needs_update =
                                !known_motor_sessions.lock().unwrap().contains(session_id)
                                    || current_set != desired_set;
                            if needs_update {
                                // Registration is now active for this agent; allow a future deferral
                                // message if it later loses motor cortical IDs again.
                                {
                                    let mut logged_guard =
                                        deferred_motor_logged_agents.lock().unwrap();
                                    logged_guard.remove(agent_id);
                                }
                                debug!(
                                    "[MOTOR-REG] Scheduling motor subscription update for agent '{}' with {} cortical IDs",
                                    agent_id,
                                    motor_cortical_ids.len()
                                );
                                pending_motor.push((
                                    *session_id,
                                    agent_id.clone(),
                                    motor_cortical_ids,
                                ));
                            }
                        }
                    }
                }

                // Pass 2 (outside handler lock): Auto-create missing cortical areas from
                // device_registrations. This can be expensive and must not hold `agent_handler`,
                // otherwise burst-loop publish path can block and stall burst progress.
                if !device_regs_to_auto_create.is_empty() {
                    if let Some(api) = api_state_holder.lock().unwrap().as_ref() {
                        debug!(
                            "[MOTOR-REG] Invoking auto_create for {} device_registration(s)",
                            device_regs_to_auto_create.len()
                        );
                        for (descriptor, device_regs) in &device_regs_to_auto_create {
                            let mut expected_ids: HashSet<String> = HashSet::new();
                            let mut derivation_failed = false;
                            match derive_motor_cortical_ids_from_device_registrations(device_regs) {
                                Ok(ids) => expected_ids.extend(ids),
                                Err(e) => {
                                    derivation_failed = true;
                                    debug!(
                                        "[MOTOR-REG] Could not derive motor IDs before auto_create for descriptor {:?}: {}",
                                        descriptor, e
                                    );
                                }
                            }
                            match derive_sensory_cortical_ids_from_device_registrations(device_regs)
                            {
                                Ok(ids) => expected_ids.extend(ids),
                                Err(e) => {
                                    derivation_failed = true;
                                    debug!(
                                        "[MOTOR-REG] Could not derive sensory IDs before auto_create for descriptor {:?}: {}",
                                        descriptor, e
                                    );
                                }
                            }
                            rt_handle.block_on(
                                auto_create_cortical_areas_from_device_registrations(
                                    api.as_ref(),
                                    device_regs,
                                ),
                            );
                            // Mark as completed only after expected motor cortical IDs exist.
                            // This avoids false-positive completion when initial payload/state
                            // causes auto_create to no-op.
                            let all_expected_present = {
                                let connectome_guard = connectome_manager_for_polling.read();
                                expected_ids.iter().all(|id_b64| {
                                    feagi_structures::genomic::cortical_area::CorticalID::try_from_base_64(id_b64)
                                        .ok()
                                        .map(|id| connectome_guard.has_cortical_area(&id))
                                        .unwrap_or(false)
                                })
                            };
                            if !derivation_failed
                                && !expected_ids.is_empty()
                                && all_expected_present
                            {
                                auto_created_descriptors
                                    .lock()
                                    .unwrap()
                                    .insert(descriptor.clone());
                            } else {
                                debug!(
                                    "[MOTOR-REG] Auto-create incomplete for descriptor {:?}; will retry",
                                    descriptor
                                );
                            }
                        }
                    } else {
                        debug!(
                            "[MOTOR-REG] Auto-create deferred: ApiState not yet available (genome may still be loading)"
                        );
                    }
                }

                // Feed transport-agnostic sensory intake (any transport that received data).
                // IMPORTANT ORDERING: this runs after registration-driven auto-create work so
                // first sensory payloads do not race ahead of cortical area provisioning.
                //
                // Drain up to a bounded per-cycle budget and keep only the newest payload
                // so sustained streams do not accumulate stale frames in memory.
                for _ in 0..sensory_drain_budget_per_cycle {
                    let mut should_break = false;
                    {
                        let mut handler_guard = agent_handler_for_loop.lock().unwrap();
                        match handler_guard.poll_agent_sensors() {
                            Ok(Some(container)) => {
                                let source_id = container
                                    .get_agent_identifier_bytes()
                                    .ok()
                                    .map(|bytes| AgentID::new(*bytes).to_base64());
                                sensory_intake_queue_for_polling
                                    .push_with_source(container.get_byte_ref().to_vec(), source_id);
                            }
                            Ok(None) => {
                                should_break = true;
                            }
                            Err(e) => {
                                error!("❌ Error polling embodiment sensors: {:?}", e);
                                should_break = true;
                            }
                        }
                    }
                    if should_break {
                        break;
                    }
                }

                // Unregister stale agents (e.g. descriptor replacement) from burst runner.
                for sid in &stale_motor {
                    let agent_id_b64 = sid.to_base64();
                    info!(
                        "[WS-REGISTRATION] Unregistering stale motor subscription for '{}'",
                        agent_id_b64
                    );
                    runtime_service_for_polling.unregister_motor_subscriptions(&agent_id_b64);
                    known_motor_sessions.lock().unwrap().remove(sid);
                    known_motor_subscriptions.lock().unwrap().remove(sid);
                }
                for sid in &stale_viz {
                    let viz_agent_id = session_id_to_viz_agent_id
                        .lock()
                        .unwrap()
                        .remove(sid)
                        .unwrap_or_else(|| sid.to_base64());
                    info!(
                        "[WS-REGISTRATION] Unregistering stale visualization subscription for '{}'",
                        viz_agent_id
                    );
                    runtime_service_for_polling
                        .unregister_visualization_subscriptions(&viz_agent_id);
                    known_visualization_sessions.lock().unwrap().remove(sid);
                }

                // Fire-and-forget: spawn registration tasks so polling loop never blocks.
                // Command/control polling stays responsive for hundreds of concurrent agents.
                for (session_id, agent_id_for_motor, motor_cortical_ids_for_task) in pending_motor {
                    let runtime_svc = runtime_service_for_polling.clone();
                    let sessions = Arc::clone(&known_motor_sessions);
                    let subscriptions = Arc::clone(&known_motor_subscriptions);
                    let motor_ids_for_cache = motor_cortical_ids_for_task.clone();
                    tokio::runtime::Handle::current().spawn(async move {
                        let motor_rate_hz = match runtime_svc.get_status().await {
                            Ok(status) if status.frequency_hz > 0.0 => status.frequency_hz,
                            Ok(status) => {
                                warn!(
                                    "⚠️ [WS-REGISTRATION] Invalid runtime frequency {}Hz for motor registration",
                                    status.frequency_hz
                                );
                                return;
                            }
                            Err(e) => {
                                warn!(
                                    "⚠️ [WS-REGISTRATION] Failed to read runtime status for motor registration: {}",
                                    e
                                );
                                return;
                            }
                        };
                        match runtime_svc
                            .register_motor_subscriptions(
                                &agent_id_for_motor,
                                motor_cortical_ids_for_task,
                                motor_rate_hz,
                            )
                            .await
                        {
                            Ok(_) => {
                                info!(
                                    "✅ [WS-REGISTRATION] Registered motor subscriptions for agent '{}' at {}Hz",
                                    agent_id_for_motor, motor_rate_hz
                                );
                                sessions.lock().unwrap().insert(session_id);
                                subscriptions
                                    .lock()
                                    .unwrap()
                                    .insert(session_id, motor_ids_for_cache.iter().cloned().collect());
                            }
                            Err(e) => {
                                warn!(
                                    "⚠️ [WS-REGISTRATION] Failed to register motor subscriptions for agent '{}': {}",
                                    agent_id_for_motor, e
                                );
                            }
                        }
                    });
                }

                for (session_id, viz_agent_id, requested_rate_hz) in pending_viz {
                    let runtime_svc = runtime_service_for_polling.clone();
                    let sessions = Arc::clone(&known_visualization_sessions);
                    let viz_id_map = Arc::clone(&session_id_to_viz_agent_id);
                    let viz_agent_id_for_map = viz_agent_id.clone();
                    tokio::runtime::Handle::current().spawn(async move {
                        let rate_hz = if requested_rate_hz > 0.0 {
                            requested_rate_hz
                        } else {
                            match runtime_svc.get_status().await {
                                Ok(status) if status.frequency_hz > 0.0 => status.frequency_hz,
                                Ok(status) => {
                                    warn!(
                                        "⚠️ [WS-REGISTRATION] Invalid runtime frequency {}Hz for visualization registration",
                                        status.frequency_hz
                                    );
                                    return;
                                }
                                Err(e) => {
                                    warn!(
                                        "⚠️ [WS-REGISTRATION] Failed to read runtime status for visualization registration: {}",
                                        e
                                    );
                                    return;
                                }
                            }
                        };
                        match runtime_svc
                            .register_visualization_subscriptions(&viz_agent_id, rate_hz)
                            .await
                        {
                            Ok(_) => {
                                info!(
                                    "✅ [WS-REGISTRATION] Registered visualization for agent at {}Hz",
                                    rate_hz
                                );
                                sessions.lock().unwrap().insert(session_id);
                                viz_id_map
                                    .lock()
                                    .unwrap()
                                    .insert(session_id, viz_agent_id_for_map);
                            }
                            Err(e) => {
                                warn!(
                                    "⚠️  [WS-REGISTRATION] Failed to register visualization: {}",
                                    e
                                );
                            }
                        }
                    });
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    info!("    ✓ Agent handler polling loop started - servers ready for connections");

    // Start services
    info!("Starting FEAGI services...");
    start_services(components, &config, &args, shutdown_flag, api_state_holder).await?;

    Ok(())
}

/// Core FEAGI components (matches components.rs)
struct FeagiComponents {
    #[allow(dead_code)]
    npu: Arc<feagi_npu_burst_engine::TracingMutex<feagi_npu_burst_engine::DynamicNPU>>,
    connectome_manager: Arc<RwLock<ConnectomeManager>>,
    runtime_service: Arc<RuntimeServiceImpl>,
    burst_runner: Arc<RwLock<BurstLoopRunner>>,
    agent_handler: Arc<std::sync::Mutex<feagi_agent::server::FeagiAgentHandler>>,
    /// Transport-agnostic sensory queue (feagi-io); polling loop pushes here when agents send sensory
    sensory_intake_queue: Arc<SensoryIntakeQueue>,
    #[cfg(feature = "plasticity")]
    plasticity_executor:
        Option<Arc<std::sync::Mutex<feagi_npu_plasticity::AsyncPlasticityExecutor>>>,
    #[cfg(feature = "plasticity")]
    memory_stats_cache: Option<feagi_npu_plasticity::MemoryStatsCache>,
    #[cfg(not(feature = "plasticity"))]
    plasticity_executor: Option<()>,
    #[cfg(not(feature = "plasticity"))]
    memory_stats_cache: Option<()>,
    use_post_burst_processor: bool,
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
                warn!(
                    "  Failed to peek genome precision ({}), defaulting to int8",
                    e
                );
                "int8".to_string()
            }
        }
    } else {
        info!("  No genome provided at startup, defaulting to int8 quantization");
        "int8".to_string()
    };

    // Initialize NPU with appropriate precision
    info!(
        "  Initializing NPU with {} quantization...",
        precision.to_uppercase()
    );

    // GPU config is available but not yet used in NPU initialization
    let _gpu_config = GpuConfig {
        use_gpu: config.resources.use_gpu,
        hybrid_enabled: config.neural.hybrid.enabled,
        gpu_threshold: config.neural.hybrid.gpu_threshold,
        gpu_memory_fraction: config.resources.gpu_memory_fraction,
    };

    // Create NPU based on quantization precision
    use feagi_npu_burst_engine::backend::CPUBackend;
    use feagi_npu_runtime::StdRuntime;

    let runtime = StdRuntime;
    let backend = CPUBackend::new();

    // Wrap NPU in TracingMutex (or Mutex if tracing disabled) to automatically log all lock acquisitions
    // When npu-lock-tracing feature is disabled, TracingMutex is a type alias for std::sync::Mutex (zero overhead)
    let npu = Arc::new(TracingMutex::new(
        match precision.as_str() {
            "fp32" | "f32" => {
                info!("    Creating FP32 NPU (32-bit floating point, highest precision)");
                feagi_npu_burst_engine::DynamicNPU::F32(feagi_npu_burst_engine::RustNPU::new(
                    runtime,
                    backend,
                    config.connectome.neuron_space,
                    config.connectome.synapse_space,
                    10, // fire_ledger_window
                )?)
            }
            "int8" => {
                info!("    Creating INT8 NPU (8-bit integer, 42% memory reduction)");
                feagi_npu_burst_engine::DynamicNPU::INT8(feagi_npu_burst_engine::RustNPU::new(
                    runtime,
                    backend,
                    config.connectome.neuron_space,
                    config.connectome.synapse_space,
                    10, // fire_ledger_window
                )?)
            }
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
        },
        "NPU",
    ));

    info!(
        "    ✓ NPU initialized with {} precision (capacity: {} neurons, {} synapses)",
        match &*npu.lock().unwrap() {
            feagi_npu_burst_engine::DynamicNPU::F32(_) => "fp32",
            feagi_npu_burst_engine::DynamicNPU::INT8(_) => "int8",
        },
        config.connectome.neuron_space,
        config.connectome.synapse_space
    );

    // Initialize ConnectomeManager
    info!("  Initializing ConnectomeManager...");
    let manager = ConnectomeManager::instance(); // Already returns Arc<RwLock<>>
    manager.write().set_npu(Arc::clone(&npu));
    info!("    ✓ ConnectomeManager initialized and connected to NPU");

    // Initialize agent handler and burst runner (from components.rs pattern)
    info!("  Creating Agent Handler (new architecture)...");

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

    let auth_backend = Box::new(DummyAuth {});
    let mut agent_handler = FeagiAgentHandler::new(auth_backend);

    // Add ZMQ servers (multiple slots so multiple agents can register)
    #[cfg(feature = "zmq-transport")]
    {
        let registration_addr = format!(
            "tcp://{}:{}",
            config.agent.bind_host, config.agent.registration_port
        );
        let registration_adv_addr = format!(
            "tcp://{}:{}",
            config.agent.advertised_host, config.agent.registration_port
        );
        let router_props = Box::new(FeagiZmqServerRouterProperties::new(
            &registration_addr,
            &registration_adv_addr,
        )?);
        agent_handler.add_and_start_command_control_server(router_props)?;

        const ZMQ_AGENT_SLOTS: u16 = 8;
        const ZMQ_SENSORY_OFFSET: u16 = 5566;
        const ZMQ_MOTOR_OFFSET: u16 = 5574;
        const ZMQ_VIZ_OFFSET: u16 = 5582;

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
            let sensory_props = Box::new(FeagiZmqServerPullerProperties::new(
                &sensory_addr,
                &sensory_adv_addr,
            )?);
            agent_handler.add_puller_server(sensory_props);

            let motor_addr = format!("tcp://{}:{}", config.zmq.bind_host, motor_port);
            let motor_adv_addr = format!("tcp://{}:{}", config.zmq.advertised_host, motor_port);
            let motor_props = Box::new(FeagiZmqServerPublisherProperties::new(
                &motor_addr,
                &motor_adv_addr,
            )?);
            agent_handler.add_publisher_server(motor_props);

            let viz_addr = format!("tcp://{}:{}", config.zmq.bind_host, viz_port);
            let viz_adv_addr = format!("tcp://{}:{}", config.zmq.advertised_host, viz_port);
            let viz_props = Box::new(FeagiZmqServerPublisherProperties::new(
                &viz_addr,
                &viz_adv_addr,
            )?);
            agent_handler.add_publisher_server(viz_props);
        }
        info!(
            "    ✓ ZMQ transport servers added ({} agent slots)",
            ZMQ_AGENT_SLOTS
        );
    }

    // Add WebSocket servers if enabled
    if config.websocket.enabled {
        let ws_registration_addr = format!(
            "{}:{}",
            config.websocket.bind_host, config.websocket.registration_port
        );
        let ws_registration_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.registration_port
        );
        let ws_router_props = Box::new(FeagiWebSocketServerRouterProperties::new_with_remote(
            &ws_registration_addr,
            &ws_registration_adv_addr,
        )?);
        agent_handler.add_and_start_command_control_server(ws_router_props)?;

        let ws_sensory_addr = format!(
            "{}:{}",
            config.websocket.bind_host, config.websocket.sensory_port
        );
        let ws_sensory_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.sensory_port
        );
        let ws_sensory_props = Box::new(FeagiWebSocketServerPullerProperties::new_with_remote(
            &ws_sensory_addr,
            &ws_sensory_adv_addr,
        )?);
        agent_handler.add_puller_server(ws_sensory_props);

        let ws_motor_addr = format!(
            "{}:{}",
            config.websocket.bind_host, config.websocket.motor_port
        );
        let ws_motor_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.motor_port
        );
        let ws_motor_props = Box::new(FeagiWebSocketServerPublisherProperties::new(
            &ws_motor_addr,
            &ws_motor_adv_addr,
        )?);
        agent_handler.add_publisher_server(ws_motor_props);

        let ws_viz_addr = format!(
            "{}:{}",
            config.websocket.bind_host, config.websocket.visualization_port
        );
        let ws_viz_adv_addr = format!(
            "{}:{}",
            config.websocket.advertised_host, config.websocket.visualization_port
        );
        let ws_viz_props = Box::new(FeagiWebSocketServerPublisherProperties::new(
            &ws_viz_addr,
            &ws_viz_adv_addr,
        )?);
        agent_handler.add_publisher_server(ws_viz_props);
        info!("      ✓ WebSocket visualization publisher: {}", ws_viz_addr);

        info!("    ✓ WebSocket transport servers added");
    }

    let agent_handler = Arc::new(Mutex::new(agent_handler));
    info!("    ✓ Agent Handler created");

    // Initialize BurstLoopRunner
    info!("  Initializing BurstLoopRunner...");
    let burst_timestep = config.neural.burst_engine_timestep;
    let burst_hz = 1.0 / burst_timestep;

    enum AgentPublishJob {
        Visualization {
            agent_id: String,
            fire_data: feagi_npu_burst_engine::RawFireQueueSnapshot,
        },
        Motor {
            agent_id: String,
            data: Vec<u8>,
        },
    }

    let (publish_tx, publish_rx) = mpsc::channel::<AgentPublishJob>();
    let publish_handler = Arc::clone(&agent_handler);
    std::thread::Builder::new()
        .name("agent-publish-dispatch".to_string())
        .spawn(move || {
            info!("    ✓ Agent publish dispatcher started");
            while let Ok(job) = publish_rx.recv() {
                let mut handler_guard = match publish_handler.lock() {
                    Ok(guard) => guard,
                    Err(e) => {
                        error!("[AGENT-PUBLISH] Failed to lock agent handler: {}", e);
                        continue;
                    }
                };

                match job {
                    AgentPublishJob::Visualization { agent_id, fire_data } => {
                        use feagi_serialization::FeagiByteContainer;
                        use feagi_structures::genomic::cortical_area::CorticalID;
                        use feagi_structures::neuron_voxels::xyzp::{
                            CorticalMappedXYZPNeuronVoxels, NeuronVoxelXYZPArrays,
                        };

                        let mut cortical_mapped = CorticalMappedXYZPNeuronVoxels::new();
                        for (_area_idx, fire_queue_data) in fire_data {
                            if let Ok(cortical_id) =
                                CorticalID::try_from_base_64(&fire_queue_data.cortical_id)
                            {
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

                        let parsed_agent_id = match AgentID::try_from_base64(&agent_id) {
                            Ok(id) => id,
                            Err(e) => {
                                static WARNED_VIZ: std::sync::OnceLock<
                                    std::sync::Mutex<std::collections::HashSet<String>>,
                                > = std::sync::OnceLock::new();
                                let warned = WARNED_VIZ.get_or_init(|| {
                                    std::sync::Mutex::new(std::collections::HashSet::new())
                                });
                                let mut warned = warned.lock().unwrap();
                                if warned.insert(agent_id.to_string()) {
                                    tracing::warn!(
                                        "Visualization agent_id '{}' is not valid base64 AgentID ({}). \
                                         Skipping viz publish. Ensure agents register with base64-encoded AgentDescriptor.",
                                        agent_id, e
                                    );
                                }
                                continue;
                            }
                        };

                        let mut container = FeagiByteContainer::new_empty();
                        if let Err(e) = container.set_agent_identifier(parsed_agent_id) {
                            error!(
                                "[AGENT-PUBLISH] Failed to set visualization agent identifier: {:?}",
                                e
                            );
                            continue;
                        }
                        if let Err(e) =
                            container.overwrite_byte_data_with_single_struct_data(&cortical_mapped, 0)
                        {
                            error!("[AGENT-PUBLISH] Failed to wrap visualization payload: {:?}", e);
                            continue;
                        }
                        if let Err(e) = handler_guard.send_visualization_data(parsed_agent_id, &container)
                        {
                            warn!("[AGENT-PUBLISH] Failed to send visualization data: {}", e);
                        }
                    }
                    AgentPublishJob::Motor { agent_id, data } => {
                        use feagi_serialization::FeagiByteContainer;

                        let parsed_agent_id = match AgentID::try_from_base64(&agent_id) {
                            Ok(id) => id,
                            Err(e) => {
                                static WARNED_MOTOR: std::sync::OnceLock<
                                    std::sync::Mutex<std::collections::HashSet<String>>,
                                > = std::sync::OnceLock::new();
                                let warned = WARNED_MOTOR.get_or_init(|| {
                                    std::sync::Mutex::new(std::collections::HashSet::new())
                                });
                                let mut warned = warned.lock().unwrap();
                                if warned.insert(agent_id.to_string()) {
                                    tracing::warn!(
                                        "Motor agent_id '{}' is not valid base64 AgentID ({}). \
                                         Skipping motor publish. Ensure agents register with base64-encoded AgentDescriptor.",
                                        agent_id, e
                                    );
                                }
                                continue;
                            }
                        };

                        let mut container = FeagiByteContainer::new_empty();
                        if let Err(e) = container.try_write_data_by_copy_and_verify(&data) {
                            error!("[AGENT-PUBLISH] Failed to parse motor data: {:?}", e);
                            continue;
                        }

                        if let Err(e) = handler_guard.send_motor_data(parsed_agent_id, &container) {
                            warn!("[AGENT-PUBLISH] Failed to send motor data: {}", e);
                        }
                    }
                }
            }
            warn!("Agent publish dispatcher stopped: channel closed");
        })
        .context("Failed to spawn agent publish dispatcher thread")?;

    // Create agent-handler-backed publishers
    struct AgentHandlerVisualizationPublisher {
        tx: Sender<AgentPublishJob>,
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

            self.tx
                .send(AgentPublishJob::Visualization {
                    agent_id: agent_id.to_string(),
                    fire_data,
                })
                .map_err(|e| format!("Failed to queue visualization publish job: {}", e))
        }
    }

    struct AgentHandlerMotorPublisher {
        tx: Sender<AgentPublishJob>,
    }

    impl feagi_npu_burst_engine::MotorPublisher for AgentHandlerMotorPublisher {
        fn publish_motor(&self, agent_id: &str, data: &[u8]) -> Result<(), String> {
            if data.is_empty() {
                return Ok(());
            }

            self.tx
                .send(AgentPublishJob::Motor {
                    agent_id: agent_id.to_string(),
                    data: data.to_vec(),
                })
                .map_err(|e| format!("Failed to queue motor publish job: {}", e))
        }
    }

    let viz_publisher = Arc::new(Mutex::new(AgentHandlerVisualizationPublisher {
        tx: publish_tx.clone(),
    }));
    let motor_publisher = Arc::new(Mutex::new(AgentHandlerMotorPublisher { tx: publish_tx }));

    let burst_runner = Arc::new(RwLock::new(BurstLoopRunner::new(
        Arc::clone(&npu),
        Some(viz_publisher),
        Some(motor_publisher),
        burst_hz,
    )));
    info!("    ✓ BurstLoopRunner initialized ({:.0}Hz)", burst_hz);

    // Initialize plasticity executor (if plasticity feature enabled) BEFORE RuntimeService
    #[cfg(feature = "plasticity")]
    let (plasticity_executor, memory_stats_cache, use_post_burst_processor, plasticity_service) = {
        use feagi_npu_plasticity::{
            create_memory_stats_cache, AsyncPlasticityExecutor, PlasticityExecutor,
        };
        use std::sync::Mutex;

        info!("╔═══════════════════════════════════════════════════════════════╗");
        info!("║  PLASTICITY SUBSYSTEM INITIALIZATION                          ║");
        info!("╚═══════════════════════════════════════════════════════════════╝");
        info!("  📊 Creating memory stats cache...");
        let cache = create_memory_stats_cache();
        let plasticity_config = build_plasticity_config(config);

        info!("  🧠 Creating AsyncPlasticityExecutor with NPU reference...");
        // Create executor with NPU reference (for querying CPU-resident FireLedger)
        let executor = Arc::new(Mutex::new(AsyncPlasticityExecutor::new(
            plasticity_config,
            cache.clone(),
            Arc::clone(&npu),
        )));

        info!("  🚀 Starting PlasticityService background thread...");
        // Start the plasticity service thread
        {
            let mut exec = executor.lock().unwrap();
            PlasticityExecutor::start(&mut *exec);
        }

        // Get PlasticityService for RuntimeService wiring
        let plasticity_service = executor.lock().unwrap().get_service().map(Arc::new);

        info!("  🔗 Wiring PlasticityExecutor into ConnectomeManager...");
        // Wire plasticity executor into ConnectomeManager for automatic memory area registration
        ConnectomeManager::instance()
            .write()
            .set_plasticity_executor(Arc::clone(&executor));

        info!("  🔗 Wiring PlasticityExecutor into BurstLoopRunner...");
        wire_plasticity_callbacks(&burst_runner, Arc::clone(&executor), Arc::clone(&npu));
        let use_post_burst_processor = burst_runner.read().has_post_burst_callback();

        info!("╔═══════════════════════════════════════════════════════════════╗");
        info!("║  ✅ PLASTICITY SUBSYSTEM READY                                ║");
        info!("║     • Memory neuron pattern detection: ENABLED                ║");
        info!("║     • STDP synaptic plasticity: ENABLED                       ║");
        info!("║     • Background processing thread: ACTIVE                    ║");
        info!("╚═══════════════════════════════════════════════════════════════╝");
        (
            Some(executor),
            Some(cache),
            use_post_burst_processor,
            plasticity_service,
        )
    };

    #[cfg(not(feature = "plasticity"))]
    let (plasticity_executor, memory_stats_cache, use_post_burst_processor, plasticity_service): (
        Option<()>,
        Option<()>,
        bool,
        Option<Arc<feagi_npu_plasticity::PlasticityService>>,
    ) = {
        info!("  ℹ️  Plasticity feature disabled (compiled without --features plasticity)");
        (None, None, false, None)
    };

    // Create RuntimeService with plasticity support if available
    let runtime_service = if let Some(service) = plasticity_service {
        info!("    ✓ Creating Runtime service WITH plasticity support");
        Arc::new(RuntimeServiceImpl::new_with_plasticity(
            Arc::clone(&burst_runner),
            service,
        ))
    } else {
        info!("    ✓ Creating Runtime service WITHOUT plasticity support");
        Arc::new(RuntimeServiceImpl::new(Arc::clone(&burst_runner)))
    };
    info!("    ✓ Runtime service created");

    // Wire up bidirectional connections between PNS and BurstLoopRunner
    info!("  Wiring PNS ↔ BurstLoopRunner connections...");

    // Transport-agnostic sensory intake (feagi-io): burst loop consumes from queue; polling loop feeds it
    let sensory_intake_queue = Arc::new(SensoryIntakeQueue::new());
    struct SensoryIntakeAdapter {
        queue: Arc<SensoryIntakeQueue>,
    }
    impl SensoryIntake for SensoryIntakeAdapter {
        fn poll_sensory_data(&mut self) -> Result<Option<SensoryIngressPayload>, String> {
            let packet = self.queue.poll_next().map(|packet| SensoryIngressPayload {
                bytes: packet.bytes,
                source_id: packet.source_id,
                received_at: packet.received_at,
            });
            Ok(packet)
        }
    }
    burst_runner
        .write()
        .set_sensory_intake(Arc::new(Mutex::new(SensoryIntakeAdapter {
            queue: Arc::clone(&sensory_intake_queue),
        })) as Arc<Mutex<dyn SensoryIntake>>);
    info!("    ✓ Sensory intake (feagi-io) wired to BurstLoopRunner");
    info!("      ✓ Sensory: transports → queue → BurstLoopRunner");
    info!("      ✓ Motor: BurstLoopRunner → Handler (publishing)");
    info!("      ✓ Visualization: BurstLoopRunner → Handler (publishing)");

    Ok(FeagiComponents {
        npu,
        connectome_manager: manager,
        runtime_service,
        burst_runner,
        agent_handler,
        sensory_intake_queue,
        plasticity_executor,
        memory_stats_cache,
        use_post_burst_processor,
    })
}

/// Load genome (new architecture - agent handler notification TODO)
/// Returns the genome's simulation_timestep (in seconds) if available
async fn load_genome_with_agent_handler(
    genome_service: &Arc<GenomeServiceImpl>,
    _agent_handler: &Arc<std::sync::Mutex<feagi_agent::server::FeagiAgentHandler>>,
    genome_path: &PathBuf,
) -> Result<Option<f64>> {
    info!("    [GENOME-LOAD] Step 1: Reading genome file...");

    // Read genome file to JSON string
    let json_str = std::fs::read_to_string(genome_path).context("Failed to read genome file")?;

    info!("    [GENOME-LOAD] Step 2: Loading genome via GenomeService...");

    // Use GenomeService::load_genome which properly stores RuntimeGenome
    let genome_info = genome_service
        .load_genome(LoadGenomeParams { json_str })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load genome: {}", e))?;

    let simulation_timestep = genome_info.simulation_timestep;
    info!(
        "    [GENOME-LOAD] Genome loaded: {} cortical areas, {}s timestep ({:.0}Hz)",
        genome_info.cortical_area_count,
        simulation_timestep,
        1.0 / simulation_timestep
    );

    info!("    [GENOME-LOAD] Step 3: Genome loaded (stream evaluation TODO)");
    // TODO: Implement genome notification in new architecture
    info!("    [GENOME-LOAD] Step 4: Complete");

    Ok(Some(simulation_timestep))
}

/// Lookup runtime cortical index from base64 cortical id (post-neurogenesis).
fn lookup_cortical_idx_from_trace_cortical_id(
    npu: &feagi_npu_burst_engine::DynamicNPU,
    id_b64: &str,
) -> Option<u32> {
    use feagi_structures::genomic::cortical_area::CorticalID;
    CorticalID::try_from_base_64(id_b64.trim())
        .ok()
        .and_then(|cid| npu.get_cortical_area_id(&cid.as_base_64()))
}

/// True when CLI idx and id-resolved idx both exist and differ (FCL vs synapse trace mismatch).
fn cortical_trace_cli_disagrees_with_id_lookup(cli: Option<u32>, idx_from_id: Option<u32>) -> bool {
    matches!((cli, idx_from_id), (Some(a), Some(b)) if a != b)
}

fn warn_if_npu_trace_cortical_idx_and_id_mismatch(
    args: &Args,
    npu: &feagi_npu_burst_engine::DynamicNPU,
) {
    let (Some(cli_idx), Some(id_b64)) = (
        args.npu_trace_cortical_idx,
        args.npu_trace_cortical_id.as_ref(),
    ) else {
        return;
    };
    let Some(id_idx) = lookup_cortical_idx_from_trace_cortical_id(npu, id_b64) else {
        return;
    };
    if cortical_trace_cli_disagrees_with_id_lookup(Some(cli_idx), Some(id_idx)) {
        tracing::warn!(
            target: "feagi-npu-trace",
            cli_cortical_idx = cli_idx,
            cortical_id_base64 = id_b64.trim(),
            resolved_cortical_idx_from_id = id_idx,
            "NPU trace: --npu-trace-cortical-idx does not match the runtime index for --npu-trace-cortical-id. FCL/dynamics/AREA_FIRES use the CLI idx; synapse trace filters by postsynaptic cortical id. Logs will disagree until both refer to the same cortical area."
        );
    }
}

/// Resolve trace cortical index for FCL/dynamics: `--npu-trace-cortical-idx` wins, else lookup from id.
///
/// When both CLI idx and id are set, the idx is returned without validating id; synapse trace still
/// filters by [`FEAGI_NPU_TRACE_CORTICAL_ID`]. Call [`warn_if_npu_trace_cortical_idx_and_id_mismatch`]
/// to detect inconsistent pairs.
fn resolve_npu_trace_cortical_idx(
    args: &Args,
    npu: &feagi_npu_burst_engine::DynamicNPU,
) -> Option<u32> {
    if let Some(idx) = args.npu_trace_cortical_idx {
        return Some(idx);
    }
    args.npu_trace_cortical_id
        .as_deref()
        .and_then(|id| lookup_cortical_idx_from_trace_cortical_id(npu, id))
}

/// Start all FEAGI services (API, ZMQ, Burst Engine)
async fn start_services(
    components: FeagiComponents,
    config: &FeagiConfig,
    args: &Args,
    shutdown_flag: Arc<AtomicBool>,
    api_state_holder: Arc<Mutex<Option<Arc<ApiState>>>>,
) -> Result<()> {
    // Signal handler already setup in main()

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
    let neuron_service = Arc::new(NeuronServiceImpl::new(Arc::clone(
        &components.connectome_manager,
    )));

    // Collect version information for all crates in this binary
    let version_info = feagi::collect_version_info();

    let system_service = Arc::new(SystemServiceImpl::new(
        Arc::clone(&components.connectome_manager),
        Some(Arc::clone(&components.burst_runner)),
        version_info,
    ));

    // TODO: Get agent registry from agent_handler in new architecture
    // For now, create minimal agent service with empty registry
    use parking_lot::RwLock as PRwLock;
    let empty_registry = Arc::new(PRwLock::new(feagi_services::AgentRegistry::new(100, 60000)));

    // Wire GenomeService and ConnectomeService to RegistrationHandler (required for auto-creation feature)
    // Create agent service with empty registry (new architecture)
    let agent_service_impl =
        AgentServiceImpl::new(Arc::clone(&components.connectome_manager), empty_registry);
    let agent_service = Arc::new(agent_service_impl);
    info!("    ✓ Agent service created");

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

    let api_port = args.api_port.unwrap_or(config.api.port);
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

    let api_state = ApiState {
        network_connection_info_provider: Some(network_provider),
        agent_service: Some(agent_service as Arc<dyn AgentService + Send + Sync>),
        genome_service: genome_service.clone() as Arc<dyn GenomeService + Send + Sync>,
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
        memory_stats_cache: components.memory_stats_cache.clone(),
        amalgamation_state: ApiState::init_amalgamation_state(),
        genome_transition_lock,
        genome_transition_in_progress,
        #[cfg(feature = "feagi-agent")]
        agent_handler: Some(Arc::clone(&components.agent_handler)),
        #[cfg(not(feature = "feagi-agent"))]
        agent_handler: None,
    };

    // Agent handler streams already started during initialization
    info!("  ✓ Agent handler control streams active (registration ready)");

    // Start HTTP API server (before genome load in case it hangs)
    info!(
        "  Starting HTTP API server on {}:{} (advertised as {}:{})...",
        api_bind_host, api_port, api_advertised_host, api_port
    );
    let app = create_http_server(api_state.clone());
    let addr = format!("{}:{}", api_bind_host, api_port);

    info!("  API routes registered, binding to {}...", addr);

    // Bind before spawning so we fail fast on port conflicts.
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind API server on {}", addr))?;

    // Spawn API server in background with graceful shutdown support
    let (shutdown_tx_api, shutdown_rx_api) = tokio::sync::oneshot::channel::<()>();
    let api_handle = tokio::spawn(async move {
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
        info!(
            "  Loading genome from: {} (API is already online)",
            genome_path.display()
        );
        match load_genome_with_agent_handler(
            &genome_service,
            &components.agent_handler,
            genome_path,
        )
        .await
        {
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

    // @npu-debug-instrumentation: resolve cortical_id -> cortical_idx; optional full-area neuron id list
    if args.genome.is_some() {
        use feagi_npu_burst_engine::set_runtime_trace_cortical_idx;

        let npu = components.npu.lock().unwrap();
        let idx_resolved = resolve_npu_trace_cortical_idx(args, &npu);

        warn_if_npu_trace_cortical_idx_and_id_mismatch(args, &npu);

        // Log id -> idx from NPU vs effective idx for FCL (CLI idx overrides id lookup).
        if let Some(ref id_b64) = args.npu_trace_cortical_id {
            match lookup_cortical_idx_from_trace_cortical_id(&npu, id_b64) {
                Some(idx_from_id) => {
                    tracing::info!(
                        target: "feagi-npu-trace",
                        cortical_id_base64 = id_b64.trim(),
                        cortical_idx_from_id = idx_from_id,
                        effective_cortical_idx_for_fcl = idx_resolved,
                        "NPU trace: cortical_id maps to cortical_idx (post-neurogenesis); FCL/dynamics use effective idx (CLI --npu-trace-cortical-idx wins when set)"
                    );
                }
                None => {
                    tracing::warn!(
                        target: "feagi-npu-trace",
                        cortical_id_base64 = id_b64.trim(),
                        "NPU trace: could not resolve cortical_id to cortical_idx (unknown id or parse error)"
                    );
                }
            }
        }

        // Dynamics/FCL/AREA_FIRES need cortical_idx: set runtime atomic + env when only cortical_id was given.
        // Include --npu-trace-area-fire-ids (not only --npu-trace) so id-only + area fires works.
        let needs_runtime_idx_from_id = args.npu_trace_cortical_idx.is_none()
            && args.npu_trace_cortical_id.is_some()
            && (args.npu_trace || args.npu_trace_dynamics || args.npu_trace_area_fire_ids);

        if needs_runtime_idx_from_id {
            set_runtime_trace_cortical_idx(idx_resolved);
            if let Some(idx) = idx_resolved {
                std::env::set_var("FEAGI_NPU_TRACE_CORTICAL_IDX", idx.to_string());
            }
        }

        if args.npu_trace_area_fire_ids {
            match idx_resolved {
                Some(idx) => {
                    let ids = npu.get_neurons_in_cortical_area(idx);
                    tracing::info!(
                        target: "feagi-npu-trace",
                        cortical_idx = idx,
                        neuron_count = ids.len(),
                        neuron_ids = ?ids,
                        "NPU trace: all neuron ids in cortical area (post-neurogenesis)"
                    );
                }
                None => {
                    tracing::warn!(
                        target: "feagi-npu-trace",
                        "NPU trace: --npu-trace-area-fire-ids requires a resolvable --npu-trace-cortical-id or --npu-trace-cortical-idx"
                    );
                }
            }
        }
    } else if args.npu_trace_area_fire_ids {
        tracing::warn!(
            target: "feagi-npu-trace",
            "NPU trace: --npu-trace-area-fire-ids requires --genome so the connectome/NPU are populated"
        );
    }

    // Make ApiState available to polling loop for auto_create when device_registrations arrive.
    // Set after genome load so cortical area creation has a valid connectome.
    *api_state_holder.lock().unwrap() = Some(Arc::new(api_state.clone()));

    // Start burst engine via service layer (safe after genome load / connectome reset completes)
    info!("  Starting burst engine...");
    components
        .runtime_service
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to start burst engine: {}", e))?;
    info!("    ✓ Burst engine running");

    // NPU lock watchdog: detect possible deadlock when burst loop stops making progress
    let runtime_svc_watchdog = components.runtime_service.clone();
    let shutdown_for_watchdog = shutdown_flag.clone();
    let rt_handle_for_watchdog = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("feagi-npu-watchdog".to_string())
        .spawn(move || {
            let rt = rt_handle_for_watchdog;
            let mut last_burst: u64 = 0;
            let mut last_progress_at = std::time::Instant::now();
            let stall_threshold = std::time::Duration::from_secs(15);
            let check_interval = std::time::Duration::from_secs(5);

            while shutdown_for_watchdog.load(Ordering::SeqCst) {
                std::thread::sleep(check_interval);
                if !shutdown_for_watchdog.load(Ordering::SeqCst) {
                    break;
                }
                match rt.block_on(runtime_svc_watchdog.get_status()) {
                    Ok(status) if status.is_running => {
                        let current = status.burst_count;
                        if current != last_burst {
                            last_burst = current;
                            last_progress_at = std::time::Instant::now();
                        } else if last_burst > 0 && last_progress_at.elapsed() > stall_threshold {
                            warn!(
                                "[NPU-WATCHDOG] Burst loop stalled: no progress for {:.1}s (burst_count={}) - possible deadlock or NPU lock contention",
                                last_progress_at.elapsed().as_secs_f64(),
                                last_burst
                            );
                            last_progress_at = std::time::Instant::now();
                        }
                    }
                    _ => {}
                }
            }
        })
        .expect("Failed to spawn NPU watchdog thread");

    // Start plasticity executor and command processing loop (if enabled)
    #[cfg(feature = "plasticity")]
    if let Some(ref plasticity_exec) = components.plasticity_executor {
        use feagi_npu_plasticity::PlasticityExecutor;

        info!("  Starting plasticity executor...");
        plasticity_exec.lock().unwrap().start();
        info!("    ✓ Plasticity executor running");

        if !components.use_post_burst_processor {
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
                            debug!("[PLASTICITY-CMD] Processing {} commands", commands.len());
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
                                            "[PLASTICITY-CMD] Registering memory neuron id={}",
                                            neuron_id
                                        );
                                        // Memory neurons are already created by the plasticity service
                                        // This command serves as a notification
                                    }
                                    PlasticityCommand::MemoryNeuronConvertedToLtm { neuron_id, .. } => {
                                        debug!(
                                            "[PLASTICITY-CMD] Memory neuron converted to LTM id={}",
                                            neuron_id
                                        );
                                    }
                                    PlasticityCommand::InjectMemoryNeuronToFCL {
                                        neuron_id,
                                        area_idx,
                                        membrane_potential,
                                        pattern_hash,
                                        is_reactivation: _,
                                        replay_frames: _,
                                    } => {
                                        debug!(
                                            "[PLASTICITY-CMD] Injecting memory neuron id={} area_idx={} potential={} pattern={}",
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
                                                "[PLASTICITY-CMD] Memory neuron staged to FCL (id={}, area_idx={}, pattern={})",
                                                neuron_id, area_idx, pattern_hash
                                            );
                                        } else {
                                            warn!(
                                                "[PLASTICITY-CMD] Missing cortical ID for area_idx={}",
                                                area_idx
                                            );
                                        }
                                    }
                                    PlasticityCommand::UpdateWeightsDelta { .. } => {
                                        // TODO: Implement STDP weight updates
                                        warn!("[PLASTICITY-CMD] STDP weight updates not yet implemented");
                                    }
                                    PlasticityCommand::UpdateStateCounters { .. } => {
                                        // Stats tracking only, no NPU action needed
                                    }
                                    PlasticityCommand::ResetMemoryNeuronsInArea { cortical_idx } => {
                                        debug!(
                                            "[PLASTICITY-CMD] Memory neurons reset in cortical area {}",
                                            cortical_idx
                                        );
                                        // Memory neuron reset is handled by PlasticityService.reset_memory_neurons_in_area()
                                        // This command is for notification/logging only
                                    }
                                }
                            }
                        }
                    }
                })
                .expect("Failed to spawn plasticity command processor thread");

            info!("    ✓ Plasticity command processor running");
        } else {
            info!("    ✓ Plasticity command processor running in post-burst callback");
        }
    }

    // TODO: Wire NPU to agent_handler sensory stream in new architecture
    info!("  ⚠ NPU ↔ Agent Handler sensory wiring TODO");

    // Data streams start DYNAMICALLY based on:
    // 1. Genome loaded (NPU has neurons)
    // 2. At least one agent with matching capability registered
    info!("  ⏸️  Data streams will start automatically when conditions are met:");
    info!("      - Sensory: genome loaded + sensory agent registered");
    info!("      - Motor: genome loaded + motor agent registered");
    info!("      - Visualization: genome loaded + viz agent registered");

    info!("");
    info!("🚀 FEAGI server is running!");
    info!(
        "   REST API (advertised): http://{}:{}",
        config.api.advertised_host, api_port
    );
    info!("   Press Ctrl+C to stop");
    info!("");

    // Wait for shutdown signal using tokio's signal handling (recommended approach)
    info!("Waiting for shutdown signal...");

    // Wait for shutdown signal (polling loop already started after initialization)
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

    info!("  Stopping Agent Handler...");
    // TODO: Implement graceful shutdown for agent_handler
    info!("    ✓ Agent handler shutdown TODO");

    info!("  Stopping API server...");
    // Trigger graceful shutdown for axum server
    let _ = shutdown_tx_api.send(());

    // Wait for server to finish, with a timeout
    match tokio::time::timeout(tokio::time::Duration::from_secs(5), api_handle).await {
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
    info!(
        "  API bind: {}:{} (advertised: {}:{})",
        config.api.bind_host, config.api.port, config.api.advertised_host, config.api.port
    );
    info!(
        "  ZMQ bind host: {} (advertised host: {})",
        config.zmq.bind_host, config.zmq.advertised_host
    );
    info!("  Ports:");
    info!("    - Sensory: {}", config.ports.zmq_sensory_port);
    info!("    - Motor: {}", config.ports.zmq_motor_port);
    info!(
        "    - Visualization: {}",
        config.ports.zmq_visualization_port
    );
    info!("  Neural:");
    info!(
        "    - Burst timestep: {}ms",
        config.neural.burst_engine_timestep
    );
    info!("    - Batch size: {}", config.neural.batch_size);
    info!("  Resources:");
    info!("    - GPU enabled: {}", config.resources.use_gpu);
    info!("    - Max neurons: {}", config.connectome.neuron_space);
}

/// Print FEAGI banner
fn print_banner() {
    println!(
        r#"
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
"#
    );
}

#[cfg(test)]
mod npu_trace_resolve_tests {
    #[test]
    fn cortical_trace_cli_disagrees_with_id_lookup_cases() {
        assert!(!super::cortical_trace_cli_disagrees_with_id_lookup(
            None,
            Some(3)
        ));
        assert!(!super::cortical_trace_cli_disagrees_with_id_lookup(
            Some(2),
            None
        ));
        assert!(!super::cortical_trace_cli_disagrees_with_id_lookup(
            Some(2),
            Some(2)
        ));
        assert!(super::cortical_trace_cli_disagrees_with_id_lookup(
            Some(2),
            Some(3)
        ));
    }
}
