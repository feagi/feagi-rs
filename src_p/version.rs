// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Version information collector for feagi-rust
//!
//! This module is responsible for gathering version information from all crates
//! compiled into the feagi-rust binary. Each variant of FEAGI (rust, embedded, etc.)
//! should have its own version collector.

use feagi_services::types::VersionInfo;
use std::collections::HashMap;

/// Collect version information for all crates compiled into feagi-rust
///
/// NO HARDCODING - all versions come from env!("CARGO_PKG_VERSION") exported
/// by each crate as `pub const VERSION`
pub fn collect_version_info() -> VersionInfo {
    let mut crates = HashMap::new();

    // Core algorithms
    crates.insert(
        "feagi_brain_development".to_string(),
        feagi_brain_development::VERSION.to_string(),
    );
    crates.insert(
        "feagi_npu_burst_engine".to_string(),
        feagi_npu_burst_engine::VERSION.to_string(),
    );
    crates.insert(
        "feagi_evolutionary".to_string(),
        feagi_evolutionary::VERSION.to_string(),
    );
    crates.insert(
        "feagi_npu_plasticity".to_string(),
        feagi_npu_plasticity::VERSION.to_string(),
    );

    // Service & API layer
    crates.insert("feagi_api".to_string(), feagi_api::VERSION.to_string());
    crates.insert(
        "feagi_services".to_string(),
        feagi_services::VERSION.to_string(),
    );
    crates.insert("feagi_io".to_string(), "0.0.1-beta.12".to_string()); // TODO: Get from feagi-io crate metadata

    // Infrastructure
    crates.insert(
        "feagi_state_manager".to_string(),
        feagi_state_manager::VERSION.to_string(),
    );
    crates.insert(
        "feagi_npu_neural".to_string(),
        feagi_npu_neural::VERSION.to_string(),
    );
    crates.insert(
        "feagi_config".to_string(),
        feagi_config::VERSION.to_string(),
    );
    crates.insert(
        "feagi_observability".to_string(),
        feagi_observability::VERSION.to_string(),
    );
    // feagi-connectome-serialization moved to feagi-io::connectome (types in feagi-npu-neural)
    crates.insert("feagi_io_connectome".to_string(), "0.0.0".to_string());

    // Runtime (consolidated - std/embedded via features)
    crates.insert(
        "feagi_npu_runtime".to_string(),
        feagi_npu_runtime::VERSION.to_string(),
    );

    // Main binary
    crates.insert("feagi".to_string(), env!("CARGO_PKG_VERSION").to_string());

    // Build metadata
    let build_timestamp = option_env!("VERGEN_BUILD_TIMESTAMP")
        .unwrap_or("not configured (add vergen to build.rs)")
        .to_string();

    let rust_version = env!("CARGO_PKG_RUST_VERSION").to_string();

    VersionInfo {
        crates,
        build_timestamp,
        rust_version,
    }
}
