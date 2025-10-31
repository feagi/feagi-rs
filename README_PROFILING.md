# FEAGI Rust Application Profiling System

This document explains how to measure memory, CPU, and storage profiles for the FEAGI Rust application across builds.

## Overview

The FEAGI profiling system provides comprehensive performance metrics tracking:

- **Memory Usage**: RSS, virtual memory, heap allocation patterns
- **CPU Performance**: Execution time, throughput, operations per second
- **Storage**: Binary sizes, debug vs release comparison
- **Benchmarks**: Statistical performance analysis with Criterion

## Quick Start

### Run Complete Profiling Suite

```bash
# From the feagi/ directory
./scripts/profile_all.sh
```

This runs:
1. Release build compilation
2. Memory profiling tests  
3. CPU profiling tests
4. Storage profiling tests
5. Criterion benchmarks

### Save Results for Tracking

```bash
./scripts/profile_all.sh --save-history
```

Results are saved to `profiling_history/<version>_<timestamp>/` for comparison across builds.

### Compare Two Builds

```bash
# Profile first build
./scripts/profile_all.sh --save-history

# Make code changes...

# Profile second build
./scripts/profile_all.sh --save-history

# Compare results
./scripts/compare_profiles.py \
    profiling_history/2.0.0_20250101_120000 \
    profiling_history/2.0.0_20250101_130000
```

##Current Status

⚠️ **Note**: The profiling tests are currently simplified stubs due to API compatibility.  
To make them fully functional, they need to be updated to match the current FEAGI Rust API.

The tests currently profile:
- Binary compilation and size metrics
- Basic memory and timing measurements
- Build configuration comparison

## What Gets Measured

### Memory Profiling
- Resident Set Size (RSS) - actual physical memory used
- Virtual memory allocation
- Memory growth patterns

**Run**: `cargo test --release --test profile_memory -- --nocapture`  
**Output**: `target/profile_memory_results.json`

### CPU Profiling  
- Execution time for operations
- Operations per second (throughput)
- Performance across different workload sizes

**Run**: `cargo test --release --test profile_cpu -- --nocapture`  
**Output**: `target/profile_cpu_results.json`

### Storage Profiling
- Binary sizes (debug, release, profiling builds)
- Comparison between build profiles
- Size reduction metrics

**Run**: `cargo test --release --test profile_storage -- --nocapture`  
**Output**: `target/profile_storage_results.json`

### Criterion Benchmarks
- Statistical analysis with confidence intervals
- Regression detection
- HTML reports with charts and graphs

**Run**: `cargo bench`  
**Output**: `target/criterion/report/index.html`

## Building Different Profiles

```bash
# Debug build (includes debug symbols, no optimization)
cargo build

# Release build (optimized, stripped)
cargo build --release

# Profiling build (optimized but with debug symbols)
cargo build --profile profiling
```

##Advanced Profiling Tools

###  Flamegraph (CPU Visualization)

```bash
# Install
cargo install flamegraph

# Generate CPU flamegraph
cargo flamegraph --bin feagi --release

# Opens flamegraph.svg in browser
```

### cargo-bloat (Binary Size Analysis)

```bash
# Install
cargo install cargo-bloat

# Analyze what takes space
cargo bloat --release

# Group by crate
cargo bloat --release --crates
```

### perf (Linux only)

```bash
# Record performance
perf record --call-graph dwarf ./target/release/feagi

# Generate report
perf report
```

### Instruments (macOS only)

Use Xcode Instruments for detailed macOS profiling:
- Time Profiler
- Allocations
- Leaks

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
        with:
          submodules: recursive
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          profile: minimal
      
      - name: Run Profiling Suite
        run: |
          cd feagi
          chmod +x scripts/profile_all.sh
          ./scripts/profile_all.sh --save-history
      
      - name: Upload Profiling Results
        uses: actions/upload-artifact@v3
        with:
          name: profiling-results-${{ github.sha }}
          path: feagi/profiling_history/
      
      - name: Upload Benchmark Reports
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-reports-${{ github.sha }}
          path: feagi/target/criterion/
```

## Performance Goals

Target metrics for FEAGI v2.0:

| Metric | Target | Notes |
|--------|--------|-------|
| Release Binary Size | < 50 MB | Stripped, LTO enabled |
| Memory per 1M neurons | < 500 MB | RSS measurement |
| Burst Processing | > 100 Hz | For 100K active neurons |
| Startup Time | < 5 seconds | From launch to API ready |
| API Response Time | < 10ms | P95 for simple queries |

Run profiling tests to see current values.

## Tracking Performance Over Time

### Automated Tracking Script

```bash
#!/bin/bash
# track_performance.sh

VERSION=$(cargo pkgid | cut -d# -f2 | cut -d: -f2)
DATE=$(date +%Y%m%d)

echo "Tracking performance for FEAGI $VERSION"

# Run profiling
cd feagi
./scripts/profile_all.sh --save-history

# Create git tag with performance metrics
git tag -a "perf-$VERSION-$DATE" -m "Performance profile for $VERSION"

echo "Tagged as perf-$VERSION-$DATE"
```

### Comparison Script

The included `compare_profiles.py` script provides:
- Color-coded improvements (green) and regressions (red)
- Percentage changes for all metrics
- Side-by-side comparison of two profiling runs

## Troubleshooting

### "Binary not found" Error

```bash
# Build the binaries first
cargo build
cargo build --release
```

### High Variance in Benchmarks

```bash
# Increase sample size
cargo bench -- --sample-size 200

# Increase warmup time
cargo bench -- --warm-up-time 10
```

### Permission Denied on Scripts

```bash
chmod +x scripts/profile_all.sh scripts/compare_profiles.py
```

## File Structure

```
feagi/
├── benches/
│   └── feagi_benchmarks.rs      # Criterion benchmarks
├── tests/
│   ├── profile_memory.rs         # Memory profiling test
│   ├── profile_cpu.rs            # CPU profiling test
│   └── profile_storage.rs        # Storage profiling test
├── scripts/
│   ├── profile_all.sh            # Run complete profiling suite
│   └── compare_profiles.py       # Compare two profiling runs
├── docs/
│   └── PROFILING_GUIDE.md        # Detailed profiling guide
├── profiling_history/            # Historical profiling data (created)
└── README_PROFILING.md           # This file
```

## Output Files

After running profiling:

```
target/
├── profile_memory_results.json   # Memory metrics
├── profile_cpu_results.json      # CPU metrics  
├── profile_storage_results.json  # Storage metrics
└── criterion/
    └── report/
        └── index.html            # Benchmark HTML reports
```

## Resources

- **Detailed Guide**: `docs/PROFILING_GUIDE.md`
- **Criterion Book**: https://bheisler.github.io/criterion.rs/book/
- **Rust Performance Book**: https://nnethercote.github.io/perf-book/
- **cargo-flamegraph**: https://github.com/flamegraph-rs/flamegraph
- **DHAT Profiler**: https://docs.rs/dhat/

## Contributing

When adding new profiling tests:

1. Add test to appropriate file (`tests/profile_*.rs`)
2. Update `benches/feagi_benchmarks.rs` if needed
3. Document new metrics in this file and `PROFILING_GUIDE.md`
4. Update performance goals table
5. Test with `./scripts/profile_all.sh`

## License

Apache-2.0 (same as FEAGI)

## Support

For questions about profiling:
- See `docs/PROFILING_GUIDE.md` for detailed information
- Check FEAGI documentation
- Open an issue on GitHub

---

**Last Updated**: October 31, 2025  
**FEAGI Version**: 2.0.0
