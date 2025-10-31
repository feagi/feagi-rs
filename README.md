# FEAGI - Framework for Evolutionary Artificial General Intelligence

**Full-Featured Server Application** - Apache-2.0 License

The main FEAGI server application that provides a complete neural processing platform with REST API, ZMQ streams, and real-time brain management.

## Features

- ✅ **REST API** - Full HTTP API for brain management and control
- ✅ **ZMQ Streams** - Real-time sensory input and motor output
- ✅ **Burst Engine** - High-performance neural processing
- ✅ **Genome Loading** - Load brain structures from genome files
- ✅ **Neuroembryogenesis** - Automatic brain development from genotypes
- ✅ **Agent Management** - Register and manage multiple agents
- ✅ **Brain Visualization** - Support for real-time brain visualization
- ✅ **Configuration-Driven** - No hardcoded values, all from `feagi_configuration.toml`
- ✅ **Cross-Platform** - Linux, macOS, Windows, Docker, Kubernetes

## Installation

### Prerequisites

- Rust 1.75+ (2021 edition)
- ZMQ libraries: `libzmq` (install via package manager)

### Build from Source

```bash
cd /Users/nadji/code/FEAGI-2.0/feagi
cargo build --release
```

The binary will be at: `target/release/feagi`

## Usage

### Basic Usage

```bash
# With auto-discovered config
feagi

# With explicit config path
feagi --config /path/to/feagi_configuration.toml

# Load a genome on startup
feagi --genome path/to/genome.json

# Enable verbose logging
feagi --verbose

# Override API port
feagi --api-port 9000
```

### With Docker

```bash
docker run -p 8000:8000 \
  -v $(pwd)/feagi_configuration.toml:/app/feagi_configuration.toml \
  -v $(pwd)/genomes:/app/genomes \
  feagi:latest --genome /app/genomes/vision_genome.json
```

### With Kubernetes

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: feagi-config
data:
  feagi_configuration.toml: |
    [api]
    host = "0.0.0.0"
    port = 8000
    ...
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: feagi
spec:
  replicas: 1
  template:
    spec:
      containers:
      - name: feagi
        image: feagi:2.0.0
        volumeMounts:
        - name: config
          mountPath: /app/feagi_configuration.toml
          subPath: feagi_configuration.toml
      volumes:
      - name: config
        configMap:
          name: feagi-config
```

## Configuration

FEAGI requires a `feagi_configuration.toml` file. The application will search for it in:

1. Path specified by `--config` flag
2. `FEAGI_CONFIG_PATH` environment variable
3. Current working directory: `./feagi_configuration.toml`
4. Parent directories (up to 5 levels)

### Minimal Configuration

```toml
[system]
max_cores = 0  # 0 = auto-detect

[api]
host = "0.0.0.0"
port = 8000

[ports]
zmq_sensory_port = 5558
zmq_motor_port = 5564
zmq_visualization_port = 5562

[zmq]
host = "0.0.0.0"

[neural]
burst_engine_timestep = 0.1  # milliseconds
```

See `../feagi-core/feagi_configuration.toml` for a complete example.

## API Endpoints

Once running, access the interactive API documentation at:

```
http://localhost:8000/swagger-ui/
```

### Key Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/health` | GET | Health check |
| `/v1/system/status` | GET | System status |
| `/v1/genome/load` | POST | Load genome |
| `/v1/cortical_areas` | GET | List cortical areas |
| `/v1/cortical_areas` | POST | Create cortical area |
| `/v1/neurons` | POST | Create neuron |
| `/v1/runtime/start` | POST | Start burst engine |
| `/v1/runtime/stop` | POST | Stop burst engine |

## Architecture

FEAGI is built from modular Rust crates:

```
feagi (application)
├── feagi-config       (Configuration loading)
├── feagi-bdu          (Brain Development Unit)
├── feagi-burst-engine (NPU/Neural Processing)
├── feagi-evo          (Genome I/O)
├── feagi-services     (Service layer)
├── feagi-api          (REST API)
├── feagi-pns          (ZMQ streams)
└── ...
```

All libraries are in `../feagi-core/crates/`

## Development

### Run in Development Mode

```bash
cargo run -- --verbose --genome ../genomes/test_genome.json
```

### Run Tests

```bash
cargo test
```

### Check Code

```bash
cargo clippy
cargo fmt --check
```

## Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `FEAGI_CONFIG_PATH` | Path to config file | `/etc/feagi/config.toml` |
| `FEAGI_API_HOST` | Override API host | `0.0.0.0` |
| `FEAGI_API_PORT` | Override API port | `9000` |
| `FEAGI_ZMQ_HOST` | Override ZMQ host | `127.0.0.1` |
| `RUST_LOG` | Log level | `debug`, `info`, `warn` |

## Troubleshooting

### Config Not Found

```
Error: Failed to load configuration
```

**Solution**: Ensure `feagi_configuration.toml` exists in the current directory or specify with `--config`.

### Port Already in Use

```
Error: Failed to bind API server: Address already in use
```

**Solution**: Change the port in config or use `--api-port` flag.

### ZMQ Connection Failed

```
Error: ZMQ bind failed
```

**Solution**: Check that ZMQ ports are available and not blocked by firewall.

## Differences from `feagi-inference-engine`

| Feature | `feagi` (This) | `feagi-inference-engine` |
|---------|----------------|--------------------------|
| **Purpose** | Full server | Embedded inference only |
| **REST API** | ✅ Full API (60 endpoints) | ❌ None |
| **ZMQ Streams** | ✅ Full PNS | ✅ Basic |
| **Genome Loading** | ✅ Full neuroembryogenesis | ❌ Load pre-trained only |
| **Brain Development** | ✅ Full BDU | ❌ None |
| **Agent Management** | ✅ Full registry | ✅ Basic |
| **Target** | Servers, cloud, desktop | Embedded, RTOS, edge |
| **License** | Apache-2.0 | Apache-2.0 or Commercial |
| **Size** | ~50MB | ~5MB |

## License

Apache-2.0 - See [LICENSE](../LICENSE) for details.

## Links

- **Documentation**: https://feagi.org/docs
- **Repository**: https://github.com/Neuraville/FEAGI-2.0
- **Issues**: https://github.com/Neuraville/FEAGI-2.0/issues
- **Discord**: https://discord.gg/feagi

## Authors

Neuraville Inc. - <feagi@neuraville.com>

Copyright 2016-2025 Neuraville Inc. All Rights Reserved.



