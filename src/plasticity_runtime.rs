// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Plasticity runtime wiring helpers.

#[cfg(feature = "plasticity")]
use std::collections::{BTreeMap, HashMap};
#[cfg(feature = "plasticity")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "plasticity")]
use std::time::Instant;

#[cfg(feature = "plasticity")]
use feagi_brain_development::models::CorticalAreaExt;
#[cfg(feature = "plasticity")]
use feagi_brain_development::ConnectomeManager;
#[cfg(feature = "plasticity")]
use feagi_npu_burst_engine::npu::MemoryReplayFrame;
#[cfg(feature = "plasticity")]
use feagi_npu_burst_engine::{BurstLoopRunner, DynamicNPU, TracingMutex};
#[cfg(feature = "plasticity")]
use feagi_npu_neural::types::NeuronId;
#[cfg(feature = "plasticity")]
use feagi_npu_plasticity::AsyncPlasticityExecutor;
#[cfg(feature = "plasticity")]
use feagi_structures::genomic::cortical_area::CorticalID;
#[cfg(feature = "plasticity")]
use parking_lot::RwLock;
#[cfg(feature = "plasticity")]
use tracing::{debug, warn};

/// Replay injection payload scheduled for a future burst.
#[cfg(feature = "plasticity")]
#[derive(Debug, Clone)]
pub struct ReplayInjection {
    pub cortical_id: CorticalID,
    pub coords: Vec<(u32, u32, u32)>,
    pub potential: f32,
}

