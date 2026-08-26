// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Service-layer adapters that back FEAGI's REST/ZMQ/WebSocket surface.
//!
//! The old NPU stack (`feagi-npu-burst-engine` + `ConnectomeManager`) backed every
//! `feagi-services` implementation that FEAGI's transports were wired to. That stack is gone and
//! `feagi-services` no longer ships built-in implementations, so adapters are expected to supply
//! their own — this file is that adapter.
//!
//! Responsibilities are split along the same line as the wrapped NPU's surface:
//!
//! - Anything the NPU owns — cortical areas, cortical mappings, burst lifecycle/frequency, runtime
//!   probes, sensory injection, and the NPU-side agent data channels — is forwarded to
//!   [`WrappedNeuronProcessingUnit`].
//! - Anything the legacy Brain Development code owns — genome load/validate/save/export/info,
//!   brain regions, and morphology definitions, plus the developed-brain figures behind system
//!   status/capacity — is answered from the BDU-side
//!   [`crate::brain_development::DevelopedBrain`] and its retained genome. Realising a morphology
//!   into synapses is a separate NPU concern, forwarded through `update_cortical_mapping`.
//! - Surfaces that have no backend on this server yet — snapshots, whole-connectome transport,
//!   engine runtime/memory metrics, and the agent registry — report themselves as unavailable
//!   rather than returning fabricated data.
//!
//! `AnalyticsService` is deliberately absent from this file: it is what `/v1/system/health_check`
//! reads, and it is answered from the BDU by [`crate::brain_development::BduAnalyticsService`].

use axum::async_trait;
use feagi_data::neurons::wrapped_types::CorticalVoxelDimensionsGenomic;
use feagi_genomic_context::brain_region::{BrainRegion, RegionID, RegionType};
use feagi_genomic_context::cortical_area::CorticalID;
use feagi_npu::standard::npu::npu_target_frequency::NPUTargetFrequency;
use feagi_npu::standard::wnpu::wnpu::{CorticalAreaParameters, WrappedNeuronProcessingUnit};

use crate::brain_development::{develop_genome_json, DevelopedBrain};
use feagi_evolutionary::runtime::RuntimeGenome;
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

