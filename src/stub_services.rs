// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Service-layer adapters that forward every REST/ZMQ/WebSocket call into the wrapped NPU.
//!
//! The old NPU stack (`feagi-npu-burst-engine` + `ConnectomeManager`) backed every
//! `feagi-services` implementation that FEAGI's transports were wired to. That stack is gone and
//! `feagi-services` no longer ships built-in implementations, so adapters are expected to supply
//! their own — this file is that adapter.
//!
//! Every method here holds a shared handle to [`WrappedNeuronProcessingUnit`] and calls whichever
//! WNPU method matches. Where a call needs a richer parameter than WNPU currently accepts, this
//! file constructs a default WNPU-shaped payload and forwards it. Every return value is either
//! synthesized from WNPU's response or built as an empty / defaulted DTO.
//!
//! `AnalyticsService` is deliberately absent from this file: it is what `/v1/system/health_check`
//! reads, and it is answered from the BDU by [`crate::brain_development::BduAnalyticsService`].

use axum::async_trait;
use feagi_data::neurons::wrapped_types::CorticalVoxelDimensionsGenomic;
use feagi_genomic_context::cortical_area::CorticalID;
use feagi_npu::standard::npu::npu_target_frequency::NPUTargetFrequency;
use feagi_npu::standard::wnpu::wnpu::{
    CorticalAreaParameters, WnpuAgentProperties, WnpuBrainRegionInfo, WnpuGenomeMetadata,
    WnpuMorphologyInfo, WrappedNeuronProcessingUnit,
};
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
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Shared handle to the wrapped NPU used by every adapter in this file.
///
/// WNPU is `!Sync` (its mutation surface takes `&mut self`), so it is kept behind a mutex and
/// every call locks briefly. The adapters never `await` while holding the guard, which keeps them
/// compatible with the `#[async_trait]` `Send` requirement.
pub type SharedWnpu = Arc<Mutex<WrappedNeuronProcessingUnit>>;

/// Turn a base64 cortical area ID (as callers send over the wire) into a WNPU [`CorticalID`].
fn parse_cortical_id(cortical_id: &str) -> ServiceResult<CorticalID> {
    CorticalID::try_from_base_64(cortical_id).map_err(|e| {
        ServiceError::InvalidInput(format!("invalid cortical area ID '{cortical_id}': {e}"))
    })
}

/// Build a [`CorticalAreaParameters`] with the fields the WNPU write path currently reads,
/// leaving everything else at zero. WNPU accepts the value verbatim; the adapters do not need to
/// synthesize a full physiology.
fn default_cortical_area_parameters(
    dimensions: (usize, usize, usize),
    neurons_per_voxel: u32,
) -> CorticalAreaParameters {
    CorticalAreaParameters {
        dimensions: CorticalVoxelDimensionsGenomic::new_from_usizes_unchecked(
            dimensions.0,
            dimensions.1,
            dimensions.2,
        ),
        neurons_per_voxel,
        fire_threshold: 0.0,
        fire_threshold_increment_x: 0.0,
        fire_threshold_increment_y: 0.0,
        fire_threshold_increment_z: 0.0,
        fire_threshold_limit: 0.0,
        leak_coefficient: 0.0,
        resting_potential: 0.0,
        refractory_period: 0,
        excitability: 1.0,
        consecutive_fire_limit: 0,
        snooze_period: 0,
        membrane_potential_accumulation: false,
        degeneration: 0.0,
        is_psp_uniform: false,
        is_membrane_potential_driven_psp: false,
    }
}

/// Build a [`CorticalAreaInfo`] from the WNPU-side registry plus any fields we can lift out of a
/// [`CreateCorticalAreaParams`] the caller supplied.
fn cortical_area_info_from_wnpu(
    wnpu: &WrappedNeuronProcessingUnit,
    cortical_id_b64: &str,
    cortical_id: &CorticalID,
) -> CorticalAreaInfo {
    let dimensions = wnpu.cortical_area_dimensions_xyz(cortical_id).unwrap_or((0, 0, 0));
    CorticalAreaInfo {
        cortical_id: cortical_id_b64.to_string(),
        cortical_id_s: cortical_id.to_string(),
        cortical_idx: 0,
        name: String::new(),
        dimensions,
        position: (0, 0, 0),
        area_type: String::new(),
        cortical_group: String::new(),
        cortical_type: String::new(),
        neuron_count: wnpu.cortical_area_neuron_count(cortical_id).unwrap_or(0),
        synapse_count: 0,
        incoming_synapse_count: wnpu.cortical_area_incoming_synapse_count(cortical_id),
        outgoing_synapse_count: wnpu.cortical_area_outgoing_synapse_count(cortical_id),
        neurons_per_voxel: wnpu.cortical_area_neurons_per_voxel(cortical_id).unwrap_or(0),
        ..CorticalAreaInfo::default()
    }
}

/// Overlay the caller-provided `CreateCorticalAreaParams` on top of the WNPU-sourced info so the
/// echo back matches what the caller submitted (name, dimensions, area_type, ...).
fn cortical_area_info_from_create(
    wnpu: &WrappedNeuronProcessingUnit,
    cortical_id_b64: &str,
    cortical_id: &CorticalID,
    params: &CreateCorticalAreaParams,
) -> CorticalAreaInfo {
    let mut info = cortical_area_info_from_wnpu(wnpu, cortical_id_b64, cortical_id);
    info.name = params.name.clone();
    info.dimensions = params.dimensions;
    info.position = params.position;
    info.area_type = params.area_type.clone();
    info.visible = params.visible.unwrap_or(true);
    info.sub_group = params.sub_group.clone();
    if let Some(neurons_per_voxel) = params.neurons_per_voxel {
        info.neurons_per_voxel = neurons_per_voxel;
    }
    info
}

