// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Placeholder service-layer implementations used while the NPU is being replaced.
//!
//! The old NPU stack (`feagi-npu-burst-engine` + `ConnectomeManager`) backed every
//! `feagi-services` implementation that FEAGI's REST API, ZMQ streams and WebSocket streams were
//! wired to. That stack is gone and `feagi-services` no longer ships built-in implementations
//! (`builtin-impls` is off because they target the pre-refactor crate layout), so adapters are
//! expected to supply their own.
//!
//! These implementations keep every interface reachable — the API server binds, routes resolve,
//! agents register and transports poll — while any operation that needs neural state answers with
//! [`ServiceError::NotImplemented`]. Replace them one trait at a time as
//! `feagi_npu::wnpu::WrappedNeuronProcessingUnit` grows the corresponding capability.
//!
//! Two deliberate exceptions return real data because they never needed an NPU:
//!
//! - [`StubSystemService::get_version`] reports the crate versions linked into this binary.
//! - The subscription bookkeeping on [`StubRuntimeService`] accepts registrations and reports the
//!   configured burst frequency, so the agent polling loop reaches a steady state instead of
//!   retrying every cycle. Nothing is published until an NPU is attached.
//!
//! `AnalyticsService` is deliberately absent: it is what `/v1/system/health_check` reads, and it is
//! answered for real from the BDU by [`crate::brain_development::BduAnalyticsService`].

use axum::async_trait;
use feagi_services::traits::agent_service::{
    AgentError, AgentProperties, AgentRegistration, AgentRegistrationResponse, AgentResult,
    AgentService, HeartbeatRequest, ManualStimulationMode as AgentManualStimulationMode,
};
use feagi_services::traits::runtime_service::ManualStimulationMode;
use feagi_services::traits::SystemService;
use feagi_services::types::*;
use feagi_services::{
    ConnectomeService, GenomeService, NeuronService, RuntimeService, SnapshotCreateOptions,
    SnapshotMetadata, SnapshotService,
};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Reason attached to every `NotImplemented` error these services return.
const PENDING_NPU: &str =
    "the old NPU has been removed; this operation returns once the new NPU is integrated";

fn pending(operation: &str) -> ServiceError {
    ServiceError::NotImplemented(format!("{operation}: {PENDING_NPU}"))
}

fn pending_agent(operation: &str) -> AgentError {
    AgentError::ServiceUnavailable(format!("{operation}: {PENDING_NPU}"))
}

// ============================================================================
// GENOME
// ============================================================================

#[derive(Default)]
pub struct StubGenomeService;

#[async_trait]
impl GenomeService for StubGenomeService {
    async fn load_genome(&self, _params: LoadGenomeParams) -> ServiceResult<GenomeInfo> {
        Err(pending("load_genome"))
    }

    async fn save_genome(&self, _params: SaveGenomeParams) -> ServiceResult<String> {
        Err(pending("save_genome"))
    }

    async fn export_region_genome(&self, _region_id: String) -> ServiceResult<String> {
        Err(pending("export_region_genome"))
    }

    async fn get_genome_info(&self) -> ServiceResult<GenomeInfo> {
        Err(pending("get_genome_info"))
    }

    async fn validate_genome(&self, _json_str: String) -> ServiceResult<bool> {
        Err(pending("validate_genome"))
    }

    async fn reset_connectome(&self) -> ServiceResult<()> {
        Err(pending("reset_connectome"))
    }

    async fn update_cortical_area(
        &self,
        _cortical_id: &str,
        _changes: HashMap<String, serde_json::Value>,
    ) -> ServiceResult<CorticalAreaInfo> {
        Err(pending("update_cortical_area"))
    }

    async fn create_cortical_areas(
        &self,
        _params: Vec<CreateCorticalAreaParams>,
    ) -> ServiceResult<Vec<CorticalAreaInfo>> {
        Err(pending("create_cortical_areas"))
    }
}

// ============================================================================
// CONNECTOME
// ============================================================================

