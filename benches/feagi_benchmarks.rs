//! FEAGI Performance Benchmarks
//!
//! Baseline benchmarks only. NPU benchmarks are pending integration of the rewritten feagi-core
//! NPU (`feagi_npu::wnpu`).
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
///
/// Placeholder until the rewritten NPU is integrated. The previous sketch targeted the removed
/// `feagi-npu-burst-engine` API; rewrite it against `feagi_npu::wnpu` instead.
fn bench_npu_init(_c: &mut Criterion) {
    println!("⚠️  NPU benchmarks are pending integration of the rewritten feagi-core NPU");
}

criterion_group!(
    benches,
    bench_vec_operations,
    bench_hashmap_operations,
    bench_npu_init,
);

criterion_main!(benches);
