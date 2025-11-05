# FEAGI Quantization Architecture Proposal

## Goal: Enable Configurable INT8/FP16/FP32 Operation

Allow FEAGI to run on various hardware accelerators (Hailo, NPUs, mobile GPUs) by supporting configurable numeric precision through `feagi_configuration.toml`.

## Overview

```
┌─────────────────────────────────────────────────────────┐
│ feagi_configuration.toml                                │
│ [quantization]                                          │
│ precision = "fp32" | "fp16" | "int8"                   │
│ scale_factors = { membrane: 127, leak: 10000, ... }   │
└─────────────────────────────────────────────────────────┘
                      ↓
┌─────────────────────────────────────────────────────────┐
│ feagi-config crate                                      │
│ - Parse quantization config                             │
│ - Validate scale factors                                │
│ - Provide QuantizationConfig struct                     │
└─────────────────────────────────────────────────────────┘
                      ↓
┌─────────────────────────────────────────────────────────┐
│ feagi-types crate (NEW: numeric abstraction)           │
│ - Trait: NeuralValue<T>                                │
│ - Implementations: FP32Value, FP16Value, INT8Value     │
│ - Generic NeuronArray<T: NeuralValue>                  │
└─────────────────────────────────────────────────────────┘
                      ↓
┌─────────────────────────────────────────────────────────┐
│ feagi-burst-engine                                      │
│ - Generic neural dynamics                               │
│ - Type-specialized backends                             │
│ - Zero-cost abstractions                                │
└─────────────────────────────────────────────────────────┘
```

## Phase 1: Configuration Layer

### `feagi_configuration.toml`

```toml
[quantization]
# Precision level: "fp32" (default), "fp16", "int8"
precision = "int8"

# Enable/disable quantization (for testing)
enabled = true

# INT8 scale factors (only used when precision = "int8")
[quantization.scale_factors]
# Membrane potentials: -127 to +127 represents real range
membrane_potential_min = -100.0
membrane_potential_max = 50.0
membrane_potential_scale = 127  # INT8 range

# Thresholds: 0 to 127 represents 0.0 to 100.0
threshold_min = 0.0
threshold_max = 100.0
threshold_scale = 127

# Leak coefficients: 0 to 10000 represents 0.0000 to 1.0000
# Using fixed-point: 0.97 → 9700
leak_coefficient_scale = 10000
leak_coefficient_precision = 4  # 4 decimal places

# Resting potentials: -127 to +127 represents -100.0 to 50.0
resting_potential_min = -100.0
resting_potential_max = 50.0
resting_potential_scale = 127

# Excitability: 0 to 10000 represents 0.0000 to 1.0000
excitability_scale = 10000
excitability_precision = 4

# Synaptic weights (already u8, no change needed)
# Postsynaptic potentials (already u8, no change needed)

[quantization.validation]
# Throw error if values exceed range (vs clipping)
strict_bounds = true

# Validate scale factors on startup
validate_on_load = true

# Log quantization errors
log_quantization_loss = true
```

### `feagi-config/src/quantization.rs`

