# FEAGI Profiling Guide

This guide explains how to use the profiling and benchmarking tools to measure FEAGI's memory, CPU, and storage characteristics across builds.

## Overview

FEAGI includes three types of profiling tools:

1. **Memory Profiling** - Measures heap allocation, RSS, and virtual memory usage
2. **CPU Profiling** - Measures execution time and operations per second
3. **Storage Profiling** - Measures binary sizes and dependency footprints
4. **Benchmarking** - Detailed performance benchmarks using Criterion

## Quick Start

### Run All Profiling Tests

```bash
# Build in release mode first
cargo build --release

# Run all profiling tests
cargo test --release profile_ -- --nocapture

# Or run individually
cargo test --release --test profile_memory -- --nocapture
cargo test --release --test profile_cpu -- --nocapture
cargo test --release --test profile_storage -- --nocapture
```

### Run Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench npu_initialization
```

## Detailed Usage

### Memory Profiling

Measures memory usage of FEAGI components:

```bash
cargo test --release --test profile_memory -- --nocapture
```

**What it measures:**
- RSS (Resident Set Size) - actual physical memory used
- Virtual memory allocation
- Memory per component (NPU, neurons, connectome manager)
- Memory growth with different neuron counts

**Output:**
- Console: Formatted tables with memory usage
- JSON: `target/profile_memory_results.json` (for tracking across builds)

**Example output:**
```
================================================================================
NPU Memory Usage
================================================================================
Component                        RSS (KB)      Virtual (KB)    Delta RSS (KB)
--------------------------------------------------------------------------------
baseline                         12345          45678                0
npu_1k_neurons                   14567          47890             +2222
npu_100k_neurons                 34567          78901            +22222
npu_1m_neurons                  234567         234567           +222222
================================================================================
```

### Advanced Memory Profiling with DHAT

For detailed heap profiling:

```bash
# Add dhat feature to Cargo.toml (optional)
cargo test --release --test profile_memory -- --nocapture

# View the generated dhat-heap.json at:
# https://nnethercote.github.io/dh_view/dh_view.html
```

### CPU Profiling

Measures execution time and throughput:

```bash
cargo test --release --test profile_cpu -- --nocapture
```

**What it measures:**
- Execution time for operations
- Operations per second
- NPU initialization speed
- Neuron creation speed
- Burst processing speed
- ConnectomeManager operation speed

**Output:**
- Console: Formatted tables with timing data
- JSON: `target/profile_cpu_results.json`

**Example output:**
```
======================================================================================
NPU Initialization Performance
======================================================================================
Operation                           Total Time  Iterations          Ops/Second
--------------------------------------------------------------------------------------
NPU init (1K neurons)                    0.123s         100              813.01
NPU init (100K neurons)                  1.234s          10                8.10
NPU init (1M neurons)                   12.345s           1                0.08
======================================================================================
```

### Flamegraph Generation

For visual CPU profiling:

```bash
# Install flamegraph
cargo install flamegraph

# Generate flamegraph
cargo flamegraph --test profile_cpu --release

# Opens flamegraph.svg in browser
```

**Interpreting flamegraphs:**
- Width = time spent in function
- Height = call stack depth
- Colors = random (for readability)
- Click to zoom into functions

### Storage Profiling

Measures binary sizes:

```bash
# Build different profiles first
cargo build
cargo build --release
cargo build --profile profiling

# Run storage profiling
cargo test --test profile_storage -- --nocapture
```

**What it measures:**
- Binary size for each build profile
- Top 20 largest dependencies
- Size reduction from debug to release
- Stripped vs unstripped sizes

**Output:**
- Console: Formatted tables with size data
- JSON: `target/profile_storage_results.json`

### Benchmarking with Criterion

Criterion provides statistical analysis of performance:

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark group
cargo bench npu_initialization

# Compare with previous baseline
cargo bench -- --save-baseline main
# Make changes...
cargo bench -- --baseline main
```

**Features:**
- Statistical analysis (mean, median, stddev)
- Regression detection
- HTML reports with charts
- Comparison between runs

**View results:**
```bash
# HTML reports generated in:
open target/criterion/report/index.html
```

## Tracking Performance Across Builds

### Set Up Performance Tracking

```bash
#!/bin/bash
# scripts/profile_build.sh

BUILD_DATE=$(date +%Y%m%d_%H%M%S)
VERSION=$(cargo pkgid | cut -d# -f2)
RESULTS_DIR="profiling_history"

mkdir -p "$RESULTS_DIR"

echo "Profiling FEAGI $VERSION at $BUILD_DATE"

# Build release
cargo build --release

# Run all profiling tests
cargo test --release profile_ -- --nocapture

# Copy results
cp target/profile_memory_results.json "$RESULTS_DIR/memory_${VERSION}_${BUILD_DATE}.json"
cp target/profile_cpu_results.json "$RESULTS_DIR/cpu_${VERSION}_${BUILD_DATE}.json"
cp target/profile_storage_results.json "$RESULTS_DIR/storage_${VERSION}_${BUILD_DATE}.json"

# Run benchmarks
cargo bench

echo "✓ Profiling complete. Results saved to $RESULTS_DIR/"
```

