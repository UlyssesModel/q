# Plan — Rust Kirk Stub Realistic + Multi-Protocol Server + Bun Benchmark Harness

**Session**: `20260616_201914_kirk-stub-rs-multiproto`
**Worktree**: `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/`
**Spec**: `agent-notes/architect.md` (authoritative — read it first)

## Goal

Port the Python `kirk-stub-realistic` package to Rust and wrap it in a single hyper-scaled Rust server that simultaneously serves the same interface over three transports (gRPC :50051, REST :8080, custom binary TCP :9090). Provide a Bun TypeScript harness that drives concurrent virtual users against any transport and produces side-by-side latency/throughput comparisons. Package the server and bench in `docker compose`.

## Phased Implementation

### Phase A — `kirk-stub-realistic/` crate (port)
1. Workspace `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`.
2. Crate skeleton with modules: `lib.rs`, `output.rs`, `eigensolver.rs`, `density_matrix.rs`, `entropy.rs`, `observables.rs`, `reconstruct.rs`, `variants.rs`, `sample.rs`, `kirk.rs`, `rng.rs`.
3. Hermitian complex eigendecomposition via `nalgebra` (real `2N` block trick — ADR-001).
4. Implement `KirkRealistic::{new, forward}` with stateful rolling-window z-score + regime classifier.
5. Implement 5 Joel-API variants + `forward_sample`.
6. Inline unit tests covering: hermitianize symmetry, eigenvalue trace, `Tr(rho) ≈ 1`, confidence in `[0, 1]`, variant NaN-free at N ∈ {2, 4, 8, 16, 32}.
7. Generate Python parity fixtures (one-off script) and place under `kirk-stub-realistic/tests/fixtures/`. The test agent will own actually running the parity script — coder seeds with at least one hand-computed fixture so `tests/parity.rs` compiles.

### Phase B — `proto/kirk.proto`
1. Drop in the proto schema from the spec verbatim.

### Phase C — `kirk-server/` binary
1. `build.rs` calls `tonic-build` on `../proto/kirk.proto`.
2. `config.rs` — `clap`-derived `Config` struct (all CLI flags per FR-014).
3. `backend.rs` — `KirkBackend { kirk: Mutex<KirkRealistic>, max_matrix_dim, shutdown_signal }`. Variant fns called without lock.
4. `shutdown.rs` — `tokio::sync::broadcast` shutdown signal + 10s drain.
5. `error.rs` — `ServerError` + transport-specific mappers.
6. `grpc/service.rs` — `tonic::async_trait` impl of all 7 RPCs.
7. `rest/{routes.rs, schema.rs}` — axum router + serde envelopes + base64 helpers + `/healthz` + `/metrics`.
8. `tcp/{framing.rs, codec.rs, handler.rs, mod.rs}` — pure-fn header parse, per-opcode payload codec, per-connection loop with pipelining (`req_id → response`).
9. `metrics.rs` — Prometheus text counters/histograms.
10. `main.rs` — runtime builder, three `tokio::spawn` listeners, signal handling, shutdown drain.
11. `spawn_blocking` wrapper used when `N >= 128` for `forward`/variant calls (ADR-007).

### Phase D — `bench-ts/` Bun harness
1. `package.json`, `tsconfig.json`, `bunfig.toml`.
2. `src/cli.ts` — `parseArgs` + dispatch (`run`, `compare`).
3. `src/rng.ts` — seeded xoshiro256** (matches Rust `rand_xoshiro`).
4. `src/matrix.ts` — pre-generated circular buffer of random matrices (warmup phase).
5. `src/transports/{rest,grpc,tcp}.ts` — persistent connections; tcp does pipelined req_id correlation.
6. `src/runner.ts` — concurrent virtual user loop, preallocated `Float64Array` for latencies, error counter, warmup-then-measure phasing.
7. `src/summary.ts` — sorted-array percentiles (p50/p90/p95/p99/p999/min/max/mean), throughput, errors.
8. `src/results.ts` — JSON result file schema + writer.
9. `src/compare.ts` — diff N result files, pretty-print deltas.
10. Proto loading helper — copy `proto/kirk.proto` into `bench-ts/proto/` at build time (Makefile target).

### Phase E — Docker + ops
1. `docker/Dockerfile` — multi-stage `rust:1-bookworm` → `gcr.io/distroless/cc-debian12`, non-root user.
2. `docker/Dockerfile.bench` — `oven/bun:1-alpine`.
3. `docker-compose.yml` — `kirk-server` (long-lived, healthcheck) + `bench` (profile=bench, env-driven).
4. `Makefile` — `build`, `run`, `bench-tcp`, `bench-grpc`, `bench-rest`, `bench-all`, `compare`, `clean`.
5. Top-level `README.md` — quick-start, architecture overview, transport comparison usage.

### Phase F — Glue
1. Workspace-wide `.gitignore` (target/, node_modules/, results/, *.lock policies).
2. Per-crate `README.md` for `kirk-stub-realistic/`, `kirk-server/`, `bench-ts/`.
3. Top-level `README.md` end-to-end example.

## Acceptance Criteria

- `cargo build --release` succeeds at the workspace root.
- `cargo test -p kirk-stub-realistic` passes (parity tolerances per architect spec NFR-001).
- `cargo run -p kirk-server` listens on 3 ports; `curl http://localhost:8080/healthz` returns 200.
- `bunx tsc --noEmit` passes on `bench-ts`.
- `docker compose up -d kirk-server` followed by `TRANSPORT=tcp docker compose run --rm bench` produces a result JSON.
- Bench at N=32, 100 users, 30s shows `tcp_p95 ≤ grpc_p95 ≤ rest_p95` (NFR-002). If the ordering inverts, mark it as TODO and document with measured numbers.

## Non-Goals (this loop)

- Authentication, TLS, Kafka streaming, GPU, multi-tenant state.
- Replacing the existing Python `/Users/charmalloc/dev/kavara/kirk-stub-realistic/` directory. The Rust port lives in the worktree only; the user can lift it standalone later.