```rust
use serde::{Deserialize, Serialize};

/// Numeric precision mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    /// 32-bit floating point (default, maximum accuracy)
    FP32,
    /// 16-bit floating point (good balance, mobile GPU)
    FP16,
    /// 8-bit integer (maximum efficiency, NPU/Hailo)
    INT8,
}

impl Default for Precision {
    fn default() -> Self {
        Self::FP32
    }
}

/// Scale factor configuration for INT8 quantization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleFactors {
    // Membrane potentials
    pub membrane_potential_min: f32,
    pub membrane_potential_max: f32,
    pub membrane_potential_scale: i32,
    
    // Thresholds
    pub threshold_min: f32,
    pub threshold_max: f32,
    pub threshold_scale: i32,
    
    // Leak coefficients (fixed-point)
    pub leak_coefficient_scale: i32,
    pub leak_coefficient_precision: u8,
    
    // Resting potentials
    pub resting_potential_min: f32,
    pub resting_potential_max: f32,
    pub resting_potential_scale: i32,
    
    // Excitability (fixed-point)
    pub excitability_scale: i32,
    pub excitability_precision: u8,
}

impl Default for ScaleFactors {
    fn default() -> Self {
        Self {
            membrane_potential_min: -100.0,
            membrane_potential_max: 50.0,
            membrane_potential_scale: 127,
            threshold_min: 0.0,
            threshold_max: 100.0,
            threshold_scale: 127,
            leak_coefficient_scale: 10000,
            leak_coefficient_precision: 4,
            resting_potential_min: -100.0,
            resting_potential_max: 50.0,
            resting_potential_scale: 127,
            excitability_scale: 10000,
            excitability_precision: 4,
        }
    }
}

/// Validation settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Throw error if values exceed range (vs clipping)
    pub strict_bounds: bool,
    /// Validate scale factors on startup
    pub validate_on_load: bool,
    /// Log quantization errors
    pub log_quantization_loss: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict_bounds: true,
            validate_on_load: true,
            log_quantization_loss: true,
        }
    }
}

/// Complete quantization configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantizationConfig {
    /// Precision level
    pub precision: Precision,
    
    /// Enable/disable quantization
    pub enabled: bool,
    
    /// Scale factors (for INT8)
    pub scale_factors: ScaleFactors,
    
    /// Validation settings
    pub validation: ValidationConfig,
}

impl Default for QuantizationConfig {
    fn default() -> Self {
        Self {
            precision: Precision::FP32,
            enabled: false,  // Safe default: FP32
            scale_factors: ScaleFactors::default(),
            validation: ValidationConfig::default(),
        }
    }
}

impl QuantizationConfig {
    /// Validate configuration
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        
        let sf = &self.scale_factors;
        
        // Validate ranges
        if sf.membrane_potential_min >= sf.membrane_potential_max {
            return Err("membrane_potential_min must be < max".to_string());
        }
        
        if sf.threshold_min >= sf.threshold_max {
            return Err("threshold_min must be < max".to_string());
        }
        
        // Validate scales
        if sf.membrane_potential_scale <= 0 {
            return Err("membrane_potential_scale must be positive".to_string());
        }
        
        if sf.leak_coefficient_scale <= 0 {
            return Err("leak_coefficient_scale must be positive".to_string());
        }
        
        // Warn about precision loss
        if self.precision == Precision::INT8 {
            let membrane_range = sf.membrane_potential_max - sf.membrane_potential_min;
            let membrane_resolution = membrane_range / sf.membrane_potential_scale as f32;
            
            if membrane_resolution > 1.0 {
                log::warn!(
                    "INT8 membrane potential resolution: {:.3} mV (low precision!)",
                    membrane_resolution
                );
            }
            
            let leak_resolution = 1.0 / sf.leak_coefficient_scale as f32;
            if leak_resolution > 0.001 {
                log::warn!(
                    "INT8 leak coefficient resolution: {:.4} (low precision!)",
                    leak_resolution
                );
            }
        }
        
        Ok(())
    }
}
```

## Phase 2: Type Abstraction Layer

### `feagi-types/src/numeric.rs`

