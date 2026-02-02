//! Storage Profiling Test
//!
//! This test measures binary sizes and storage characteristics of FEAGI builds.
//!
//! ## Running
//! ```bash
//! cargo test --test profile_storage -- --nocapture
//! ```
//!
//! ## Output
//! - Console output with binary size statistics
//! - JSON file: `target/profile_storage_results.json`

use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Storage metrics for a binary
#[derive(Debug, Clone)]
struct StorageMetrics {
    binary_name: String,
    size_bytes: u64,
    size_mb: f64,
    profile: String,
    stripped: bool,
}

impl StorageMetrics {
    fn from_file(
        path: &Path,
        binary_name: String,
        profile: String,
        stripped: bool,
    ) -> Option<Self> {
        fs::metadata(path).ok().map(|metadata| {
            let size_bytes = metadata.len();
            let size_mb = size_bytes as f64 / (1024.0 * 1024.0);

            Self {
                binary_name,
                size_bytes,
                size_mb,
                profile,
                stripped,
            }
        })
    }
}

/// Profile binary sizes across different build configurations
fn profile_binary_sizes() -> Vec<StorageMetrics> {
    let mut metrics = Vec::new();

    // Check debug build
    let debug_path = Path::new("target/debug/feagi");
    if let Some(m) =
        StorageMetrics::from_file(debug_path, "feagi".to_string(), "debug".to_string(), false)
    {
        metrics.push(m);
    }

    // Check release build
    let release_path = Path::new("target/release/feagi");
    if let Some(m) = StorageMetrics::from_file(
        release_path,
        "feagi".to_string(),
        "release".to_string(),
        true,
    ) {
        metrics.push(m);
    }

    // Check profiling build (if exists)
    let profiling_path = Path::new("target/profiling/feagi");
    if let Some(m) = StorageMetrics::from_file(
        profiling_path,
        "feagi".to_string(),
        "profiling".to_string(),
        false,
    ) {
        metrics.push(m);
    }

    metrics
}

/// Get dependency crate sizes
fn profile_dependency_sizes() -> Vec<StorageMetrics> {
    let mut metrics = Vec::new();

    // Check release/deps directory for rlibs
    let deps_dir = Path::new("target/release/deps");

    if deps_dir.exists() {
        if let Ok(entries) = fs::read_dir(deps_dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Look for .rlib files (compiled libraries)
                if path.extension().and_then(|s| s.to_str()) == Some("rlib") {
                    if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                        // Skip test artifacts
                        if !filename.contains("test") {
                            if let Some(m) = StorageMetrics::from_file(
                                &path,
                                filename.to_string(),
                                "release".to_string(),
                                false,
                            ) {
                                metrics.push(m);
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort by size (largest first)
    metrics.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    // Keep only top 20
    metrics.truncate(20);

    metrics
}

/// Print storage metrics table
fn print_storage_metrics_table(metrics: &[StorageMetrics], title: &str) {
    println!("\n{}", "=".repeat(80));
    println!("{}", title);
    println!("{}", "=".repeat(80));
    println!(
        "{:<40} {:<15} {:>12} {:>10}",
        "Binary", "Profile", "Size (MB)", "Stripped"
    );
    println!("{}", "-".repeat(80));

    for metric in metrics {
        println!(
            "{:<40} {:<15} {:>12.2} {:>10}",
            metric.binary_name,
            metric.profile,
            metric.size_mb,
            if metric.stripped { "Yes" } else { "No" }
        );
    }

    println!("{}", "=".repeat(80));
}

/// Calculate size comparison
fn print_size_comparison(metrics: &[StorageMetrics]) {
    if metrics.len() < 2 {
        return;
    }

    println!("\n{}", "=".repeat(80));
    println!("Size Comparison");
    println!("{}", "=".repeat(80));

    // Find debug and release builds
    let debug = metrics.iter().find(|m| m.profile == "debug");
    let release = metrics.iter().find(|m| m.profile == "release");

    if let (Some(debug), Some(release)) = (debug, release) {
        let ratio = debug.size_mb / release.size_mb;
        let reduction = ((debug.size_mb - release.size_mb) / debug.size_mb) * 100.0;

        println!("Debug build:    {:.2} MB", debug.size_mb);
        println!("Release build:  {:.2} MB", release.size_mb);
        println!("Ratio:          {:.2}x larger (debug vs release)", ratio);
        println!("Size reduction: {:.1}% (debug to release)", reduction);
    }

    println!("{}", "=".repeat(80));
}

/// Save storage metrics to JSON
fn save_storage_metrics_to_json(
    binary_metrics: &[StorageMetrics],
    dependency_metrics: &[StorageMetrics],
) -> std::io::Result<()> {
    let output = json!({
        "timestamp": format!("{:?}", std::time::SystemTime::now()),
        "version": env!("CARGO_PKG_VERSION"),
        "binaries": binary_metrics.iter().map(|m| {
            json!({
                "name": m.binary_name,
                "profile": m.profile,
                "size_bytes": m.size_bytes,
                "size_mb": m.size_mb,
                "stripped": m.stripped,
            })
        }).collect::<Vec<_>>(),
        "top_dependencies": dependency_metrics.iter().map(|m| {
            json!({
                "name": m.binary_name,
                "size_bytes": m.size_bytes,
                "size_mb": m.size_mb,
            })
        }).collect::<Vec<_>>(),
    });

    let mut file = fs::File::create("target/profile_storage_results.json")?;
    file.write_all(serde_json::to_string_pretty(&output)?.as_bytes())?;

    println!("\n✓ Results saved to: target/profile_storage_results.json");

    Ok(())
}

#[test]
fn test_storage_profiling() {
    println!("\n💾 FEAGI Storage Profiling");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));

    // Profile binary sizes
    let binary_metrics = profile_binary_sizes();

    if binary_metrics.is_empty() {
        println!("\n⚠️  No binaries found. Build the project first:");
        println!("  cargo build");
        println!("  cargo build --release");
        return;
    }

    print_storage_metrics_table(&binary_metrics, "FEAGI Binary Sizes");
    print_size_comparison(&binary_metrics);

    // Profile dependency sizes
    let dependency_metrics = profile_dependency_sizes();
    if !dependency_metrics.is_empty() {
        print_storage_metrics_table(&dependency_metrics, "Top 20 Largest Dependencies");
    }

    // Save results
    save_storage_metrics_to_json(&binary_metrics, &dependency_metrics)
        .expect("Failed to save storage metrics");

    println!("\n✅ Storage profiling complete!");

    // Print recommendations
    println!("\n📊 Recommendations:");
    if let Some(release) = binary_metrics.iter().find(|m| m.profile == "release") {
        if release.size_mb > 50.0 {
            println!(
                "  ⚠️  Release binary is quite large ({:.2} MB)",
                release.size_mb
            );
            println!("  Consider:");
            println!("    - Reviewing dependency tree: cargo tree");
            println!("    - Using cargo-bloat: cargo install cargo-bloat && cargo bloat --release");
            println!("    - Enabling LTO and optimization in Cargo.toml (already done)");
        } else {
            println!(
                "  ✓ Release binary size is reasonable ({:.2} MB)",
                release.size_mb
            );
        }
    }
}