/// Convert WNPU's brain region info into the service-layer DTO.
fn brain_region_info_from_wnpu(info: WnpuBrainRegionInfo) -> BrainRegionInfo {
    BrainRegionInfo {
        region_id: info.region_id,
        name: info.name,
        region_type: info.region_type,
        parent_id: info.parent_id,
        cortical_areas: info.cortical_areas,
        child_regions: info.child_regions,
        properties: info.properties,
    }
}

/// Convert WNPU's morphology info into the service-layer DTO.
fn morphology_info_from_wnpu(info: WnpuMorphologyInfo) -> MorphologyInfo {
    MorphologyInfo {
        morphology_type: info.morphology_type,
        class: info.class,
        parameters: info.parameters,
    }
}

/// Convert WNPU's genome metadata into the service-layer DTO.
fn genome_info_from_wnpu(
    metadata: WnpuGenomeMetadata,
    cortical_area_count: usize,
    brain_region_count: usize,
) -> GenomeInfo {
    GenomeInfo {
        genome_id: metadata.genome_id,
        genome_title: metadata.genome_title,
        version: metadata.version,
        cortical_area_count,
        brain_region_count,
        simulation_timestep: metadata.simulation_timestep,
        genome_num: metadata.genome_num,
        genome_timestamp: metadata.genome_timestamp,
    }
}

/// Convert WNPU's agent property snapshot into the service-layer DTO.
fn agent_properties_from_wnpu(props: WnpuAgentProperties) -> AgentProperties {
    AgentProperties {
        agent_type: props.agent_type,
        agent_ip: props.agent_ip,
        agent_data_port: props.agent_data_port,
        agent_router_address: props.agent_router_address,
        agent_version: props.agent_version,
        controller_version: props.controller_version,
        capabilities: props.capabilities,
        chosen_transport: props.chosen_transport,
    }
}

/// Convert WNPU's snapshot metadata into the service-layer DTO.
fn snapshot_metadata_from_wnpu(
    metadata: feagi_npu::standard::wnpu::wnpu::WnpuSnapshotMetadata,
) -> SnapshotMetadata {
    SnapshotMetadata {
        snapshot_id: metadata.snapshot_id,
        created_at: metadata.created_at,
        name: metadata.name,
        description: metadata.description,
        stateful: metadata.stateful,
        size_bytes: metadata.size_bytes,
    }
}

// ============================================================================
// GENOME
// ============================================================================

pub struct StubGenomeService {
    wnpu: SharedWnpu,
}

impl StubGenomeService {
    pub fn new(wnpu: SharedWnpu) -> Self {
        Self { wnpu }
    }
}

#[async_trait]
impl GenomeService for StubGenomeService {
    async fn load_genome(&self, params: LoadGenomeParams) -> ServiceResult<GenomeInfo> {
        let mut wnpu = self.wnpu.lock();
        wnpu.load_genome_json(&params.json_str)
            .map_err(|e| ServiceError::Backend(format!("wnpu.load_genome_json: {e}")))?;
        let metadata = wnpu.genome_metadata();
        let cortical_area_count = wnpu.cortical_area_ids().len();
        let brain_region_count = wnpu.brain_region_ids().len();
        Ok(genome_info_from_wnpu(
            metadata,
            cortical_area_count,
            brain_region_count,
        ))
    }

    async fn save_genome(&self, _params: SaveGenomeParams) -> ServiceResult<String> {
        self.wnpu
            .lock()
            .save_genome_json()
            .map_err(|e| ServiceError::Backend(format!("wnpu.save_genome_json: {e}")))
    }

    async fn export_region_genome(&self, region_id: String) -> ServiceResult<String> {
        self.wnpu
            .lock()
            .export_region_genome_json(&region_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.export_region_genome_json: {e}")))
    }

    async fn get_genome_info(&self) -> ServiceResult<GenomeInfo> {
        let wnpu = self.wnpu.lock();
        let metadata = wnpu.genome_metadata();
        let cortical_area_count = wnpu.cortical_area_ids().len();
        let brain_region_count = wnpu.brain_region_ids().len();
        Ok(genome_info_from_wnpu(
            metadata,
            cortical_area_count,
            brain_region_count,
        ))
    }

    async fn validate_genome(&self, json_str: String) -> ServiceResult<bool> {
        self.wnpu
            .lock()
            .validate_genome_json(&json_str)
            .map_err(|e| ServiceError::Backend(format!("wnpu.validate_genome_json: {e}")))
    }

    async fn reset_connectome(&self) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .clear_connectome()
            .map_err(|e| ServiceError::Backend(format!("wnpu.clear_connectome: {e}")))
    }

    async fn update_cortical_area(
        &self,
        cortical_id: &str,
        changes: HashMap<String, serde_json::Value>,
    ) -> ServiceResult<CorticalAreaInfo> {
        let parsed = parse_cortical_id(cortical_id)?;
        let mut wnpu = self.wnpu.lock();
        wnpu.apply_cortical_area_property_updates(&parsed, changes)
            .map_err(|e| {
                ServiceError::Backend(format!("wnpu.apply_cortical_area_property_updates: {e}"))
            })?;
        Ok(cortical_area_info_from_wnpu(&wnpu, cortical_id, &parsed))
    }

