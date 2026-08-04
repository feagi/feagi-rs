// Copyright 2025 Neuraville Inc.
// SPDX-License-Identifier: Apache-2.0

//! FEAGI server binary.
//!
//! Starts the NPU burst loop and serves the HTTP API. See the crate docs in `lib.rs` for what the
//! in-progress NPU currently supports.

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

use feagi::{FeagiConfig, FeagiInstance, DEFAULT_API_PORT, DEFAULT_BURST_HZ};

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

    // Bind before starting the burst loop so a port conflict fails fast.
    let listener = instance.bind().await?;

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
    server.abort();

    Ok(())
}
