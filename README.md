# FEAGI Server (Rust)

The FEAGI Rust server is the main runtime for the Framework for Evolutionary Artificial General Intelligence. It runs the neural burst engine, exposes the HTTP API, and manages agent I/O transports.

This package is part of the FEAGI 2.0 monorepo. Core crates live under `../feagi-core/crates/` and are consumed from crates.io in `Cargo.toml`.

## What this provides

- HTTP API for runtime control, genome I/O, and connectome management
- Transport layer for sensory input, motor output, and visualization
- Burst engine runtime (CPU by default, GPU optional)
- Genome-driven brain development and agent registration
- Configuration-driven runtime (TOML file with env and CLI overrides)

## Build and run

### Prerequisites

- Rust toolchain (edition 2021; see `Cargo.toml`)

### Build from source

```bash
cd feagi-rs
cargo build --release
```

Binary output: `target/release/feagi`.

### Run

```bash
# Auto-discover configuration file
feagi

# Specify config path
feagi --config /path/to/feagi_configuration.toml

# Load a genome on startup
feagi --genome /path/to/brain.genome

# Restore a connectome on startup
feagi --connectome /path/to/trained-brain.connectome
```

Genome and connectome startup artifacts are mutually exclusive.

## CLI options

The `feagi` binary is built when the `cli` feature is enabled (default).

| Option | Description |
|--------|-------------|
| `-f, --config <PATH>` | Config path (overrides search) |
| `-g, --genome <PATH>` | Genome to load on startup |
| `-c, --connectome <PATH>` | Connectome to restore on startup |
| `--verbose` | Verbose logging |
| `--api-port <PORT>` | Override API port |
| `--burst-hz <HZ>` | Override burst frequency |
| `--viz-transport <auto|websocket|shm>` | Override visualization transport policy |
| `--precision <fp32|int8>` | Override NPU precision |
| `--debug <CRATE>` | Enable per-crate debug (repeatable) |
| `--debug-all` | Enable debug for all crates |
| `--npu-trace` | Enable NPU trace logging |
| `--npu-trace-synapse` | Enable synapse trace logging |
| `--npu-trace-dynamics` | Enable dynamics trace logging |
| `--npu-trace-src <NEURON_ID>` | Filter trace source neuron |
| `--npu-trace-dst <NEURON_ID>` | Filter trace destination neuron |
| `--npu-trace-neuron <NEURON_ID>` | Filter trace neuron |

## Configuration

Configuration is loaded from `feagi_configuration.toml` with the following precedence:

1. CLI `--config` (explicit path)
2. `FEAGI_CONFIG_PATH` environment variable
3. Current directory `./feagi_configuration.toml`
4. Parent directories (up to 5 levels)

Overrides are applied in this order: TOML file, environment variables, then CLI overrides.

### Minimal configuration (defaults shown)

```toml
[system]
max_cores = 0

[api]
host = "0.0.0.0"
port = 8000

[ports]
zmq_req_rep_port = 5555
zmq_pub_sub_port = 5556
zmq_push_pull_port = 5557
zmq_sensory_port = 5558
zmq_visualization_port = 5562
zmq_rest_port = 5563
zmq_motor_port = 5564

[zmq]
host = "0.0.0.0"
enabled = true

[neural]
burst_engine_timestep = 0.1
```

The full schema is defined in `feagi-core/crates/feagi-config/src/types.rs`.

## Environment overrides

Supported variables (from `feagi-config`):

| Variable | Maps to |
|----------|---------|
| `FEAGI_CONFIG_PATH` | Config file path |
| `FEAGI_API_HOST` | `api.host` |
| `FEAGI_API_PORT` | `api.port` |
| `FEAGI_API_WORKERS` | `api.workers` |
| `FEAGI_API_RELOAD` | `api.reload` |
| `FEAGI_ZMQ_HOST` | `zmq.host` |
| `FEAGI_DATA_DIR` | `system.data_dir` |
| `FEAGI_MAX_CORES` | `system.max_cores` |
| `FEAGI_LOG_LEVEL` | `system.log_level` |
| `FEAGI_AGENT_DEFAULT_HOST` | `agents.default_host` |
| `FEAGI_ZMQ_REQ_REP_PORT` | `ports.zmq_req_rep_port` |
| `FEAGI_ZMQ_PUB_SUB_PORT` | `ports.zmq_pub_sub_port` |
| `FEAGI_ZMQ_PUSH_PULL_PORT` | `ports.zmq_push_pull_port` |
| `FEAGI_ZMQ_SENSORY_PORT` | `ports.zmq_sensory_port` |
| `FEAGI_ZMQ_VISUALIZATION_PORT` | `ports.zmq_visualization_port` |
| `FEAGI_ZMQ_REST_PORT` | `ports.zmq_rest_port` |
| `FEAGI_ZMQ_MOTOR_PORT` | `ports.zmq_motor_port` |

## Transports

- ZMQ transport is available via the `zeromq` crate (pure Rust).
- `feagi-io` defaults to ZMQ and UDP transports.
- WebSocket transport is enabled in this binary via the `feagi-io` `websocket-transport` feature.

## Genome autosave

Autosaved genomes are written to `.genome/` in the working directory. This folder is ignored by git.

## API documentation

Swagger UI is served at:

```
http://<api-host>:<api-port>/swagger-ui/
```

## Development

### Use local `feagi-core` crates

For local development, you can override crates.io with local paths:

```bash
cp .cargo/config.toml.example .cargo/config.toml
```

To return to crates.io behavior:

```bash
rm .cargo/config.toml
```

### Tests and linting

```bash
cargo test
cargo clippy
cargo fmt --check
```

## Troubleshooting

### Configuration not found

```
Error: Failed to load configuration
```

Ensure `feagi_configuration.toml` exists in one of the search locations or set `FEAGI_CONFIG_PATH`.

### Port already in use

```
Error: Failed to bind API server: Address already in use
```

Update the port in config or use `--api-port`.

### ZMQ bind failure

```
Error: ZMQ bind failed
```

Verify that the configured ports are available.

### Windows linking errors

If you see `LNK1169` or `LNK2005`, ensure you are using the MSVC toolchain and do not mix MSYS2/MinGW libraries on `PATH`.

## License

Apache-2.0. See [LICENSE](../LICENSE).

## Authors

Neuraville Inc. <feagi@neuraville.com>

Copyright 2025-2026 Neuraville Inc. All Rights Reserved.