    async fn create_cortical_areas(
        &self,
        params: Vec<CreateCorticalAreaParams>,
    ) -> ServiceResult<Vec<CorticalAreaInfo>> {
        let mut out = Vec::with_capacity(params.len());
        let mut wnpu = self.wnpu.lock();
        for create_params in params {
            let parsed = parse_cortical_id(&create_params.cortical_id)?;
            let wnpu_params = default_cortical_area_parameters(
                create_params.dimensions,
                create_params.neurons_per_voxel.unwrap_or(1),
            );
            wnpu.add_cortical_area(&parsed, wnpu_params)
                .map_err(|e| ServiceError::Backend(format!("wnpu.add_cortical_area: {e}")))?;
            out.push(cortical_area_info_from_create(
                &wnpu,
                &create_params.cortical_id,
                &parsed,
                &create_params,
            ));
        }
        Ok(out)
    }
}

// ============================================================================
// CONNECTOME
// ============================================================================

pub struct StubConnectomeService {
    wnpu: SharedWnpu,
}

impl StubConnectomeService {
    pub fn new(wnpu: SharedWnpu) -> Self {
        Self { wnpu }
    }
}

#[async_trait]
impl ConnectomeService for StubConnectomeService {
    async fn create_cortical_area(
        &self,
        params: CreateCorticalAreaParams,
    ) -> ServiceResult<CorticalAreaInfo> {
        let parsed = parse_cortical_id(&params.cortical_id)?;
        let mut wnpu = self.wnpu.lock();
        let wnpu_params = default_cortical_area_parameters(
            params.dimensions,
            params.neurons_per_voxel.unwrap_or(1),
        );
        wnpu.add_cortical_area(&parsed, wnpu_params)
            .map_err(|e| ServiceError::Backend(format!("wnpu.add_cortical_area: {e}")))?;
        Ok(cortical_area_info_from_create(
            &wnpu,
            &params.cortical_id,
            &parsed,
            &params,
        ))
    }

    async fn update_cortical_area(
        &self,
        cortical_id: &str,
        params: UpdateCorticalAreaParams,
    ) -> ServiceResult<CorticalAreaInfo> {
        let parsed = parse_cortical_id(cortical_id)?;
        let mut wnpu = self.wnpu.lock();
        let dimensions = params
            .dimensions
            .or_else(|| wnpu.cortical_area_dimensions_xyz(&parsed))
            .unwrap_or((0, 0, 0));
        let neurons_per_voxel = wnpu.cortical_area_neurons_per_voxel(&parsed).unwrap_or(1);
        let wnpu_params = default_cortical_area_parameters(dimensions, neurons_per_voxel);
        wnpu.reconfigure_cortical_area(&parsed, wnpu_params)
            .map_err(|e| ServiceError::Backend(format!("wnpu.reconfigure_cortical_area: {e}")))?;
        let mut info = cortical_area_info_from_wnpu(&wnpu, cortical_id, &parsed);
        if let Some(name) = params.name {
            info.name = name;
        }
        if let Some(position) = params.position {
            info.position = position;
        }
        info.dimensions = dimensions;
        if let Some(area_type) = params.area_type {
            info.area_type = area_type;
        }
        if let Some(visible) = params.visible {
            info.visible = visible;
        }
        Ok(info)
    }

    async fn delete_cortical_area(&self, cortical_id: &str) -> ServiceResult<()> {
        let parsed = parse_cortical_id(cortical_id)?;
        self.wnpu
            .lock()
            .remove_cortical_area(&parsed)
            .map(|_| ())
            .map_err(|e| ServiceError::Backend(format!("wnpu.remove_cortical_area: {e}")))
    }

    async fn get_cortical_area(&self, cortical_id: &str) -> ServiceResult<CorticalAreaInfo> {
        let parsed = parse_cortical_id(cortical_id)?;
        let wnpu = self.wnpu.lock();
        Ok(cortical_area_info_from_wnpu(&wnpu, cortical_id, &parsed))
    }

    async fn list_cortical_areas(&self) -> ServiceResult<Vec<CorticalAreaInfo>> {
        let wnpu = self.wnpu.lock();
        Ok(wnpu
            .cortical_area_ids()
            .into_iter()
            .map(|id| {
                let b64 = id.as_base_64();
                cortical_area_info_from_wnpu(&wnpu, &b64, &id)
            })
            .collect())
    }

    async fn get_cortical_area_ids(&self) -> ServiceResult<Vec<String>> {
        Ok(self
            .wnpu
            .lock()
            .cortical_area_ids()
            .into_iter()
            .map(|id| id.as_base_64())
            .collect())
    }

    async fn cortical_area_exists(&self, cortical_id: &str) -> ServiceResult<bool> {
        let Ok(parsed) = parse_cortical_id(cortical_id) else {
            return Ok(false);
        };
        Ok(self.wnpu.lock().has_cortical_area(&parsed))
    }

    async fn get_cortical_area_properties(
        &self,
        cortical_id: &str,
    ) -> ServiceResult<HashMap<String, serde_json::Value>> {
        let parsed = parse_cortical_id(cortical_id)?;
        let wnpu = self.wnpu.lock();
        let info = cortical_area_info_from_wnpu(&wnpu, cortical_id, &parsed);
        drop(wnpu);
        json_object_from_serialize(&info)
    }

