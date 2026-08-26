//! # FEAGI - Framework for Evolutionary Artificial General Intelligence
//!
//! Full-featured FEAGI server application with REST API, ZMQ streams, and neural processing.
//!
//! ## Features
//! - REST API (HTTP) for brain management and control
//! - ZMQ and WebSocket streams for sensory input, motor output and visualization
//! - Agent registration and management
//! - Configuration-driven (no hardcoded values)
//!
//! ## Neural processing status
//!
//! The old NPU (`feagi-npu-burst-engine`, `feagi-npu-plasticity` and the `ConnectomeManager` they
//! were wired to) has been removed ahead of integrating the rewritten NPU through
//! `feagi_npu::wnpu::WrappedNeuronProcessingUnit`. Every interface below still starts and every
//! route still resolves, but operations that need a running engine answer with a not-implemented
//! error. See `feagi::stub_services`.
//!
//! `/v1/system/health_check` is an exception, because it describes the brain rather than driving it:
//! it is served from what the BDU developed, via `feagi::brain_development`.
//!
//! ## License
//! Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

use feagi::brain_development::{develop_genome_file, BduAnalyticsService, DevelopedBrain};
use feagi::network_provider::FeagiNetworkConnectionInfoProvider;
use feagi::stub_services::{
    StubAgentService, StubConnectomeService, StubGenomeService, StubNeuronService,
    StubRuntimeService, StubSnapshotService, StubSystemService,
};
use feagi_agent::command_and_control::agent_embodiment_configuration_message::AgentEmbodimentConfigurationMessage;
use feagi_agent::command_and_control::FeagiMessage;
use feagi_api::common::agent_registration::{
    auto_create_cortical_areas_from_device_registrations,
    derive_motor_cortical_ids_from_device_registrations,
    derive_sensory_cortical_ids_from_device_registrations,
};
use feagi_api::endpoints::network::NetworkConnectionInfoProvider;
use feagi_api::transports::http::server::{create_http_server, ApiState};
use feagi_config::{load_config, validate_config, FeagiConfig};
use feagi_io::{AgentID, SensoryIntakeQueue};
use feagi_observability::{init_logging_default, parse_debug_flags};
use feagi_services::traits::agent_service::AgentService;
use feagi_services::traits::SystemService as SystemServiceTrait;
use feagi_services::{
    AnalyticsService, ConnectomeService, GenomeService, NeuronService, RuntimeService,
    SnapshotService,
};
use feagi_state_manager::StateManager;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use feagi_npu::standard::npu::npu_target_frequency::NPUTargetFrequency;
use feagi_npu::standard::wnpu::wnpu::WrappedNeuronProcessingUnit;

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