#[derive(Default)]
pub struct StubConnectomeService;

#[async_trait]
impl ConnectomeService for StubConnectomeService {
    async fn create_cortical_area(
        &self,
        _params: CreateCorticalAreaParams,
    ) -> ServiceResult<CorticalAreaInfo> {
        Err(pending("create_cortical_area"))
    }

    async fn update_cortical_area(
        &self,
        _cortical_id: &str,
        _params: UpdateCorticalAreaParams,
    ) -> ServiceResult<CorticalAreaInfo> {
        Err(pending("update_cortical_area"))
    }

    async fn delete_cortical_area(&self, _cortical_id: &str) -> ServiceResult<()> {
        Err(pending("delete_cortical_area"))
    }

    async fn get_cortical_area(&self, _cortical_id: &str) -> ServiceResult<CorticalAreaInfo> {
        Err(pending("get_cortical_area"))
    }

    async fn list_cortical_areas(&self) -> ServiceResult<Vec<CorticalAreaInfo>> {
        Err(pending("list_cortical_areas"))
    }

    async fn get_cortical_area_ids(&self) -> ServiceResult<Vec<String>> {
        Err(pending("get_cortical_area_ids"))
    }

    /// Reports `false` rather than an error: callers use this as a presence probe and a missing
    /// connectome genuinely contains no cortical areas.
    async fn cortical_area_exists(&self, _cortical_id: &str) -> ServiceResult<bool> {
        Ok(false)
    }

    async fn get_cortical_area_properties(
        &self,
        _cortical_id: &str,
    ) -> ServiceResult<HashMap<String, serde_json::Value>> {
        Err(pending("get_cortical_area_properties"))
    }

    async fn get_all_cortical_area_properties(
        &self,
    ) -> ServiceResult<Vec<HashMap<String, serde_json::Value>>> {
        Err(pending("get_all_cortical_area_properties"))
    }

    async fn get_neuron_properties(
        &self,
        _neuron_id: u64,
    ) -> ServiceResult<HashMap<String, serde_json::Value>> {
        Err(pending("get_neuron_properties"))
    }

    async fn create_brain_region(
        &self,
        _params: CreateBrainRegionParams,
    ) -> ServiceResult<BrainRegionInfo> {
        Err(pending("create_brain_region"))
    }

    async fn delete_brain_region(&self, _region_id: &str) -> ServiceResult<()> {
        Err(pending("delete_brain_region"))
    }

    async fn update_brain_region(
        &self,
        _region_id: &str,
        _properties: HashMap<String, serde_json::Value>,
    ) -> ServiceResult<BrainRegionInfo> {
        Err(pending("update_brain_region"))
    }

    async fn get_brain_region(&self, _region_id: &str) -> ServiceResult<BrainRegionInfo> {
        Err(pending("get_brain_region"))
    }

    async fn list_brain_regions(&self) -> ServiceResult<Vec<BrainRegionInfo>> {
        Err(pending("list_brain_regions"))
    }

    async fn get_brain_region_ids(&self) -> ServiceResult<Vec<String>> {
        Err(pending("get_brain_region_ids"))
    }

    async fn brain_region_exists(&self, _region_id: &str) -> ServiceResult<bool> {
        Ok(false)
    }

    async fn get_root_region_id(&self) -> ServiceResult<Option<String>> {
        Err(pending("get_root_region_id"))
    }

    async fn get_morphologies(&self) -> ServiceResult<HashMap<String, MorphologyInfo>> {
        Err(pending("get_morphologies"))
    }

    async fn create_morphology(
        &self,
        _morphology_id: String,
        _morphology: feagi_evolutionary::Morphology,
    ) -> ServiceResult<()> {
        Err(pending("create_morphology"))
    }

    async fn update_morphology(
        &self,
        _morphology_id: String,
        _morphology: feagi_evolutionary::Morphology,
    ) -> ServiceResult<()> {
        Err(pending("update_morphology"))
    }

