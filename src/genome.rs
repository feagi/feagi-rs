// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Loading a genome file into the running NPU.
//!
//! Reading the file is all this module does on its own. Everything after that is the shared
//! loading path in `feagi-api`, which the REST endpoints use as well: a genome loaded with
//! `--genome` and one uploaded to `/v1/genome/*` must produce the same brain, and they only do so
//! if there is one implementation to produce it.

use std::path::Path;

use feagi_api::services::genome::realise_genome_json;
use feagi_services::types::errors::ServiceError;

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

impl From<ServiceError> for GenomeError {
    /// Maps the shared loader's failures onto this crate's error, which distinguishes a genome
    /// that could not be read from one the engine could not build.
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::InvalidInput(message) => GenomeError::Parse(message),
            other => GenomeError::Corticogenesis(other.to_string()),
        }
    }
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
    let realisation = realise_genome_json(json, shared_genome, npu)?;

    Ok(GenomeLoadSummary {
        genome_title: realisation.info.genome_title,
        areas_added: realisation.areas_added,
        neurons_added: realisation.neurons_added,
        mappings_deferred: realisation.mappings_deferred,
    })
}