/// Log sensory `device_registrations` content relevant to SmartIMU IPU auto-create diagnostics.
fn log_smart_imu_io_auto_hints(device_regs: &serde_json::Value, context: &str) {
    let Some(input) = device_regs
        .get("input_units_and_encoder_properties")
        .and_then(|v| v.as_object())
    else {
        warn!(
            target: "feagi-rs",
            "[IO-AUTO] {context}: missing `input_units_and_encoder_properties` — \
             no sensory units in this payload; SmartIMU (and other IPU) cannot be auto-created from it"
        );
        return;
    };

    let mut keys: Vec<&String> = input.keys().collect();
    keys.sort();
    info!(
        target: "feagi-rs",
        "[IO-AUTO] {context}: input sensory unit keys: {:?}",
        keys
    );

    match input.get("SmartIMU") {
        None => {
            warn!(
                target: "feagi-rs",
                "[IO-AUTO] {context}: no `SmartIMU` key under input_units — \
                 connector export has no SmartIMU register call (ROS bridge mapping / SDK / cache export). \
                 Motor-only or vision-only payloads omit this."
            );
        }
        Some(smart_val) => {
            let Some(arr) = smart_val.as_array() else {
                warn!(
                    target: "feagi-rs",
                    "[IO-AUTO] {context}: `SmartIMU` value is not an array — invalid device_registrations shape"
                );
                return;
            };
            for (i, entry) in arr.iter().enumerate() {
                let Some(pair) = entry.as_array() else {
                    warn!(
                        target: "feagi-rs",
                        "[IO-AUTO] {context}: SmartIMU[{i}] is not a [unit_def, encoder_properties] pair"
                    );
                    continue;
                };
                let unit_def = pair.first();
                let dg_len = unit_def
                    .and_then(|u| u.get("device_grouping"))
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let group_ix = unit_def.and_then(|u| u.get("cortical_unit_index"));
                info!(
                    target: "feagi-rs",
                    "[IO-AUTO] {context}: SmartIMU[{i}] cortical_unit_index={group_ix:?} device_grouping_len={dg_len}"
                );
                if dg_len == 0 {
                    warn!(
                        target: "feagi-rs",
                        "[IO-AUTO] {context}: SmartIMU[{i}] has EMPTY device_grouping — \
                         feagi-api auto_create skips this sensory unit (no IPU area from this entry)"
                    );
                }
            }
        }
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

    /// Enable debug logging for specific crates
    /// Example: --debug feagi-api --debug feagi-services
    /// Or use: --debug-feagi-api --debug-feagi-services
    /// Use --debug-all to enable debug for all crates
    #[arg(long, action = clap::ArgAction::Append)]
    debug: Vec<String>,

    /// Enable debug logging for all crates
    #[arg(long)]
    debug_all: bool,
    // Note: --debug-{crate-name} flags are parsed automatically via parse_debug_flags()
    //
    // The `--precision` and `--npu-trace-*` flags were removed with the old NPU. Equivalent
    // instrumentation must be re-introduced against the new NPU's own tracing surface.
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

    warn!("⚠️  Neural processing is offline: the old NPU has been removed.");
    warn!(
        "    Transports and the REST API are live; neural operations return 501/not-implemented."
    );

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
    let components = initialize_components(&config).await?;
    info!("✓ Core components initialized");

    // Start agent handler polling loop IMMEDIATELY (servers need polling to accept connections)
    let agent_handler_for_loop = Arc::clone(&components.agent_handler);
    let shutdown_flag_for_polling = Arc::clone(&shutdown_flag);
    let runtime_service_for_polling = components.runtime_service.clone();
    let sensory_intake_queue_for_polling = Arc::clone(&components.sensory_intake_queue);
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
                            log_smart_imu_io_auto_hints(
                                device_regs,
                                "before AgentConfiguration auto_create",
                            );
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
                                        "[MOTOR-REG] ApiState not yet available; \
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
                let pending_agent_configurations =
                    handler_guard.drain_pending_agent_configurations();

                drop(handler_guard);
                if !pending_agent_configurations.is_empty() {
                    if let Some(api) = api_state_holder.lock().unwrap().as_ref() {
                        for (session_id, device_regs) in &pending_agent_configurations {
                            info!(
                                "[MOTOR-REG] Processing queued AgentConfiguration for session {}",
                                session_id.to_base64()
                            );
                            rt_handle.block_on(
                                auto_create_cortical_areas_from_device_registrations(
                                    api.as_ref(),
                                    device_regs,
                                ),
                            );
                        }
                    } else {
                        warn!(
                            "[MOTOR-REG] ApiState not yet available; queued AgentConfiguration auto-create deferred"
                        );
                    }
                }

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
                                    warn!(
                                        "[MOTOR-REG] Could not derive sensory IDs while checking auto-create completion for descriptor {:?}: {}",
                                        agent_descriptor, e
                                    );
                                }
                            }

                            if derivation_failed || expected_ids.is_empty() {
                                true
                            } else {
                                !all_cortical_areas_present(
                                    &rt_handle,
                                    &api_state_holder,
                                    &expected_ids,
                                )
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
                // Gate motor registration attempts until API state is ready. During startup/restart,
                // agents can connect before that, which causes repeated unresolved motor-ID
                // resolution attempts.
                //
                // The pre-refactor fallback — deriving motor areas from the ConnectomeManager's
                // output areas when an agent sends no device_registrations — is gone with the old
                // NPU. Agents without device_registrations are deferred instead.
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
                                "[MOTOR-REG] No device registrations for agent '{}' (descriptor {:?}); cannot resolve motor areas without an NPU-backed connectome",
                                agent_id,
                                agent_descriptor
                            );
                            Vec::new()
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
                // device_registrations. This can be expensive and must not hold `agent_handler`.
                //
                // Guard: skip entirely when the brain is not initialized. Cortical area creation
                // requires an active genome; without one this only floods the log with repeated
                // "No genome loaded" warnings. With no NPU attached the brain is never initialized,
                // so this branch stays dormant until the new NPU lands.
                let genome_is_ready = brain_is_initialized(&rt_handle, &api_state_holder);
                if !device_regs_to_auto_create.is_empty() && genome_is_ready {
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
                                    warn!(
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
                            // Mark as completed only after expected cortical IDs exist. This avoids
                            // false-positive completion when initial payload/state causes
                            // auto_create to no-op.
                            let all_expected_present = expected_ids.iter().all(|id_b64| {
                                rt_handle
                                    .block_on(api.connectome_service.cortical_area_exists(id_b64))
                                    .unwrap_or(false)
                            });
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
                        debug!("[MOTOR-REG] Auto-create deferred: ApiState not yet available");
                    }
                }

                // Feed transport-agnostic sensory intake (any transport that received data).
                // IMPORTANT ORDERING: this runs after registration-driven auto-create work so
                // first sensory payloads do not race ahead of cortical area provisioning.
                //
                // Drain up to a bounded per-cycle budget and keep only the newest payload
                // so sustained streams do not accumulate stale frames in memory.
                //
                // The queue currently has no consumer: the burst engine used to drain it. It is
                // depth-bounded, so payloads are simply discarded until an NPU is attached.
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

                // Unregister stale agents (e.g. descriptor replacement).
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