    async fn delete_morphology(&self, _morphology_id: &str) -> ServiceResult<()> {
        Err(pending("delete_morphology"))
    }

    async fn rename_morphology(&self, _old_id: &str, _new_id: &str) -> ServiceResult<()> {
        Err(pending("rename_morphology"))
    }

    async fn update_cortical_mapping(
        &self,
        _src_area_id: String,
        _dst_area_id: String,
        _mapping_data: Vec<serde_json::Value>,
    ) -> ServiceResult<usize> {
        Err(pending("update_cortical_mapping"))
    }

    // `connectome-io` is not optional in practice: feagi-api requires it, so cargo always unifies
    // the feature on. Implemented unconditionally because feagi-rs has no matching feature flag.
    async fn export_connectome(&self) -> ServiceResult<ConnectomeSnapshot> {
        Err(pending("export_connectome"))
    }

    async fn import_connectome(&self, _snapshot: ConnectomeSnapshot) -> ServiceResult<()> {
        Err(pending("import_connectome"))
    }
}

// ============================================================================
// NEURON
// ============================================================================

#[derive(Default)]
pub struct StubNeuronService;

#[async_trait]
impl NeuronService for StubNeuronService {
    async fn create_neuron(&self, _params: CreateNeuronParams) -> ServiceResult<NeuronInfo> {
        Err(pending("create_neuron"))
    }

    async fn delete_neuron(&self, _neuron_id: u64) -> ServiceResult<()> {
        Err(pending("delete_neuron"))
    }

    async fn get_neuron(&self, _neuron_id: u64) -> ServiceResult<NeuronInfo> {
        Err(pending("get_neuron"))
    }

    async fn get_neuron_at_coordinates(
        &self,
        _cortical_id: &str,
        _coordinates: (u32, u32, u32),
    ) -> ServiceResult<Option<NeuronInfo>> {
        Err(pending("get_neuron_at_coordinates"))
    }

    async fn list_neurons_in_area(
        &self,
        _cortical_id: &str,
        _limit: Option<usize>,
    ) -> ServiceResult<Vec<NeuronInfo>> {
        Err(pending("list_neurons_in_area"))
    }

    async fn get_neuron_count(&self, _cortical_id: &str) -> ServiceResult<usize> {
        Err(pending("get_neuron_count"))
    }

    async fn neuron_exists(&self, _neuron_id: u64) -> ServiceResult<bool> {
        Ok(false)
    }
}

// ============================================================================
// RUNTIME
// ============================================================================

/// Runtime control with no burst engine behind it.
///
/// Subscription registration is accepted and remembered so the agent polling loop converges;
/// the recorded sets are what the future NPU integration needs in order to start publishing.
pub struct StubRuntimeService {
    configured_frequency_hz: f64,
    motor_subscriptions: RwLock<HashMap<String, Vec<String>>>,
    visualization_subscriptions: RwLock<HashSet<String>>,
}

impl StubRuntimeService {
    pub fn new(configured_frequency_hz: f64) -> Self {
        Self {
            configured_frequency_hz,
            motor_subscriptions: RwLock::new(HashMap::new()),
            visualization_subscriptions: RwLock::new(HashSet::new()),
        }
    }
}

#[async_trait]
impl RuntimeService for StubRuntimeService {
    async fn start(&self) -> ServiceResult<()> {
        Err(pending("start"))
    }

    async fn stop(&self) -> ServiceResult<()> {
        // Stopping a burst engine that was never started is a no-op, not a failure. Shutdown paths
        // depend on this so Ctrl+C stays clean.
        Ok(())
    }

    async fn pause(&self) -> ServiceResult<()> {
        Err(pending("pause"))
    }

    async fn resume(&self) -> ServiceResult<()> {
        Err(pending("resume"))
    }

    async fn step(&self) -> ServiceResult<()> {
        Err(pending("step"))
    }

