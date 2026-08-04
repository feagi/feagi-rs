# Legacy feagi-rs sources

This is the feagi-rs implementation from before the move to the `feagi-core` NPU rewrite. It is kept
for reference only and is **not part of the Cargo build**.

It cannot be compiled as-is. It depends on a crate stack that no longer exists: `feagi-api`,
`feagi-services`, `feagi-brain-development`, `feagi-evolutionary`, `feagi-structures`, `feagi-config`,
`feagi-io`, `feagi-agent`, `feagi-state-manager`, `feagi-observability`, and the old
`feagi-npu-{neural,runtime,burst-engine,plasticity}` crates. Those were either removed from the
`feagi-core` workspace or moved to `feagi-core/_crates_being_removed_replaced/`, and the ones still
on disk inherit workspace dependencies that were deleted, so their manifests no longer parse.

## Contents

| Path | Was |
|------|-----|
| `src/main.rs` | Standalone server binary: config loading, genome load ordering, agent polling, ZMQ/WebSocket wiring, NPU tracing flags |
| `src/components.rs` | Component construction and HTTP server startup for embedded use |
| `src/lib.rs` | `FeagiInstance` library entry point |
| `src/network_provider.rs` | Connection info provider for the network endpoints |
| `src/plasticity_runtime.rs` | Plasticity callback wiring |
| `src/version.rs` | Per-crate version collection for `/v1/system/version` |
| `tests/` | Replay runtime test and CPU/memory/storage profiling harnesses |
| `benches/` | Criterion benchmarks |
| `examples/` | Sample agent using the sensorimotor crate |

## Why it is worth keeping

These files are the most complete record of how the previous architecture sequenced startup, in
particular the ordering constraint that the burst engine must not run while neuroembryogenesis
mutates the connectome. That ordering will matter again once genome loading is ported to the new
NPU.
