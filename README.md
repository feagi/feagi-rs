# FEAGI Server (Rust)

The FEAGI Rust server is the main runtime for the Framework for Evolutionary Artificial General Intelligence. It runs the neural burst engine and exposes the HTTP API.

This package is part of the FEAGI 2.0 monorepo. It is built on the in-progress `feagi-core` NPU rewrite, consumed as path dependencies from a sibling `../feagi-core/` checkout.

## Status

The NPU rewrite (`feagi-core/crates/feagi-npu`, `DynamicNPU`) currently supports **adding cortical areas** and **running bursts**. This server exposes exactly that.

Subsystems from the previous architecture — genome loading, neuroembryogenesis, synaptogenesis, agent registration, ZMQ/WebSocket transports, plasticity, visualization — have not been ported. Their HTTP routes are registered and answer `501 Not Implemented` with an explanatory body, so clients can distinguish "not available yet" from a bad URL.

The previous implementation is preserved under [`legacy/`](legacy/) for reference. It does not compile against the current `feagi-core`.

## What this provides

- HTTP API for creating cortical areas and controlling the burst engine
- Burst loop running on a dedicated thread at a configurable frequency
- Embeddable library (`feagi`) alongside the standalone `feagi` binary

## Build and run

### Prerequisites

- Rust toolchain (edition 2021; see `Cargo.toml`)
- A sibling `feagi-core` checkout at `../feagi-core`

### Build

```bash
cd feagi-rs
cargo build --release
```

Binary output: `target/release/feagi`.

### Run

```bash
# Defaults: 0.0.0.0:8000, 10 Hz, burst engine auto-started
feagi

# Bind elsewhere and run faster
feagi --api-host 127.0.0.1 --api-port 8123 --burst-hz 50

# Start with the engine paused, then drive it over HTTP
feagi --no-autostart
```

## CLI options

The `feagi` binary is built when the `cli` feature is enabled (default).

| Option | Default | Description |
|--------|---------|-------------|
| `--api-host <IP>` | `0.0.0.0` | Host interface for the HTTP API |
| `--api-port <PORT>` | `8000` | Port for the HTTP API |
| `--burst-hz <HZ>` | `10` | Burst frequency |
| `--no-autostart` | off | Start with the burst engine paused |
| `-v, --verbose` | off | Debug-level logging |

Logging honors `RUST_LOG` when set; otherwise it follows `--verbose`.

## HTTP API

### Cortical areas

`POST /v1/cortical_area/cortical_area`

```bash
curl -X POST http://127.0.0.1:8000/v1/cortical_area/cortical_area \
  -H 'Content-Type: application/json' \
  -d '{"cortical_id": "cust0042", "cortical_dimensions": [8, 8, 4], "neurons_per_voxel": 2}'
```

```json
{
  "cortical_id": "cust0042",
  "cortical_id_base64": "Y3VzdDAwNDI=",
  "cortical_dimensions": [8, 8, 4],
  "neurons_per_voxel": 2,
  "neuron_count": 512
}
```

`cortical_id` accepts either the 8-character raw form (`cust0042`) or its base64 encoding. The first byte selects the area type: `c` custom, `m` memory, `_` core, `i` brain input, `o` brain output. `neurons_per_voxel` defaults to 1, so the area holds `x * y * z * neurons_per_voxel` neurons.

Responses: `201` on success, `400` for a malformed ID or a zero-sized axis, `409` if the ID already exists.

| Route | Method | Description |
|-------|--------|-------------|
| `/v1/cortical_area/cortical_area` | POST | Create a cortical area |
| `/v1/cortical_area/cortical_area/:cortical_id` | GET | Fetch one area |
| `/v1/cortical_area/cortical_area_id_list` | GET | List area IDs |
| `/v1/cortical_area/cortical_area_list` | GET | List areas with dimensions |

### Burst engine

| Route | Method | Description |
|-------|--------|-------------|
| `/v1/burst_engine/status` | GET | Running state, burst count, frequency, area count |
| `/v1/burst_engine/start` | POST | Start the burst loop |
| `/v1/burst_engine/stop` | POST | Stop the burst loop |
| `/v1/burst_engine/burst` | POST | Run exactly one burst (works while stopped) |
| `/v1/burst_engine/burst_frequency` | PUT | Set frequency, body `{"burst_frequency_hz": 50}` |

### System

| Route | Method | Description |
|-------|--------|-------------|
| `/v1/system/health_check` | GET | Engine state and area count |
| `/v1/system/version` | GET | Crate and NPU version info |

## Embedding

```rust
use feagi::{FeagiConfig, FeagiInstance};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let instance = FeagiInstance::new(FeagiConfig::default());
    instance.start_burst_engine();
    instance.serve().await
}
```

`FeagiInstance::npu()` exposes the `NpuHandle` for driving the NPU directly without going through HTTP.

## Development

Core crates are consumed as path dependencies, so no `[patch.crates-io]` setup is needed. `feagi-rs` and `feagi-core` must sit side by side.

```bash
cargo test
cargo clippy
cargo fmt --check
```

## Troubleshooting

### Port already in use

```
Error: failed to bind HTTP API to 0.0.0.0:8000
```

Use `--api-port` to pick another port.

### Endpoint returns 501

The route exists but its subsystem has not been ported to the NPU rewrite. See [Status](#status).

### Windows linking errors

If you see `LNK1169` or `LNK2005`, ensure you are using the MSVC toolchain and do not mix MSYS2/MinGW libraries on `PATH`.

## License

Apache-2.0. See [LICENSE](../LICENSE).

## Authors

Neuraville Inc. <feagi@neuraville.com>

Copyright 2025-2026 Neuraville Inc. All Rights Reserved.
