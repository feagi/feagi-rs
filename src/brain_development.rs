// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Brain development, and the health data derived from it.
//!
//! `/v1/system/health_check` describes what the brain *is* rather than what the engine is doing:
//! how many cortical areas exist, how many neurons they hold, whether the genome behind them
//! developed cleanly. All of that is decided by the BDU (`feagi-brain-development`) when it
//! translates a genome into a connectome, so [`DevelopedBrain`] keeps the BDU's own report of the
//! last development run and [`BduAnalyticsService`] answers the endpoint from it.
//!
//! Three groups of fields cannot come from the BDU and are sourced deliberately:
//!
//! - `burst_engine_active` and `burst_count` belong to the engine, and are read from whichever
//!   [`RuntimeService`] the server is running.
//! - `neuron_capacity` and `synapse_capacity` come from `[connectome]` config: they are allocation
//!   limits chosen before any brain exists, so no brain can report them.
//! - `genome_validity` is the migration chain's validator verdict, which [`develop_genome_file`]
//!   publishes to the state manager for the endpoint to read back.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::async_trait;
use feagi_brain_development::{develop_connectome_requests, CorticogenesisReport};
use feagi_config::ConnectomeConfig;
use feagi_services::types::*;
use feagi_services::{AnalyticsService, RuntimeService};
use feagi_state_manager::{GenomeState, StateManager};
use parking_lot::RwLock;
use tracing::{info, warn};

/// The brain the BDU has developed, shared between the genome loader and the API services.
///
/// Holds the BDU's report rather than recomputing it: health is polled continuously (the brain
/// visualizer polls it several times a second) while development happens once per genome, so the
/// report cannot change between polls and re-deriving it would pay for the whole translation each
/// time. `None` means no genome has been developed and the brain is empty.
pub struct DevelopedBrain {
    report: RwLock<Option<CorticogenesisReport>>,
    neuron_capacity: usize,
    synapse_capacity: usize,
}

impl DevelopedBrain {
    /// Creates an undeveloped brain whose capacities are the configured allocation limits.
    pub fn new(connectome: &ConnectomeConfig) -> Self {
        Self {
            report: RwLock::new(None),
            neuron_capacity: connectome.neuron_space,
            synapse_capacity: connectome.synapse_space,
        }
    }

    /// Records what corticogenesis produced, replacing any previous brain.
    pub fn record(&self, report: CorticogenesisReport) {
        *self.report.write() = Some(report);
    }

    /// Forgets the current brain, returning the server to its undeveloped state.
    pub fn clear(&self) {
        *self.report.write() = None;
    }

    /// The last development run, or `None` while no genome has been developed.
    pub fn report(&self) -> Option<CorticogenesisReport> {
        self.report.read().clone()
    }
}

/// Loads a genome from disk and develops it into `brain`.
///
/// Mirrors `feagi_api::services::genome::realise_genome_json`, minus the step that hands the
/// resulting requests to an engine: that needs `WrappedNeuronProcessingUnit::request_change`, which
/// is not implemented yet. The genome is still developed rather than skipped, because the BDU's
/// report is what health reports the brain to be, and development failures (unrepresentable
/// dimensions, a missing `neurons_per_voxel`) are worth surfacing at load time rather than at
/// NPU-integration time.
pub fn develop_genome_file(path: &Path, brain: &DevelopedBrain) -> Result<CorticogenesisReport> {
    let state_manager = StateManager::instance();
    state_manager.read().set_genome_state(GenomeState::Loading);

    let result = develop_genome_file_inner(path, brain);
    if result.is_err() {
        state_manager.read().set_genome_state(GenomeState::Error);
    }
    result
}

