// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! WebSocket broadcast of NPU state, built on `feagi_io`'s poll-based server publisher.
//!
//! The engine has no neural output to stream yet — `RayonBurstEngine::get_motor_data` and
//! `set_sensor_data` are still `todo!()` — so this broadcasts what the NPU does know: the burst
//! counter, the burst loop's configuration, and the cortical area registry. Frames are FEAGI byte
//! containers carrying a [`FeagiJSON`] payload, the same envelope the sensory pullers accept, so
//! the transport is a real exercised path and streaming neuron activity later is a change to
//! [`write_state_frame`] alone.
//!
//! `feagi_io` servers are poll-driven rather than async, so a dedicated thread owns the publisher
//! and calls `poll()` on a fixed cadence. `poll()` is also what accepts connections and advances
//! handshakes, so that cadence bounds how quickly a new subscriber is picked up.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::json;
use tracing::{debug, info, warn};

use feagi_io::protocol_implementations::websocket::websocket_std::FeagiWebSocketServerPublisherProperties;
use feagi_io::traits_and_enums::server::{FeagiServerPublisher, FeagiServerPublisherProperties};
use feagi_io::traits_and_enums::shared::FeagiEndpointState;
use feagi_serialization::{FeagiByteContainer, FeagiJSON};

use crate::npu::NpuHandle;

/// Default port for the NPU state WebSocket.
pub const DEFAULT_WEBSOCKET_PORT: u16 = 9050;

/// Default broadcast rate, which is also the rate at which new connections are accepted.
pub const DEFAULT_WEBSOCKET_HZ: u64 = 10;

/// How long to wait for the publisher thread to report whether it bound successfully.
const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum WebSocketError {
    #[error("invalid websocket address: {0}")]
    InvalidAddress(String),
    #[error("failed to bind websocket publisher: {0}")]
    Bind(String),
    #[error("failed to spawn websocket publisher thread: {0}")]
    Spawn(String),
    #[error("websocket publisher thread did not report readiness in time")]
    StartTimeout,
    #[error("websocket publish rate must be greater than zero")]
    InvalidPublishRate,
}

/// Where the WebSocket publisher binds, and how often it broadcasts.
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    /// Local bind address, e.g. `ws://0.0.0.0:9050`.
    pub bind_address: String,
    /// The address handed to agents. This differs from the bind address whenever the server
    /// listens on a wildcard interface or sits behind a proxy.
    pub advertised_address: String,
    pub publish_hz: u64,
}

impl WebSocketConfig {
    /// Binds on `host:port` and advertises the same, substituting loopback for the wildcard
    /// addresses since a client cannot connect to those.
    pub fn new(host: &str, port: u16, publish_hz: u64) -> Self {
        let advertised_host = match host {
            "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
            other => other,
        };
        Self {
            bind_address: format!("ws://{host}:{port}"),
            advertised_address: format!("ws://{advertised_host}:{port}"),
            publish_hz,
        }
    }
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self::new("0.0.0.0", DEFAULT_WEBSOCKET_PORT, DEFAULT_WEBSOCKET_HZ)
    }
}

/// Reportable state of the broadcaster, for the HTTP status endpoint.
#[derive(Debug, Clone)]
pub struct WebSocketStatus {
    pub bind_address: String,
    pub advertised_address: String,
    pub publish_hz: u64,
    pub running: bool,
    pub frames_published: u64,
    pub last_error: Option<String>,
}

/// State shared with the publisher thread.
struct Shared {
    running: AtomicBool,
    frames_published: AtomicU64,
    last_error: Mutex<Option<String>>,
}