### Compare Performance Between Versions

```python
# scripts/compare_profiles.py
import json
import sys
from pathlib import Path

def compare_memory(old_file, new_file):
    with open(old_file) as f:
        old_data = json.load(f)
    with open(new_file) as f:
        new_data = json.load(f)
    
    # Compare specific metrics
    for component in new_data['npu_memory']:
        # Find matching component in old data
        old_component = next(
            (c for c in old_data['npu_memory'] if c['component'] == component['component']),
            None
        )
        if old_component:
            delta = component['rss_kb'] - old_component['rss_kb']
            pct = (delta / old_component['rss_kb']) * 100 if old_component['rss_kb'] > 0 else 0
            print(f"{component['component']}: {delta:+d} KB ({pct:+.1f}%)")

if __name__ == '__main__':
    compare_memory(sys.argv[1], sys.argv[2])
```

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Performance Profiling

on:
  push:
    branches: [main]
  pull_request:

jobs:
  profile:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Build Release
        run: cargo build --release
      
      - name: Run Memory Profiling
        run: cargo test --release --test profile_memory -- --nocapture
      
      - name: Run CPU Profiling
        run: cargo test --release --test profile_cpu -- --nocapture
      
      - name: Run Storage Profiling
        run: cargo test --release --test profile_storage -- --nocapture
      
      - name: Run Benchmarks
        run: cargo bench
      
      - name: Upload Results
        uses: actions/upload-artifact@v3
        with:
          name: profiling-results
          path: |
            target/profile_*.json
            target/criterion/
```

## Profiling Best Practices

### 1. Always Use Release Builds

```bash
# DON'T profile debug builds (unless specifically testing debug performance)
cargo test --test profile_cpu

# DO profile release builds
cargo test --release --test profile_cpu
```

### 2. Run Multiple Times

```bash
# Run profiling multiple times to account for variance
for i in {1..5}; do
    cargo test --release --test profile_cpu -- --nocapture
done
```

### 3. Isolate the System

- Close other applications
- Disable CPU frequency scaling if possible
- Run on same hardware for comparisons

### 4. Monitor Trends, Not Absolutes

- Track performance over time
- Look for regressions (>5% slowdown)
- Document expected ranges

### 5. Profile Realistic Workloads

The tests include typical workloads:
- 1K-1M neurons (small to large brains)
- 100-10K burst iterations
- Realistic connectome operations

## Troubleshooting

### "Binary not found" in storage profiling

```bash
# Build the binaries first
cargo build
cargo build --release
```

### Benchmarks show high variance

```bash
# Increase sample size
cargo bench -- --sample-size 100

# Increase warmup time
cargo bench -- --warm-up-time 5
```

### Memory profiling shows unexpected results

```bash
# Ensure you're in release mode
cargo test --release --test profile_memory -- --nocapture

# Check for memory leaks with Valgrind (Linux)
valgrind --leak-check=full --show-leak-kinds=all \
    cargo test --release --test profile_memory
```

## Additional Tools

### cargo-bloat

Identify what takes space in the binary:

```bash
cargo install cargo-bloat
cargo bloat --release
cargo bloat --release --crates  # Group by crate
```

### cargo-tree

Analyze dependency tree:

```bash
cargo tree
cargo tree --duplicates  # Find duplicate dependencies
cargo tree --edges normal  # Exclude dev dependencies
```

### perf (Linux only)

Advanced CPU profiling:

```bash
# Record performance data
perf record --call-graph dwarf cargo test --release --test profile_cpu

# Generate report
perf report
```

### Instruments (macOS only)

```bash
# Use Xcode Instruments for detailed profiling
instruments -t "Time Profiler" cargo test --release --test profile_cpu
```

## Performance Goals

Current targets (as of v2.0.0):

| Metric | Target | Current |
|--------|--------|---------|
| NPU init (1M neurons) | < 100ms | TBD |
| Neuron creation | > 10K/sec | TBD |
| Burst processing (100K neurons) | > 100 Hz | TBD |
| Memory per neuron | < 100 bytes | TBD |
| Release binary size | < 50 MB | TBD |

*Run profiling tests to populate "Current" values*

## References

- [Criterion.rs Documentation](https://bheisler.github.io/criterion.rs/book/)
- [DHAT Heap Profiler](https://docs.rs/dhat/)
- [The Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [cargo-flamegraph](https://github.com/flamegraph-rs/flamegraph)