    async fn get_status(&self) -> ServiceResult<RuntimeStatus> {
        Ok(RuntimeStatus {
            is_running: false,
            is_paused: false,
            frequency_hz: self.configured_frequency_hz,
            burst_count: 0,
            current_rate_hz: 0.0,
            last_burst_neuron_count: 0,
            avg_burst_time_ms: 0.0,
        })
    }

    async fn set_frequency(&self, _frequency_hz: f64) -> ServiceResult<()> {
        Err(pending("set_frequency"))
    }

    async fn get_burst_count(&self) -> ServiceResult<u64> {
        Ok(0)
    }

    async fn reset_burst_count(&self) -> ServiceResult<()> {
        Ok(())
    }

    async fn get_fcl_snapshot(&self) -> ServiceResult<Vec<(u64, f32)>> {
        Err(pending("get_fcl_snapshot"))
    }

    async fn get_fcl_snapshot_with_cortical_idx(&self) -> ServiceResult<Vec<(u64, u32, f32)>> {
        Err(pending("get_fcl_snapshot_with_cortical_idx"))
    }

    async fn get_fire_queue_sample(
        &self,
    ) -> ServiceResult<HashMap<u32, (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<f32>)>> {
        Err(pending("get_fire_queue_sample"))
    }

    async fn get_fire_ledger_configs(&self) -> ServiceResult<Vec<(u32, usize)>> {
        Err(pending("get_fire_ledger_configs"))
    }

    async fn configure_fire_ledger_window(
        &self,
        _cortical_idx: u32,
        _window_size: usize,
    ) -> ServiceResult<()> {
        Err(pending("configure_fire_ledger_window"))
    }

    async fn get_fcl_sampler_config(&self) -> ServiceResult<(f64, u32)> {
        Err(pending("get_fcl_sampler_config"))
    }

    async fn set_fcl_sampler_config(
        &self,
        _frequency: Option<f64>,
        _consumer: Option<u32>,
    ) -> ServiceResult<()> {
        Err(pending("set_fcl_sampler_config"))
    }

    async fn get_area_fcl_sample_rate(&self, _area_id: u32) -> ServiceResult<f64> {
        Err(pending("get_area_fcl_sample_rate"))
    }

    async fn set_area_fcl_sample_rate(
        &self,
        _area_id: u32,
        _sample_rate: f64,
    ) -> ServiceResult<()> {
        Err(pending("set_area_fcl_sample_rate"))
    }

    async fn inject_sensory_by_coordinates(
        &self,
        _cortical_id: &str,
        _xyzp_data: &[(u32, u32, u32, f32)],
        _mode: ManualStimulationMode,
    ) -> ServiceResult<usize> {
        Err(pending("inject_sensory_by_coordinates"))
    }

    async fn register_motor_subscriptions(
        &self,
        agent_id: &str,
        cortical_ids: Vec<String>,
        _rate_hz: f64,
    ) -> ServiceResult<()> {
        self.motor_subscriptions
            .write()
            .insert(agent_id.to_string(), cortical_ids);
        Ok(())
    }

    async fn register_visualization_subscriptions(
        &self,
        agent_id: &str,
        _rate_hz: f64,
    ) -> ServiceResult<()> {
        self.visualization_subscriptions
            .write()
            .insert(agent_id.to_string());
        Ok(())
    }

    fn unregister_motor_subscriptions(&self, agent_id: &str) {
        self.motor_subscriptions.write().remove(agent_id);
    }

    fn unregister_visualization_subscriptions(&self, agent_id: &str) {
        self.visualization_subscriptions.write().remove(agent_id);
    }

    async fn reset_cortical_area_states(
        &self,
        _cortical_indices: &[u32],
    ) -> ServiceResult<Vec<(u32, usize)>> {
        Err(pending("reset_cortical_area_states"))
    }

    fn clear_all_motor_subscriptions(&self) {
        self.motor_subscriptions.write().clear();
    }

    fn clear_all_visualization_subscriptions(&self) {
        self.visualization_subscriptions.write().clear();
    }
}

// ============================================================================
// SYSTEM
// ============================================================================