fn develop_genome_file_inner(path: &Path, brain: &DevelopedBrain) -> Result<CorticogenesisReport> {
    // The migration chain reports validator findings instead of failing on them, so a genome with
    // blocking errors still develops and the finding is published as `genome_validity` for health.
    let (mut genome, chain_report) = feagi_evolutionary::load_genome_with_report_from_file(path)
        .with_context(|| format!("failed to parse genome '{}'", path.display()))?;

    let state_manager = StateManager::instance();
    state_manager
        .read()
        .get_core_state()
        .set_genome_validity(Some(chain_report.is_blocking_clean()));

    if !chain_report.is_blocking_clean() {
        warn!(
            "    ⚠ Genome has {} blocking validator error(s); developing in degraded mode \
             (genome_validity=false). First: {}",
            chain_report.blocking_errors.len(),
            chain_report
                .blocking_errors
                .first()
                .map(String::as_str)
                .unwrap_or("")
        );
    }

    let (_, morphologies_added) = feagi_evolutionary::ensure_core_components(&mut genome);
    if morphologies_added > 0 {
        info!(
            "    ✓ Added {} missing core morphologies",
            morphologies_added
        );
    }

    // Runs after the core areas are in place so they are placed in the hierarchy too. A genome need
    // not declare any region, and a cortical area cannot be placed without a root.
    let (root_region_id, root_was_created) =
        feagi_evolutionary::ensure_root_brain_region(&mut genome);
    if root_was_created {
        info!(
            "    ✓ Genome declared no root brain region; created {}",
            root_region_id
        );
    }

    let (requests, report) = develop_connectome_requests(&genome)
        .with_context(|| format!("corticogenesis failed for genome '{}'", path.display()))?;

    // The requests are the BDU's output for an engine to apply. There is no entry point on the
    // wrapped NPU to submit them through yet, so they are dropped here and the brain exists only as
    // the report that health serves. This is where `request_change` gets called once it lands.
    if !requests.is_empty() {
        warn!(
            "    ⚠ {} connectome request(s) produced but not submitted: the wrapped NPU has no \
             entry point for them yet, so the brain is described but not simulated",
            requests.len()
        );
    }

    if report.mappings_deferred > 0 {
        warn!(
            "    ⚠ Genome declares {} mapping(s) corticogenesis cannot realise yet; the brain has \
             no synapses and activity would not propagate between areas",
            report.mappings_deferred
        );
    }

    brain.record(report.clone());
    state_manager.read().set_genome_state(GenomeState::Loaded);

    Ok(report)
}

/// [`AnalyticsService`] answered from the BDU's development report.
///
/// Every count describes the developed brain, so it is read from [`DevelopedBrain`]. The runtime
/// service is consulted only for the two fields the engine owns: whether bursts are running, and
/// how many have elapsed.
pub struct BduAnalyticsService {
    brain: Arc<DevelopedBrain>,
    runtime: Arc<dyn RuntimeService + Send + Sync>,
}

impl BduAnalyticsService {
    pub fn new(brain: Arc<DevelopedBrain>, runtime: Arc<dyn RuntimeService + Send + Sync>) -> Self {
        Self { brain, runtime }
    }

    /// Cortical areas the BDU developed. An undeveloped brain holds none, which is a reportable
    /// state rather than an error.
    fn cortical_area_count(&self) -> usize {
        self.brain
            .report()
            .map(|report| report.areas_added)
            .unwrap_or(0)
    }

    /// Neurons the BDU allocated across every developed area.
    fn neuron_count(&self) -> usize {
        self.brain
            .report()
            .map(|report| report.neurons_added as usize)
            .unwrap_or(0)
    }

    /// Whether the engine is turning the brain over, and how many bursts it has completed.
    ///
    /// A runtime service that cannot answer is reported as stopped: health has to answer in every
    /// state, and "the engine did not report" is indistinguishable from "not bursting" to a caller
    /// deciding whether the brain is live.
    async fn burst_state(&self) -> (bool, u64) {
        match self.runtime.get_status().await {
            Ok(status) => (status.is_running, status.burst_count),
            Err(_) => (false, 0),
        }
    }
}

#[async_trait]
impl AnalyticsService for BduAnalyticsService {
    /// Reports health in every state, including before a genome has been developed.
    ///
    /// This is what a monitor polls to learn whether the server came up, so it answers rather than
    /// failing and describes what is missing through its fields.
    async fn get_system_health(&self) -> ServiceResult<SystemHealth> {
        let cortical_area_count = self.cortical_area_count();
        let (burst_engine_active, burst_count) = self.burst_state().await;

        // `None` covers both "no genome developed yet" and "state manager momentarily locked";
        // either way there is no verdict to report.
        let state_manager = StateManager::instance();
        let genome_validity = state_manager
            .try_read()
            .and_then(|state_manager| state_manager.get_core_state().get_genome_validity());

        Ok(SystemHealth {
            burst_engine_active,
            // A brain is ready only once it has structure and the engine is turning it over.
            // Reporting readiness on either alone lets a visualizer leave its loading screen before
            // there is anything to show.
            brain_readiness: cortical_area_count > 0 && burst_engine_active,
            genome_validity,
            neuron_count: self.neuron_count(),
            neuron_capacity: self.brain.neuron_capacity,
            synapse_capacity: self.brain.synapse_capacity,
            cortical_area_count,
            burst_count,
        })
    }

    async fn get_cortical_area_stats(
        &self,
        _cortical_id: &str,
    ) -> ServiceResult<CorticalAreaStats> {
        Err(per_area_unavailable("cortical area stats"))
    }

    async fn get_all_cortical_area_stats(&self) -> ServiceResult<Vec<CorticalAreaStats>> {
        Err(per_area_unavailable("cortical area stats"))
    }

    async fn get_connectivity_stats(
        &self,
        _source_area: &str,
        _target_area: &str,
    ) -> ServiceResult<ConnectivityStats> {
        Err(per_area_unavailable("connectivity stats"))
    }

