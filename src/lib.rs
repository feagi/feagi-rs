// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! # FEAGI - Framework for Evolutionary Artificial General Intelligence
//!
//! Server application built on the in-progress `feagi-core` NPU rewrite
//! (`feagi_npu::dynamic_npu::DynamicNPU`).
//!
//! The REST surface and its OpenAPI document come from the `feagi-api` crate, which owns the
//! published endpoint contract. This crate supplies what sits behind it: the NPU, the loaded
//! genome, the adapter that lets the API services drive the engine, and the registration transport
//! agents connect through.
//!
//! Transports belong to `feagi-agent` and `feagi-io`, not to the NPU: the agent handler owns every
//! socket an agent touches, and the engine reaches agents by handing data to it. The NPU does not
//! publish anything on its own.
//!
//! The NPU currently supports adding cortical areas and running bursts. Endpoints needing
//! capabilities the engine does not yet have -- per-neuron and per-synapse introspection in
//! particular -- keep their published paths and schemas and answer `501 Not Implemented`.
//!
//! ## Embedding
//!
//! ```no_run
//! use feagi::{FeagiConfig, FeagiInstance, RegistrationConfig};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let instance = FeagiInstance::new(FeagiConfig::default());
//! instance.start_burst_engine();
//! instance.start_agent_registration(RegistrationConfig::from_config_file()?)?;
//! instance.serve().await?;
//! # Ok(())
//! # }
//! ```

pub mod agent;
pub mod api;
pub mod genome;
pub mod npu;
pub mod npu_access;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tracing::info;

pub use agent::{
    AgentRegistrationError, RegistrationConfig, RegistrationServer, RegistrationStatus,
};
pub use feagi_api::services::{empty_shared_genome, SharedGenome};
pub use npu::{CorticalAreaRecord, NpuError, NpuHandle, DEFAULT_BURST_HZ};

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
}

impl Default for FeagiConfig {
    fn default() -> Self {
        Self {
            api_host: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            api_port: DEFAULT_API_PORT,
            burst_hz: DEFAULT_BURST_HZ,
        }
    }
}

impl FeagiConfig {
    pub fn api_address(&self) -> SocketAddr {
        SocketAddr::new(self.api_host, self.api_port)
    }
}

/// A FEAGI server: one NPU, the HTTP API in front of it, and the socket agents register over.
pub struct FeagiInstance {
    config: FeagiConfig,
    npu: NpuHandle,
    /// The genome the API services read. Shared rather than owned so that loading a new genome is
    /// visible to an already-running router.
    genome: SharedGenome,
    /// The services the REST layer reads, built once so that the registration transport and the
    /// router share one agent handler: the `/v1/agent/*` endpoints report on the same registry the
    /// registration socket writes to.
    api_state: OnceLock<feagi_api::transports::http::server::ApiState>,
    registration: Mutex<Option<RegistrationServer>>,
}

impl FeagiInstance {
    pub fn new(config: FeagiConfig) -> Self {
        let npu = NpuHandle::new(config.burst_hz);
        Self {
            config,
            npu,
            genome: feagi_api::services::empty_shared_genome(),
            api_state: OnceLock::new(),
            registration: Mutex::new(None),
        }
    }

    /// The API services, built on first use and shared by every caller afterwards.
    fn api_state(&self) -> &feagi_api::transports::http::server::ApiState {
        self.api_state.get_or_init(|| {
            feagi_api::services::create_api_state_from_genome(
                Arc::clone(&self.genome),
                Some(Arc::new(self.npu.clone())),
                self.version_info(),
            )
        })
    }

    /// Handle to the genome the API services read.
    ///
    /// Genome loading publishes through this handle so the REST layer reflects the change without
    /// the router being rebuilt.
    pub fn genome(&self) -> &SharedGenome {
        &self.genome
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

    /// Binds the agent registration socket and starts answering registration handshakes.
    ///
    /// Registering is how an agent learns which endpoint carries the capability it asked for, so
    /// this has to be running before a visualizer or connector can reach any data stream.
    pub fn start_agent_registration(
        &self,
        config: RegistrationConfig,
    ) -> Result<RegistrationStatus, AgentRegistrationError> {
        let mut slot = self.registration.lock();
        if let Some(existing) = slot.as_ref() {
            return Ok(existing.status());
        }

        let handler = self
            .api_state()
            .agent_handler
            .clone()
            .ok_or(AgentRegistrationError::HandlerUnavailable)?;

        let server = RegistrationServer::start(handler, config)?;
        let status = server.status();
        *slot = Some(server);
        Ok(status)
    }

    /// Stops answering registrations. Returns `false` if the socket was not running.
    pub fn stop_agent_registration(&self) -> bool {
        self.registration.lock().take().is_some()
    }

    pub fn agent_registration_status(&self) -> Option<RegistrationStatus> {
        self.registration.lock().as_ref().map(|s| s.status())
    }

    /// Versions of the crates linked into this binary, reported by `/v1/system/version`.
    ///
    /// Only the final binary knows what was actually linked, so the value is built here rather
    /// than inside the API or service crates.
    fn version_info(&self) -> feagi_services::types::VersionInfo {
        let mut crates = std::collections::HashMap::new();
        crates.insert("feagi_rs".to_string(), VERSION.to_string());

        feagi_services::types::VersionInfo {
            crates,
            build_timestamp: String::new(),
            rust_version: String::new(),
        }
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
    ///
    /// The router comes from `feagi-api`, which owns the published endpoint contract and its
    /// OpenAPI document, so the served surface and `/swagger-ui/` stay in step with the spec.
    pub async fn serve_on(&self, listener: tokio::net::TcpListener) -> Result<()> {
        let address = listener.local_addr()?;
        let router =
            feagi_api::transports::http::server::create_http_server(self.api_state().clone());

        info!(target: "feagi-rs", "HTTP API listening on http://{address}");
        info!(target: "feagi-rs", "API documentation at http://{address}/swagger-ui/");
        axum::serve(listener, router)
            .await
            .context("HTTP server terminated unexpectedly")
    }
}
