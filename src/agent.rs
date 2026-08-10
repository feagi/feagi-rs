// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! Agent registration over the command-and-control WebSocket.
//!
//! Agents -- the Brain Visualizer among them -- register before any data stream is opened: they
//! connect to the registration socket, present a descriptor and the capabilities they want, and are
//! answered with an agent id and one endpoint per granted capability. `feagi_agent`'s
//! [`FeagiAgentHandler`] implements that protocol and owns the endpoint bookkeeping; this module
//! binds the socket it listens on and drives it.
//!
//! The handler is poll-driven rather than async, so a dedicated thread owns the polling cadence.
//! Polling is what accepts connections, advances WebSocket handshakes, and answers one message per
//! call, so that cadence bounds how quickly a registering agent is served.
//!
//! The same thread also polls the publisher sockets, for the reason given on [`poll_once`]. Sensory
//! pullers are deliberately left unpolled: polling them dequeues sensory payloads, and there is
//! nothing feeding them into the NPU yet, so servicing them here would silently drop agent input.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use feagi_agent::server::FeagiAgentHandler;
use feagi_config::load_config;
use feagi_io::protocol_implementations::websocket::websocket_std::FeagiWebSocketServerRouterProperties;

/// How long the polling thread sleeps between calls into the handler.
///
/// A registration handshake takes several polls (accept, WebSocket upgrade, request, response), so
/// this bounds how long an agent waits to be registered. It is deliberately not exposed as a knob:
/// registration is a handshake that happens once per agent, not a data path anyone tunes.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, thiserror::Error)]
pub enum AgentRegistrationError {
    #[error("invalid registration address '{address}': {reason}")]
    InvalidAddress { address: String, reason: String },
    #[error("failed to start registration server on {address}: {reason}")]
    Start { address: String, reason: String },
    #[error("failed to spawn registration polling thread: {0}")]
    Spawn(String),
    #[error("the API was built without an agent handler, so registration cannot be served")]
    HandlerUnavailable,
    #[error("failed to read the FEAGI configuration: {0}")]
    Config(String),
    #[error("the WebSocket transport is disabled in the FEAGI configuration")]
    WebSocketTransportDisabled,
}

/// Where the registration socket binds and what it advertises.
#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    /// Local bind address, e.g. `127.0.0.1:9053`.
    pub bind_address: String,
    /// The address handed to agents, which differs from the bind address whenever the server
    /// listens on a wildcard interface or sits behind a proxy.
    pub advertised_address: String,
}

impl RegistrationConfig {
    /// Reads the registration endpoint from `feagi_configuration.toml`.
    ///
    /// The configuration is the single source of truth for this port because it is also what the
    /// API advertises to agents through `/v1/network/connection_info`; deriving it anywhere else
    /// would let the advertised endpoint and the bound socket drift apart.
    pub fn from_config_file() -> Result<Self, AgentRegistrationError> {
        let config = load_config(None, None)
            .map_err(|err| AgentRegistrationError::Config(err.to_string()))?;

        if !config.websocket.enabled {
            return Err(AgentRegistrationError::WebSocketTransportDisabled);
        }

        Ok(Self {
            bind_address: format_ws_authority(
                &config.websocket.bind_host,
                config.websocket.registration_port,
            ),
            advertised_address: format_ws_authority(
                &config.websocket.advertised_host,
                config.websocket.registration_port,
            ),
        })
    }
}

/// Renders `host:port`, bracketing IPv6 literals so the authority parses unambiguously.
fn format_ws_authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Reportable state of the registration server.
#[derive(Debug, Clone)]
pub struct RegistrationStatus {
    pub bind_address: String,
    pub advertised_address: String,
    pub running: bool,
    /// Command-and-control messages the poll handed back.
    ///
    /// Registration and heartbeat are answered inside the handler, which reports them as nothing
    /// to surface, so this counts only what the handler leaves for the caller to act on. It is not
    /// a count of registrations.
    pub messages_surfaced: u64,
    pub last_error: Option<String>,
}

/// State shared with the polling thread.
#[derive(Debug)]
struct Shared {
    running: AtomicBool,
    messages_surfaced: AtomicU64,
    last_error: parking_lot::Mutex<Option<String>>,
}

