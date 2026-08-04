// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Runtime wrapper around the in-progress `feagi_npu::dynamic_npu::DynamicNPU`.
//!
//! `DynamicNPU` currently exposes only three operations: construction, submitting a
//! [`ConnectomeRequest`], and stepping a single burst. It keeps no queryable record of what has
//! been added, so this module maintains the cortical area registry that the HTTP layer reports
//! from, and owns the burst loop thread.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::{debug, info, warn};

use feagi_data::neurons::DimensionalCorticalArea4DDimensions;
use feagi_data::quantization_levels::feagi_index_quantization::{
    FeagiIndexQuantization, FeagiIndexQuantizationGenomic,
};
use feagi_genomic_context::cortical_area::CorticalID;
use feagi_models::connectome_requests::connectome_request::ConnectomeRequest;
use feagi_models::neuron::models::feagi_advanced::{
    FeagiAdvancedModelCorticalWriter, FeagiAdvancedModelStandardQuant,
};
use feagi_npu::dynamic_npu::DynamicNPU;

/// Neuron index quantization used by the genomic-level engine the NPU is currently fixed to.
type NeuronQuant = <FeagiIndexQuantizationGenomic as FeagiIndexQuantization>::NeuronIndexQuant;

/// Burst rate used when nothing else is configured.
pub const DEFAULT_BURST_HZ: u64 = 10;

#[derive(Debug, thiserror::Error)]
pub enum NpuError {
    #[error("invalid cortical id: {0}")]
    InvalidCorticalId(String),
    #[error("cortical area '{0}' already exists")]
    DuplicateCorticalArea(String),
    #[error("invalid dimensions {x}x{y}x{z}x{d}: every axis must be greater than zero")]
    ZeroDimension { x: u64, y: u64, z: u64, d: u64 },
    #[error("dimensions {x}x{y}x{z}x{d} are not representable in the engine's index quantization")]
    UnrepresentableDimensions { x: u64, y: u64, z: u64, d: u64 },
    #[error("burst frequency must be greater than zero")]
    InvalidBurstFrequency,
}

/// A cortical area that has been handed to the NPU.
///
/// The engine does not expose its own view of this, so it is recorded here at submission time.
#[derive(Debug, Clone)]
pub struct CorticalAreaRecord {
    pub id: CorticalID,
    pub dimensions: [u64; 4],
    pub neuron_count: u64,
}

impl CorticalAreaRecord {
    /// The 8 raw ID bytes rendered as text, which is how humans write these IDs (e.g. `cust0001`).
    /// Falls back to base64 for IDs that are not valid UTF-8.
    pub fn id_ascii(&self) -> String {
        cortical_id_to_ascii(&self.id)
    }
}

/// Renders a [`CorticalID`] as its raw 8-byte ASCII form, falling back to base64.
pub fn cortical_id_to_ascii(id: &CorticalID) -> String {
    std::str::from_utf8(id.as_bytes())
        .map(|s| s.to_string())
        .unwrap_or_else(|_| id.as_base_64())
}

/// Parses a cortical ID written either as 8 raw ASCII characters (`cust0001`) or as base64.
///
/// The raw form is tried first because it is what the HTTP API historically accepted.
pub fn parse_cortical_id(raw: &str) -> Result<CorticalID, NpuError> {
    let bytes = raw.as_bytes();
    if bytes.len() == CorticalID::CORTICAL_ID_LENGTH {
        let mut fixed = [0u8; CorticalID::CORTICAL_ID_LENGTH];
        fixed.copy_from_slice(bytes);
        if let Ok(id) = CorticalID::try_from_bytes(&fixed) {
            return Ok(id);
        }
    }

    CorticalID::try_from_base_64(raw).map_err(|_| {
        NpuError::InvalidCorticalId(format!(
            "'{raw}' is neither 8 ASCII bytes starting with one of c/m/_/i/o, nor base64 encoding such an ID"
        ))
    })
}

/// The NPU plus the bookkeeping the engine does not do for us.
struct NpuState {
    npu: DynamicNPU,
    areas: Vec<CorticalAreaRecord>,
}

impl NpuState {
    fn new() -> Self {
        Self {
            npu: DynamicNPU::new(),
            areas: Vec::new(),
        }
    }
}