```rust
/// Trait for neural computation values
/// 
/// Abstracts over FP32, FP16, and INT8 representations
pub trait NeuralValue: 
    Copy + Clone + Send + Sync + std::fmt::Debug + 'static 
{
    /// Storage type (f32, f16, or i8)
    type Storage: Copy + Send + Sync;
    
    /// Convert from float
    fn from_float(value: f32, config: &QuantizationConfig) -> Self;
    
    /// Convert to float
    fn to_float(&self, config: &QuantizationConfig) -> f32;
    
    /// Add operation
    fn add(&self, other: &Self, config: &QuantizationConfig) -> Self;
    
    /// Multiply operation
    fn mul(&self, other: &Self, config: &QuantizationConfig) -> Self;
    
    /// Compare (greater than or equal)
    fn ge(&self, other: &Self) -> bool;
    
    /// Zero value
    fn zero(config: &QuantizationConfig) -> Self;
    
    /// One value
    fn one(config: &QuantizationConfig) -> Self;
    
    /// Clamp to valid range
    fn clamp(&self, min: &Self, max: &Self) -> Self;
}

/// FP32 implementation (default, no quantization)
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FP32Value(pub f32);

impl NeuralValue for FP32Value {
    type Storage = f32;
    
    #[inline(always)]
    fn from_float(value: f32, _config: &QuantizationConfig) -> Self {
        Self(value)
    }
    
    #[inline(always)]
    fn to_float(&self, _config: &QuantizationConfig) -> f32 {
        self.0
    }
    
    #[inline(always)]
    fn add(&self, other: &Self, _config: &QuantizationConfig) -> Self {
        Self(self.0 + other.0)
    }
    
    #[inline(always)]
    fn mul(&self, other: &Self, _config: &QuantizationConfig) -> Self {
        Self(self.0 * other.0)
    }
    
    #[inline(always)]
    fn ge(&self, other: &Self) -> bool {
        self.0 >= other.0
    }
    
    #[inline(always)]
    fn zero(_config: &QuantizationConfig) -> Self {
        Self(0.0)
    }
    
    #[inline(always)]
    fn one(_config: &QuantizationConfig) -> Self {
        Self(1.0)
    }
    
    #[inline(always)]
    fn clamp(&self, min: &Self, max: &Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }
}

/// INT8 implementation (quantized)
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct INT8Value(pub i8);

impl NeuralValue for INT8Value {
    type Storage = i8;
    
    #[inline]
    fn from_float(value: f32, config: &QuantizationConfig) -> Self {
        let sf = &config.scale_factors;
        
        // Map float range to INT8 range (-127 to +127)
        // Example: -100.0 to 50.0 → -127 to +127
        let normalized = (value - sf.membrane_potential_min) / 
                        (sf.membrane_potential_max - sf.membrane_potential_min);
        let scaled = normalized * (sf.membrane_potential_scale as f32 * 2.0) - 
                     sf.membrane_potential_scale as f32;
        
        Self(scaled.round().clamp(-127.0, 127.0) as i8)
    }
    
    #[inline]
    fn to_float(&self, config: &QuantizationConfig) -> f32 {
        let sf = &config.scale_factors;
        
        // Reverse the quantization
        let normalized = (self.0 as f32 + sf.membrane_potential_scale as f32) / 
                        (sf.membrane_potential_scale as f32 * 2.0);
        normalized * (sf.membrane_potential_max - sf.membrane_potential_min) + 
        sf.membrane_potential_min
    }
    
    #[inline]
    fn add(&self, other: &Self, config: &QuantizationConfig) -> Self {
        // Saturating add to prevent overflow
        let result = self.0.saturating_add(other.0);
        Self(result.clamp(-127, 127))
    }
    
    #[inline]
    fn mul(&self, other: &Self, config: &QuantizationConfig) -> Self {
        // Fixed-point multiplication with rescaling
        let result = ((self.0 as i32) * (other.0 as i32)) / 127;
        Self(result.clamp(-127, 127) as i8)
    }
    
    #[inline(always)]
    fn ge(&self, other: &Self) -> bool {
        self.0 >= other.0
    }
    
    #[inline(always)]
    fn zero(_config: &QuantizationConfig) -> Self {
        Self(0)
    }
    
    #[inline(always)]
    fn one(config: &QuantizationConfig) -> Self {
        Self(127) // Max positive value
    }
    
    #[inline(always)]
    fn clamp(&self, min: &Self, max: &Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }
}

/// Leak coefficient (special fixed-point for 0.0-1.0 range)
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct INT8LeakCoefficient(pub i16);  // Using i16 for 10000 scale

impl INT8LeakCoefficient {
    #[inline]
    pub fn from_float(value: f32, config: &QuantizationConfig) -> Self {
        let scale = config.scale_factors.leak_coefficient_scale as f32;
        Self((value * scale).round() as i16)
    }
    
    #[inline]
    pub fn to_float(&self, config: &QuantizationConfig) -> f32 {
        self.0 as f32 / config.scale_factors.leak_coefficient_scale as f32
    }
    
    #[inline]
    pub fn apply(&self, value: INT8Value, config: &QuantizationConfig) -> INT8Value {
        // Fixed-point multiply: (value * leak) / scale
        let result = ((value.0 as i32) * (self.0 as i32)) / 
                    config.scale_factors.leak_coefficient_scale;
        INT8Value(result.clamp(-127, 127) as i8)
    }
}
```

