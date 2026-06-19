# Changelog

All notable changes to this project will be documented in this file.

## 0.1.0 - 2026-06-19 — Initial Release

### kirk-stub-realistic
- Six-stage compute kernel: hermitianize, eigendecomposition (nalgebra 2N block trick), Boltzmann softmax, density matrix reconstruction, Shannon entropy, rolling-window z-score and regime classification.
- `KirkRealistic` struct with `new(temperature, window_size)` and `forward(&mut self, ...)` API.
- Five stateless Joel-API variant functions: `inference_entropy`, `inference_features`, `active_inference`, `active_inference_entropy`, `active_inference_features`.
- `forward_sample(n, rng)` for shape-correct random samples.
- `#![forbid(unsafe_code)]`, no C/LAPACK dependencies.
- Numerical parity vs Python `numpy` reference verified at N ∈ {8, 16, 32} (entropy relative error <= 1e-4, Frobenius <= 1e-3).
- Unit tests (`tests/basic.rs`, 7 tests) and parity tests (`tests/parity.rs`, 4 tests) with JSON fixtures.

### kirk-server
- Single Rust binary: three concurrent transport listeners (gRPC :50051 tonic, REST :8080 axum, TCP :9090 raw tokio) sharing one `Arc<KirkBackend>`.
- All 7 RPCs from `proto/kirk.proto` implemented on all three transports.
- REST: JSON in/out with base64 little-endian f32 matrices; `/healthz` and `/metrics` (Prometheus text) endpoints.
- Custom TCP: 16-byte little-endian header, 8 opcodes, 8 error codes, pipelined `req_id` correlation.
- CLI flags: `--bind`, `--workers`, `--temperature`, `--window-size`, `--max-matrix-dim` (1..=4096), `--max-connections`, `--max-in-flight-per-conn`, `--tcp-write-timeout-ms`, `--log-level`, `--healthcheck`.
- Security hardening: base64 pre-decode size validation, TCP connection and in-flight semaphores, write timeouts, body limits on REST (64 MiB) and gRPC (64 MiB), `#![forbid(unsafe_code)]`.
- Graceful shutdown on SIGINT/SIGTERM with 10 s drain deadline.
- 44 tests (unit + integration across all three transports + cross-transport parity).

### bench-ts
- Bun TypeScript harness with `run` and `compare` subcommands.
- Three transport clients: REST (`fetch` keepalive), gRPC (`@grpc/grpc-js`), TCP (`Bun.connect()` with `req_id` pipelining).
- Seeded Xoshiro256** RNG for reproducible payloads.
- Pre-generated `MatrixPool` circular buffer excludes generation cost from latency measurements.
- Preallocated `Float64Array` for latency collection (no GC pressure during measurement).
- Sorted-array percentile aggregator (p50/p90/p95/p99/p999).
- JSON result output; side-by-side comparison table.

### Infrastructure
- Multi-stage Docker build: `rust:1-bookworm` builder to `distroless/cc-debian12` runtime (non-root UID 10001, no shell, no package manager).
- `docker-compose.yml`: `kirk-server` service with `--healthcheck` CLI probe; `bench` service under `bench` profile with bind-mounted results.
- Makefile: `build`, `test`, `run`, `image`, `up`, `down`, `bench-rest`, `bench-grpc`, `bench-tcp`, `bench-all`, `compare`, `clean`.