/// Shared, cloneable handle to the NPU and its burst loop.
#[derive(Clone)]
pub struct NpuHandle {
    state: Arc<Mutex<NpuState>>,
    burst_count: Arc<AtomicU64>,
    burst_hz: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    burst_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl NpuHandle {
    pub fn new(burst_hz: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(NpuState::new())),
            burst_count: Arc::new(AtomicU64::new(0)),
            burst_hz: Arc::new(AtomicU64::new(burst_hz.max(1))),
            running: Arc::new(AtomicBool::new(false)),
            burst_thread: Arc::new(Mutex::new(None)),
        }
    }

    /// Submits a new dimensional cortical area to the NPU and records it.
    ///
    /// `d` is the per-voxel neuron density, so the area holds `x * y * z * d` neurons.
    pub fn add_cortical_area(
        &self,
        id: CorticalID,
        x: u64,
        y: u64,
        z: u64,
        d: u64,
    ) -> Result<CorticalAreaRecord, NpuError> {
        if x == 0 || y == 0 || z == 0 || d == 0 {
            return Err(NpuError::ZeroDimension { x, y, z, d });
        }

        let to_usize = |v: u64| usize::try_from(v).ok();
        let (Some(ux), Some(uy), Some(uz), Some(ud)) =
            (to_usize(x), to_usize(y), to_usize(z), to_usize(d))
        else {
            return Err(NpuError::UnrepresentableDimensions { x, y, z, d });
        };

        let dimensions =
            DimensionalCorticalArea4DDimensions::<NeuronQuant>::try_new_from_usizes(ux, uy, uz, ud)
                .map_err(|_| NpuError::UnrepresentableDimensions { x, y, z, d })?;

        let neuron_count = x
            .checked_mul(y)
            .and_then(|v| v.checked_mul(z))
            .and_then(|v| v.checked_mul(d))
            .ok_or(NpuError::UnrepresentableDimensions { x, y, z, d })?;

        let record = CorticalAreaRecord {
            id,
            dimensions: [x, y, z, d],
            neuron_count,
        };

        let writer = FeagiAdvancedModelCorticalWriter::DefaultNewDimensional {
            dimensions,
            _p: PhantomData::<FeagiAdvancedModelStandardQuant>,
        };

        // Held across the request so a burst can't run mid-mutation.
        let mut state = self.state.lock();
        if state.areas.iter().any(|existing| existing.id == record.id) {
            return Err(NpuError::DuplicateCorticalArea(record.id_ascii()));
        }

        state
            .npu
            .request(ConnectomeRequest::CorticalAreaAdd(record.id, writer.into()));
        state.areas.push(record.clone());

        info!(
            target: "feagi-rs",
            cortical_id = %record.id_ascii(),
            neurons = record.neuron_count,
            "added cortical area {}x{}x{}x{}", x, y, z, d
        );

        Ok(record)
    }

    pub fn cortical_areas(&self) -> Vec<CorticalAreaRecord> {
        self.state.lock().areas.clone()
    }

    pub fn cortical_area(&self, id: &CorticalID) -> Option<CorticalAreaRecord> {
        self.state
            .lock()
            .areas
            .iter()
            .find(|area| &area.id == id)
            .cloned()
    }

    /// Runs exactly one burst, regardless of whether the burst loop is running.
    pub fn step_once(&self) -> u64 {
        self.state.lock().npu.execute_single_burst();
        self.burst_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn burst_count(&self) -> u64 {
        self.burst_count.load(Ordering::Relaxed)
    }

    pub fn burst_hz(&self) -> u64 {
        self.burst_hz.load(Ordering::Relaxed)
    }

    pub fn set_burst_hz(&self, hz: u64) -> Result<(), NpuError> {
        if hz == 0 {
            return Err(NpuError::InvalidBurstFrequency);
        }
        self.burst_hz.store(hz, Ordering::Relaxed);
        info!(target: "feagi-rs", "burst frequency set to {hz} Hz");
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Starts the burst loop thread. Returns `false` if it was already running.
    pub fn start(&self) -> bool {
        let mut thread_slot = self.burst_thread.lock();
        if thread_slot.is_some() && self.running.load(Ordering::Relaxed) {
            return false;
        }

        // A previous loop may have exited but not been joined yet.
        if let Some(handle) = thread_slot.take() {
            let _ = handle.join();
        }

        self.running.store(true, Ordering::Relaxed);

        let state = Arc::clone(&self.state);
        let burst_count = Arc::clone(&self.burst_count);
        let burst_hz = Arc::clone(&self.burst_hz);
        let running = Arc::clone(&self.running);

        let handle = std::thread::Builder::new()
            .name("feagi-burst-loop".to_string())
            .spawn(move || {
                info!(target: "feagi-rs", "burst loop started");
                while running.load(Ordering::Relaxed) {
                    let started = Instant::now();

                    state.lock().npu.execute_single_burst();
                    let count = burst_count.fetch_add(1, Ordering::Relaxed) + 1;

                    let hz = burst_hz.load(Ordering::Relaxed).max(1);
                    let target = Duration::from_nanos(1_000_000_000 / hz);
                    let elapsed = started.elapsed();
                    match target.checked_sub(elapsed) {
                        Some(remaining) => std::thread::sleep(remaining),
                        None => debug!(
                            target: "feagi-rs",
                            "burst {count} took {elapsed:?}, over the {target:?} budget for {hz} Hz"
                        ),
                    }
                }
                info!(target: "feagi-rs", "burst loop stopped");
            });

        match handle {
            Ok(handle) => {
                *thread_slot = Some(handle);
                true
            }
            Err(err) => {
                warn!(target: "feagi-rs", "failed to spawn burst loop thread: {err}");
                self.running.store(false, Ordering::Relaxed);
                false
            }
        }
    }

    /// Signals the burst loop to stop and waits for it. Returns `false` if it was not running.
    pub fn stop(&self) -> bool {
        let was_running = self.running.swap(false, Ordering::Relaxed);
        if let Some(handle) = self.burst_thread.lock().take() {
            let _ = handle.join();
        }
        was_running
    }
}

impl Drop for NpuHandle {
    fn drop(&mut self) {
        // Only the last handle owns the thread; the rest see an empty slot.
        if Arc::strong_count(&self.burst_thread) == 1 {
            self.stop();
        }
    }
}