/// Owns the polling thread; answers registration handshakes until stopped or dropped.
#[derive(Debug)]
pub struct RegistrationServer {
    config: RegistrationConfig,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl RegistrationServer {
    /// Binds the registration socket and starts answering handshakes on `handler`.
    ///
    /// The socket is bound before the thread is spawned so a port conflict fails here rather than
    /// silently inside the loop.
    pub fn start(
        handler: Arc<Mutex<FeagiAgentHandler>>,
        config: RegistrationConfig,
    ) -> Result<Self, AgentRegistrationError> {
        let router = FeagiWebSocketServerRouterProperties::new_with_remote(
            &config.bind_address,
            &config.advertised_address,
        )
        .map_err(|err| AgentRegistrationError::InvalidAddress {
            address: config.bind_address.clone(),
            reason: err.to_string(),
        })?;

        {
            let mut guard =
                lock_handler(&handler).map_err(|reason| AgentRegistrationError::Start {
                    address: config.bind_address.clone(),
                    reason,
                })?;
            guard
                .add_and_start_command_control_server(Box::new(router))
                .map_err(|err| AgentRegistrationError::Start {
                    address: config.bind_address.clone(),
                    reason: err.to_string(),
                })?;
        }

        let shared = Arc::new(Shared {
            running: AtomicBool::new(true),
            messages_surfaced: AtomicU64::new(0),
            last_error: parking_lot::Mutex::new(None),
        });

        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("feagi-agent-registration".to_string())
            .spawn(move || run_poll_loop(&handler, &thread_shared))
            .map_err(|err| AgentRegistrationError::Spawn(err.to_string()))?;

        info!(
            target: "feagi-rs",
            "agent registration listening on ws://{} (advertised ws://{})",
            config.bind_address, config.advertised_address
        );

        Ok(Self {
            config,
            shared,
            thread: Some(thread),
        })
    }

    /// Stops polling. Returns `false` if the server was already stopped.
    ///
    /// The bound socket closes with the handler that owns it.
    pub fn stop(&self) -> bool {
        self.shared.running.swap(false, Ordering::Relaxed)
    }

    pub fn status(&self) -> RegistrationStatus {
        RegistrationStatus {
            bind_address: self.config.bind_address.clone(),
            advertised_address: self.config.advertised_address.clone(),
            running: self.shared.running.load(Ordering::Relaxed),
            messages_surfaced: self.shared.messages_surfaced.load(Ordering::Relaxed),
            last_error: self.shared.last_error.lock().clone(),
        }
    }
}

impl Drop for RegistrationServer {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Polls the handler on a fixed cadence until stopped.
fn run_poll_loop(handler: &Mutex<FeagiAgentHandler>, shared: &Shared) {
    while shared.running.load(Ordering::Relaxed) {
        let started = Instant::now();

        match lock_handler(handler) {
            Ok(mut guard) => poll_once(&mut guard, shared),
            Err(reason) => {
                // A poisoned handler means another thread panicked while holding it; the handler's
                // internal state is no longer trustworthy, so stop rather than keep answering.
                record_error(shared, reason);
                shared.running.store(false, Ordering::Relaxed);
                return;
            }
        }

        if let Some(remaining) = POLL_INTERVAL.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

/// Advances every server socket the handler owns by one step.
///
/// Registration is only the first of them. The publisher sockets an agent is handed at
/// registration are accepted and upgraded by these polls too, so a publisher that is never polled
/// leaves the agent's connection stuck mid-handshake: the port is bound and the TCP connect
/// succeeds, but the WebSocket upgrade is never answered. They are polled whether or not there is
/// anything to publish, so an agent can finish connecting before the first payload exists.
fn poll_once(handler: &mut FeagiAgentHandler, shared: &Shared) {
    match handler.poll_command_and_control() {
        Ok(Some((agent_id, message))) => {
            shared.messages_surfaced.fetch_add(1, Ordering::Relaxed);
            debug!(
                target: "feagi-rs",
                "agent registration: handled {:?} from session {}",
                message,
                agent_id.to_base64()
            );
        }
        Ok(None) => {}
        Err(err) => record_error(shared, err.to_string()),
    }

    if let Err(err) = handler.poll_agent_visualizers() {
        record_error(
            shared,
            format!("visualization publisher poll failed: {err}"),
        );
    }

    if let Err(err) = handler.poll_agent_motors() {
        record_error(shared, format!("motor publisher poll failed: {err}"));
    }
}

fn lock_handler(
    handler: &Mutex<FeagiAgentHandler>,
) -> Result<std::sync::MutexGuard<'_, FeagiAgentHandler>, String> {
    handler
        .lock()
        .map_err(|_| "agent handler lock is poisoned".to_string())
}

fn record_error(shared: &Shared, message: String) {
    warn!(target: "feagi-rs", "agent registration: {message}");
    *shared.last_error.lock() = Some(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use feagi_agent::server::auth::DummyAuth;

    #[test]
    fn ipv4_authority_is_host_and_port() {
        assert_eq!(format_ws_authority("127.0.0.1", 9053), "127.0.0.1:9053");
    }

    #[test]
    fn ipv6_authority_is_bracketed() {
        assert_eq!(format_ws_authority("::1", 9053), "[::1]:9053");
    }

    #[test]
    fn unbindable_address_is_reported_as_a_start_failure() {
        let handler = Arc::new(Mutex::new(FeagiAgentHandler::new(Box::new(DummyAuth {}))));
        // 240.0.0.0/4 is reserved and cannot be bound on any interface.
        let config = RegistrationConfig {
            bind_address: "240.0.0.1:9053".to_string(),
            advertised_address: "240.0.0.1:9053".to_string(),
        };

        let error = RegistrationServer::start(handler, config)
            .expect_err("an unbindable address must fail at start");
        assert!(matches!(error, AgentRegistrationError::Start { .. }));
    }
}
