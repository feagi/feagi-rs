// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Loading a genome file into the running NPU.
//!
//! The work splits across three crates and this module is only the seam between them:
//! `feagi-evolutionary` reads the file and brings it to the current schema, then
//! `feagi-brain-development` performs corticogenesis to produce connectome requests, and finally
//! those requests are submitted to the NPU here. Nothing about the genome format or the
//! translation lives in this crate.

use std::path::Path;

use tracing::{info, warn};

use feagi_brain_development::corticogenesis::develop_connectome_requests;
use feagi_evolutionary::load_genome_from_json;

use crate::npu::NpuHandle;
use crate::SharedGenome;

#[derive(Debug, thiserror::Error)]
pub enum GenomeError {
    #[error("failed to read genome file '{path}': {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse genome: {0}")]
    Parse(String),
    #[error("corticogenesis failed: {0}")]
    Corticogenesis(String),
}

/// What a genome load produced in the NPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenomeLoadSummary {
    pub genome_title: String,
    pub areas_added: usize,
    pub neurons_added: u64,
    /// Mappings the genome declares that corticogenesis could not realise. Non-zero means the
    /// brain has areas but no synapses between them, so nothing propagates.
    pub mappings_deferred: usize,
}

/// Reads a genome file and realises it in the NPU.
pub fn load_genome_file(
    npu: &NpuHandle,
    shared_genome: &SharedGenome,
    path: &Path,
) -> Result<GenomeLoadSummary, GenomeError> {
    let json = std::fs::read_to_string(path).map_err(|source| GenomeError::Read {
        path: path.display().to_string(),
        source,
    })?;

    load_genome_json(npu, shared_genome, &json)
}

/// Realises an in-memory genome document in the NPU and publishes it to the API services.
pub fn load_genome_json(
    npu: &NpuHandle,
    shared_genome: &SharedGenome,
    json: &str,
) -> Result<GenomeLoadSummary, GenomeError> {
    let genome =
        load_genome_from_json(json).map_err(|error| GenomeError::Parse(error.to_string()))?;

    let (requests, report) = develop_connectome_requests(&genome)
        .map_err(|error| GenomeError::Corticogenesis(error.to_string()))?;

    npu.submit_connectome_requests(requests);

    let summary = GenomeLoadSummary {
        genome_title: genome.metadata.genome_title.clone(),
        areas_added: report.areas_added,
        neurons_added: report.neurons_added,
        mappings_deferred: report.mappings_deferred,
    };

    // Publish only after corticogenesis succeeds, so the REST layer never reports a genome the
    // NPU was unable to realise.
    *shared_genome.write() = Some(genome);

    info!(
        target: "feagi-rs",
        genome = %summary.genome_title,
        areas = summary.areas_added,
        neurons = summary.neurons_added,
        "genome loaded"
    );

    if summary.mappings_deferred > 0 {
        warn!(
            target: "feagi-rs",
            deferred = summary.mappings_deferred,
            "genome declares mappings that corticogenesis cannot realise yet; \
             the brain has no synapses and activity will not propagate between areas"
        );
    }

    Ok(summary)
}
