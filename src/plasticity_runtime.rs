// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Plasticity runtime wiring helpers.
//!
//! The command applier and the memory/twin fire bridge live in
//! `feagi_services::plasticity_runtime`; this module only wires them into the burst loop.

#[cfg(feature = "plasticity")]
use std::collections::BTreeMap;
#[cfg(feature = "plasticity")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "plasticity")]
use feagi_brain_development::ConnectomeManager;
#[cfg(feature = "plasticity")]
use feagi_npu_burst_engine::{BurstLoopRunner, DynamicNPU, TracingMutex};
#[cfg(feature = "plasticity")]
use feagi_npu_plasticity::AsyncPlasticityExecutor;
#[cfg(feature = "plasticity")]
pub use feagi_services::plasticity_runtime::{PlasticityRuntime, ReplayInjection};
#[cfg(feature = "plasticity")]
use parking_lot::RwLock;

/// Wire plasticity callbacks into the burst loop (notify + post-burst command application).
#[cfg(feature = "plasticity")]
pub fn wire_plasticity_callbacks(
    burst_runner: &Arc<RwLock<BurstLoopRunner>>,
    executor: Arc<Mutex<AsyncPlasticityExecutor>>,
    npu: Arc<TracingMutex<DynamicNPU>>,
) -> Arc<Mutex<BTreeMap<u64, Vec<ReplayInjection>>>> {
    let executor_for_callback = Arc::clone(&executor);
    burst_runner
        .write()
        .set_plasticity_notify_callback(move |timestep: u64| {
            if let Ok(exec) = executor_for_callback.lock() {
                use feagi_npu_plasticity::PlasticityExecutor;
                exec.notify_burst(timestep);
            }
        });

    let runtime = Arc::new(PlasticityRuntime::new(
        npu,
        ConnectomeManager::instance(),
        executor,
    ));
    let replay_schedule = runtime.replay_schedule();
    burst_runner
        .write()
        .set_post_burst_callback(move |timestep: u64| runtime.apply_post_burst(timestep));
    replay_schedule
}