## Phase 3: Generic NeuronArray

### `feagi-types/src/npu.rs` (modified)

```rust
/// Generic neuron array supporting multiple precisions
pub struct NeuronArray<T: NeuralValue = FP32Value> {
    pub capacity: usize,
    pub count: usize,
    
    // Core neural state (generic over T)
    pub membrane_potentials: Vec<T>,
    pub thresholds: Vec<T>,
    pub resting_potentials: Vec<T>,
    
    // Leak coefficients (special handling for INT8)
    pub leak_coefficients: Vec<LeakCoefficient<T>>,
    
    // Excitability (0.0-1.0, needs fixed-point for INT8)
    pub excitabilities: Vec<Excitability<T>>,
    
    // Integer types (no change needed)
    pub neuron_types: Vec<i32>,
    pub refractory_periods: Vec<u16>,
    pub refractory_countdowns: Vec<u16>,
    pub consecutive_fire_counts: Vec<u16>,
    pub consecutive_fire_limits: Vec<u16>,
    pub snooze_periods: Vec<u16>,
    
    // Boolean types (no change needed)
    pub mp_charge_accumulation: Vec<bool>,
    pub valid_mask: Vec<bool>,
    
    // Quantization config (stored for conversions)
    quantization_config: QuantizationConfig,
}

/// Type alias for FP32 (default)
pub type NeuronArrayFP32 = NeuronArray<FP32Value>;

/// Type alias for INT8
pub type NeuronArrayINT8 = NeuronArray<INT8Value>;

impl<T: NeuralValue> NeuronArray<T> {
    pub fn new(capacity: usize, config: QuantizationConfig) -> Self {
        Self {
            capacity,
            count: 0,
            membrane_potentials: vec![T::zero(&config); capacity],
            thresholds: vec![T::zero(&config); capacity],
            resting_potentials: vec![T::zero(&config); capacity],
            leak_coefficients: vec![LeakCoefficient::zero(&config); capacity],
            excitabilities: vec![Excitability::zero(&config); capacity],
            neuron_types: vec![0; capacity],
            refractory_periods: vec![0; capacity],
            refractory_countdowns: vec![0; capacity],
            consecutive_fire_counts: vec![0; capacity],
            consecutive_fire_limits: vec![0; capacity],
            snooze_periods: vec![0; capacity],
            mp_charge_accumulation: vec![true; capacity],
            valid_mask: vec![false; capacity],
            quantization_config: config,
        }
    }
    
    /// Create neuron with automatic quantization
    pub fn create_neuron(
        &mut self,
        id: usize,
        threshold: f32,
        leak: f32,
        resting: f32,
    ) -> Result<()> {
        if id >= self.capacity {
            return Err(Error::IndexOutOfBounds(id, self.capacity));
        }
        
        // Convert to appropriate type using config
        self.thresholds[id] = T::from_float(threshold, &self.quantization_config);
        self.leak_coefficients[id] = LeakCoefficient::from_float(leak, &self.quantization_config);
        self.resting_potentials[id] = T::from_float(resting, &self.quantization_config);
        self.valid_mask[id] = true;
        
        // Log quantization loss if enabled
        if self.quantization_config.validation.log_quantization_loss {
            let threshold_recovered = self.thresholds[id].to_float(&self.quantization_config);
            let error = (threshold - threshold_recovered).abs();
            if error > 0.1 {
                log::warn!(
                    "Quantization loss for neuron {}: threshold {} → {} (error: {})",
                    id, threshold, threshold_recovered, error
                );
            }
        }
        
        Ok(())
    }
}
```