/// Owns the publisher thread; broadcasts NPU state until stopped or dropped.
pub struct WebSocketBroadcaster {
    config: WebSocketConfig,
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl WebSocketBroadcaster {
    /// Binds the publisher and starts broadcasting.
    ///
    /// Returns only once the socket is bound or has failed to bind, so a port conflict surfaces to
    /// the caller instead of dying silently in the background thread.
    pub fn start(config: WebSocketConfig, npu: NpuHandle) -> Result<Self, WebSocketError> {
        if config.publish_hz == 0 {
            return Err(WebSocketError::InvalidPublishRate);
        }

        let properties = FeagiWebSocketServerPublisherProperties::new(
            &config.bind_address,
            &config.advertised_address,
        )
        .map_err(|err| WebSocketError::InvalidAddress(err.to_string()))?;

        let shared = Arc::new(Shared {
            running: AtomicBool::new(true),
            frames_published: AtomicU64::new(0),
            last_error: Mutex::new(None),
        });

        // Properties are `Send + Sync` but publishers are not, so the publisher is constructed on
        // the thread that will own it.
        let boxed_properties: Box<dyn FeagiServerPublisherProperties> = Box::new(properties);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let thread_shared = Arc::clone(&shared);
        let publish_hz = config.publish_hz;
        let bind_address = config.bind_address.clone();

        let thread = std::thread::Builder::new()
            .name("feagi-ws-publisher".to_string())
            .spawn(move || {
                let mut publisher = boxed_properties.as_boxed_server_publisher();

                if let Err(err) = publisher.request_start() {
                    thread_shared.running.store(false, Ordering::Relaxed);
                    let _ = ready_tx.send(Err(err.to_string()));
                    return;
                }
                if ready_tx.send(Ok(())).is_err() {
                    // The caller stopped waiting, so don't leave the port bound behind it.
                    let _ = publisher.request_stop();
                    return;
                }

                info!(target: "feagi-rs", "websocket publisher listening on {bind_address}");
                run_publish_loop(publisher.as_mut(), &npu, &thread_shared, publish_hz);

                if let Err(err) = publisher.request_stop() {
                    debug!(target: "feagi-rs", "websocket publisher stop reported: {err}");
                }
                info!(target: "feagi-rs", "websocket publisher stopped");
            })
            .map_err(|err| WebSocketError::Spawn(err.to_string()))?;

        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                config,
                shared,
                thread: Mutex::new(Some(thread)),
            }),
            Ok(Err(message)) => {
                let _ = thread.join();
                Err(WebSocketError::Bind(message))
            }
            Err(_) => {
                // Leave the thread to notice the flag and unwind on its own rather than blocking
                // startup on a thread that is already misbehaving.
                shared.running.store(false, Ordering::Relaxed);
                Err(WebSocketError::StartTimeout)
            }
        }
    }

    pub fn config(&self) -> &WebSocketConfig {
        &self.config
    }

    pub fn status(&self) -> WebSocketStatus {
        WebSocketStatus {
            bind_address: self.config.bind_address.clone(),
            advertised_address: self.config.advertised_address.clone(),
            publish_hz: self.config.publish_hz,
            running: self.shared.running.load(Ordering::Relaxed),
            frames_published: self.shared.frames_published.load(Ordering::Relaxed),
            last_error: self.shared.last_error.lock().clone(),
        }
    }

    /// Stops broadcasting and closes the socket, waiting for the thread to finish.
    pub fn stop(&self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.lock().take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WebSocketBroadcaster {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Polls the publisher and broadcasts a state frame on every tick.
///
/// A frame goes out even when the NPU is idle, which gives subscribers a heartbeat and means a
/// client connecting between bursts learns the current state immediately rather than at the next
/// state change.
fn run_publish_loop(
    publisher: &mut dyn FeagiServerPublisher,
    npu: &NpuHandle,
    shared: &Shared,
    publish_hz: u64,
) {
    let interval = Duration::from_nanos(1_000_000_000 / publish_hz.max(1));
    let mut container = FeagiByteContainer::new_empty();

    while shared.running.load(Ordering::Relaxed) {
        let started = Instant::now();

        match publisher.poll().clone() {
            FeagiEndpointState::ActiveWaiting | FeagiEndpointState::ActiveHasData => {
                if write_state_frame(&mut container, npu).is_err() {
                    record_error(shared, "failed to serialize NPU state frame".to_string());
                } else if let Err(err) = publisher.publish_data(container.get_byte_ref()) {
                    record_error(shared, err.to_string());
                } else {
                    shared.frames_published.fetch_add(1, Ordering::Relaxed);
                }
            }
            FeagiEndpointState::Errored(err) => {
                record_error(shared, err.to_string());
                // Acknowledge the error so the endpoint returns to Inactive and can be rebound; a
                // publisher left in Errored never recovers on its own.
                if publisher.confirm_error_and_close().is_ok() {
                    if let Err(err) = publisher.request_start() {
                        record_error(shared, err.to_string());
                    }
                }
            }
            FeagiEndpointState::Inactive | FeagiEndpointState::Pending => {}
        }

        if let Some(remaining) = interval.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

fn record_error(shared: &Shared, message: String) {
    warn!(target: "feagi-rs", "websocket publisher: {message}");
    *shared.last_error.lock() = Some(message);
}

/// Writes the current NPU state into `container` as a FEAGI frame carrying a JSON payload.
///
/// The frame's increment counter follows the burst counter, so a subscriber can order frames and
/// notice ones it missed.
fn write_state_frame(container: &mut FeagiByteContainer, npu: &NpuHandle) -> Result<(), ()> {
    let burst_count = npu.burst_count();
    let areas: Vec<_> = npu
        .cortical_areas()
        .into_iter()
        .map(|area| {
            let [x, y, z, d] = area.dimensions;
            json!({
                "cortical_id": area.id_ascii(),
                "cortical_dimensions": [x, y, z],
                "neurons_per_voxel": d,
                "neuron_count": area.neuron_count,
            })
        })
        .collect();

    let payload = FeagiJSON::from_json_value(json!({
        "kind": "npu_state",
        "burst_count": burst_count,
        "burst_frequency_hz": npu.burst_hz(),
        "burst_running": npu.is_running(),
        "cortical_areas": areas,
    }));

    // The header counter is 16-bit, so it wraps; subscribers compare it for adjacency only.
    container.overwrite_byte_data_with_single_struct_data(&payload, burst_count as u16)
}
