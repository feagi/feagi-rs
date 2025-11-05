# FEAGI Profiling System - Quick Start Guide

## What I've Created For You

I've set up a comprehensive profiling infrastructure for the FEAGI Rust application that will help you track memory, CPU, and storage metrics across builds.

## ✅ What's Working Right Now

### 1. Storage Profiling (Fully Functional)

```bash
cd feagi

# Build different profiles
cargo build
cargo build --release

# Run storage profiling
cargo test --release --test profile_storage -- --nocapture
```

**This works immediately and will show you:**
- Binary sizes for debug and release builds
- Size comparison (how much release is smaller)
- Top 20 largest dependencies
- Results saved to `target/profile_storage_results.json`

### 2. Basic Memory & CPU Profiling (Stub Versions)

```bash
# Memory profiling (stub version)
cargo test --release --test profile_memory -- --nocapture

# CPU profiling (stub version)
cargo test --release --test profile_cpu -- --nocapture
```

These run basic measurements but need to be updated with FEAGI-specific API calls.

### 3. Automated Profiling Script

```bash
# Run complete profiling suite
./scripts/profile_all.sh

# Run and save results for tracking over time
./scripts/profile_all.sh --save-history
```

**This script:**
- Builds release binary
- Runs all profiling tests
- Runs benchmarks
- Optionally saves results to `profiling_history/`

### 4. Comparison Tool

```bash
# Profile build 1
./scripts/profile_all.sh --save-history

# Make code changes...

# Profile build 2  
./scripts/profile_all.sh --save-history

# Compare results
./scripts/compare_profiles.py \
    profiling_history/2.0.0_20250101_120000 \
    profiling_history/2.0.0_20250101_130000
```

Shows color-coded improvements (green) and regressions (red).

## 📁 Files Created

```
feagi/
├── Cargo.toml (updated with profiling dependencies)
├── README_PROFILING.md (comprehensive guide)
├── PROFILING_GUIDE.md (detailed documentation)
├── PROFILING_SYSTEM_SUMMARY.md (implementation details)
├── PROFILING_QUICK_START.md (this file)
│
├── scripts/
│   ├── profile_all.sh (run all profiling)
│   └── compare_profiles.py (compare two runs)
│
├── tests/
│   ├── profile_memory.rs (stub - needs FEAGI API updates)
│   ├── profile_cpu.rs (stub - needs FEAGI API updates)
│   └── profile_storage.rs (✓ fully working)
│
├── benches/
│   └── feagi_benchmarks.rs (stub - needs FEAGI API updates)
│
└── docs/
    └── PROFILING_GUIDE.md (detailed guide)
```

## 🚀 Quick Demo

```bash
cd feagi

# 1. Make scripts executable (if not already)
chmod +x scripts/*.sh scripts/*.py

# 2. Run storage profiling (works now!)
cargo build && cargo build --release
cargo test --release --test profile_storage -- --nocapture

# 3. See results
cat target/profile_storage_results.json | jq .

# 4. Run complete suite (storage + stubs)
./scripts/profile_all.sh

# 5. View results
ls -lh target/release/feagi  # Binary size
open target/criterion/report/index.html  # Benchmarks (if any ran)
```

## 📊 What Gets Measured

### Storage Profiling (✓ Working Now)
- Binary sizes (debug, release)
- Size reduction percentage
- Largest dependencies
- **Output**: `target/profile_storage_results.json`

### Memory Profiling (Needs FEAGI API)
- RSS (physical memory)
- Virtual memory
- Per-component memory usage
- **Output**: `target/profile_memory_results.json`

### CPU Profiling (Needs FEAGI API)
- Execution time
- Operations per second
- Component-specific timing
- **Output**: `target/profile_cpu_results.json`

### Benchmarks (Needs FEAGI API)
- Statistical analysis
- Regression detection
- HTML reports with graphs
- **Output**: `target/criterion/report/index.html`

## 🔧 Alternative: Use External Tools Now

While the custom tests are being completed, you can profile using standard Rust tools:

### Flamegraph (CPU Visualization)

```bash
cargo install flamegraph
cd feagi
cargo flamegraph --bin feagi --release
# Opens flamegraph.svg
```

### cargo-bloat (Binary Size Analysis)

```bash
cargo install cargo-bloat
cd feagi
cargo bloat --release          # What takes space
cargo bloat --release --crates # Group by crate
```

### perf (Linux Only)

```bash
cd feagi
cargo build --release
perf record --call-graph dwarf ./target/release/feagi
perf report
```

## 📝 To Complete the Implementation

The stub tests need FEAGI API updates. See `PROFILING_SYSTEM_SUMMARY.md` for details.

**What needs to be done:**
1. Understand current FEAGI Rust API structure
2. Update test code to match actual API
3. Add specific neuron/NPU/connectome measurements

**Estimated effort:** 1-2 hours once API is understood

## 📚 Documentation

- **README_PROFILING.md** - Overview and quick start
- **PROFILING_GUIDE.md** - Detailed guide with examples
- **PROFILING_SYSTEM_SUMMARY.md** - Implementation details and next steps

## 🎯 Immediate Next Steps

### For Immediate Value:

1. **Use storage profiling** (works now):
   ```bash
   cargo build && cargo build --release
   cargo test --release --test profile_storage -- --nocapture
   ```

2. **Use external tools** (flamegraph, cargo-bloat)

3. **Track binary size over time** using the storage profiling

### To Get Full Profiling:

1. Study the FEAGI Rust API
2. Update the stub tests with correct API calls
3. Run `./scripts/profile_all.sh` for complete metrics

## 💡 CI/CD Integration

Example GitHub Actions workflow in `README_PROFILING.md` shows how to:
- Run profiling on every commit
- Save results as artifacts
- Compare performance across PRs

## 📞 Support

- **Detailed docs**: `docs/PROFILING_GUIDE.md`
- **Implementation notes**: `PROFILING_SYSTEM_SUMMARY.md`
- **Criterion docs**: https://bheisler.github.io/criterion.rs/book/
- **Rust perf book**: https://nnethercote.github.io/perf-book/

---

**Summary**: Storage profiling works now, memory/CPU profiling needs FEAGI API updates, all infrastructure and documentation is in place.

**Your next action**: Run `./scripts/profile_all.sh` to see what works, then decide if you want to complete the API integration or use external tools.




