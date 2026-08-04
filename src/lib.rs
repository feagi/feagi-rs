// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! # FEAGI - Framework for Evolutionary Artificial General Intelligence
//!
//! Server application built on the in-progress `feagi-core` NPU rewrite
//! (`feagi_npu::dynamic_npu::DynamicNPU`).
//!
//! The NPU currently supports adding cortical areas and running bursts, so that is what this
//! server exposes over HTTP and over the `feagi-io` WebSocket transport. Subsystems from the
//! previous architecture (genome loading, agents, ZMQ streaming, plasticity, synaptogenesis) are
//! not wired up; their HTTP routes answer with `501 Not Implemented`. The previous implementation
//! is kept for reference under `legacy/`.
//!
//! ## Embedding
//!
//! ```no_run
//! use feagi::{FeagiInstance, FeagiConfig};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let instance = FeagiInstance::new(FeagiConfig::default());
//! instance.start_websocket()?;
//! instance.start_burst_engine();
//! instance.serve().await?;
//! # Ok(())
//! # }
//! ```

pub mod api;
pub mod npu;
pub mod ws;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tracing::info;

pub use api::{create_http_server, ApiState};
pub use npu::{CorticalAreaRecord, NpuError, NpuHandle, DEFAULT_BURST_HZ};
pub use ws::{
    WebSocketBroadcaster, WebSocketConfig, WebSocketError, WebSocketStatus, DEFAULT_WEBSOCKET_HZ,
    DEFAULT_WEBSOCKET_PORT,
};

/// Version of this crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default port for the HTTP API.
pub const DEFAULT_API_PORT: u16 = 8000;

/// Runtime configuration for a [`FeagiInstance`].
#[derive(Debug, Clone)]
pub struct FeagiConfig {
    pub api_host: IpAddr,
    pub api_port: u16,
    pub burst_hz: u64,
    /// WebSocket broadcast settings, or `None` to run without the transport.
    pub websocket: Option<WebSocketConfig>,
}

impl Default for FeagiConfig {
    fn default() -> Self {
        Self {
            api_host: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            api_port: DEFAULT_API_PORT,
            burst_hz: DEFAULT_BURST_HZ,
            websocket: Some(WebSocketConfig::default()),
        }
    }
}

impl FeagiConfig {
    pub fn api_address(&self) -> SocketAddr {
        SocketAddr::new(self.api_host, self.api_port)
    }
}

/// A FEAGI server: one NPU, the HTTP API in front of it, and the WebSocket state broadcast.
pub struct FeagiInstance {
    config: FeagiConfig,
    npu: NpuHandle,
    /// Behind a lock so the transport can be started and stopped through a shared handle, the way
    /// the burst engine is.
    websocket: Mutex<Option<Arc<WebSocketBroadcaster>>>,
}

impl FeagiInstance {
    pub fn new(config: FeagiConfig) -> Self {
        let npu = NpuHandle::new(config.burst_hz);
        Self {
            config,
            npu,
            websocket: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &FeagiConfig {
        &self.config
    }

    /// Handle to the NPU, for driving it directly when embedding.
    pub fn npu(&self) -> &NpuHandle {
        &self.npu
    }

    pub fn start_burst_engine(&self) -> bool {
        self.npu.start()
    }

    pub fn stop_burst_engine(&self) -> bool {
        self.npu.stop()
    }

    /// Binds the WebSocket publisher configured in [`FeagiConfig::websocket`] and starts
    /// broadcasting NPU state. Does nothing when no WebSocket config is set.
    ///
    /// Call this before [`Self::serve`], because the HTTP status endpoint reports the broadcaster
    /// that exists at the time the router is built.
    pub fn start_websocket(&self) -> Result<Option<WebSocketStatus>, WebSocketError> {
        let Some(ws_config) = self.config.websocket.clone() else {
            return Ok(None);
        };

        let mut slot = self.websocket.lock();
        if let Some(existing) = slot.as_ref() {
            return Ok(Some(existing.status()));
        }

        let broadcaster = WebSocketBroadcaster::start(ws_config, self.npu.clone())?;
        let status = broadcaster.status();
        *slot = Some(Arc::new(broadcaster));
        Ok(Some(status))
    }

    /// Stops the WebSocket publisher. Returns `false` if it was not running.
    ///
    /// The socket closes once the last handle handed out to the API layer is dropped.
    pub fn stop_websocket(&self) -> bool {
        match self.websocket.lock().take() {
            Some(broadcaster) => {
                broadcaster.stop();
                true
            }
            None => false,
        }
    }

    pub fn websocket_status(&self) -> Option<WebSocketStatus> {
        self.websocket.lock().as_ref().map(|ws| ws.status())
    }

    /// Binds the API socket without serving, so callers can learn the bound port
    /// (useful when `api_port` is 0) before handing the listener to [`Self::serve_on`].
    pub async fn bind(&self) -> Result<tokio::net::TcpListener> {
        let address = self.config.api_address();
        tokio::net::TcpListener::bind(address)
            .await
            .with_context(|| format!("failed to bind HTTP API to {address}"))
    }

    /// Serves the HTTP API until the process is shut down.
    pub async fn serve(&self) -> Result<()> {
        let listener = self.bind().await?;
        self.serve_on(listener).await
    }

    /// Serves the HTTP API on an already-bound listener.
    pub async fn serve_on(&self, listener: tokio::net::TcpListener) -> Result<()> {
        let address = listener.local_addr()?;
        let router = create_http_server(ApiState::new(
            self.npu.clone(),
            self.websocket.lock().as_ref().map(Arc::clone),
        ));

        info!(target: "feagi-rs", "HTTP API listening on http://{address}");
        axum::serve(listener, router)
            .await
            .context("HTTP server terminated unexpectedly")
    }
}