## Phase 4: Generic Neural Dynamics

### `feagi-burst-engine/src/neural_dynamics.rs` (modified)

```rust
/// Generic neural dynamics supporting multiple precisions
pub fn process_neural_dynamics<T: NeuralValue>(
    fcl: &FireCandidateList,
    neuron_array: &mut NeuronArray<T>,
    burst_count: u64,
) -> Result<DynamicsResult> {
    let candidates = fcl.get_all_candidates();
    let config = &neuron_array.quantization_config;
    
    let mut fired_neurons = Vec::new();
    let mut refractory_count = 0;
    
    for &(neuron_id, candidate_potential) in &candidates {
        let idx = neuron_id.0 as usize;
        
        if !neuron_array.valid_mask[idx] {
            continue;
        }
        
        // Check refractory period (unchanged, still u16)
        if neuron_array.refractory_countdowns[idx] > 0 {
            neuron_array.refractory_countdowns[idx] -= 1;
            refractory_count += 1;
            continue;
        }
        
        // Neural dynamics with generic types
        let old_potential = neuron_array.membrane_potentials[idx];
        let candidate = T::from_float(candidate_potential, config);
        let mut potential = old_potential.add(&candidate, config);
        
        // Apply leak (using leak coefficient)
        let leak = &neuron_array.leak_coefficients[idx];
        potential = leak.apply(potential, config);
        
        // Clamp to resting potential
        let resting = neuron_array.resting_potentials[idx];
        potential = potential.clamp(&resting, &T::from_float(1000.0, config));
        
        // Check threshold
        let threshold = neuron_array.thresholds[idx];
        if potential.ge(&threshold) {
            // Check excitability (probabilistic firing)
            let excitability = neuron_array.excitabilities[idx].to_float(config);
            let random = excitability_random(neuron_id.0, burst_count);
            
            if random <= excitability {
                // Neuron fires!
                fired_neurons.push(FiringNeuron {
                    neuron_id,
                    membrane_potential: potential.to_float(config),
                    // ... other fields
                });
                
                // Reset potential
                potential = resting;
                
                // Set refractory
                neuron_array.refractory_countdowns[idx] = 
                    neuron_array.refractory_periods[idx];
            }
        }
        
        // Update membrane potential
        neuron_array.membrane_potentials[idx] = potential;
    }
    
    Ok(DynamicsResult {
        fire_queue: build_fire_queue(fired_neurons, burst_count),
        neurons_processed: candidates.len(),
        neurons_fired: fired_neurons.len(),
        neurons_in_refractory: refractory_count,
    })
}
```

## Phase 5: Backend Selection

### `feagi-burst-engine/src/backend/mod.rs` (enhanced)

```rust
/// Select backend based on quantization config
pub fn select_backend_with_quantization(
    neuron_count: usize,
    synapse_count: usize,
    config: &BackendConfig,
    quant_config: &QuantizationConfig,
) -> BackendDecision {
    // If INT8 quantization enabled, prefer INT8-capable backends
    if quant_config.enabled && quant_config.precision == Precision::INT8 {
        // Check for INT8-specific accelerators
        #[cfg(feature = "hailo")]
        if is_hailo_available() {
            return BackendDecision {
                backend_type: BackendType::HAILO,
                reason: "INT8 quantization enabled, Hailo accelerator available".to_string(),
                estimated_speedup: 50.0,  // Hailo is very fast for INT8
            };
        }
        
        // Check for INT8 NPU support
        #[cfg(feature = "npu")]
        if is_npu_available() && supports_int8() {
            return BackendDecision {
                backend_type: BackendType::NPU,
                reason: "INT8 quantization enabled, NPU available".to_string(),
                estimated_speedup: 30.0,
            };
        }
        
        // Fall back to GPU with INT8 shaders
        #[cfg(feature = "gpu")]
        if is_gpu_available() {
            return BackendDecision {
                backend_type: BackendType::WGPU_INT8,
                reason: "INT8 quantization enabled, using GPU with INT8 shaders".to_string(),
                estimated_speedup: 15.0,
            };
        }
    }
    
    // Standard backend selection for FP32/FP16
    select_backend(neuron_count, synapse_count, config)
}
```

