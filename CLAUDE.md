# CLAUDE.md — kirk-stub-rs-multiproto

## Codebase Overview

Three components in one Cargo workspace plus a Bun TypeScript harness:

| Component | Language | Role |
|-----------|----------|------|
| `kirk-stub-realistic/` | Rust library | Six-stage compute kernel (hermitianize → eigh → softmax → density matrix → observables → entropy) |
| `kirk-server/` | Rust binary | Multi-protocol server (gRPC :50051, REST :8080, TCP :9090) |
| `bench-ts/` | Bun TypeScript | Benchmark harness (`run` + `compare` subcommands) |
| `proto/kirk.proto` | protobuf | Single source of truth for gRPC schema |
| `docker/` | Dockerfile | Multi-stage build for server and bench images |

## Key Commands

```bash
# Build
cargo build --release --workspace

# Test
cargo test --workspace

# Lint (must be clean before commits)
cargo clippy --workspace --all-targets -- -D warnings

# Run server locally
cargo run --release -p kirk-server -- --bind 127.0.0.1

# Bench (requires Bun)
cd bench-ts && bun install
bun src/cli.ts run --transport tcp --users 50 --duration 10s

# Docker
make up        # start server
make bench-all # run all three transports
make compare   # compare results
make down      # tear down
```

## Where to Put Things

- **New transport**: add `src/<name>/` to `kirk-server/`, register the listener in `src/lib.rs::start_server_with`, add a `TransportClient` impl in `bench-ts/src/transports/`.
- **Kernel changes**: edit `kirk-stub-realistic/src/` only. If the pipeline stages change, update parity fixtures in `kirk-stub-realistic/tests/fixtures/` by re-running `/tmp/gen_fixtures.py` against the Python reference.
- **Proto changes**: edit `proto/kirk.proto`, then `cargo build -p kirk-server` to regenerate stubs. The bench loads the proto at runtime; no extra step needed there.
- **New CLI flags for the server**: add to `kirk-server/src/config.rs`. Thread the value through `ServerSettings` in `src/lib.rs` if it affects listener behavior. Document in `kirk-server/README.md`.
- **New REST endpoint**: add to `kirk-server/src/rest/routes.rs` and `schema.rs`. Update the endpoint table in `kirk-server/README.md`.

## Coding Conventions

- **No unsafe code** — `#![forbid(unsafe_code)]` is on all three crates. Do not add it.
- **Mutex choice** — use `parking_lot::Mutex` (not `tokio::sync::Mutex`) for the `KirkRealistic` guard. The lock is held only for sync work and is never held across an `.await`.
- **TCP write timeouts** — all socket writes in the TCP handler are wrapped in `tokio::time::timeout`. Do not add unbounded socket writes.
- **Integer arithmetic on user-supplied N** — use `checked_mul` / `checked_add`; validate `N` against `MAX_ALLOWED_MATRIX_DIM` before any per-N arithmetic.
- **Error handling** — return `Result<_, ServerError>` from all server code. Map errors to the appropriate HTTP status / gRPC code / TCP error code at the transport layer.
- **Logging** — do not log matrix contents. Log only `N`, op name, timing, and error codes.

## Important: Python Reference is Read-Only

Do not modify `/Users/charmalloc/dev/kavara/kirk-stub-realistic/`. It is the upstream Python reference. The Rust kernel must match it numerically within NFR-001 tolerances (see `kirk-stub-realistic/README.md`). The parity fixture generator at `/tmp/gen_fixtures.py` reads from the Python reference tree.

## File Structure Quick Reference

```
kirk-stub-realistic/src/
  lib.rs           pub re-exports
  kirk.rs          KirkRealistic (stateful)
  output.rs        KirkOutput, KirkSampleOutput, KirkError
  eigensolver.rs   hermitianize + 2N block eigh
  density_matrix.rs softmax + rho
  entropy.rs       Shannon entropy
  variants.rs      5 stateless Joel-API functions
  sample.rs        forward_sample
  rng.rs           seeded_rng helper

kirk-server/src/
  main.rs          clap parse, runtime builder, --healthcheck path
  config.rs        Config struct (all flags)
  backend.rs       Arc<KirkBackend>, check_dim, spawn_blocking threshold
  lib.rs           start_server / start_server_with, ServerSettings
  shutdown.rs      broadcast + 10 s drain
  metrics.rs       Prometheus counters + histograms
  grpc/            tonic KirkService impl
  rest/            axum router, JSON schema, base64 helpers
  tcp/             framing, codec, per-connection handler

bench-ts/src/
  cli.ts           entry point
  runner.ts        measurement loop
  compare.ts       side-by-side diff
  rng.ts           Xoshiro256**
  matrix.ts        MatrixPool
  summary.ts       percentile aggregation
  results.ts       ResultFile schema
  transports/      grpc.ts, rest.ts, tcp.ts
```

## Pre-Commit Checklist

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

TypeScript (if bench-ts was changed):
```bash
cd bench-ts && bun x tsc --noEmit
```
