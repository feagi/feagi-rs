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
///
/// The split NPU crates (`feagi-npu-neural`, `feagi-npu-runtime`, `feagi-npu-burst-engine`,
/// `feagi-npu-plasticity`) were dissolved in the NPU rewrite and their replacement is not linked
/// into this binary yet, so no NPU version is reported.
pub fn collect_version_info() -> VersionInfo {
    let mut crates = HashMap::new();

    // Core algorithms
    crates.insert(
        "feagi_brain_development".to_string(),
        feagi_brain_development::VERSION.to_string(),
    );
    crates.insert(
        "feagi_evolutionary".to_string(),
        feagi_evolutionary::VERSION.to_string(),
    );

    // Service & API layer
    crates.insert("feagi_api".to_string(), feagi_api::VERSION.to_string());
    crates.insert(
        "feagi_services".to_string(),
        feagi_services::VERSION.to_string(),
    );

    // Infrastructure
    crates.insert(
        "feagi_state_manager".to_string(),
        feagi_state_manager::VERSION.to_string(),
    );
    crates.insert(
        "feagi_config".to_string(),
        feagi_config::VERSION.to_string(),
    );
    crates.insert(
        "feagi_observability".to_string(),
        feagi_observability::VERSION.to_string(),
    );

    // Main binary
    crates.insert("feagi".to_string(), env!("CARGO_PKG_VERSION").to_string());

    // Build metadata
    let build_timestamp = option_env!("VERGEN_BUILD_TIMESTAMP")
        .unwrap_or("not configured (add vergen to build.rs)")
        .to_string();

    // Cargo sets this to the empty string when the package declares no `rust-version`.
    let rust_version = match option_env!("CARGO_PKG_RUST_VERSION") {
        Some(version) if !version.is_empty() => version.to_string(),
        _ => "not declared (add rust-version to Cargo.toml)".to_string(),
    };

    VersionInfo {
        crates,
        build_timestamp,
        rust_version,
    }
}