## Phase 6: Configuration Example

### Complete `feagi_configuration.toml`

```toml
# FEAGI Configuration with Quantization Support

[neural]
burst_engine_timestep = 10.0
batch_size = 256

[resources]
use_gpu = true
backend = "auto"  # Will auto-select based on quantization

# Quantization Configuration
[quantization]
# Precision: "fp32", "fp16", or "int8"
# fp32: Default, maximum accuracy, works everywhere
# fp16: Mobile GPUs, good balance
# int8: NPUs, Hailo, maximum efficiency
precision = "int8"
enabled = true

[quantization.scale_factors]
# Membrane potentials: Range -100.0 to 50.0 mV
membrane_potential_min = -100.0
membrane_potential_max = 50.0
membrane_potential_scale = 127  # INT8 max

# Thresholds: Range 0.0 to 100.0 mV  
threshold_min = 0.0
threshold_max = 100.0
threshold_scale = 127

# Leak coefficients: Fixed-point 0.0000 to 1.0000
# Scale 10000: 0.9700 → 9700 (i16)
leak_coefficient_scale = 10000
leak_coefficient_precision = 4

# Resting potentials: Same as membrane
resting_potential_min = -100.0
resting_potential_max = 50.0
resting_potential_scale = 127

# Excitability: Fixed-point 0.0000 to 1.0000
excitability_scale = 10000
excitability_precision = 4

[quantization.validation]
# Error on out-of-range values
strict_bounds = true

# Validate config on startup
validate_on_load = true

# Log quantization errors
log_quantization_loss = true

# Example configurations for different hardware:

# Config 1: RB5 Adreno 650 (FP32, maximum accuracy)
# [quantization]
# precision = "fp32"
# enabled = false

# Config 2: Mobile GPU (FP16, good balance)
# [quantization]
# precision = "fp16"
# enabled = true

# Config 3: Hailo-8 (INT8, maximum efficiency)
# [quantization]
# precision = "int8"
# enabled = true
# (scale_factors as above)
```

## Performance Impact Analysis

### Expected Accuracy vs Performance

| Precision | Accuracy Loss | Speedup (Hailo) | Speedup (GPU) | Memory |
|-----------|---------------|-----------------|---------------|--------|
| **FP32** | 0% (baseline) | ❌ N/A | 1x | 100% |
| **FP16** | <1% | ❌ N/A | 1.5-2x | 50% |
| **INT8** | 5-15% | 50-100x | 3-5x | 25% |

### Quantization Loss Breakdown

```
Membrane Potential:
  Range: -100.0 to 50.0 (150.0 total)
  INT8 resolution: 150.0 / 254 = 0.59 mV
  Impact: MODERATE - Sub-threshold dynamics affected

Threshold:
  Range: 0.0 to 100.0
  INT8 resolution: 100.0 / 127 = 0.79 mV
  Impact: LOW - Threshold checks still work

Leak Coefficient (0.9700):
  Fixed-point: 9700 / 10000
  Precision: 0.0001
  Impact: LOW - Leak still effective

Excitability (0.7500):
  Fixed-point: 7500 / 10000
  Precision: 0.0001
  Impact: LOW - Probability maintained

Overall Neural Behavior:
  Firing patterns: 85-90% similar to FP32
  Timing jitter: +/- 1-2 bursts
  Learning stability: Reduced but usable
```

