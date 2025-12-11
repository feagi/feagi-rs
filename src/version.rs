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
    crates.insert("feagi_bdu".to_string(), feagi_bdu::VERSION.to_string());
    crates.insert("feagi_burst_engine".to_string(), feagi_burst_engine::VERSION.to_string());
    crates.insert("feagi_evo".to_string(), feagi_evo::VERSION.to_string());
    crates.insert("feagi_plasticity".to_string(), feagi_plasticity::VERSION.to_string());
    
    // Service & API layer
    crates.insert("feagi_api".to_string(), feagi_api::VERSION.to_string());
    crates.insert("feagi_services".to_string(), feagi_services::VERSION.to_string());
    crates.insert("feagi_pns".to_string(), feagi_pns::VERSION.to_string());
    
    // Infrastructure
    crates.insert("feagi_state_manager".to_string(), feagi_state_manager::VERSION.to_string());
    crates.insert("feagi_neural".to_string(), feagi_neural::VERSION.to_string());
    crates.insert("feagi_config".to_string(), feagi_config::VERSION.to_string());
    crates.insert("feagi_observability".to_string(), feagi_observability::VERSION.to_string());
    crates.insert("feagi_connectome_serialization".to_string(), feagi_connectome_serialization::VERSION.to_string());
    
    // Runtime (std only - embedded would use different runtime)
    crates.insert("feagi_runtime".to_string(), feagi_runtime::VERSION.to_string());
    crates.insert("feagi_runtime_std".to_string(), feagi_runtime_std::VERSION.to_string());
    
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