    async fn get_all_cortical_area_properties(
        &self,
    ) -> ServiceResult<Vec<HashMap<String, serde_json::Value>>> {
        let wnpu = self.wnpu.lock();
        let ids = wnpu.cortical_area_ids();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let b64 = id.as_base_64();
            let info = cortical_area_info_from_wnpu(&wnpu, &b64, &id);
            out.push(json_object_from_serialize(&info)?);
        }
        Ok(out)
    }

    async fn get_neuron_properties(
        &self,
        neuron_id: u64,
    ) -> ServiceResult<HashMap<String, serde_json::Value>> {
        Ok(self.wnpu.lock().neuron_properties(neuron_id))
    }

    async fn create_brain_region(
        &self,
        params: CreateBrainRegionParams,
    ) -> ServiceResult<BrainRegionInfo> {
        let mut wnpu = self.wnpu.lock();
        wnpu.add_brain_region(&params.region_id, params.parent_id.as_deref())
            .map_err(|e| ServiceError::Backend(format!("wnpu.add_brain_region: {e}")))?;
        Ok(BrainRegionInfo {
            region_id: params.region_id,
            name: params.name,
            region_type: params.region_type,
            parent_id: params.parent_id,
            cortical_areas: Vec::new(),
            child_regions: Vec::new(),
            properties: params.properties.unwrap_or_default(),
        })
    }

    async fn delete_brain_region(&self, region_id: &str) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .remove_brain_region(region_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.remove_brain_region: {e}")))
    }

    async fn update_brain_region(
        &self,
        region_id: &str,
        properties: HashMap<String, serde_json::Value>,
    ) -> ServiceResult<BrainRegionInfo> {
        let mut wnpu = self.wnpu.lock();
        wnpu.update_brain_region(region_id, properties.clone())
            .map_err(|e| ServiceError::Backend(format!("wnpu.update_brain_region: {e}")))?;
        let mut info = wnpu
            .brain_region_info(region_id)
            .map(brain_region_info_from_wnpu)
            .unwrap_or_else(|| BrainRegionInfo {
                region_id: region_id.to_string(),
                name: String::new(),
                region_type: String::new(),
                parent_id: None,
                cortical_areas: Vec::new(),
                child_regions: Vec::new(),
                properties: HashMap::new(),
            });
        for (key, value) in properties {
            info.properties.insert(key, value);
        }
        Ok(info)
    }

    async fn get_brain_region(&self, region_id: &str) -> ServiceResult<BrainRegionInfo> {
        Ok(self
            .wnpu
            .lock()
            .brain_region_info(region_id)
            .map(brain_region_info_from_wnpu)
            .unwrap_or_else(|| BrainRegionInfo {
                region_id: region_id.to_string(),
                name: String::new(),
                region_type: String::new(),
                parent_id: None,
                cortical_areas: Vec::new(),
                child_regions: Vec::new(),
                properties: HashMap::new(),
            }))
    }

    async fn list_brain_regions(&self) -> ServiceResult<Vec<BrainRegionInfo>> {
        let wnpu = self.wnpu.lock();
        Ok(wnpu
            .brain_region_ids()
            .into_iter()
            .filter_map(|id| wnpu.brain_region_info(&id))
            .map(brain_region_info_from_wnpu)
            .collect())
    }

    async fn get_brain_region_ids(&self) -> ServiceResult<Vec<String>> {
        Ok(self.wnpu.lock().brain_region_ids())
    }

    async fn brain_region_exists(&self, region_id: &str) -> ServiceResult<bool> {
        Ok(self.wnpu.lock().has_brain_region(region_id))
    }

    async fn get_root_region_id(&self) -> ServiceResult<Option<String>> {
        Ok(self.wnpu.lock().root_brain_region_id())
    }

    async fn get_morphologies(&self) -> ServiceResult<HashMap<String, MorphologyInfo>> {
        let wnpu = self.wnpu.lock();
        Ok(wnpu
            .morphology_ids()
            .into_iter()
            .filter_map(|id| wnpu.morphology_info(&id).map(|info| (id, info)))
            .map(|(id, info)| (id, morphology_info_from_wnpu(info)))
            .collect())
    }

    async fn create_morphology(
        &self,
        morphology_id: String,
        _morphology: feagi_evolutionary::Morphology,
    ) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .add_morphology(&morphology_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.add_morphology: {e}")))
    }

    async fn update_morphology(
        &self,
        morphology_id: String,
        _morphology: feagi_evolutionary::Morphology,
    ) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .update_morphology_definition(&morphology_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.update_morphology_definition: {e}")))
    }

    async fn delete_morphology(&self, morphology_id: &str) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .remove_morphology(morphology_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.remove_morphology: {e}")))
    }

    async fn rename_morphology(&self, old_id: &str, new_id: &str) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .rename_morphology(old_id, new_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.rename_morphology: {e}")))
    }

    async fn update_cortical_mapping(
        &self,
        src_area_id: String,
        dst_area_id: String,
        mapping_data: Vec<serde_json::Value>,
    ) -> ServiceResult<usize> {
        let source = parse_cortical_id(&src_area_id)?;
        let destination = parse_cortical_id(&dst_area_id)?;
        self.wnpu
            .lock()
            .apply_cortical_mapping_update(&source, &destination, mapping_data)
            .map_err(|e| ServiceError::Backend(format!("wnpu.apply_cortical_mapping_update: {e}")))
    }

    // `connectome-io` is not optional in practice: feagi-api requires it, so cargo always unifies
    // the feature on. Implemented unconditionally because feagi-rs has no matching feature flag.
    async fn export_connectome(&self) -> ServiceResult<ConnectomeSnapshot> {
        let bytes = self
            .wnpu
            .lock()
            .export_connectome_bytes()
            .map_err(|e| ServiceError::Backend(format!("wnpu.export_connectome_bytes: {e}")))?;
        Ok(ConnectomeSnapshot {
            version: 1,
            neurons: SerializableNeuronArray::default(),
            synapses: SerializableSynapseArray::default(),
            cortical_area_names: Default::default(),
            burst_count: 0,
            power_amount: 0.0,
            fire_ledger_window: 0,
            metadata: ConnectomeMetadata {
                timestamp: 0,
                description: format!("wnpu export ({} bytes)", bytes.len()),
                source: "wnpu".to_string(),
                tags: Default::default(),
            },
        })
    }

    async fn import_connectome(&self, _snapshot: ConnectomeSnapshot) -> ServiceResult<()> {
        // WNPU accepts the connectome as opaque bytes; forwarding an empty payload keeps the
        // shape consistent while the real serialization layer is being decided.
        self.wnpu
            .lock()
            .import_connectome_bytes(&[])
            .map_err(|e| ServiceError::Backend(format!("wnpu.import_connectome_bytes: {e}")))
    }
}