/// Report whether every `cortical_id` (base64) is present in the connectome.
///
/// Replaces the pre-refactor `ConnectomeManager::has_cortical_area` lookups: the polling loop now
/// asks the service layer instead of holding a connectome handle.
fn all_cortical_areas_present(
    rt_handle: &tokio::runtime::Handle,
    api_state_holder: &Arc<Mutex<Option<Arc<ApiState>>>>,
    cortical_ids: &std::collections::HashSet<String>,
) -> bool {
    let Some(api) = api_state_holder.lock().unwrap().as_ref().cloned() else {
        return false;
    };
    cortical_ids.iter().all(|id_b64| {
        rt_handle
            .block_on(api.connectome_service.cortical_area_exists(id_b64))
            .unwrap_or(false)
    })
}

/// Report whether a genome has been developed into a usable brain.
fn brain_is_initialized(
    rt_handle: &tokio::runtime::Handle,
    api_state_holder: &Arc<Mutex<Option<Arc<ApiState>>>>,
) -> bool {
    let Some(api) = api_state_holder.lock().unwrap().as_ref().cloned() else {
        return false;
    };
    rt_handle
        .block_on(api.analytics_service.is_brain_initialized())
        .unwrap_or(false)
}

/// Core FEAGI components (matches components.rs)
struct FeagiComponents {
    /// Wrapped NPU shared with every stub-service adapter and the runtime service. Held behind a
    /// `parking_lot::Mutex` because WNPU's mutation surface takes `&mut self`; every call site
    /// locks briefly and never crosses an `await`.
    neuron_processing_unit: Arc<parking_lot::Mutex<WrappedNeuronProcessingUnit>>,
    /// What the BDU has developed; the source of the health endpoint's brain figures.
    developed_brain: Arc<DevelopedBrain>,
    runtime_service: Arc<StubRuntimeService>,
    agent_handler: Arc<std::sync::Mutex<feagi_agent::server::FeagiAgentHandler>>,
    /// Transport-agnostic sensory queue (feagi-io); polling loop pushes here when agents send sensory
    sensory_intake_queue: Arc<SensoryIntakeQueue>,
}

/// Initialize all core FEAGI components
async fn initialize_components(config: &FeagiConfig) -> Result<FeagiComponents> {
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

    let npu = WrappedNeuronProcessingUnit::new(
        feagi_data::quantization_levels::feagi_index_quantization::FeagiIndexQuantizationLevel::Genomic,
        vec![] // TODo
    ).context("Failed to construct wrapped NPU")?;
    // Shared behind a `Mutex` so the stub service adapters (`stub_services`) can forward each
    // API-layer call into the wrapped NPU without holding a full clone or grabbing the ownership
    // path used by the burst engine.
    let npu = Arc::new(parking_lot::Mutex::new(npu));

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

    let burst_hz = 1.0 / config.neural.burst_engine_timestep;

    let burst_frequency = NPUTargetFrequency::new_from_frequency(burst_hz);

    // WNPU `run_at_frequency` is idempotent-on-restart and safe to call before agents connect.
    // Any error is logged rather than treated as fatal because WNPU is still a placeholder and
    // startup should stay reachable so the transports come up.
    if let Err(e) = npu.lock().run_at_frequency(burst_frequency) {
        warn!(
            "    ⚠ Wrapped NPU rejected initial run_at_frequency({} Hz): {}",
            burst_hz, e
        );
    }

    let runtime_service = Arc::new(StubRuntimeService::new(burst_hz, Arc::clone(&npu)));
    info!(
        "    ✓ Runtime service created and wired to the wrapped NPU (target {:.0}Hz)",
        burst_hz
    );

    let developed_brain = Arc::new(DevelopedBrain::new(&config.connectome));
    info!(
        "    ✓ Brain development state created (capacity: {} neurons, {} synapses)",
        config.connectome.neuron_space, config.connectome.synapse_space
    );

    // Transport-agnostic sensory intake (feagi-io): the polling loop feeds it. The burst engine
    // used to drain it; nothing does until the new NPU is attached.
    let sensory_intake_queue = Arc::new(SensoryIntakeQueue::new());
    info!("    ⚠ Sensory intake queue has no consumer (awaiting new NPU)");
    info!("      ✓ Sensory: transports → queue → (no consumer)");
    info!("      ⚠ Motor: no burst engine to publish from");
    info!("      ⚠ Visualization: no burst engine to publish from");

    Ok(FeagiComponents {
        neuron_processing_unit: npu,
        developed_brain,
        runtime_service,
        agent_handler,
        sensory_intake_queue,
    })
}

