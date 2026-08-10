// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Loading a genome file all the way into a live NPU.

use std::path::PathBuf;

use feagi::genome::{load_genome_file, GenomeError};
use feagi::npu::NpuHandle;

/// The shared barebones genome, read from `feagi-evolutionary` rather than copied here so the
/// two cannot drift apart.
fn barebones_genome_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../feagi-core/crates/feagi-evolutionary/genomes/barebones_genome.json")
}

#[test]
fn barebones_genome_populates_the_npu() {
    let npu = NpuHandle::new(10);
    let shared_genome = feagi::empty_shared_genome();

    let summary = load_genome_file(&npu, &shared_genome, &barebones_genome_path())
        .expect("genome should load");

    assert!(
        summary.areas_added > 0,
        "barebones genome should contribute cortical areas"
    );
    assert!(
        summary.neurons_added > 0,
        "barebones genome should contribute neurons"
    );
    assert_eq!(
        npu.cortical_areas().len(),
        summary.areas_added,
        "every area the summary reports should appear in the NPU registry"
    );
}

#[test]
fn registry_neuron_counts_match_the_summary() {
    let npu = NpuHandle::new(10);
    let shared_genome = feagi::empty_shared_genome();

    let summary = load_genome_file(&npu, &shared_genome, &barebones_genome_path())
        .expect("genome should load");

    let registry_neurons: u64 = npu
        .cortical_areas()
        .iter()
        .map(|area| area.neuron_count)
        .sum();

    assert_eq!(
        registry_neurons, summary.neurons_added,
        "the registry and corticogenesis should agree on the neuron count"
    );
}

#[test]
fn loaded_genome_bursts_without_panicking() {
    let npu = NpuHandle::new(10);
    let shared_genome = feagi::empty_shared_genome();
    load_genome_file(&npu, &shared_genome, &barebones_genome_path()).expect("genome should load");

    for _ in 0..32 {
        npu.step_once();
    }

    assert_eq!(npu.burst_count(), 32);
}

#[test]
fn a_loaded_genome_eventually_reports_fire_activity() {
    let npu = NpuHandle::new(10);
    let shared_genome = feagi::empty_shared_genome();
    load_genome_file(&npu, &shared_genome, &barebones_genome_path()).expect("genome should load");

    // The placeholder neuron model fires on a fixed period, so a full period is enough to see
    // activity without depending on which burst within it fires.
    let saw_activity = (0..32).any(|_| {
        npu.step_once();
        npu.fire_queue_snapshot()
            .iter()
            .any(|(_, snapshot)| !snapshot.is_empty())
    });

    assert!(
        saw_activity,
        "a populated NPU should report firing neurons within one firing period"
    );
}

#[test]
fn a_missing_genome_file_is_reported_as_a_read_error() {
    let npu = NpuHandle::new(10);
    let shared_genome = feagi::empty_shared_genome();
    let missing = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/no_such_genome.json");

    match load_genome_file(&npu, &shared_genome, &missing) {
        Err(GenomeError::Read { .. }) => {}
        Err(other) => panic!("expected a read error, got {other}"),
        Ok(_) => panic!("loading a missing file should fail"),
    }

    assert!(
        npu.cortical_areas().is_empty(),
        "a failed load should leave the NPU untouched"
    );
}

#[test]
fn malformed_genome_json_is_reported_as_a_parse_error() {
    let npu = NpuHandle::new(10);
    let shared_genome = feagi::empty_shared_genome();

    match feagi::genome::load_genome_json(&npu, &shared_genome, "{ not a genome }") {
        Err(GenomeError::Parse(_)) => {}
        Err(other) => panic!("expected a parse error, got {other}"),
        Ok(_) => panic!("loading malformed json should fail"),
    }
}