/// Wire plasticity callbacks into the burst loop (notify + replay scheduling).
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

    let replay_schedule: Arc<Mutex<BTreeMap<u64, Vec<ReplayInjection>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let ltm_twin_map: Arc<Mutex<HashMap<u32, u32>>> = Arc::new(Mutex::new(HashMap::new()));
    let ltm_twin_reverse: Arc<Mutex<HashMap<u32, u32>>> = Arc::new(Mutex::new(HashMap::new()));
    let ltm_memory_area: Arc<Mutex<HashMap<u32, u32>>> = Arc::new(Mutex::new(HashMap::new()));
    let ltm_twin_fire_potential: Arc<Mutex<HashMap<u32, f32>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let bridge_injected_memory: Arc<Mutex<HashMap<u32, u64>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let bridge_injected_twin: Arc<Mutex<HashMap<u32, u64>>> = Arc::new(Mutex::new(HashMap::new()));
    let replay_schedule_for_post = Arc::clone(&replay_schedule);
    let ltm_twin_map_for_post = Arc::clone(&ltm_twin_map);
    let ltm_twin_reverse_for_post = Arc::clone(&ltm_twin_reverse);
    let ltm_memory_area_for_post = Arc::clone(&ltm_memory_area);
    let ltm_twin_fire_potential_for_post = Arc::clone(&ltm_twin_fire_potential);
    let bridge_injected_memory_for_post = Arc::clone(&bridge_injected_memory);
    let bridge_injected_twin_for_post = Arc::clone(&bridge_injected_twin);
    let executor_for_post = Arc::clone(&executor);
    let npu_for_post = Arc::clone(&npu);
    burst_runner.write().set_post_burst_callback(move |timestep: u64| {
        let callback_start = Instant::now();
        let Ok(exec) = executor_for_post.lock() else {
            warn!(
                "[PLASTICITY-CMD] Post-burst lock failed at burst {}",
                timestep
            );
            return;
        };
        let (commands, max_ops_per_burst, deferred_commands) = match exec.get_service() {
            Some(service) => {
                let max_ops = service.max_ops_per_burst();
                if max_ops == 0 {
                    warn!(
                        "[PLASTICITY-CMD] max_ops_per_burst is 0 at burst {} - skipping command processing",
                        timestep
                    );
                    (Vec::new(), max_ops, service.pending_command_count())
                } else {
                    let pending_before = service.pending_command_count();
                    let drained = service.dequeue_commands(max_ops);
                    let deferred = pending_before.saturating_sub(drained.len());
                    (drained, max_ops, deferred)
                }
            }
            None => {
                warn!(
                    "[PLASTICITY-CMD] Post-burst service unavailable at burst {}",
                    timestep
                );
                (Vec::new(), 0, 0)
            }
        };
        drop(exec);

        if deferred_commands > 0 {
            debug!(
                "[PLASTICITY-CMD] Burst {} command budget reached ({}/burst), deferred {} command(s)",
                timestep,
                max_ops_per_burst,
                deferred_commands
            );
        }
        let commands_processed = commands.len();
        let command_phase_start = Instant::now();
        let scheduled_replays: Vec<(u64, ReplayInjection)> = Vec::new();
        if !commands.is_empty() {
            for cmd in commands {
                match cmd {
                    feagi_npu_plasticity::PlasticityCommand::RegisterMemoryNeuron {
                        neuron_id,
                        area_idx,
                        ..
                    } => {
                        let cortical_id_opt = {
                            let instance = ConnectomeManager::instance();
                            let cm = instance.read();
                            cm.get_cortical_id(area_idx).cloned()
                        };
                        let Some(cortical_id) = cortical_id_opt else {
                            warn!(
                                "[PLASTICITY-CMD] Missing cortical ID for area_idx={}",
                                area_idx
                            );
                            continue;
                        };
                        let mut npu_lock = npu_for_post.lock().unwrap();
                        npu_lock.register_dynamic_neuron_mapping(neuron_id, cortical_id);
                        debug!(
                            "[PLASTICITY-CMD] Registered memory neuron mapping id={} area_idx={}",
                            neuron_id, area_idx
                        );
                    }
                    feagi_npu_plasticity::PlasticityCommand::MemoryNeuronConvertedToLtm {
                        neuron_id,
                        area_idx,
                        pattern_hash: _,
                    } => {
                        let mut ltm_twin_map = ltm_twin_map_for_post.lock().unwrap();
                        if ltm_twin_map.contains_key(&neuron_id) {
                            continue;
                        }

                        let cortical_id_opt = {
                            let instance = ConnectomeManager::instance();
                            let cm = instance.read();
                            cm.get_cortical_id(area_idx).cloned()
                        };
                        let Some(cortical_id) = cortical_id_opt else {
                            warn!(
                                "[PLASTICITY-LTM] Missing cortical ID for area_idx={}",
                                area_idx
                            );
                            continue;
                        };

                        let (threshold, threshold_limit, leak, refractory, excitability, fire_limit, snooze, mp_charge) =
                        {
                            let instance = ConnectomeManager::instance();
                            let cm = instance.read();
                            let Some(area) = cm.get_cortical_area(&cortical_id) else {
                                warn!(
                                    "[PLASTICITY-LTM] Missing cortical area for id={}",
                                    cortical_id.as_base_64()
                                );
                                continue;
                            };
                            (
                                area.firing_threshold(),
                                area.firing_threshold_limit(),
                                area.leak_coefficient(),
                                area.refractory_period(),
                                area.neuron_excitability(),
                                area.consecutive_fire_count() as u16,
                                area.snooze_period(),
                                area.mp_charge_accumulation(),
                            )
                        };

                        let twin_id_u64 = {
                            let instance = ConnectomeManager::instance();
                            let mut cm = instance.write();
                            cm.add_neuron(
                                &cortical_id,
                                0,
                                0,
                                0,
                                threshold,
                                threshold_limit,
                                leak,
                                0.0,
                                0,
                                refractory,
                                excitability,
                                fire_limit,
                                snooze,
                                mp_charge,
                            )
                        };

                        let Ok(twin_id_u64) = twin_id_u64 else {
                            warn!(
                                "[PLASTICITY-LTM] Failed to create twin neuron for memory_id={}",
                                neuron_id
                            );
                            continue;
                        };
                        let Ok(twin_id) = u32::try_from(twin_id_u64) else {
                            warn!(
                                "[PLASTICITY-LTM] Twin neuron id overflow: {}",
                                twin_id_u64
                            );
                            continue;
                        };

                        ltm_twin_map.insert(neuron_id, twin_id);
                        ltm_twin_reverse_for_post
                            .lock()
                            .unwrap()
                            .insert(twin_id, neuron_id);
                        ltm_memory_area_for_post
                            .lock()
                            .unwrap()
                            .insert(neuron_id, area_idx);
                        ltm_twin_fire_potential_for_post
                            .lock()
                            .unwrap()
                            .insert(twin_id, threshold);
                    }
                    feagi_npu_plasticity::PlasticityCommand::InjectMemoryNeuronToFCL {
                        neuron_id,
                        area_idx,
                        membrane_potential,
                        pattern_hash,
                        is_reactivation: _,
                        replay_frames,
                    } => {
                        debug!(
                            "[PLASTICITY-REPLAY] Memory neuron inject: area_idx={} pattern={} replay_frames={}",
                            area_idx,
                            pattern_hash,
                            replay_frames.len()
                        );
                        let cortical_id_opt = {
                            let instance = ConnectomeManager::instance();
                            let cm = instance.read();
                            cm.get_cortical_id(area_idx).cloned()
                        };

                        if let Some(cortical_id) = cortical_id_opt {
                            debug!(
                                "[PLASTICITY-CMD] Injected memory neuron id={} area_idx={} pattern={}",
                                neuron_id, area_idx, pattern_hash
                            );

                            if !replay_frames.is_empty() {
                                let frames: Vec<MemoryReplayFrame> = replay_frames
                                    .iter()
                                    .map(|frame| MemoryReplayFrame {
                                        offset: frame.offset,
                                        upstream_area_idx: frame.upstream_area_idx,
                                        coords: frame.coords.clone(),
                                    })
                                    .collect();
                                let mut npu_lock = npu_for_post.lock().unwrap();
                                npu_lock.register_memory_replay_frames(neuron_id, frames);
                                debug!(
                                    "[PLASTICITY-REPLAY] Registered replay frames in NPU: neuron_id={} frames={}",
                                    neuron_id,
                                    replay_frames.len()
                                );
                            }
                            let mut npu_lock = npu_for_post.lock().unwrap();
                            npu_lock.register_dynamic_neuron_mapping(neuron_id, cortical_id);
                            npu_lock.inject_memory_neuron_to_fcl(
                                neuron_id,
                                area_idx,
                                membrane_potential,
                            );
                        } else {
                            warn!(
                                "[PLASTICITY-CMD] Missing cortical ID for area_idx={}",
                                area_idx
                            );
                        }
                    }
                    feagi_npu_plasticity::PlasticityCommand::UpdateWeightsDelta { .. } => {
                        warn!("[PLASTICITY-CMD] STDP weight updates not yet implemented");
                    }
                    feagi_npu_plasticity::PlasticityCommand::UpdateStateCounters { .. } => {}
                    feagi_npu_plasticity::PlasticityCommand::ResetMemoryNeuronsInArea {
                        cortical_idx,
                    } => {
                        debug!(
                            "[PLASTICITY-CMD] Memory neurons reset in cortical area {}",
                            cortical_idx
                        );
                    }
                }
            }
        }
        let command_phase_ms = command_phase_start.elapsed().as_secs_f64() * 1000.0;
        if command_phase_ms > 5.0 {
            debug!(
                "[PLASTICITY-CMD] Burst {} command phase took {:.2}ms",
                timestep,
                command_phase_ms
            );
        }

        let fire_queue_phase_start = Instant::now();
        let fire_queue_sample = {
            let mut npu_lock = npu_for_post.lock().unwrap();
            npu_lock.force_sample_fire_queue()
        };
        let fire_queue_phase_ms = fire_queue_phase_start.elapsed().as_secs_f64() * 1000.0;
        if fire_queue_phase_ms > 5.0 {
            debug!(
                "[PLASTICITY-CMD] Burst {} fire-queue sampling phase took {:.2}ms",
                timestep,
                fire_queue_phase_ms
            );
        }
        if let Some(sample) = fire_queue_sample {
            let ltm_bridge_phase_start = Instant::now();
            let ltm_twin_map = ltm_twin_map_for_post.lock().unwrap();
            let ltm_twin_reverse = ltm_twin_reverse_for_post.lock().unwrap();
            let mut fired_memory: HashMap<u32, f32> = HashMap::new();
            let mut fired_twins: HashMap<u32, f32> = HashMap::new();

            for (_area_idx, (neuron_ids, _xs, _ys, _zs, potentials)) in sample {
                for (idx, neuron_id) in neuron_ids.iter().enumerate() {
                    let Some(potential) = potentials.get(idx).copied() else {
                        continue;
                    };
                    if ltm_twin_map.contains_key(neuron_id) {
                        fired_memory.insert(*neuron_id, potential);
                    }
                    if ltm_twin_reverse.contains_key(neuron_id) {
                        fired_twins.insert(*neuron_id, potential);
                    }
                }
            }

            let mut bridge_injected_memory = bridge_injected_memory_for_post.lock().unwrap();
            let mut bridge_injected_twin = bridge_injected_twin_for_post.lock().unwrap();
            let ltm_memory_area = ltm_memory_area_for_post.lock().unwrap();
            let ltm_twin_fire_potential = ltm_twin_fire_potential_for_post.lock().unwrap();
            let mut twin_injections: Vec<(NeuronId, f32)> = Vec::new();
            let mut memory_injections: Vec<(u32, u32, f32)> = Vec::new();

            for (memory_id, twin_id) in ltm_twin_map.iter() {
                let memory_fired = fired_memory.get(memory_id);
                let twin_fired = fired_twins.get(twin_id);

                if memory_fired.is_some() && twin_fired.is_some() {
                    continue;
                }

                if let Some(mem_potential) = memory_fired {
                    if bridge_injected_memory.remove(memory_id) == Some(timestep) {
                        continue;
                    }
                    let Some(twin_potential) = ltm_twin_fire_potential.get(twin_id) else {
                        warn!(
                            "[PLASTICITY-LTM] Missing twin potential for twin_id={}",
                            twin_id
                        );
                        continue;
                    };
                    let inject_potential = mem_potential.max(*twin_potential);
                    twin_injections.push((NeuronId(*twin_id), inject_potential));
                    bridge_injected_twin.insert(*twin_id, timestep + 1);
                }

                if let Some(twin_potential) = twin_fired {
                    if bridge_injected_twin.remove(twin_id) == Some(timestep) {
                        continue;
                    }
                    let Some(area_idx) = ltm_memory_area.get(memory_id) else {
                        warn!(
                            "[PLASTICITY-LTM] Missing area for memory_id={}",
                            memory_id
                        );
                        continue;
                    };
                    memory_injections.push((*memory_id, *area_idx, *twin_potential));
                    bridge_injected_memory.insert(*memory_id, timestep + 1);
                }
            }

            if !twin_injections.is_empty() || !memory_injections.is_empty() {
                let mut npu_lock = npu_for_post.lock().unwrap();
                if !twin_injections.is_empty() {
                    npu_lock.inject_sensory_with_potentials(&twin_injections);
                }
                if !memory_injections.is_empty() {
                    for (memory_id, area_idx, potential) in memory_injections {
                        npu_lock.inject_memory_neuron_to_fcl(memory_id, area_idx, potential);
                    }
                }
            }
            let ltm_bridge_phase_ms = ltm_bridge_phase_start.elapsed().as_secs_f64() * 1000.0;
            if ltm_bridge_phase_ms > 5.0 {
                debug!(
                    "[PLASTICITY-CMD] Burst {} LTM bridge phase took {:.2}ms",
                    timestep,
                    ltm_bridge_phase_ms
                );
            }
        }

        if !scheduled_replays.is_empty() {
            let mut schedule = replay_schedule_for_post.lock().unwrap();
            for (target_burst, injection) in scheduled_replays {
                schedule.entry(target_burst).or_default().push(injection);
            }
        }

        let next_burst = timestep + 1;
        let replay_injections = {
            let mut schedule = replay_schedule_for_post.lock().unwrap();
            schedule.remove(&next_burst)
        };
        if let Some(injections) = replay_injections {
            debug!(
                "[PLASTICITY-REPLAY] Executing {} replay injections at burst {}",
                injections.len(),
                next_burst
            );
            let mut npu_lock = npu_for_post.lock().unwrap();
            for injection in injections {
                let xyzp_data: Vec<(u32, u32, u32, f32)> = injection
                    .coords
                    .iter()
                    .map(|(x, y, z)| (*x, *y, *z, injection.potential))
                    .collect();
                debug!(
                    "[PLASTICITY-REPLAY] Injecting coords={} into twin_id={}",
                    xyzp_data.len(),
                    injection.cortical_id.as_base_64()
                );
                npu_lock.inject_sensory_xyzp_by_id(&injection.cortical_id, &xyzp_data);
            }
        } else {
            debug!(
                "[PLASTICITY-REPLAY] No replay injections scheduled for burst {}",
                next_burst
            );
        }

        let callback_elapsed_ms = callback_start.elapsed().as_secs_f64() * 1000.0;
        if callback_elapsed_ms > 20.0 {
            warn!(
                "[PLASTICITY-CMD] Post-burst callback for burst {} took {:.2}ms (commands_processed={}, deferred={})",
                timestep,
                callback_elapsed_ms,
                commands_processed,
                deferred_commands
            );
        } else if callback_elapsed_ms > 5.0 {
            debug!(
                "[PLASTICITY-CMD] Post-burst callback for burst {} took {:.2}ms (commands_processed={}, deferred={})",
                timestep,
                callback_elapsed_ms,
                commands_processed,
                deferred_commands
            );
        }

    });

    replay_schedule
}