/// Start all FEAGI services (API + transports)
async fn start_services(
    components: FeagiComponents,
    config: &FeagiConfig,
    args: &Args,
    shutdown_flag: Arc<AtomicBool>,
    api_state_holder: Arc<Mutex<Option<Arc<ApiState>>>>,
) -> Result<()> {
    // Signal handler already setup in main()

    info!("  Creating service layer (analytics from BDU, rest wired to wrapped NPU)...");

    let wnpu_handle = Arc::clone(&components.neuron_processing_unit);
    let genome_service = Arc::new(StubGenomeService::new(Arc::clone(&wnpu_handle)));
    let connectome_service = Arc::new(StubConnectomeService::new(Arc::clone(&wnpu_handle)));
    // Analytics is not a stub: `/v1/system/health_check` is served from the BDU's development
    // report plus the runtime service's burst state. See `feagi::brain_development`.
    let analytics_service = Arc::new(BduAnalyticsService::new(
        Arc::clone(&components.developed_brain),
        components.runtime_service.clone() as Arc<dyn RuntimeService + Send + Sync>,
    ));
    let neuron_service = Arc::new(StubNeuronService::new(Arc::clone(&wnpu_handle)));
    let snapshot_service = Arc::new(StubSnapshotService::new(Arc::clone(&wnpu_handle)));
    let agent_service = Arc::new(StubAgentService::new(Arc::clone(&wnpu_handle)));
    let system_service = Arc::new(StubSystemService::new(
        feagi::collect_version_info(),
        Arc::clone(&wnpu_handle),
    ));

    info!("    ✓ Services created (health check live; other neural operations forward to WNPU placeholders)");

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

    let filesystem_data_root = ApiState::filesystem_data_root_from_config(&config.system.data_dir);
    info!(
        "    ✓ Filesystem data root ([system].data_dir / FEAGI_DATA_DIR, else ~/.feagi): {}",
        filesystem_data_root.display()
    );

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

    // Agent handler streams already started during initialization
    info!("  ✓ Agent handler control streams active (registration ready)");

    // Start HTTP API server
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

    // Development is run for its report, not for a simulation: the BDU produces the connectome
    // requests but the wrapped NPU has no entry point to accept them, so the brain is described to
    // callers (health, readiness) without being simulated. A genome that fails to develop is
    // reported and startup continues, because the API and transports are still useful without one.
    if let Some(genome_path) = &args.genome {
        info!("  Developing genome '{}'...", genome_path.display());
        match develop_genome_file(genome_path, &components.developed_brain) {
            Ok(report) => info!(
                "    ✓ Brain developed: {} cortical areas, {} neurons",
                report.areas_added, report.neurons_added
            ),
            Err(e) => {
                error!(
                    "  ✗ Could not develop genome '{}': {:#}",
                    genome_path.display(),
                    e
                );
                error!("    Continuing with an undeveloped brain.");
            }
        }
    } else {
        info!("  No genome specified, starting with an undeveloped brain");
    }

    // Make ApiState available to polling loop for auto_create when device_registrations arrive.
    *api_state_holder.lock().unwrap() = Some(Arc::new(api_state.clone()));

    // Start burst engine via service layer.
    info!("  Starting burst engine...");
    match components.runtime_service.start().await {
        Ok(_) => info!("    ✓ Burst engine running"),
        Err(e) => warn!("    ⚠ Burst engine not started: {}", e),
    }

    info!("");
    info!("🚀 FEAGI server is running!");
    info!(
        "   REST API (advertised): http://{}:{}",
        config.api.advertised_host, api_port
    );
    info!("   Neural processing: OFFLINE (awaiting new NPU integration)");
    info!("   Press Ctrl+C to stop");
    info!("");

    // Wait for shutdown signal (polling loop already started after initialization)
    info!("Waiting for shutdown signal...");

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
    match components.runtime_service.stop().await {
        Ok(_) => info!("    ✓ Burst engine stopped"),
        Err(e) => warn!("    ⚠ Burst engine stop reported: {}", e),
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