/// System introspection. Version reporting is real; everything requiring neural state is not.
pub struct StubSystemService {
    version_info: VersionInfo,
}

impl StubSystemService {
    pub fn new(version_info: VersionInfo) -> Self {
        Self { version_info }
    }
}

#[async_trait]
impl SystemService for StubSystemService {
    async fn get_health(&self) -> ServiceResult<HealthStatus> {
        Err(pending("get_health"))
    }

    async fn get_status(&self) -> ServiceResult<SystemStatus> {
        Err(pending("get_status"))
    }

    async fn get_version(&self) -> ServiceResult<VersionInfo> {
        Ok(self.version_info.clone())
    }

    async fn is_initialized(&self) -> ServiceResult<bool> {
        Ok(false)
    }

    async fn get_burst_count(&self) -> ServiceResult<u64> {
        Ok(0)
    }

    async fn get_runtime_stats(&self) -> ServiceResult<RuntimeStats> {
        Err(pending("get_runtime_stats"))
    }

    async fn get_memory_usage(&self) -> ServiceResult<MemoryUsage> {
        Err(pending("get_memory_usage"))
    }

    async fn get_capacity(&self) -> ServiceResult<CapacityInfo> {
        Err(pending("get_capacity"))
    }
}

// ============================================================================
// AGENT
// ============================================================================

/// Agent-facing service. Registration itself is handled by `FeagiAgentHandler` over the
/// transports; this trait's remaining surface reaches into neural state, so it reports
/// unavailable.
#[derive(Default)]
pub struct StubAgentService;

#[async_trait]
impl AgentService for StubAgentService {
    async fn register_agent(
        &self,
        _registration: AgentRegistration,
    ) -> AgentResult<AgentRegistrationResponse> {
        Err(pending_agent("register_agent"))
    }

    async fn heartbeat(&self, _request: HeartbeatRequest) -> AgentResult<()> {
        Ok(())
    }

    async fn list_agents(&self) -> AgentResult<Vec<String>> {
        Ok(Vec::new())
    }

    async fn get_agent_properties(&self, agent_id: &str) -> AgentResult<AgentProperties> {
        Err(AgentError::NotFound(agent_id.to_string()))
    }

    async fn get_shared_memory_info(
        &self,
    ) -> AgentResult<HashMap<String, HashMap<String, serde_json::Value>>> {
        Ok(HashMap::new())
    }

    async fn deregister_agent(&self, _agent_id: &str) -> AgentResult<()> {
        Ok(())
    }

    async fn manual_stimulation(
        &self,
        _stimulation_payload: HashMap<String, Vec<Vec<i32>>>,
        _mode: AgentManualStimulationMode,
    ) -> AgentResult<HashMap<String, serde_json::Value>> {
        Err(pending_agent("manual_stimulation"))
    }

    fn try_set_runtime_service(&self, _runtime_service: Arc<dyn RuntimeService + Send + Sync>) {}
}

// ============================================================================
// SNAPSHOT
// ============================================================================

#[derive(Default)]
pub struct StubSnapshotService;

#[async_trait]
impl SnapshotService for StubSnapshotService {
    async fn create_snapshot(
        &self,
        _options: SnapshotCreateOptions,
    ) -> ServiceResult<SnapshotMetadata> {
        Err(pending("create_snapshot"))
    }

    async fn restore_snapshot(&self, _snapshot_id: &str) -> ServiceResult<()> {
        Err(pending("restore_snapshot"))
    }

    async fn list_snapshots(&self) -> ServiceResult<Vec<SnapshotMetadata>> {
        Ok(Vec::new())
    }

    async fn delete_snapshot(&self, _snapshot_id: &str) -> ServiceResult<()> {
        Err(pending("delete_snapshot"))
    }

    async fn get_snapshot_artifact(
        &self,
        _snapshot_id: &str,
        _format: &str,
    ) -> ServiceResult<Vec<u8>> {
        Err(pending("get_snapshot_artifact"))
    }
}