// Re-export the connectome snapshot component types so we can construct empty defaults inline
// without touching every field explicitly.
use feagi_services::types::connectome_snapshot::{
    ConnectomeMetadata, SerializableNeuronArray, SerializableSynapseArray,
};

/// Serialize an owned value into a flat `HashMap<String, serde_json::Value>` for the property
/// endpoints that return JSON objects.
fn json_object_from_serialize<T: serde::Serialize>(
    value: &T,
) -> ServiceResult<HashMap<String, serde_json::Value>> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::Object(map)) => Ok(map.into_iter().collect()),
        Ok(_) => Err(ServiceError::Internal(
            "expected serialized value to be a JSON object".to_string(),
        )),
        Err(e) => Err(ServiceError::Internal(format!("serde_json: {e}"))),
    }
}

// ============================================================================
// NEURON
// ============================================================================

pub struct StubNeuronService {
    wnpu: SharedWnpu,
}

impl StubNeuronService {
    pub fn new(wnpu: SharedWnpu) -> Self {
        Self { wnpu }
    }
}

#[async_trait]
impl NeuronService for StubNeuronService {
    async fn create_neuron(&self, params: CreateNeuronParams) -> ServiceResult<NeuronInfo> {
        let parsed = parse_cortical_id(&params.cortical_id)?;
        let mut wnpu = self.wnpu.lock();
        let neuron_id = wnpu
            .add_neuron_at(&parsed, params.coordinates)
            .map_err(|e| ServiceError::Backend(format!("wnpu.add_neuron_at: {e}")))?;
        Ok(NeuronInfo {
            id: neuron_id,
            cortical_id: params.cortical_id,
            cortical_idx: 0,
            coordinates: params.coordinates,
            properties: params.properties.unwrap_or_default(),
        })
    }

    async fn delete_neuron(&self, neuron_id: u64) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .remove_neuron(neuron_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.remove_neuron: {e}")))
    }

    async fn get_neuron(&self, neuron_id: u64) -> ServiceResult<NeuronInfo> {
        let wnpu = self.wnpu.lock();
        let (cortical_id, coordinates) =
            wnpu.neuron_location(neuron_id)
                .unwrap_or((CorticalID::try_from_bytes(&[b'c', b'_', b'_', b'_', b'_', b'_', b'_', b'_'])
                    .expect("valid custom cortical ID"), (0, 0, 0)));
        let cortical_id_b64 = cortical_id.as_base_64();
        let properties = wnpu.neuron_properties(neuron_id);
        Ok(NeuronInfo {
            id: neuron_id,
            cortical_id: cortical_id_b64,
            cortical_idx: 0,
            coordinates,
            properties,
        })
    }

    async fn get_neuron_at_coordinates(
        &self,
        cortical_id: &str,
        coordinates: (u32, u32, u32),
    ) -> ServiceResult<Option<NeuronInfo>> {
        let parsed = parse_cortical_id(cortical_id)?;
        let wnpu = self.wnpu.lock();
        let Some(neuron_id) = wnpu.neuron_at_voxel(&parsed, coordinates) else {
            return Ok(None);
        };
        let properties = wnpu.neuron_properties(neuron_id);
        Ok(Some(NeuronInfo {
            id: neuron_id,
            cortical_id: cortical_id.to_string(),
            cortical_idx: 0,
            coordinates,
            properties,
        }))
    }

    async fn list_neurons_in_area(
        &self,
        cortical_id: &str,
        limit: Option<usize>,
    ) -> ServiceResult<Vec<NeuronInfo>> {
        let parsed = parse_cortical_id(cortical_id)?;
        let wnpu = self.wnpu.lock();
        Ok(wnpu
            .list_neuron_ids_in_area(&parsed, limit)
            .into_iter()
            .map(|neuron_id| {
                let (_, coordinates) = wnpu.neuron_location(neuron_id).unwrap_or((parsed, (0, 0, 0)));
                NeuronInfo {
                    id: neuron_id,
                    cortical_id: cortical_id.to_string(),
                    cortical_idx: 0,
                    coordinates,
                    properties: wnpu.neuron_properties(neuron_id),
                }
            })
            .collect())
    }

    async fn get_neuron_count(&self, cortical_id: &str) -> ServiceResult<usize> {
        let parsed = parse_cortical_id(cortical_id)?;
        Ok(self.wnpu.lock().cortical_area_neuron_count(&parsed).unwrap_or(0))
    }

    async fn neuron_exists(&self, neuron_id: u64) -> ServiceResult<bool> {
        Ok(self.wnpu.lock().has_neuron(neuron_id))
    }
}

// ============================================================================
// RUNTIME
// ============================================================================

/// Runtime control forwarded to the wrapped NPU.
///
/// Subscription registration is retained as local bookkeeping so the agent polling loop converges;
/// WNPU's subscription surface will consume this state once its subscribe/unsubscribe methods take
/// real parameters.
pub struct StubRuntimeService {
    /// Target burst rate reported through [`RuntimeService::get_status`] and used by every path
    /// that resumes the engine after a pause. Set from the configured burst frequency at startup
    /// and updated by [`RuntimeService::set_frequency`].
    current_frequency_hz: RwLock<f64>,
    wnpu: SharedWnpu,
    motor_subscriptions: RwLock<HashMap<String, Vec<String>>>,
    visualization_subscriptions: RwLock<HashSet<String>>,
}

