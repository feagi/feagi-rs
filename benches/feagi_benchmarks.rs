//! FEAGI Performance Benchmarks
//!
//! Comprehensive benchmarks for FEAGI NPU operations.
//!
//! ## Running Benchmarks
//! ```bash
//! cd feagi
//! cargo bench
//! ```
//!
//! Results are saved to `target/criterion/` with HTML reports.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

// NOTE: These benchmarks are placeholders until the feagi crate structure is finalized.
// Once the Rust NPU API is stabilized in feagi-core, update these benchmarks to use it.

/// Benchmark basic Vec operations (baseline for comparison)
fn bench_vec_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("vec_operations");

    for size in [1_000, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut v = Vec::with_capacity(size);
                for i in 0..size {
                    v.push(black_box(i));
                }
                black_box(v);
            });
        });
    }

    group.finish();
}

/// Benchmark HashMap operations (baseline for comparison)
fn bench_hashmap_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashmap_operations");

    for size in [1_000, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut map = std::collections::HashMap::with_capacity(size);
                for i in 0..size {
                    map.insert(black_box(i), black_box(i * 2));
                }
                black_box(map);
            });
        });
    }

    group.finish();
}

/// TODO: Benchmark NPU initialization
/// This will be completed once the feagi-core NPU API is integrated
fn bench_npu_init(_c: &mut Criterion) {
    println!("⚠️  NPU benchmarks require integration with feagi-core");
    println!("   See feagi-core/crates/feagi-burst-engine/benches/backend_comparison.rs");
    println!("   for comprehensive CPU vs GPU burst processing benchmarks");

    // Example placeholder for future implementation:
    // let mut group = c.benchmark_group("npu_init");
    //
    // for (neurons, synapses, label) in [
    //     (1_000, 10_000, "small"),
    //     (10_000, 100_000, "medium"),
    //     (100_000, 1_000_000, "large"),
    // ] {
    //     group.throughput(Throughput::Elements(neurons as u64));
    //     group.bench_with_input(
    //         BenchmarkId::new("create_npu", label),
    //         &(neurons, synapses),
    //         |b, &(n, s)| {
    //             b.iter(|| {
    //                 let npu = RustNPU::<f32>::new_cpu_only(n, s, 10);
    //                 black_box(npu);
    //             });
    //         },
    //     );
    // }
    //
    // group.finish();
}

criterion_group!(
    benches,
    bench_vec_operations,
    bench_hashmap_operations,
    bench_npu_init,
);

criterion_main!(benches);