## Migration Path

### Step 1: Add Config Support (Backward Compatible)

```bash
# Current FEAGI works as before (FP32 default)
./feagi --config feagi_configuration.toml
```

### Step 2: Test with FP16 (Validation)

```toml
[quantization]
precision = "fp16"
enabled = true
```

```bash
# Should show minimal accuracy loss
cargo test --release
```

### Step 3: Enable INT8 (Experimental)

```toml
[quantization]
precision = "int8"
enabled = true
```

### Step 4: Add Hardware-Specific Backends

```bash
# Build with Hailo support
cargo build --release --features gpu,hailo

# Auto-selects Hailo for INT8
./feagi --config feagi_int8.toml
```

## Implementation Checklist

- [ ] Phase 1: Add `QuantizationConfig` to feagi-config
- [ ] Phase 2: Implement `NeuralValue` trait in feagi-types
- [ ] Phase 3: Create `FP32Value`, `FP16Value`, `INT8Value` implementations
- [ ] Phase 4: Make `NeuronArray<T>` generic
- [ ] Phase 5: Make `neural_dynamics<T>` generic
- [ ] Phase 6: Update GPU shaders for INT8 support
- [ ] Phase 7: Add Hailo backend (optional)
- [ ] Phase 8: Add quantization tests
- [ ] Phase 9: Benchmark accuracy loss
- [ ] Phase 10: Documentation and examples

## Testing Strategy

### Accuracy Tests

```rust
#[test]
fn test_quantization_accuracy() {
    let config_fp32 = QuantizationConfig {
        precision: Precision::FP32,
        enabled: false,
        ..Default::default()
    };
    
    let config_int8 = QuantizationConfig {
        precision: Precision::INT8,
        enabled: true,
        ..Default::default()
    };
    
    // Run same burst with both precisions
    let result_fp32 = run_burst(&config_fp32);
    let result_int8 = run_burst(&config_int8);
    
    // Compare firing patterns
    let similarity = compare_firing_patterns(&result_fp32, &result_int8);
    
    assert!(similarity > 0.85, "INT8 should maintain 85%+ accuracy");
}
```

### Performance Tests

```rust
#[test]
fn test_int8_performance() {
    let config = QuantizationConfig {
        precision: Precision::INT8,
        enabled: true,
        ..Default::default()
    };
    
    let start = Instant::now();
    run_burst_int8(&config);
    let int8_time = start.elapsed();
    
    // Should be faster than FP32 (less memory bandwidth)
    assert!(int8_time < fp32_time * 0.8);
}
```

## Benefits

### For Users

✅ **Hardware Flexibility**: Run on NPUs, mobile GPUs, Hailo
✅ **Power Efficiency**: INT8 uses 4x less bandwidth
✅ **Cost Savings**: Can use cheaper INT8 accelerators
✅ **Backward Compatible**: FP32 still default

### For Developers

✅ **Zero-Cost Abstraction**: Generic types compile to specialized code
✅ **Type Safety**: Compiler catches quantization errors
✅ **Testable**: Can validate accuracy automatically
✅ **Modular**: Easy to add FP16, BF16, etc.

## Conclusion

This architecture enables FEAGI to run on INT8 hardware like Hailo while maintaining backward compatibility with FP32. Users simply configure `precision = "int8"` in `feagi_configuration.toml`, and the system automatically:

1. Quantizes all neural values
2. Selects appropriate backend (Hailo, NPU, or GPU INT8)
3. Maintains 85-90% accuracy
4. Achieves 3-100x speedup depending on hardware

**Estimated Implementation**: 4-6 weeks for complete INT8 support

---

**Last Updated**: October 31, 2025  
**Status**: Architectural proposal
**Complexity**: High
**Value**: Enables FEAGI on NPU/Hailo hardware




