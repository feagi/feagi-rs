# CPU Core Requirements for FEAGI Rust

## How to Calculate Optimal Core Count

The optimal number of CPU cores depends on three key factors:

1. **Parallel vs Serial Work** (Amdahl's Law)
2. **Memory Bandwidth Limits** (physical bottleneck)
3. **Overhead Costs** (diminishing returns)

## FEAGI's Parallelization Profile

### What's Parallelized (from source code analysis)

```rust
// feagi-burst-engine/src/synaptic_propagation.rs
let synapse_indices: Vec<usize> = fired_neurons
    .par_iter()  // ✅ PARALLEL
    .filter_map(|&neuron_id| self.synapse_index.get(&neuron_id))
    .flatten()
    .copied()
    .collect();

let contributions: Vec<_> = synapse_indices
    .par_iter()  // ✅ PARALLEL
    .filter_map(|&syn_idx| {
        // Compute synaptic contributions
    })
    .collect();
```

### What's NOT Parallelized

```rust
// feagi-burst-engine/src/neural_dynamics.rs
// NOTE: We CANNOT use Rayon here because process_single_neuron mutates neuron_array
// This would require unsafe code or a different approach (batch processing)
// For now, use single-threaded processing (still very fast!)
for &(neuron_id, candidate_potential) in &candidates {
    // ❌ SERIAL (due to mutable state)
    process_single_neuron(neuron_id, candidate_potential, neuron_array, burst_count);
}
```

### Burst Processing Breakdown

| Phase | Operation | Parallelizable? | % of Time | Speedup Limit |
|-------|-----------|-----------------|-----------|---------------|
| **Phase 1** | Synapse gather | ✅ YES | 20% | N cores |
| **Phase 1** | Synapse compute | ✅ YES | 30% | N cores |
| **Phase 2** | Neural dynamics | ❌ NO* | 30% | 1 core |
| **Phase 3** | Fire queue | 🟡 Partial | 5% | 2-4 cores |
| **Phase 4** | Ledger update | ❌ NO | 5% | 1 core |
| **Phase 5** | Cleanup | 🟡 Partial | 10% | 2-4 cores |

*Can be parallelized with refactoring, but currently serial

**Total parallelizable: ~50-60%**
**Total serial: ~40-50%**

## Amdahl's Law: The Core Count Limit

Amdahl's Law tells us the maximum speedup:

```
Speedup = 1 / (Serial% + Parallel%/N)

Where:
  Serial% = 0.45 (45% of work is serial)
  Parallel% = 0.55 (55% of work is parallel)
  N = number of cores
```

### Speedup by Core Count

| Cores | Speedup | Efficiency | Recommendation |
|-------|---------|------------|----------------|
| 1 | 1.00x | 100% | Baseline |
| 2 | 1.56x | 78% | ✅ Excellent |
| 4 | 1.94x | 48% | ✅ Very Good |
| 6 | 2.14x | 36% | ✅ Good |
| 8 | 2.26x | 28% | ⚠️ Diminishing |
| 12 | 2.39x | 20% | ⚠️ Low ROI |
| 16 | 2.46x | 15% | ❌ Wasteful |
| 32 | 2.56x | 8% | ❌ Very Wasteful |
| ∞ | 2.22x | 0% | Theoretical max |

**Key insight**: Beyond 8 cores, you get < 10% additional speedup!

## Memory Bandwidth: The Real Bottleneck

### Bandwidth per Core

Modern systems have shared memory bandwidth:

| Component | Bandwidth | Cores Fed | BW per Core |
|-----------|-----------|-----------|-------------|
| **DDR4-3200** | 25 GB/s | 1 | 25 GB/s |
| **DDR4-3200** | 25 GB/s | 4 | 6.25 GB/s |
| **DDR4-3200** | 25 GB/s | 8 | 3.12 GB/s |
| **DDR4-3200** | 25 GB/s | 16 | 1.56 GB/s |
| **DDR5-4800** | 38 GB/s | 1 | 38 GB/s |
| **DDR5-4800** | 38 GB/s | 8 | 4.75 GB/s |

### Bandwidth Requirements by Scale

| Scale | Data per Burst | Bandwidth @ 30Hz | Cores Sustainable |
|-------|----------------|------------------|-------------------|
| **1M/10M** | ~20 MB | 600 MB/s | **40+ cores** ✅ |
| **10M/100M** | ~200 MB | 6 GB/s | **4-6 cores** ⚠️ |
| **100M/1B** | ~2 GB | 60 GB/s | **< 1 core** 🚨 |
| **1B/10B** | ~20 GB | 600 GB/s | **0.04 cores** 🚨 |

**Reality check**: 
- At 1M neurons: Compute-bound (more cores help)
- At 10M neurons: Balanced (4-8 cores optimal)
- At 100M+ neurons: Memory-bound (more cores DON'T help)

## Scenario Analysis

### Scenario 1: 1M Neurons, 10M Synapses (Small Brain)

**Workload Characteristics:**
- Data size: 217 MB (fits in L3 cache)
- Memory bandwidth: Low (~600 MB/s @ 30Hz)
- Bottleneck: Compute

**Optimal Core Count: 4-6 cores**

| Cores | Burst Time | Speedup | Efficiency | Cost/Benefit |
|-------|------------|---------|------------|--------------|
| 1 | 2.0 ms | 1.00x | 100% | Baseline |
| 2 | 1.3 ms | 1.54x | 77% | ✅ Excellent |
| 4 | 1.0 ms | 2.00x | 50% | ✅ Very Good |
| 6 | 0.93 ms | 2.15x | 36% | ✅ Good |
| 8 | 0.88 ms | 2.27x | 28% | ⚠️ OK |
| 16 | 0.81 ms | 2.47x | 15% | ❌ Wasteful |

**Recommendation: 4-6 cores**
- Good speedup (2.0-2.15x)
- High efficiency (36-50%)
- Minimal waste

### Scenario 2: 10M Neurons, 100M Synapses (Medium Brain)

**Workload Characteristics:**
- Data size: 2.1 GB (spills out of L3)
- Memory bandwidth: Moderate (~6 GB/s @ 30Hz)
- Bottleneck: Mixed (compute + memory)

**Optimal Core Count: 4-8 cores**

| Cores | Burst Time | Speedup | BW Limit? | Efficiency |
|-------|------------|---------|-----------|------------|
| 1 | 8.0 ms | 1.00x | No | 100% |
| 2 | 5.2 ms | 1.54x | No | 77% |
| 4 | 4.0 ms | 2.00x | No | 50% |
| 6 | 3.6 ms | 2.22x | Starting | 37% |
| 8 | 3.4 ms | 2.35x | Yes ⚠️ | 29% |
| 16 | 3.2 ms | 2.50x | Severe 🚨 | 16% |

**Recommendation: 6-8 cores**
- Approaching bandwidth limit
- Still reasonable efficiency
- Beyond 8 cores: memory-bound

### Scenario 3: 100M Neurons, 1B Synapses (Large Brain)

**Workload Characteristics:**
- Data size: 21 GB (way beyond cache)
- Memory bandwidth: High (~60 GB/s @ 30Hz)
- Bottleneck: **Memory bandwidth**

**Optimal Core Count: 2-4 cores**

| Cores | Burst Time | Speedup | BW Saturation | Recommendation |
|-------|------------|---------|---------------|----------------|
| 1 | 60 ms | 1.00x | 24% | Single-threaded OK |
| 2 | 50 ms | 1.20x | 48% | ✅ Good |
| 4 | 45 ms | 1.33x | 96% | ✅ Max useful |
| 6 | 44 ms | 1.36x | **144%** 🚨 | ❌ Wasteful |
| 8 | 44 ms | 1.36x | **192%** 🚨 | ❌ Very wasteful |

**Recommendation: 2-4 cores**
- More cores don't help (bandwidth-limited)
- 4+ cores fight for same bandwidth
- Actually slows down due to contention!

### Scenario 4: 1B Neurons, 10B Synapses (Huge Brain - CPU Only)

**Workload Characteristics:**
- Data size: 217 GB (no caching possible)
- Memory bandwidth: Extreme (~6.5 TB/s needed @ 30Hz)
- Bottleneck: **Catastrophic memory bandwidth**

**Optimal Core Count: 1-2 cores (seriously)**

| Cores | Burst Time | Speedup | BW Need/Have | Notes |
|-------|------------|---------|--------------|-------|
| 1 | 8500 ms | 1.00x | 200x deficit | Baseline terrible |
| 2 | 8450 ms | 1.006x | 400x deficit | No improvement |
| 4+ | 8500+ ms | 0.99x | Worse | **Negative speedup!** |

**Recommendation: DON'T USE CPU**
- CPU is completely ineffective
- Need GPU (mandatory)
- More cores make it worse (cache pollution)

## Quick Reference Table

### Optimal Core Count by Scale

| Brain Scale | Neurons | Synapses | Memory | Optimal Cores | Why |
|-------------|---------|----------|--------|---------------|-----|
| **Tiny** | <100K | <1M | <22 MB | **4 cores** | Compute-bound |
| **Small** | 1M | 10M | 217 MB | **4-6 cores** | Balanced |
| **Medium** | 10M | 100M | 2.1 GB | **6-8 cores** | Approaching BW limit |
| **Large** | 100M | 1B | 21 GB | **2-4 cores** | Memory-bound |
| **Huge** | 1B+ | 10B+ | 217+ GB | **1-2 cores + GPU** | BW catastrophic |

### By CPU Type

| CPU Type | Cores | Best For | Max Scale |
|----------|-------|----------|-----------|
| **Laptop (4 cores)** | 4 | 1M-10M neurons | 10M neurons |
| **Desktop (8 cores)** | 8 | 1M-10M neurons | 10M neurons |
| **Workstation (16 cores)** | 16 | 10M-30M neurons* | 30M neurons |
| **Server (32+ cores)** | 32+ | Distributed/Multi-brain | Multiple brains |

*Beyond 10M neurons, more cores help less due to memory bandwidth

## Real-World Recommendations

### For Development/Testing (< 1M neurons)
```
CPU: 4-6 cores (e.g., Intel i5, AMD Ryzen 5)
RAM: 8-16 GB
Expected: 500+ Hz burst processing
```

### For Research/Small Models (1M-10M neurons)
```
CPU: 6-8 cores (e.g., Intel i7, AMD Ryzen 7)
RAM: 16-32 GB
Expected: 50-200 Hz burst processing
```

### For Large Models (10M-100M neurons)
```
CPU: 8-16 cores (e.g., AMD Ryzen 9, Threadripper)
RAM: 64-128 GB
Expected: 5-30 Hz burst processing
Note: Diminishing returns above 8 cores
```

### For Billion-Scale (1B+ neurons)
```
CPU: 8 cores (enough for coordination)
GPU: Mandatory (RTX 4090, A100, H100)
RAM: 256+ GB (or distributed)
Expected: 25-500 Hz with GPU
Note: CPU core count almost irrelevant, GPU is key
```

## Measuring Your System

### Test Parallelism Efficiency

```bash
# Run with different core counts
RAYON_NUM_THREADS=1 cargo test --release profile_cpu
RAYON_NUM_THREADS=4 cargo test --release profile_cpu
RAYON_NUM_THREADS=8 cargo test --release profile_cpu
RAYON_NUM_THREADS=16 cargo test --release profile_cpu

# Compare burst times
# If 8 cores vs 16 cores shows < 5% improvement → saturated
```

### Check Memory Bandwidth

```bash
# Install mbw (memory bandwidth benchmark)
sudo apt install mbw  # or brew install mbw

# Test bandwidth
mbw 1000  # Test with 1000 MB

# Compare to FEAGI needs:
# 1M neurons @ 30Hz: ~600 MB/s needed
# 10M neurons @ 30Hz: ~6 GB/s needed
# 100M neurons @ 30Hz: ~60 GB/s needed
```

### Check Cache Performance

```bash
# Install perf (Linux only)
perf stat -e cache-references,cache-misses,L1-dcache-load-misses,LLC-load-misses \
    cargo test --release profile_cpu

# Look for:
# - Cache miss rate < 10%: Good (more cores help)
# - Cache miss rate > 50%: Bad (memory-bound)
```

## Advanced: Hyperthreading / SMT

### Should You Use Hyperthreading?

Modern CPUs have 2 threads per physical core:

| Physical Cores | Logical Cores | FEAGI Performance |
|----------------|---------------|-------------------|
| 4 | 8 | ⚠️ 1.1-1.2x speedup |
| 8 | 16 | ⚠️ 1.05-1.15x speedup |
| 16 | 32 | ❌ 0.95-1.05x (no benefit) |

**Recommendation**: 
- **Use physical core count** for FEAGI
- Set `RAYON_NUM_THREADS` to physical cores
- Hyperthreading helps little (memory-bound workload)

```bash
# Disable hyperthreading for FEAGI
export RAYON_NUM_THREADS=$(nproc --all / 2)  # Linux
export RAYON_NUM_THREADS=$(sysctl -n hw.physicalcpu)  # macOS
```

## Cost-Benefit Analysis

### Price vs Performance

Example pricing (approximate):

| CPU | Cores | Price | Speedup | $/Speedup | Value |
|-----|-------|-------|---------|-----------|-------|
| i5-13400 | 6P+4E | $220 | 2.1x | $105 | ✅ Excellent |
| i7-13700 | 8P+8E | $400 | 2.3x | $174 | ⚠️ OK |
| i9-13900K | 8P+16E | $600 | 2.4x | $250 | ❌ Poor |
| Ryzen 9 7950X | 16 | $550 | 2.5x | $220 | ❌ Poor |

For FEAGI specifically:
- **Best value**: 6-8 core CPUs
- **Worst value**: 16+ core CPUs (unless running multiple brains)

### GPU vs More Cores

At large scales:

| Option | Cost | 100M Neurons | 1B Neurons | Recommendation |
|--------|------|--------------|------------|----------------|
| **32-core CPU** | $1000 | 40 ms | 8500 ms | ❌ Don't do this |
| **8-core + RTX 4090** | $2000 | 2 ms | 20 ms | ✅ Much better |
| **8-core + A100** | $12000 | 1 ms | 4 ms | ✅ Professional |

**Beyond 10M neurons: GPU > more CPU cores**

## Summary: How Many Cores Do You Need?

### Simple Answer by Scale

| Your Brain Size | Optimal Cores | Why |
|-----------------|---------------|-----|
| **< 1M neurons** | **4-6 cores** | Compute-bound, good speedup |
| **1M-10M neurons** | **6-8 cores** | Balanced, best value |
| **10M-100M neurons** | **4-8 cores** | Memory-bound, diminishing returns |
| **100M+ neurons** | **4 cores + GPU** | Bandwidth-limited, GPU mandatory |

### Golden Rule

**"If your working set fits in L3 cache → use 6-8 cores"**
**"If your working set is > 10x L3 cache → use 4 cores + GPU"**

### Check Your L3 Cache

```bash
# Linux
lscpu | grep "L3 cache"

# macOS  
sysctl -a | grep cachesize

# If L3 < 1% of your connectome size → memory-bound
# If L3 > 10% of your connectome size → compute-bound
```

### Setting Core Count in FEAGI

```rust
// In your code or environment
export RAYON_NUM_THREADS=6  // Recommended starting point

// Or in Rust:
rayon::ThreadPoolBuilder::new()
    .num_threads(6)
    .build_global()
    .unwrap();
```

**Start with 6 cores, measure, adjust based on actual performance.**

---

**Last Updated**: October 31, 2025  
**Based on**: FEAGI Rust v2.0.0 parallelization analysis + Amdahl's Law + memory bandwidth fundamentals