impl StubRuntimeService {
    pub fn new(configured_frequency_hz: f64, wnpu: SharedWnpu) -> Self {
        Self {
            current_frequency_hz: RwLock::new(configured_frequency_hz),
            wnpu,
            motor_subscriptions: RwLock::new(HashMap::new()),
            visualization_subscriptions: RwLock::new(HashSet::new()),
        }
    }
}

#[async_trait]
impl RuntimeService for StubRuntimeService {
    async fn start(&self) -> ServiceResult<()> {
        let frequency = *self.current_frequency_hz.read();
        self.wnpu
            .lock()
            .run_at_frequency(NPUTargetFrequency::new_from_frequency(frequency))
            .map_err(|e| ServiceError::Backend(format!("wnpu.run_at_frequency: {e}")))
    }

    async fn stop(&self) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .stop()
            .map_err(|e| ServiceError::Backend(format!("wnpu.stop: {e}")))
    }

    async fn pause(&self) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .pause()
            .map_err(|e| ServiceError::Backend(format!("wnpu.pause: {e}")))
    }

    async fn resume(&self) -> ServiceResult<()> {
        let frequency = *self.current_frequency_hz.read();
        self.wnpu
            .lock()
            .run_at_frequency(NPUTargetFrequency::new_from_frequency(frequency))
            .map_err(|e| ServiceError::Backend(format!("wnpu.run_at_frequency: {e}")))
    }

    async fn step(&self) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .step_once()
            .map_err(|e| ServiceError::Backend(format!("wnpu.step_once: {e}")))
    }

    async fn get_status(&self) -> ServiceResult<RuntimeStatus> {
        let wnpu = self.wnpu.lock();
        let is_running = wnpu.is_running();
        let burst_count = wnpu.bursts_completed().unwrap_or(0);
        drop(wnpu);
        Ok(RuntimeStatus {
            is_running,
            is_paused: false,
            frequency_hz: *self.current_frequency_hz.read(),
            burst_count,
            current_rate_hz: 0.0,
            last_burst_neuron_count: 0,
            avg_burst_time_ms: 0.0,
        })
    }

    async fn set_frequency(&self, frequency_hz: f64) -> ServiceResult<()> {
        if !frequency_hz.is_finite() || frequency_hz <= 0.0 {
            return Err(ServiceError::InvalidInput(format!(
                "frequency_hz must be a positive finite value, got {frequency_hz}"
            )));
        }
        self.wnpu
            .lock()
            .run_at_frequency(NPUTargetFrequency::new_from_frequency(frequency_hz))
            .map_err(|e| ServiceError::Backend(format!("wnpu.run_at_frequency: {e}")))?;
        *self.current_frequency_hz.write() = frequency_hz;
        Ok(())
    }

    async fn get_burst_count(&self) -> ServiceResult<u64> {
        self.wnpu
            .lock()
            .bursts_completed()
            .map_err(|e| ServiceError::Backend(format!("wnpu.bursts_completed: {e}")))
    }

    async fn reset_burst_count(&self) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .reset_burst_count()
            .map_err(|e| ServiceError::Backend(format!("wnpu.reset_burst_count: {e}")))
    }

    async fn get_fcl_snapshot(&self) -> ServiceResult<Vec<(u64, f32)>> {
        Ok(self.wnpu.lock().fcl_snapshot())
    }

    async fn get_fcl_snapshot_with_cortical_idx(&self) -> ServiceResult<Vec<(u64, u32, f32)>> {
        Ok(self.wnpu.lock().fcl_snapshot_with_cortical_idx())
    }

    async fn get_fire_queue_sample(
        &self,
    ) -> ServiceResult<HashMap<u32, (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>, Vec<f32>)>> {
        Ok(self.wnpu.lock().fire_queue_sample())
    }

    async fn get_fire_ledger_configs(&self) -> ServiceResult<Vec<(u32, usize)>> {
        Ok(self.wnpu.lock().fire_ledger_configs())
    }

    async fn configure_fire_ledger_window(
        &self,
        cortical_idx: u32,
        window_size: usize,
    ) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .configure_fire_ledger_window(cortical_idx, window_size)
            .map_err(|e| ServiceError::Backend(format!("wnpu.configure_fire_ledger_window: {e}")))
    }

    async fn get_fcl_sampler_config(&self) -> ServiceResult<(f64, u32)> {
        Ok(self.wnpu.lock().fcl_sampler_config())
    }

    async fn set_fcl_sampler_config(
        &self,
        frequency: Option<f64>,
        consumer: Option<u32>,
    ) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .set_fcl_sampler_config(frequency, consumer)
            .map_err(|e| ServiceError::Backend(format!("wnpu.set_fcl_sampler_config: {e}")))
    }

    async fn get_area_fcl_sample_rate(&self, area_id: u32) -> ServiceResult<f64> {
        Ok(self.wnpu.lock().area_fcl_sample_rate(area_id))
    }

    async fn set_area_fcl_sample_rate(
        &self,
        area_id: u32,
        sample_rate: f64,
    ) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .set_area_fcl_sample_rate(area_id, sample_rate)
            .map_err(|e| ServiceError::Backend(format!("wnpu.set_area_fcl_sample_rate: {e}")))
    }

    async fn inject_sensory_by_coordinates(
        &self,
        cortical_id: &str,
        xyzp_data: &[(u32, u32, u32, f32)],
        _mode: ManualStimulationMode,
    ) -> ServiceResult<usize> {
        let parsed = parse_cortical_id(cortical_id)?;
        self.wnpu
            .lock()
            .inject_sensory_by_coordinates(&parsed, xyzp_data)
            .map_err(|e| {
                ServiceError::Backend(format!("wnpu.inject_sensory_by_coordinates: {e}"))
            })
    }

    async fn register_motor_subscriptions(
        &self,
        agent_id: &str,
        cortical_ids: Vec<String>,
        _rate_hz: f64,
    ) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .subscribe_agent_to_npu(())
            .map_err(|e| ServiceError::Backend(format!("wnpu.subscribe_agent_to_npu: {e}")))?;
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
        self.wnpu
            .lock()
            .subscribe_agent_to_npu(())
            .map_err(|e| ServiceError::Backend(format!("wnpu.subscribe_agent_to_npu: {e}")))?;
        self.visualization_subscriptions
            .write()
            .insert(agent_id.to_string());
        Ok(())
    }

    fn unregister_motor_subscriptions(&self, agent_id: &str) {
        let _ = self.wnpu.lock().unsubscribe_agent_from_npu(());
        self.motor_subscriptions.write().remove(agent_id);
    }

    fn unregister_visualization_subscriptions(&self, agent_id: &str) {
        let _ = self.wnpu.lock().unsubscribe_agent_from_npu(());
        self.visualization_subscriptions.write().remove(agent_id);
    }

    async fn reset_cortical_area_states(
        &self,
        cortical_indices: &[u32],
    ) -> ServiceResult<Vec<(u32, usize)>> {
        let mut wnpu = self.wnpu.lock();
        let mut out = Vec::with_capacity(cortical_indices.len());
        for &idx in cortical_indices {
            let neurons_reset = wnpu.reset_cortical_area_state_by_idx(idx).map_err(|e| {
                ServiceError::Backend(format!("wnpu.reset_cortical_area_state_by_idx: {e}"))
            })?;
            out.push((idx, neurons_reset));
        }
        Ok(out)
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

/// System introspection. Version reporting is real; the rest is answered from WNPU counts.
pub struct StubSystemService {
    version_info: VersionInfo,
    wnpu: SharedWnpu,
}

impl StubSystemService {
    pub fn new(version_info: VersionInfo, wnpu: SharedWnpu) -> Self {
        Self { version_info, wnpu }
    }
}

#[async_trait]
impl SystemService for StubSystemService {
    async fn get_health(&self) -> ServiceResult<HealthStatus> {
        let wnpu = self.wnpu.lock();
        let health = wnpu.system_health();
        drop(wnpu);
        Ok(HealthStatus {
            overall_status: health.overall_status,
            components: health
                .components
                .into_iter()
                .map(|c| ComponentHealth {
                    name: c.name,
                    status: c.status,
                    message: c.message,
                })
                .collect(),
            timestamp: String::new(),
        })
    }

    async fn get_status(&self) -> ServiceResult<SystemStatus> {
        let wnpu = self.wnpu.lock();
        let cortical_area_count = wnpu.cortical_area_ids().len();
        let brain_region_count = wnpu.brain_region_ids().len();
        let neuron_count = wnpu.total_neuron_count();
        let synapse_count = wnpu.total_synapse_count();
        let is_running = wnpu.is_running();
        let burst_count = wnpu.bursts_completed().unwrap_or(0);
        drop(wnpu);
        Ok(SystemStatus {
            is_initialized: cortical_area_count > 0,
            burst_engine_running: is_running,
            burst_count,
            neuron_count,
            synapse_count,
            cortical_area_count,
            brain_region_count,
            uptime_seconds: 0,
            current_burst_rate_hz: 0.0,
            avg_burst_time_ms: 0.0,
        })
    }

    async fn get_version(&self) -> ServiceResult<VersionInfo> {
        Ok(self.version_info.clone())
    }

    async fn is_initialized(&self) -> ServiceResult<bool> {
        Ok(!self.wnpu.lock().cortical_area_ids().is_empty())
    }

    async fn get_burst_count(&self) -> ServiceResult<u64> {
        Ok(self.wnpu.lock().bursts_completed().unwrap_or(0))
    }

    async fn get_runtime_stats(&self) -> ServiceResult<RuntimeStats> {
        let stats = self.wnpu.lock().runtime_stats();
        Ok(RuntimeStats {
            total_bursts: stats.total_bursts,
            total_neurons_fired: stats.total_neurons_fired,
            total_processing_time_ms: stats.total_processing_time_ms,
            avg_burst_time_ms: stats.avg_burst_time_ms,
            avg_neurons_per_burst: stats.avg_neurons_per_burst,
            current_rate_hz: stats.current_rate_hz,
            peak_rate_hz: stats.peak_rate_hz,
            uptime_seconds: stats.uptime_seconds,
        })
    }

    async fn get_memory_usage(&self) -> ServiceResult<MemoryUsage> {
        let usage = self.wnpu.lock().memory_usage();
        Ok(MemoryUsage {
            npu_neurons_bytes: usage.npu_neurons_bytes,
            npu_synapses_bytes: usage.npu_synapses_bytes,
            npu_total_bytes: usage.npu_total_bytes,
            connectome_metadata_bytes: usage.connectome_metadata_bytes,
            total_allocated_bytes: usage.total_allocated_bytes,
            system_total_bytes: usage.system_total_bytes,
            system_available_bytes: usage.system_available_bytes,
        })
    }

    async fn get_capacity(&self) -> ServiceResult<CapacityInfo> {
        let capacity = self.wnpu.lock().capacity();
        Ok(CapacityInfo {
            current_neurons: capacity.current_neurons,
            max_neurons: capacity.max_neurons,
            neuron_utilization_percent: capacity.neuron_utilization_percent,
            current_synapses: capacity.current_synapses,
            max_synapses: capacity.max_synapses,
            synapse_utilization_percent: capacity.synapse_utilization_percent,
            current_cortical_areas: capacity.current_cortical_areas,
            max_cortical_areas: capacity.max_cortical_areas,
        })
    }
}

// ============================================================================
// AGENT
// ============================================================================

/// Agent-facing service. Registration itself is handled by `FeagiAgentHandler` over the
/// transports; the remaining trait methods forward to WNPU's subscription surface.
pub struct StubAgentService {
    wnpu: SharedWnpu,
}

impl StubAgentService {
    pub fn new(wnpu: SharedWnpu) -> Self {
        Self { wnpu }
    }
}

#[async_trait]
impl AgentService for StubAgentService {
    async fn register_agent(
        &self,
        registration: AgentRegistration,
    ) -> AgentResult<AgentRegistrationResponse> {
        self.wnpu
            .lock()
            .subscribe_agent_to_npu(())
            .map_err(|e| AgentError::RegistrationFailed(format!("wnpu.subscribe_agent_to_npu: {e}")))?;
        Ok(AgentRegistrationResponse {
            status: "ok".to_string(),
            message: format!("agent '{}' subscribed", registration.agent_id),
            success: true,
            transport: None,
            rates: None,
            transports: None,
            recommended_transport: None,
            shm_paths: None,
            cortical_areas: serde_json::Value::Object(serde_json::Map::new()),
        })
    }

    async fn heartbeat(&self, _request: HeartbeatRequest) -> AgentResult<()> {
        Ok(())
    }

    async fn list_agents(&self) -> AgentResult<Vec<String>> {
        Ok(self.wnpu.lock().subscribed_agent_ids())
    }

    async fn get_agent_properties(&self, agent_id: &str) -> AgentResult<AgentProperties> {
        Ok(self
            .wnpu
            .lock()
            .subscribed_agent_properties(agent_id)
            .map(agent_properties_from_wnpu)
            .unwrap_or_else(empty_agent_properties))
    }

    async fn get_shared_memory_info(
        &self,
    ) -> AgentResult<HashMap<String, HashMap<String, serde_json::Value>>> {
        Ok(self.wnpu.lock().shared_memory_info())
    }

    async fn deregister_agent(&self, _agent_id: &str) -> AgentResult<()> {
        self.wnpu
            .lock()
            .unsubscribe_agent_from_npu(())
            .map_err(|e| AgentError::Internal(format!("wnpu.unsubscribe_agent_from_npu: {e}")))
    }

    async fn manual_stimulation(
        &self,
        stimulation_payload: HashMap<String, Vec<Vec<i32>>>,
        _mode: AgentManualStimulationMode,
    ) -> AgentResult<HashMap<String, serde_json::Value>> {
        let injected = self
            .wnpu
            .lock()
            .manual_stimulate(stimulation_payload)
            .map_err(|e| AgentError::Internal(format!("wnpu.manual_stimulate: {e}")))?;
        let mut response = HashMap::new();
        response.insert(
            "injected_neurons".to_string(),
            serde_json::Value::from(injected),
        );
        Ok(response)
    }

    fn try_set_runtime_service(&self, _runtime_service: Arc<dyn RuntimeService + Send + Sync>) {}
}

/// Empty `AgentProperties`, used when WNPU has no record of the requested agent. The trait's
/// `get_agent_properties` returns `Result<AgentProperties, AgentError>`; keeping the fallback
/// local means adapters never see an error just because the agent has not registered yet.
fn empty_agent_properties() -> AgentProperties {
    AgentProperties {
        agent_type: String::new(),
        agent_ip: String::new(),
        agent_data_port: 0,
        agent_router_address: String::new(),
        agent_version: String::new(),
        controller_version: String::new(),
        capabilities: HashMap::new(),
        chosen_transport: None,
    }
}

// ============================================================================
// SNAPSHOT
// ============================================================================

pub struct StubSnapshotService {
    wnpu: SharedWnpu,
}

impl StubSnapshotService {
    pub fn new(wnpu: SharedWnpu) -> Self {
        Self { wnpu }
    }
}

#[async_trait]
impl SnapshotService for StubSnapshotService {
    async fn create_snapshot(
        &self,
        options: SnapshotCreateOptions,
    ) -> ServiceResult<SnapshotMetadata> {
        Ok(snapshot_metadata_from_wnpu(self.wnpu.lock().create_snapshot(
            options.name,
            options.description,
            options.stateful,
        )))
    }

    async fn restore_snapshot(&self, snapshot_id: &str) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .restore_snapshot(snapshot_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.restore_snapshot: {e}")))
    }

    async fn list_snapshots(&self) -> ServiceResult<Vec<SnapshotMetadata>> {
        Ok(self
            .wnpu
            .lock()
            .list_snapshots()
            .into_iter()
            .map(snapshot_metadata_from_wnpu)
            .collect())
    }

    async fn delete_snapshot(&self, snapshot_id: &str) -> ServiceResult<()> {
        self.wnpu
            .lock()
            .delete_snapshot(snapshot_id)
            .map_err(|e| ServiceError::Backend(format!("wnpu.delete_snapshot: {e}")))
    }

    async fn get_snapshot_artifact(
        &self,
        snapshot_id: &str,
        format: &str,
    ) -> ServiceResult<Vec<u8>> {
        Ok(self.wnpu.lock().snapshot_artifact_bytes(snapshot_id, format))
    }
}