    async fn get_total_neuron_count(&self) -> ServiceResult<usize> {
        Ok(self.neuron_count())
    }

    /// Always zero: corticogenesis defers every mapping the genome declares, so no synapse has
    /// been created. See `CorticogenesisReport::mappings_deferred`, which development logs.
    async fn get_total_synapse_count(&self) -> ServiceResult<usize> {
        Ok(0)
    }

    /// Empty: the BDU's report is an aggregate over the whole brain, and per-area occupancy is a
    /// property of the engine holding the neurons.
    async fn get_populated_areas(&self) -> ServiceResult<Vec<(String, usize)>> {
        Ok(Vec::new())
    }

    async fn get_neuron_density(&self, _cortical_id: &str) -> ServiceResult<f32> {
        Err(per_area_unavailable("neuron density"))
    }

    async fn is_brain_initialized(&self) -> ServiceResult<bool> {
        Ok(self.cortical_area_count() > 0)
    }

    async fn is_burst_engine_ready(&self) -> ServiceResult<bool> {
        Ok(self.burst_state().await.0)
    }

    /// Every neuron the BDU develops is a regular one: memory neurons are created during a run by
    /// neuroplasticity, not allocated at development time.
    async fn get_regular_neuron_count(&self) -> ServiceResult<usize> {
        Ok(self.neuron_count())
    }

    /// Always zero while nothing creates memory neurons; see [`Self::get_regular_neuron_count`].
    async fn get_memory_neuron_count(&self) -> ServiceResult<usize> {
        Ok(0)
    }
}

/// Reports a per-area or per-neuron statistic as unavailable.
///
/// The BDU reports the brain it built as a whole, so a breakdown has to be read from the engine
/// holding it, and there is no engine to read yet.
fn per_area_unavailable(statistic: &str) -> ServiceError {
    ServiceError::NotImplemented(format!(
        "{statistic}: the BDU reports the developed brain in aggregate, and per-area figures need \
         a running NPU to read them from"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub_services::StubRuntimeService;

    fn service(brain: Arc<DevelopedBrain>) -> BduAnalyticsService {
        BduAnalyticsService::new(
            brain,
            Arc::new(StubRuntimeService::new(10.0)) as Arc<dyn RuntimeService + Send + Sync>,
        )
    }

    fn brain() -> Arc<DevelopedBrain> {
        Arc::new(DevelopedBrain::new(&ConnectomeConfig::default()))
    }

    /// Health has to answer before anything is developed, since that is when a monitor is waiting
    /// to learn whether the server came up at all.
    #[tokio::test]
    async fn undeveloped_brain_reports_health_rather_than_failing() {
        let health = service(brain()).get_system_health().await.unwrap();

        assert_eq!(health.cortical_area_count, 0);
        assert_eq!(health.neuron_count, 0);
        assert!(!health.brain_readiness);
        // Capacities are config limits, so they are reported even with no brain to fill them.
        assert_eq!(
            health.neuron_capacity,
            ConnectomeConfig::default().neuron_space
        );
    }

    #[tokio::test]
    async fn health_counts_come_from_the_development_report() {
        let brain = brain();
        brain.record(CorticogenesisReport {
            areas_added: 7,
            neurons_added: 4200,
            mappings_deferred: 3,
        });
        let service = service(brain);

        let health = service.get_system_health().await.unwrap();
        assert_eq!(health.cortical_area_count, 7);
        assert_eq!(health.neuron_count, 4200);
        assert!(service.is_brain_initialized().await.unwrap());

        // Every developed neuron is a regular one, and the deferred mappings mean no synapse
        // exists to count.
        assert_eq!(service.get_regular_neuron_count().await.unwrap(), 4200);
        assert_eq!(service.get_memory_neuron_count().await.unwrap(), 0);
        assert_eq!(service.get_total_synapse_count().await.unwrap(), 0);
    }

    /// A brain with structure is still not ready while nothing is bursting, so a visualizer keeps
    /// its loading screen up instead of showing a static brain.
    #[tokio::test]
    async fn readiness_needs_a_running_engine_as_well_as_structure() {
        let brain = brain();
        brain.record(CorticogenesisReport {
            areas_added: 1,
            neurons_added: 10,
            mappings_deferred: 0,
        });

        let health = service(brain).get_system_health().await.unwrap();
        assert!(!health.burst_engine_active);
        assert!(!health.brain_readiness);
    }

    #[tokio::test]
    async fn clearing_returns_the_brain_to_undeveloped() {
        let brain = brain();
        brain.record(CorticogenesisReport {
            areas_added: 2,
            neurons_added: 20,
            mappings_deferred: 0,
        });
        brain.clear();

        assert_eq!(
            service(brain)
                .get_system_health()
                .await
                .unwrap()
                .cortical_area_count,
            0
        );
    }
}
