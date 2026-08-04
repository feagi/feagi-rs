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

use feagi::{
    FeagiConfig, FeagiInstance, WebSocketConfig, DEFAULT_API_PORT, DEFAULT_BURST_HZ,
    DEFAULT_WEBSOCKET_HZ, DEFAULT_WEBSOCKET_PORT,
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

    /// Host interface for the NPU state WebSocket
    #[arg(long, default_value = "0.0.0.0")]
    websocket_host: String,

    /// Port for the NPU state WebSocket
    #[arg(long, default_value_t = DEFAULT_WEBSOCKET_PORT)]
    websocket_port: u16,

    /// How many NPU state frames per second to broadcast over the WebSocket
    #[arg(long, default_value_t = DEFAULT_WEBSOCKET_HZ)]
    websocket_hz: u64,

    /// Run without the NPU state WebSocket
    #[arg(long)]
    no_websocket: bool,

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
    if !args.no_websocket && args.websocket_hz == 0 {
        anyhow::bail!("--websocket-hz must be greater than zero");
    }

    let websocket = if args.no_websocket {
        None
    } else {
        Some(WebSocketConfig::new(
            &args.websocket_host,
            args.websocket_port,
            args.websocket_hz,
        ))
    };

    let config = FeagiConfig {
        api_host: args.api_host,
        api_port: args.api_port,
        burst_hz: args.burst_hz,
        websocket,
    };

    info!(target: "feagi-rs", "FEAGI {} starting", feagi::VERSION);

    let instance = Arc::new(FeagiInstance::new(config));

    // Bind both sockets before starting the burst loop so a port conflict fails fast.
    let listener = instance.bind().await?;
    match instance.start_websocket()? {
        Some(status) => {
            info!(target: "feagi-rs", "NPU state stream at {} ({} Hz)", status.advertised_address, status.publish_hz)
        }
        None => info!(target: "feagi-rs", "websocket transport disabled (--no-websocket)"),
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
    instance.stop_websocket();
    server.abort();

    Ok(())
}
