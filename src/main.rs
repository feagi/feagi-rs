// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! FEAGI server binary.
//!
//! Starts the NPU burst loop and serves the HTTP API. See the crate docs in `lib.rs` for what the
//! in-progress NPU currently supports.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use feagi::{
    AgentRegistrationError, FeagiConfig, FeagiInstance, RegistrationConfig, DEFAULT_API_PORT,
    DEFAULT_BURST_HZ,
};

/// FEAGI Server - neural processing and brain management
#[derive(Parser, Debug)]
#[command(name = "feagi", version, author, about, long_about = None)]
struct Args {
    /// Host interface for the HTTP API
    #[arg(long, default_value = "0.0.0.0")]
    api_host: IpAddr,

    /// Port for the HTTP API
    #[arg(long, default_value_t = DEFAULT_API_PORT)]
    api_port: u16,

    /// Burst frequency in Hz
    #[arg(long, default_value_t = DEFAULT_BURST_HZ)]
    burst_hz: u64,

    /// Start with the burst engine paused; resume via POST /v1/burst_engine/start
    #[arg(long)]
    no_autostart: bool,

    /// Genome file to load before the burst engine starts
    #[arg(long, value_name = "PATH")]
    genome: Option<PathBuf>,

    /// Enable verbose (debug) logging
    #[arg(short, long)]
    verbose: bool,
}

fn init_logging(verbose: bool) {
    let default_level = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("feagi={default_level},{default_level}")));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging(args.verbose);

    if args.burst_hz == 0 {
        anyhow::bail!("--burst-hz must be greater than zero");
    }

    let config = FeagiConfig {
        api_host: args.api_host,
        api_port: args.api_port,
        burst_hz: args.burst_hz,
    };

    info!(target: "feagi-rs", "FEAGI {} starting", feagi::VERSION);

    let instance = Arc::new(FeagiInstance::new(config));

    // Bind both sockets before starting the burst loop so a port conflict fails fast.
    let listener = instance.bind().await?;

    // Agent registration follows `feagi_configuration.toml` rather than a switch of its own: the
    // transport an agent is told to use and the socket it reaches have to be decided in one place.
    match RegistrationConfig::from_config_file() {
        Ok(registration) => {
            let status = instance.start_agent_registration(registration)?;
            info!(
                target: "feagi-rs",
                "agent registration at ws://{}",
                status.advertised_address
            );
        }
        Err(AgentRegistrationError::WebSocketTransportDisabled) => {
            info!(target: "feagi-rs", "agent registration disabled ([websocket] enabled = false)")
        }
        Err(err) => return Err(err.into()),
    }

    // Load before the first burst so the engine never runs against a half-built connectome.
    if let Some(path) = args.genome.as_deref() {
        let summary = feagi::genome::load_genome_file(instance.npu(), instance.genome(), path)
            .with_context(|| format!("failed to load genome '{}'", path.display()))?;
        info!(
            target: "feagi-rs",
            "genome '{}' loaded: {} areas, {} neurons",
            summary.genome_title, summary.areas_added, summary.neurons_added
        );
    }

    if args.no_autostart {
        info!(target: "feagi-rs", "burst engine paused (--no-autostart)");
    } else {
        instance.start_burst_engine();
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let shutdown = Arc::clone(&shutdown);
        ctrlc::set_handler(move || shutdown.store(true, Ordering::Relaxed))
            .context("failed to install Ctrl-C handler")?;
    }

    let serve_instance = Arc::clone(&instance);
    let server = tokio::spawn(async move { serve_instance.serve_on(listener).await });

    while !shutdown.load(Ordering::Relaxed) {
        if server.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    info!(target: "feagi-rs", "shutting down");
    instance.stop_burst_engine();
    instance.stop_agent_registration();
    server.abort();

    Ok(())
}