/// Derives the wrapped NPU's mapping name for a source->destination pair. The NPU references
/// mappings by name; a pair identifies a single mapping, so the two base64 cortical IDs joined
/// with an arrow form a stable unique name the NPU can later resolve for removal.
fn cortical_mapping_name(source_b64: &str, destination_b64: &str) -> String {
    format!("{source_b64}->{destination_b64}")
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

/// The genome DTO reported when no genome has been developed yet.
fn empty_genome_info() -> GenomeInfo {
    GenomeInfo {
        genome_id: String::new(),
        genome_title: String::new(),
        version: String::new(),
        cortical_area_count: 0,
        brain_region_count: 0,
        simulation_timestep: 0.0,
        genome_num: None,
        genome_timestamp: None,
    }
}

/// Build the service-layer genome DTO from the BDU's retained runtime genome.
///
/// Every field is read straight off the developed genome: this is the "BDU contains the genome
/// state" path, so the service never fabricates metadata it does not hold.
fn genome_info_from_runtime(genome: &RuntimeGenome) -> GenomeInfo {
    GenomeInfo {
        genome_id: genome.metadata.genome_id.clone(),
        genome_title: genome.metadata.genome_title.clone(),
        version: genome.metadata.version.clone(),
        cortical_area_count: genome.cortical_areas.len(),
        brain_region_count: genome.brain_regions.len(),
        simulation_timestep: genome.physiology.simulation_timestep,
        genome_num: None,
        genome_timestamp: Some(genome.metadata.timestamp as i64),
    }
}

// ============================================================================
// GENOME
// ============================================================================

pub struct StubGenomeService {
    wnpu: SharedWnpu,
    /// BDU-side brain state. Genome load/validate/save/export/info are answered from here rather
    /// than the NPU, which owns only cortical areas and mappings.
    brain: Arc<DevelopedBrain>,
}

impl StubGenomeService {
    pub fn new(wnpu: SharedWnpu, brain: Arc<DevelopedBrain>) -> Self {
        Self { wnpu, brain }
    }
}

#[async_trait]
impl GenomeService for StubGenomeService {
    async fn load_genome(&self, params: LoadGenomeParams) -> ServiceResult<GenomeInfo> {
        develop_genome_json(&params.json_str, &self.brain)
            .map_err(|e| ServiceError::Backend(format!("develop_genome_json: {e}")))?;
        self.brain
            .genome()
            .as_ref()
            .map(genome_info_from_runtime)
            .ok_or_else(|| {
                ServiceError::Backend("genome developed but not retained by the BDU".to_string())
            })
    }

    async fn save_genome(&self, _params: SaveGenomeParams) -> ServiceResult<String> {
        let genome = self
            .brain
            .genome()
            .ok_or_else(|| ServiceError::InvalidInput("no genome is currently loaded".to_string()))?;
        feagi_evolutionary::save_genome_to_json(&genome)
            .map_err(|e| ServiceError::Backend(format!("save_genome_to_json: {e}")))
    }

    async fn export_region_genome(&self, region_id: String) -> ServiceResult<String> {
        let genome = self
            .brain
            .genome()
            .ok_or_else(|| ServiceError::InvalidInput("no genome is currently loaded".to_string()))?;
        let subset =
            feagi_evolutionary::subset_runtime_genome_for_region_branch(&genome, &region_id)
                .map_err(|e| {
                    ServiceError::Backend(format!("subset_runtime_genome_for_region_branch: {e}"))
                })?;
        feagi_evolutionary::save_genome_to_json(&subset)
            .map_err(|e| ServiceError::Backend(format!("save_genome_to_json: {e}")))
    }

    async fn get_genome_info(&self) -> ServiceResult<GenomeInfo> {
        Ok(self
            .brain
            .genome()
            .as_ref()
            .map(genome_info_from_runtime)
            .unwrap_or_else(empty_genome_info))
    }

    async fn validate_genome(&self, json_str: String) -> ServiceResult<bool> {
        // A genome that does not even parse is not valid; a genome that parses is valid only when
        // the migration chain reports no blocking findings.
        Ok(match feagi_evolutionary::load_genome_with_report(&json_str) {
            Ok((_, chain_report)) => chain_report.is_blocking_clean(),
            Err(_) => false,
        })
    }

    async fn reset_connectome(&self) -> ServiceResult<()> {
        // Clear both sides: the NPU discards its cortical areas/mappings, and the BDU forgets the
        // developed brain and its genome so health reports an undeveloped brain again.
        self.wnpu
            .lock()
            .clear_connectome()
            .map_err(|e| ServiceError::Backend(format!("wnpu.clear_connectome: {e}")))?;
        self.brain.clear();
        Ok(())
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
    /// The BDU-side developed brain, whose retained genome owns the brain-region tree. Region
    /// reads and edits go through it so they stay consistent with genome save/export.
    brain: Arc<DevelopedBrain>,
}

impl StubConnectomeService {
    pub fn new(wnpu: SharedWnpu, brain: Arc<DevelopedBrain>) -> Self {
        Self { wnpu, brain }
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

    // ------------------------------------------------------------------------------------------
    // Brain regions live in the genome, not the NPU. They are read from and edited on the genome
    // the BDU retained on load, so region edits stay consistent with genome save/export. This
    // mirrors the genome-backed connectome service in `feagi-api`.
    // ------------------------------------------------------------------------------------------

    async fn create_brain_region(
        &self,
        params: CreateBrainRegionParams,
    ) -> ServiceResult<BrainRegionInfo> {
        let region_id = RegionID::from_string(&params.region_id).map_err(|e| {
            ServiceError::InvalidInput(format!(
                "invalid region ID '{}': {}",
                params.region_id, e
            ))
        })?;

        // The genome carries a single region type, so anything else is a caller mistake rather
        // than a value to coerce.
        if !params.region_type.eq_ignore_ascii_case("undefined") {
            return Err(ServiceError::InvalidInput(format!(
                "unknown region type '{}'; the genome format defines only 'undefined'",
                params.region_type
            )));
        }

        let mut region = BrainRegion::new(region_id, params.name, RegionType::Undefined)
            .map_err(|e| ServiceError::InvalidInput(e.to_string()))?;

        for (key, value) in params.properties.unwrap_or_default() {
            region.add_property(key, value);
        }
        // The parent link lives in the child's properties; this is the same representation the
        // genome parser writes and the saver reads back.
        if let Some(parent_id) = params.parent_id {
            region.add_property(
                "parent_region_id".to_string(),
                serde_json::Value::String(parent_id),
            );
        }

        let region_id_key = params.region_id;
        self.brain
            .with_genome_mut(|g| {
                if g.brain_regions.contains_key(&region_id_key) {
                    return Err(ServiceError::AlreadyExists {
                        resource: "brain_region".to_string(),
                        id: region_id_key.clone(),
                    });
                }
                if let Some(parent_id) = region
                    .properties
                    .get("parent_region_id")
                    .and_then(|value| value.as_str())
                {
                    if !g.brain_regions.contains_key(parent_id) {
                        return Err(ServiceError::NotFound {
                            resource: "brain_region".to_string(),
                            id: parent_id.to_string(),
                        });
                    }
                }

                g.brain_regions.insert(region_id_key.clone(), region);
                let created = &g.brain_regions[&region_id_key];
                Ok(region_to_info(&region_id_key, created, &g.brain_regions))
            })
            .ok_or_else(|| no_genome_loaded("brain region creation"))?
    }

    async fn delete_brain_region(&self, region_id: &str) -> ServiceResult<()> {
        // Deleting a region deletes the cortical areas beneath it, and the engine has no operation
        // for removing an area, so a region that still holds areas is reported rather than partly
        // deleted. Matches the genome-backed service in `feagi-api`.
        let has_areas = self
            .brain
            .with_genome(|g| {
                g.brain_regions
                    .get(region_id)
                    .map(|region| !region.cortical_areas.is_empty())
            })
            .flatten()
            .ok_or_else(|| region_not_found(region_id))?;

        if has_areas {
            return Err(ServiceError::NotImplemented(
                "deleting a region that holds cortical areas requires removing those areas from \
                 the engine, which the current NPU cannot do"
                    .to_string(),
            ));
        }

        self.brain
            .with_genome_mut(|g| {
                // A region cannot be removed while others still point at it as their parent.
                let orphans: Vec<String> = g
                    .brain_regions
                    .iter()
                    .filter(|(_, candidate)| {
                        candidate
                            .properties
                            .get("parent_region_id")
                            .and_then(|value| value.as_str())
                            == Some(region_id)
                    })
                    .map(|(child_id, _)| child_id.clone())
                    .collect();

                if !orphans.is_empty() {
                    return Err(ServiceError::Conflict(format!(
                        "region '{}' still has child regions: {}",
                        region_id,
                        orphans.join(", ")
                    )));
                }

                g.brain_regions.remove(region_id);
                Ok(())
            })
            .ok_or_else(|| no_genome_loaded("brain region deletion"))?
    }

    async fn update_brain_region(
        &self,
        region_id: &str,
        properties: HashMap<String, serde_json::Value>,
    ) -> ServiceResult<BrainRegionInfo> {
        self.brain
            .with_genome_mut(|g| {
                if !g.brain_regions.contains_key(region_id) {
                    return Err(region_not_found(region_id));
                }

                // Reparenting is applied through the same property the hierarchy is read from, so
                // it must name a region that exists and cannot be the region itself.
                if let Some(parent_id) = properties
                    .get("parent_region_id")
                    .and_then(|value| value.as_str())
                {
                    if !g.brain_regions.contains_key(parent_id) {
                        return Err(ServiceError::NotFound {
                            resource: "brain_region".to_string(),
                            id: parent_id.to_string(),
                        });
                    }
                    if parent_id == region_id {
                        return Err(ServiceError::InvalidInput(
                            "a region cannot be its own parent".to_string(),
                        ));
                    }
                }

                let region = g
                    .brain_regions
                    .get_mut(region_id)
                    .expect("presence checked above");
                for (key, value) in properties {
                    // `name` is a field rather than a property, so it is applied where readers look.
                    if key == "name" || key == "title" {
                        if let Some(name) = value.as_str() {
                            region.name = name.to_string();
                            continue;
                        }
                    }
                    region.add_property(key, value);
                }

                let updated = &g.brain_regions[region_id];
                Ok(region_to_info(region_id, updated, &g.brain_regions))
            })
            .ok_or_else(|| no_genome_loaded("brain region update"))?
    }

    async fn get_brain_region(&self, region_id: &str) -> ServiceResult<BrainRegionInfo> {
        self.brain
            .with_genome(|g| {
                g.brain_regions
                    .get(region_id)
                    .map(|region| region_to_info(region_id, region, &g.brain_regions))
            })
            .flatten()
            .ok_or_else(|| region_not_found(region_id))
    }

    async fn list_brain_regions(&self) -> ServiceResult<Vec<BrainRegionInfo>> {
        Ok(self
            .brain
            .with_genome(|g| {
                g.brain_regions
                    .iter()
                    .map(|(region_id, region)| region_to_info(region_id, region, &g.brain_regions))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn get_brain_region_ids(&self) -> ServiceResult<Vec<String>> {
        Ok(self
            .brain
            .with_genome(|g| g.brain_regions.keys().cloned().collect())
            .unwrap_or_default())
    }

    async fn brain_region_exists(&self, region_id: &str) -> ServiceResult<bool> {
        Ok(self
            .brain
            .with_genome(|g| g.brain_regions.contains_key(region_id))
            .unwrap_or(false))
    }

    async fn get_root_region_id(&self) -> ServiceResult<Option<String>> {
        // The genome parser records each region's parent under `parent_region_id`; the root is the
        // one that has none. Matches `BrainRegionHierarchy::get_root_region_id`.
        Ok(self
            .brain
            .with_genome(|g| {
                g.brain_regions
                    .iter()
                    .find(|(_, region)| {
                        !matches!(
                            region.properties.get("parent_region_id"),
                            Some(v) if !v.is_null()
                        )
                    })
                    .map(|(region_id, _)| region_id.clone())
            })
            .flatten())
    }

    // ------------------------------------------------------------------------------------------
    // Morphologies are user-defined rule sets for wiring cortical areas. Their definitions and
    // names are metadata that lives in the genome, so they are read and edited on the retained
    // genome (same as brain regions, mirroring `feagi-api`). Realising a mapping into actual
    // synapses is a separate concern handled by the NPU through `update_cortical_mapping`; the
    // morphology payload (type, parameters, name) is forwarded there for the NPU to act on and to
    // track by name for later removal.
    // ------------------------------------------------------------------------------------------

    async fn get_morphologies(&self) -> ServiceResult<HashMap<String, MorphologyInfo>> {
        // No genome loaded means no morphologies, the same empty answer the other listings give.
        // The per-morphology conversion can fail, so the closure yields a `Result`.
        let Some(result) = self.brain.with_genome(|g| {
            g.morphologies
                .morphology_ids()
                .into_iter()
                .map(|morphology_id| {
                    // `morphology_ids` returned this key, so the registry holds it.
                    let morphology = g.morphologies.get(&morphology_id).ok_or_else(|| {
                        ServiceError::Internal(format!(
                            "morphology registry listed '{}' but cannot return it",
                            morphology_id
                        ))
                    })?;
                    let parameters =
                        serde_json::to_value(&morphology.parameters).map_err(|err| {
                            ServiceError::Internal(format!(
                                "morphology '{}' has parameters that cannot be serialised: {}",
                                morphology_id, err
                            ))
                        })?;
                    Ok((
                        morphology_id,
                        MorphologyInfo {
                            morphology_type: morphology_type_name(&morphology.morphology_type)
                                .to_string(),
                            class: morphology.class.clone(),
                            parameters,
                        },
                    ))
                })
                .collect::<ServiceResult<HashMap<String, MorphologyInfo>>>()
        }) else {
            return Ok(HashMap::new());
        };
        result
    }

    async fn create_morphology(
        &self,
        morphology_id: String,
        morphology: feagi_evolutionary::Morphology,
    ) -> ServiceResult<()> {
        if morphology_id.trim().is_empty() {
            return Err(ServiceError::InvalidInput(
                "morphology_id must be non-empty".to_string(),
            ));
        }

        self.brain
            .with_genome_mut(|g| {
                if g.morphologies.contains(&morphology_id) {
                    return Err(ServiceError::AlreadyExists {
                        resource: "morphology".to_string(),
                        id: morphology_id.clone(),
                    });
                }
                g.morphologies.add_morphology(morphology_id, morphology);
                Ok(())
            })
            .ok_or_else(|| no_genome_loaded("morphology creation"))?
    }

    async fn update_morphology(
        &self,
        _morphology_id: String,
        _morphology: feagi_evolutionary::Morphology,
    ) -> ServiceResult<()> {
        // Redefining a morphology has to regenerate the synapses of every mapping that uses it.
        // The NPU builds connectivity from mapping submissions and offers no regeneration
        // operation, so writing the new definition to the genome alone would leave the running
        // brain wired to the old one.
        Err(ServiceError::NotImplemented(
            "changing a morphology requires regenerating the synapses of the mappings that use \
             it, which the current NPU cannot do"
                .to_string(),
        ))
    }

    async fn delete_morphology(&self, morphology_id: &str) -> ServiceResult<()> {
        if morphology_id.trim().is_empty() {
            return Err(ServiceError::InvalidInput(
                "morphology_id must be non-empty".to_string(),
            ));
        }

        self.brain
            .with_genome_mut(|g| {
                if g.morphologies.remove_morphology(morphology_id) {
                    Ok(())
                } else {
                    Err(ServiceError::NotFound {
                        resource: "morphology".to_string(),
                        id: morphology_id.to_string(),
                    })
                }
            })
            .ok_or_else(|| no_genome_loaded("morphology deletion"))?
    }

    async fn rename_morphology(&self, old_id: &str, new_id: &str) -> ServiceResult<()> {
        let old_id = old_id.trim();
        let new_id = new_id.trim();

        if old_id.is_empty() {
            return Err(ServiceError::InvalidInput(
                "old_id must be non-empty".to_string(),
            ));
        }
        if new_id.is_empty() {
            return Err(ServiceError::InvalidInput(
                "new_id must be non-empty".to_string(),
            ));
        }
        if old_id == new_id {
            return Err(ServiceError::InvalidInput(
                "old_id and new_id must differ".to_string(),
            ));
        }

        self.brain
            .with_genome_mut(|g| {
                let morphology =
                    g.morphologies
                        .get(old_id)
                        .cloned()
                        .ok_or_else(|| ServiceError::NotFound {
                            resource: "morphology".to_string(),
                            id: old_id.to_string(),
                        })?;

                if morphology.class == "core" {
                    return Err(ServiceError::InvalidInput(format!(
                        "core morphologies cannot be renamed: '{}'",
                        old_id
                    )));
                }
                if g.morphologies.contains(new_id) {
                    return Err(ServiceError::AlreadyExists {
                        resource: "morphology".to_string(),
                        id: new_id.to_string(),
                    });
                }

                g.morphologies.remove_morphology(old_id);
                g.morphologies.add_morphology(new_id.to_string(), morphology);

                // Mapping rules name their morphology by id, so a rename that stopped at the
                // registry would leave those rules pointing at something that no longer exists.
                let mut replaced = 0usize;
                for area in g.cortical_areas.values_mut() {
                    for value in area.properties.values_mut() {
                        rename_morphology_references(value, old_id, new_id, &mut replaced);
                    }
                }
                for region in g.brain_regions.values_mut() {
                    for value in region.properties.values_mut() {
                        rename_morphology_references(value, old_id, new_id, &mut replaced);
                    }
                }

                Ok(())
            })
            .ok_or_else(|| no_genome_loaded("morphology rename"))?
    }

    async fn update_cortical_mapping(
        &self,
        src_area_id: String,
        dst_area_id: String,
        mapping_data: Vec<serde_json::Value>,
    ) -> ServiceResult<usize> {
        let source = parse_cortical_id(&src_area_id)?;
        let destination = parse_cortical_id(&dst_area_id)?;
        // The wrapped NPU references mappings by name. A source->destination pair identifies at
        // most one mapping, so the name is derived deterministically from the two endpoints; a
        // later delete can address the same mapping by reconstructing this name.
        let mapping_name = cortical_mapping_name(&src_area_id, &dst_area_id);
        self.wnpu
            .lock()
            .apply_cortical_mapping_update(&mapping_name, &source, &destination, mapping_data)
            .map_err(|e| ServiceError::Backend(format!("wnpu.apply_cortical_mapping_update: {e}")))
    }

    // Connectome transport (snapshotting the whole connectome to/from bytes) is a legacy Brain
    // Development responsibility; the NPU no longer exposes it and this adapter has no BDU path for
    // it, so both directions report as unavailable.
    async fn export_connectome(&self) -> ServiceResult<ConnectomeSnapshot> {
        Err(connectome_transport_unavailable())
    }

    async fn import_connectome(&self, _snapshot: ConnectomeSnapshot) -> ServiceResult<()> {
        Err(connectome_transport_unavailable())
    }
}

/// Builds the DTO for a region, deriving its child list from the parent links of every other
/// region (the genome records only each child's parent). Mirrors `feagi-api`'s `region_to_info`.
fn region_to_info(
    region_id: &str,
    region: &BrainRegion,
    all_regions: &HashMap<String, BrainRegion>,
) -> BrainRegionInfo {
    fn parent_of(candidate: &BrainRegion) -> Option<&str> {
        candidate
            .properties
            .get("parent_region_id")
            .and_then(|value| value.as_str())
    }

    BrainRegionInfo {
        region_id: region_id.to_string(),
        name: region.name.clone(),
        region_type: region.region_type.to_string(),
        parent_id: parent_of(region).map(String::from),
        cortical_areas: region
            .cortical_areas
            .iter()
            .map(|area_id| area_id.to_string())
            .collect(),
        child_regions: all_regions
            .iter()
            .filter(|(_, candidate)| parent_of(candidate) == Some(region_id))
            .map(|(child_id, _)| child_id.clone())
            .collect(),
        properties: region.properties.clone(),
    }
}

/// Reports a brain region the genome does not contain.
fn region_not_found(region_id: &str) -> ServiceError {
    ServiceError::NotFound {
        resource: "brain_region".to_string(),
        id: region_id.to_string(),
    }
}

/// Reports that a region edit was attempted before any genome was loaded.
fn no_genome_loaded(operation: &str) -> ServiceError {
    ServiceError::InvalidState(format!(
        "no genome is loaded, so {operation} cannot be performed"
    ))
}

/// Names a morphology's type using the same spellings the genome format uses.
///
/// Matched explicitly rather than derived from the serde representation so the wire vocabulary is
/// visible here and cannot drift with a serde attribute change. Mirrors `feagi-api`.
fn morphology_type_name(morphology_type: &feagi_evolutionary::MorphologyType) -> &'static str {
    match morphology_type {
        feagi_evolutionary::MorphologyType::Vectors => "vectors",
        feagi_evolutionary::MorphologyType::Patterns => "patterns",
        feagi_evolutionary::MorphologyType::Functions => "functions",
        feagi_evolutionary::MorphologyType::Composite => "composite",
    }
}

/// Rewrites references to a renamed morphology wherever mapping rules name it: as a `morphology_id`
/// or `mapper_morphology` field, or as the leading element of a rule tuple. Mirrors `feagi-api`.
fn rename_morphology_references(
    value: &mut serde_json::Value,
    old_id: &str,
    new_id: &str,
    replaced: &mut usize,
) {
    match value {
        serde_json::Value::Object(object) => {
            for field in ["morphology_id", "mapper_morphology"] {
                if let Some(reference) = object.get_mut(field) {
                    if reference.as_str() == Some(old_id) {
                        *reference = serde_json::Value::String(new_id.to_string());
                        *replaced += 1;
                    }
                }
            }
            for child in object.values_mut() {
                rename_morphology_references(child, old_id, new_id, replaced);
            }
        }
        serde_json::Value::Array(array) => {
            // A rule tuple leads with the morphology it applies.
            if let Some(first) = array.first_mut() {
                if first.as_str() == Some(old_id) {
                    *first = serde_json::Value::String(new_id.to_string());
                    *replaced += 1;
                }
            }
            for child in array.iter_mut() {
                rename_morphology_references(child, old_id, new_id, replaced);
            }
        }
        _ => {}
    }
}

/// Reports whole-connectome transport as unavailable in this adapter.
fn connectome_transport_unavailable() -> ServiceError {
    ServiceError::NotImplemented(
        "connectome import/export is a legacy Brain Development responsibility and is not exposed \
         by this server yet"
            .to_string(),
    )
}

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

/// System introspection.
///
/// Structure figures (cortical areas, neurons, regions) describe the developed brain and are read
/// from the BDU's [`DevelopedBrain`]; burst state (running, burst count) is the engine's and is
/// read from the wrapped NPU. Version reporting is real. Engine-only metrics that neither side can
/// supply yet (runtime stats, memory usage) are reported as unavailable.
pub struct StubSystemService {
    version_info: VersionInfo,
    wnpu: SharedWnpu,
    brain: Arc<DevelopedBrain>,
}

impl StubSystemService {
    pub fn new(version_info: VersionInfo, wnpu: SharedWnpu, brain: Arc<DevelopedBrain>) -> Self {
        Self {
            version_info,
            wnpu,
            brain,
        }
    }

    /// Cortical areas and neurons the BDU developed, defaulting to zero for an undeveloped brain.
    fn structure_counts(&self) -> (usize, usize, usize) {
        match self.brain.report() {
            Some(report) => (report.areas_added, report.neurons_added as usize, 0),
            None => (0, 0, 0),
        }
    }

    /// Brain regions declared by the developed genome, or zero when none is loaded.
    fn brain_region_count(&self) -> usize {
        self.brain
            .genome()
            .map(|genome| genome.brain_regions.len())
            .unwrap_or(0)
    }
}

#[async_trait]
impl SystemService for StubSystemService {
    async fn get_health(&self) -> ServiceResult<HealthStatus> {
        // Health answers in every state; the detailed brain figures are served through
        // `AnalyticsService` (`/v1/system/health_check`) from the same BDU source.
        Ok(HealthStatus {
            overall_status: "healthy".to_string(),
            components: Vec::new(),
            timestamp: String::new(),
        })
    }

    async fn get_status(&self) -> ServiceResult<SystemStatus> {
        let (cortical_area_count, neuron_count, synapse_count) = self.structure_counts();
        let brain_region_count = self.brain_region_count();
        let (is_running, burst_count) = {
            let wnpu = self.wnpu.lock();
            (wnpu.is_running(), wnpu.bursts_completed().unwrap_or(0))
        };
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
        Ok(self.structure_counts().0 > 0)
    }

    async fn get_burst_count(&self) -> ServiceResult<u64> {
        Ok(self.wnpu.lock().bursts_completed().unwrap_or(0))
    }

    async fn get_runtime_stats(&self) -> ServiceResult<RuntimeStats> {
        Err(ServiceError::NotImplemented(
            "runtime statistics need a running burst engine, which this server does not expose yet"
                .to_string(),
        ))
    }

    async fn get_memory_usage(&self) -> ServiceResult<MemoryUsage> {
        Err(ServiceError::NotImplemented(
            "memory usage needs a running engine to measure, which this server does not expose yet"
                .to_string(),
        ))
    }

    async fn get_capacity(&self) -> ServiceResult<CapacityInfo> {
        // Capacities are configured allocation limits held by the BDU; current occupancy comes
        // from the development report. Areas have no configured maximum, so it is reported as zero.
        let (current_cortical_areas, current_neurons, current_synapses) = self.structure_counts();
        let max_neurons = self.brain.neuron_capacity();
        let max_synapses = self.brain.synapse_capacity();
        let utilization = |current: usize, max: usize| -> f64 {
            if max == 0 {
                0.0
            } else {
                (current as f64 / max as f64) * 100.0
            }
        };
        Ok(CapacityInfo {
            current_neurons,
            max_neurons,
            neuron_utilization_percent: utilization(current_neurons, max_neurons),
            current_synapses,
            max_synapses,
            synapse_utilization_percent: utilization(current_synapses, max_synapses),
            current_cortical_areas,
            max_cortical_areas: 0,
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

    // The agent registry - who is connected, their network properties, and their shared-memory
    // descriptors - is owned by the transport layer (`FeagiAgentHandler`) and the legacy code, not
    // by the NPU. WNPU exposes only the data-channel side, so these registry reads are unavailable
    // through this adapter.
    async fn list_agents(&self) -> AgentResult<Vec<String>> {
        Err(agent_registry_unavailable())
    }

    async fn get_agent_properties(&self, _agent_id: &str) -> AgentResult<AgentProperties> {
        Err(agent_registry_unavailable())
    }

    async fn get_shared_memory_info(
        &self,
    ) -> AgentResult<HashMap<String, HashMap<String, serde_json::Value>>> {
        Err(agent_registry_unavailable())
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

/// Reports an agent-registry read as unavailable in this adapter.
fn agent_registry_unavailable() -> AgentError {
    AgentError::ServiceUnavailable(
        "the agent registry is owned by the transport/legacy layer and is not exposed through the \
         NPU adapter"
            .to_string(),
    )
}

// ============================================================================
// SNAPSHOT
// ============================================================================

/// Snapshotting the connectome (and optionally runtime state) is a legacy Brain Development
/// responsibility. The NPU no longer exposes it and this adapter has no BDU path for it, so every
/// snapshot operation reports as unavailable.
pub struct StubSnapshotService;

impl StubSnapshotService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubSnapshotService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SnapshotService for StubSnapshotService {
    async fn create_snapshot(
        &self,
        _options: SnapshotCreateOptions,
    ) -> ServiceResult<SnapshotMetadata> {
        Err(snapshot_unavailable())
    }

    async fn restore_snapshot(&self, _snapshot_id: &str) -> ServiceResult<()> {
        Err(snapshot_unavailable())
    }

    async fn list_snapshots(&self) -> ServiceResult<Vec<SnapshotMetadata>> {
        Err(snapshot_unavailable())
    }

    async fn delete_snapshot(&self, _snapshot_id: &str) -> ServiceResult<()> {
        Err(snapshot_unavailable())
    }

    async fn get_snapshot_artifact(
        &self,
        _snapshot_id: &str,
        _format: &str,
    ) -> ServiceResult<Vec<u8>> {
        Err(snapshot_unavailable())
    }
}

/// Reports a snapshot operation as unavailable in this adapter.
fn snapshot_unavailable() -> ServiceError {
    ServiceError::NotImplemented(
        "connectome snapshots are a legacy Brain Development responsibility and are not exposed by \
         this server yet"
            .to_string(),
    )
}
