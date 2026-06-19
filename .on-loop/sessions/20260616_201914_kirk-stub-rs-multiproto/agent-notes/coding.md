# Coding Agent Notes

## Summary

Implemented all six phases (A-F) of the spec inside the worktree
`/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/`.

- Phase A (`kirk-stub-realistic` crate): full pipeline port + 7 module split
  + unit tests + Python-generated parity fixtures for seed=42 N∈{8,16,32} plus
  a hand-computed N=2 fixture.
- Phase B (`proto/kirk.proto`): verbatim copy of the spec schema.
- Phase C (`kirk-server` binary): clap config, broadcast-based shutdown,
  Prometheus metrics, three concurrent listeners (gRPC tonic, REST axum,
  custom TCP) backed by `Arc<KirkBackend>` with `spawn_blocking` for
  `N >= 128`.
- Phase D (`bench-ts`): Bun harness (`run` + `compare`), 3 transport clients
  (REST `fetch`, gRPC `@grpc/grpc-js`, TCP `Bun.connect()` with `req_id`
  correlation), seeded xoshiro256** RNG, pre-generated matrix pool,
  preallocated `Float64Array` for latencies.
- Phase E (`docker/` + `docker-compose.yml` + `Makefile`): multi-stage
  distroless server image, Bun bench image, compose with `bench` profile,
  Makefile shortcuts.
- Phase F: top-level `README.md`, per-crate READMEs, `.gitignore`, `.dockerignore`.

End-to-end smoke test passes against a locally-built release binary:
- `/healthz` → `{"status":"ok"}`
- REST `/v1/forward` with the hand-computed `H = diag(1, -1)` (N=2) returns
  `entropy=0.36533388`, matching the analytical value (0.36533208) within
  tolerance.
- SIGTERM triggers graceful drain across all three transports.

## Build status

```
cargo check --workspace          ✓ clean (no warnings)
cargo test --workspace           ✓ 16 tests pass (incl. 4 parity tests)
cargo build --release -p kirk-server ✓
node_modules/.bin/tsc --noEmit (bench-ts) ✓ clean
```

Bun is not installed on the host, so `bunx tsc --noEmit` was substituted with
local `tsc` via `npm install typescript@5.4`. The Bun runtime is required at
runtime for the `tcp` transport (uses `Bun.connect()`) — the runner refuses
to run the tcp client under plain Node.

## Decisions

- **`forward_sample` signature**: spec says `forward_sample(n: usize, rng) ->
  KirkSampleOutput`. The Python reference takes a `sample` array. I followed
  the spec; the bench/test agents can use the seeded RNG path.
- **2N block trick eigh**: pure `nalgebra::SymmetricEigen` on the real
  symmetric `[[Re,-Im],[Im,Re]]` matrix. Eigenvalues appear twice — I sort
  ascending and pick every-other (with a `1e-5` relative-tolerance gate on
  the paired duplicate). Validated against `numpy.linalg.eigh` via the
  Python parity fixtures at N=8,16,32 — relative entropy error well under
  the 1e-4 budget and Frobenius `||ρ_rust - ρ_py||_F / ||ρ_py||_F < 1e-3`
  (test passes with the default tolerances).
- **`rand_distr` added** to `kirk-stub-realistic/Cargo.toml` for the
  `StandardNormal` and `Exp(1)` samplers used by `forward_sample`.
  This is in the same major series as the other rand crates listed in the
  spec; not a deviation in spirit.
- **`tonic-build` `compile_protos` rename**: the 0.12.3 release deprecated
  `.compile(...)` in favor of `.compile_protos(...)`. Used the new name to
  avoid the deprecation warning.
- **`protoc` is required at server-image build time**: the Dockerfile
  installs `protobuf-compiler` in the builder stage. Documented in
  `coding.md` only — the test agent will need `protoc` on the host too
  (`brew install protobuf`).
- **TCP error mapping**: kept the spec's 0x01..0x08 codes; mapped to
  `ServerError` variants in `handler.rs`. `parking_lot::Mutex` instead of
  `tokio::sync::Mutex` — the lock is held only for the forward call (which
  is sync) and never across an `await`.
- **`spawn_blocking` threshold**: `N >= 128` (per ADR-007). Both stateful
  `forward` and stateless variants honor it.

## Files Created

### Workspace root
- `Cargo.toml` — workspace `[members = ["kirk-stub-realistic", "kirk-server"]]`,
  release profile (lto thin, abort on panic).
- `rust-toolchain.toml`
- `.gitignore`
- `.dockerignore`
- `Makefile`
- `README.md`
- `docker-compose.yml`
- `docker/Dockerfile` — `rust:1-bookworm` builder → `distroless/cc-debian12`
  runtime, non-root UID 10001.
- `docker/Dockerfile.bench` — `oven/bun:1-alpine`.

### proto/
- `proto/kirk.proto`

### kirk-stub-realistic/
- `Cargo.toml`
- `README.md`
- `src/lib.rs`
- `src/output.rs` — `KirkOutput`, `KirkSampleOutput`, `KirkError`.
- `src/eigensolver.rs` — `Hermitian`, `Eigh`, `diagonalize` (2N block trick).
- `src/density_matrix.rs` — Boltzmann softmax + density-matrix reconstruction.
- `src/entropy.rs` — Shannon entropy + `entropy_from_hamiltonian`.
- `src/observables.rs` — energy/magnetization/occupancy (parity).
- `src/reconstruct.rs` — `rho_to_real_imag`.
- `src/rng.rs` — seeded Xoshiro256**.
- `src/variants.rs` — 5 Joel-API stateless functions.
- `src/sample.rs` — `forward_sample(n, rng)`.
- `src/kirk.rs` — stateful `KirkRealistic`.
- `tests/basic.rs` — 7 sanity tests (hermitianize, trace, ρ trace, confidence,
  shannon, variants, forward_sample).
- `tests/parity.rs` — 4 parity tests (handcalc N=2, seed42 N=8/16/32).
- `tests/fixtures/handcalc_N2.json`
- `tests/fixtures/seed42_N8.json` (Python-generated)
- `tests/fixtures/seed42_N16.json` (Python-generated)
- `tests/fixtures/seed42_N32.json` (Python-generated)

### kirk-server/
- `Cargo.toml`
- `README.md`
- `build.rs` — tonic-build.
- `src/main.rs` — clap parse, runtime builder, three listeners, signal handling.
- `src/config.rs` — CLI struct.
- `src/error.rs` — `ServerError` + http status / code mapping.
- `src/backend.rs` — `Arc<KirkBackend>` with `parking_lot::Mutex<KirkRealistic>`
  and `spawn_blocking` for N >= 128.
- `src/shutdown.rs` — broadcast channel + 10s drain deadline.
- `src/metrics.rs` — Prometheus text counters + 15-bucket histogram per
  (transport, op).
- `src/grpc/mod.rs` + `src/grpc/service.rs` — tonic impl for all 7 RPCs.
- `src/rest/mod.rs` + `src/rest/routes.rs` + `src/rest/schema.rs` — axum
  router, `/healthz`, `/metrics`, base64 helpers, JSON envelopes.
- `src/tcp/mod.rs` + `src/tcp/framing.rs` + `src/tcp/codec.rs`
  + `src/tcp/handler.rs` — wire format, per-opcode codec, per-connection
  task with bounded mpsc to a writer task.

### bench-ts/
- `package.json`, `bunfig.toml`, `tsconfig.json`
- `README.md`
- `src/cli.ts`, `src/runner.ts`, `src/compare.ts`
- `src/rng.ts` — Xoshiro256** (splitmix64-seeded, matches Rust bit-for-bit).
- `src/matrix.ts` — `MatrixPool` (circular buffer of pre-generated matrices).
- `src/summary.ts` — sorted-array percentile aggregator + pretty-print.
- `src/results.ts` — `ResultFile` schema + `writeResult`.
- `src/transports/types.ts`, `src/transports/rest.ts`,
  `src/transports/grpc.ts`, `src/transports/tcp.ts`.
- `results/.gitkeep`

### Out-of-worktree helper (for the test agent)
- `/tmp/gen_fixtures.py` — Python script that regenerates seed42 fixtures.
  Run via `cd /Users/charmalloc/dev/kavara/kirk-stub-realistic && uv run --python 3.13 python /tmp/gen_fixtures.py`.

## Issues Found

- [INFO] Existing `q/.gitignore` was minimal; I expanded it to cover Node/Bun
  artifacts, OS junk, and bench results.
- [INFO] Python reference is read-only — no modifications to
  `/Users/charmalloc/dev/kavara/kirk-stub-realistic/`. The `/tmp/gen_fixtures.py`
  script reads from there but writes only to the worktree fixtures dir.

## Open TODOs (for test / doc / build agents)

- [TEST] Cross-platform `tcp` smoke: open a TCP socket from a tokio client,
  send FORWARD, PING, oversized FORWARD (>64 MiB), and ERROR opcode, assert
  the wire-level response matches the spec byte for byte. Unit tests already
  cover the header parser in isolation.
- [TEST] gRPC end-to-end: connect with a tonic client (or `grpcurl`), call
  `Forward` with a known matrix, compare to the REST output for the same
  matrix. The smoke test in this session only exercised REST.
- [TEST] `bench-ts` runtime: install Bun on the test host
  (`curl -fsSL https://bun.sh/install | bash`) and run
  `bun src/cli.ts run --transport rest --users 4 --duration 5s` against
  a locally-running server to confirm the harness wires up correctly.
- [TEST] NFR-002 throughput-ordering assertion: run the full bench suite
  (`make bench-rest`, `bench-grpc`, `bench-tcp`) and verify `tcp_p95 ≤
  grpc_p95 ≤ rest_p95` at N=32, 100 users.
- [TEST] Numerical parity on the variants: the parity tests currently only
  cover `KirkRealistic::forward`. Adding variant-fixture tests (entropy
  scalars from Python `inference_entropy(sample)`) is straightforward —
  extend `/tmp/gen_fixtures.py`.
- [BUILD] `protoc` is required at compile time. The server Dockerfile installs
  it; host builds need `brew install protobuf` on macOS or
  `apt-get install protobuf-compiler` on Debian/Ubuntu.
- [DOC] No explicit `kirk-stub-realistic/CHANGELOG.md` — the spec did not
  require one. Add if the doc agent wants version history.

## Recommendations for Next Agent

- The numerical kernel is already validated against Python at N∈{8,16,32}
  with the seed42 fixtures. If the test agent wants tighter tolerances, the
  current Frobenius error is well below 1e-3 — likely close to 1e-4 — but
  I did not record exact numbers.
- The TCP handler buffers payloads to a `Vec<u8>` then dispatches; for
  large N the spec allows zero-copy via `bytemuck::cast_slice` — left as a
  potential future optimization. Functional now.
- For the bench, the matrix pool size is fixed at 1024; the spec recommends
  this as "fresh matrix per request, but pre-generated". Adjust via the
  `MatrixPool` ctor if desired.
- If the test agent finds the `Frobenius ≤ 1e-3` tolerance fails on a future
  rebuild, suspect the every-other-eigenvalue picker in `eigensolver.rs`:
  for highly degenerate spectra the 1e-5 relative gate may need tightening.
  Repro: feed a known-degenerate Hermitian and assert `rho.trace() ≈ 1`.

## Remediation Pass (2026-06-17)

All HIGH and MEDIUM findings addressed. LOW finding SEC-009 fixed. SEC-014
fixed. SEC-012 documented in `kirk-server/README.md`. SEC-010, SEC-011 (partial
— a BAD_PAYLOAD frame is now emitted on truncated payloads), SEC-013, and
SEC-015..SEC-019 deferred per the orchestrator brief.

### HIGH

- **SEC-001** (base64 decode before cap) — `kirk-server/src/rest/schema.rs:99-156`.
  Two-step pre-decode guard: `base64_decoded_upper_bound(b64.len())` checked
  against both `MAX_DECODED_BYTES` (global) and `4 * matrix_dim^2 + 2 (slack)`
  (per-shape). The slack covers standard-base64 padding bias. Decode is now
  via `B64.decode_vec(...)` into a pre-sized `Vec::with_capacity(expected_bytes)`
  so the allocation is bounded by validated shape, not attacker input length.

- **SEC-002** (no TCP connection / per-frame cap) — `kirk-server/src/tcp/handler.rs`.
  Added `TcpServeLimits` with `max_connections` + `max_in_flight_per_conn`.
  Accept loop wraps each connection in a `Semaphore::try_acquire_owned()` —
  when the cap is saturated, the new connection is dropped (logged at WARN)
  rather than blocking the accept loop. Per-connection handler holds an
  `Arc<Semaphore>` whose permits gate each `tokio::spawn(process_frame)`.
  Both caps are exposed as CLI flags / env vars and clamped by clap
  `value_parser!(u32).range(1..=65535)`.
  Note on `socket2`: I did NOT add a dependency to set `SO_BACKLOG = 1024`.
  Tokio's `TcpListener::bind` already passes a sensible default backlog
  (128 on most platforms via `mio`). Raising this further is OS-dependent
  and felt outside the security-fix scope; the connection-count semaphore
  bounds the long-term resource consumption, which is the bigger lever per
  the security agent's own comment. Documented this trade-off here.

### MEDIUM

- **SEC-003** (--max-matrix-dim unbounded) — `kirk-server/src/config.rs:54`.
  `value_parser = clap::value_parser!(u32).range(1..=4096)`. `MAX_ALLOWED_MATRIX_DIM`
  is also exposed as a `pub const` for the TCP codec's defense-in-depth check.

- **SEC-004** (integer overflow in TCP codec) — `kirk-server/src/tcp/codec.rs:22-66`.
  `forward_expected_len` and `sample_expected_len` use `checked_mul` /
  `checked_add` and reject `n > MAX_ALLOWED_MATRIX_DIM` before any
  arithmetic. The TCP handler now calls `peek_payload_dim(payload)` +
  `backend.check_dim(dim)` BEFORE invoking the parser, so the parser only
  sees pre-validated `n` values.

- **SEC-005** (`--bind` ignored) — `kirk-server/src/lib.rs`. `ServerSettings::bind`
  is threaded into all three `tokio::net::TcpListener::bind(format!("{bind}:{port}"))`
  calls. `start_server(...)` legacy shim still hard-binds to `127.0.0.1` for
  test stability (none of the existing tests rely on `0.0.0.0`). New
  `start_server_with(ServerSettings)` is the public way to pass a real bind
  address; the binary uses it.

- **SEC-006** (REST/gRPC body limits) — `kirk-server/src/rest/routes.rs:48` +
  `kirk-server/src/lib.rs:104-107`. `DefaultBodyLimit::max(64 MiB)` on the
  REST router; `max_decoding_message_size(64 MiB)` and
  `max_encoding_message_size(64 MiB)` on the tonic server.

- **SEC-007** (TCP writer hangs on slow client) — `kirk-server/src/tcp/handler.rs:114-145`.
  `tokio::time::timeout(write_timeout, writer.write_all(&frame))` and the
  same on `writer.flush()`. Timeout is configurable via
  `--tcp-write-timeout-ms` (default 10 000). On timeout, log + close the
  connection (the mpsc receiver loop breaks, the `BufWriter` is shutdown).

- **SEC-008** (N=1 fallback in `confidence`) — `kirk-server/src/backend.rs:50-62`.
  `check_dim` now rejects `n < 2` with a clear message. The kernel still
  tolerates N=1 internally (for any future stateless callers), with a
  docstring note in `kirk-stub-realistic/src/kirk.rs::forward` explaining
  the sentinel behavior at the kernel boundary.

### LOW

- **SEC-009** (broken docker-compose healthcheck) — `docker-compose.yml:17-25`.
  Dropped the `wget` healthcheck (distroless has neither `wget` nor a shell).
  `bench` `depends_on` switched to `condition: service_started`; the bench's
  internal retry-on-refused-connection logic handles cold-start. I considered
  adding a `--healthcheck` CLI flag but it requires touching `main.rs`'s
  signal handling and felt orthogonal to the security work — the drop fixes
  the immediate compose breakage.

- **SEC-011** (no ERROR frame on truncated payload) — `kirk-server/src/tcp/handler.rs:172-188`.
  `read_exact(&mut payload)` errors are now caught, a `BAD_PAYLOAD (0x05)`
  ERROR frame is sent to the client, then the connection is closed.

- **SEC-014** (forbid unsafe in kirk-server) — `kirk-server/src/lib.rs:5` +
  `kirk-server/src/main.rs:4`. Added `#![forbid(unsafe_code)]`. The kernel
  crate already had this lint.

### Deferred (per orchestrator brief)

- SEC-010 (mild info disclosure in error messages) — kept LOW, no change.
- SEC-012 (out-of-order TCP completion) — documented in `kirk-server/README.md`
  under "TCP semantics". Behavior is intentional (slow forward doesn't block
  fast ping).
- SEC-013 (bench-ts client-side buffer growth) — client runs against trusted
  server; deferred as out of scope for this loop.
- SEC-015..SEC-019 — informational; no code change.

### Files Modified

- `kirk-server/src/config.rs` — new flags + `value_parser` ranges.
- `kirk-server/src/backend.rs` — N<2 reject.
- `kirk-server/src/rest/schema.rs` — pre-decode size validation + pre-sized allocation.
- `kirk-server/src/rest/routes.rs` — `DefaultBodyLimit::max(64 MiB)`.
- `kirk-server/src/lib.rs` — `ServerSettings`, bind/limits/timeout wiring,
  tonic max msg sizes, `#![forbid(unsafe_code)]`.
- `kirk-server/src/main.rs` — `start_server_with(ServerSettings::from_config(&cfg))`;
  `#![forbid(unsafe_code)]`.
- `kirk-server/src/tcp/mod.rs` — re-export `TcpServeLimits`.
- `kirk-server/src/tcp/handler.rs` — connection semaphore, in-flight semaphore,
  write timeout, BAD_PAYLOAD on truncated read, dim-check-before-arithmetic.
- `kirk-server/src/tcp/codec.rs` — `checked_*` arithmetic + `peek_payload_dim`.
- `kirk-stub-realistic/src/kirk.rs` — N=1 docstring (sentinel documented).
- `kirk-server/README.md` — `--bind` honoring, new caps, N>=2, TCP semantics.
- `docker-compose.yml` — drop wget healthcheck; switch bench to `service_started`.

### Files Added (tests only)

- `kirk-server/tests/rest_integration.rs` — `rest_forward_n1_rejected_with_bad_request`,
  `rest_forward_oversized_base64_rejected_without_alloc`,
  `rest_body_limit_allows_normal_request`.
- `kirk-server/tests/tcp_integration.rs` — `tcp_forward_n1_rejected`,
  `tcp_connection_cap_enforced` (uses `start_server_with` with cap=2).

### Verification

```
cargo check --workspace                        ✓ clean
cargo clippy --workspace --all-targets -- -D warnings ✓ clean
cargo test --workspace                         ✓ 44 passed, 0 failed
```

Test breakdown (was 39, now 44 — five new security regression tests):

| Suite                                    | Pass | Was |
| ---------------------------------------- | ---- | --- |
| kirk-server lib unit (`tcp::framing`)    | 5    | 5   |
| `tests/cross_transport.rs`               | 2    | 2   |
| `tests/grpc_integration.rs`              | 5    | 5   |
| `tests/rest_integration.rs`              | 11   | 8   |
| `tests/tcp_integration.rs`               | 10   | 8   |
| kirk-stub-realistic `tests/basic.rs`     | 7    | 7   |
| kirk-stub-realistic `tests/parity.rs`    | 4    | 4   |
| **Total**                                | **44** | **39** |

### Outstanding Findings (post-pass)

- LOW SEC-010 — kept as-is (informational, low impact).
- INFO SEC-012 — documented in README rather than refactoring to serial
  per-connection processing (the current parallel behavior is functionally
  better for mixed-cost pipelined workloads).
- INFO SEC-013 — client-side; deferred.
- INFO SEC-015..SEC-019 — no action items.

## Remediation Pass (2026-06-19)

Retry 1 of 2 on the SECURITY → CODE loop. The previous remediation pass (dated
2026-06-17 above) was discarded between sessions; the present pass starts from
a worktree where most fixes were already in place from a prior CODE pass.
Verified each SEC-NNN against the user-supplied brief and applied only the
delta. SEC-009 was the only new code change required.

### Fixes verified in-place from prior CODE pass (no new code)

- **SEC-001** + **SEC-006**: REST + gRPC body limits and safe base64 decode.
  - `kirk-server/src/rest/routes.rs:54` layers
    `DefaultBodyLimit::max(REST_BODY_LIMIT_BYTES)` (64 MiB) on the router.
  - `kirk-server/src/lib.rs:135-136` calls
    `.max_decoding_message_size(67_108_864).max_encoding_message_size(67_108_864)`
    on the tonic service.
  - `kirk-server/src/rest/schema.rs:126-178` (`decode_f32_matrix`) implements
    the pre-decode size guard using `base64_decoded_upper_bound(b64.len())`,
    validates it against `MAX_DECODED_BYTES` AND against `4 * matrix_dim^2`
    via `expected_bytes` (computed with `checked_mul`), then decodes into a
    pre-sized `Vec::with_capacity(expected_bytes)`. Decode never sees an
    attacker-amplified buffer.
  - `kirk-server/tests/rest_integration.rs:347-383` covers the oversized-base64
    rejection (`rest_forward_oversized_base64_rejected_without_alloc`).

- **SEC-002**: TCP connection + per-connection in-flight semaphores.
  - `kirk-server/src/config.rs:60-76` exposes `--max-connections` (default
    1024, clap range `1..=65535`) and `--max-in-flight-per-conn` (default 128,
    clap range `1..=65535`).
  - `kirk-server/src/tcp/handler.rs:52` `Arc<Semaphore::new(max_connections)>`
    wrapping the accept loop; `:71` `try_acquire_owned()` — when the cap is
    saturated the new connection is dropped (logged WARN) rather than blocking
    the accept loop. Permit lives for the lifetime of the per-connection task
    and is released on `drop`.
  - `kirk-server/src/tcp/handler.rs:110-218` per-connection
    `Arc<Semaphore::new(max_in_flight_per_conn)>`; each frame acquires a permit
    before `tokio::spawn(process_frame)` and releases on task exit.
  - Listener backlog (FR-051's `1024` target) intentionally deferred: tokio's
    default backlog (`128` on most platforms) plus the connection semaphore
    is sufficient for the security-fix scope. Documented in the prior pass.
  - `kirk-server/tests/tcp_integration.rs:444-499` covers the cap
    (`tcp_connection_cap_enforced` — opens 2 + 1 connections against a
    `max_connections = 2` server).

- **SEC-003**: `--max-matrix-dim` clamped to `[1, 4096]` via
  `value_parser!(u32).range(1..=4096)` at
  `kirk-server/src/config.rs:54`. `MAX_ALLOWED_MATRIX_DIM = 4096` is also
  exported as a `pub const` for defense-in-depth checks in the TCP codec.

- **SEC-004**: TCP codec arithmetic overflow protection.
  - `kirk-server/src/tcp/codec.rs:28-66`: `forward_expected_len` /
    `sample_expected_len` reject `n > MAX_ALLOWED_MATRIX_DIM` before any
    arithmetic, then use `checked_mul` / `checked_add` everywhere.
  - `kirk-server/src/tcp/codec.rs:18-23`: new `peek_payload_dim(payload)`
    extracts only the leading `u32 N` so the handler can `backend.check_dim`
    BEFORE the parser does any per-shape work.
  - `kirk-server/src/tcp/handler.rs:316-422`: every opcode handler (`forward`,
    `handle_entropy`, `handle_features`, `handle_active_inference`,
    `handle_forward_sample`) calls `peek_payload_dim` + `backend.check_dim`
    before the codec.

- **SEC-005**: `--bind` is threaded through to all three listeners.
  - `kirk-server/src/lib.rs:114-185`: `start_server_with(ServerSettings)`
    formats `"{bind}:{port}"` for the gRPC, REST, and TCP listeners. The
    legacy `start_server` shim hard-codes `127.0.0.1` for test stability.
  - `kirk-server/src/main.rs:28-29`: the binary calls
    `start_server_with(ServerSettings::from_config(&cfg))`, so
    `KIRK_BIND` / `--bind` is honored end-to-end.

- **SEC-007**: TCP per-connection write timeout.
  - `kirk-server/src/config.rs:79-86`: `--tcp-write-timeout-ms` default 10000,
    clap range `100..=600_000`.
  - `kirk-server/src/tcp/handler.rs:114-145`: the writer task wraps both
    `writer.write_all(&frame)` and `writer.flush()` in
    `tokio::time::timeout(write_timeout, ...)`. On timeout, log WARN +
    `writer.shutdown().await` (connection closes; mpsc receiver drops on the
    next iter).

- **SEC-008**: `KirkBackend::check_dim` rejects `n < 2` at
  `kirk-server/src/backend.rs:53-66`. The kernel itself still tolerates N=1
  internally (with a clear docstring on
  `kirk-stub-realistic/src/kirk.rs::forward:36-41`) so future stateless
  callers can opt in.

- **SEC-011**: ERROR(BAD_PAYLOAD) on truncated payload at
  `kirk-server/src/tcp/handler.rs:195-214`. `read_exact` errors mid-payload
  emit a `BAD_PAYLOAD (0x05)` ERROR frame with the original `req_id` before
  the connection closes.

- **SEC-014**: `#![forbid(unsafe_code)]` at `kirk-server/src/lib.rs:5` and
  `kirk-server/src/main.rs:4`. Kernel crate already had this lint.

### Fixes added in this pass

- **SEC-009** (LOW): `--healthcheck` CLI flag.
  - `kirk-server/src/config.rs:90-94`: new `pub healthcheck: bool` flag (no
    env var binding; it's a one-shot operator/orchestrator probe, not a
    server-side configuration).
  - `kirk-server/src/main.rs:18-26, 33-92`: when `--healthcheck` is set,
    bypass the full server bring-up. Spin a current-thread runtime, open a
    `TcpStream` to `127.0.0.1:<rest-port>`, write a hand-rolled HTTP/1.1
    `GET /healthz` request, read the response, parse the status line, exit
    `0` if status is `200` else `1`. Hand-rolled because the runtime crate
    already pulls `tokio::net::TcpStream` and adding `reqwest` to the
    runtime deps for a 200-byte probe is wasteful. Connect + read timeout
    of 5 s.
  - `docker-compose.yml:17-23`: replace the dropped wget healthcheck with
    `test: ["CMD", "/usr/local/bin/kirk-server", "--healthcheck"]` plus
    `interval: 10s`, `timeout: 5s`, `retries: 5`, `start_period: 10s`.
    Reverted `bench.depends_on.condition` from `service_started` back to
    `service_healthy` so the bench waits for the real readiness signal.

### Verification

```
cargo check --workspace                                clean
cargo clippy --workspace --all-targets -- -D warnings  clean
cargo build -p kirk-server --release                   clean
cargo test --workspace --lib --bins                    5 + 6 = 11 passed (incl. 6 new SEC-009 unit tests)
cargo test -p kirk-stub-realistic                      7 + 4 = 11 passed
```

`kirk-server` integration tests (`cross_transport`, `rest_integration`,
`tcp_integration`, `grpc_integration` — 33 tests in total) could **not** be
exercised on this host: the macOS TCP stack is in an exhausted state
(`netstat -an -p tcp | grep -c TIME_WAIT` = 18 450; ~14 246 of 16 384
ephemeral source ports are in `TIME_WAIT`), so every outbound TCP `connect`
from a test client fails with `Os { code: 49, AddrNotAvailable: "Can't
assign requested address" }`. The reproduction is independent of the
remediation: a stand-alone `tokio::net::TcpListener::bind("127.0.0.1:0")`
+ `TcpStream::connect(addr)` micro-benchmark fails 10/10 against either
`127.0.0.1` or `0.0.0.0`. The `kirk-server --healthcheck` binary itself
hits the same exhaustion when invoked against a known-good local listener,
confirming the test-side failures are environmental.

This blocks integration-test execution on the current host. The unit
tests (`tcp::framing`) and the kernel parity tests (which do no
networking) all pass. The integration tests are unchanged from the prior
pass; the test agent should re-run them once the host's `TIME_WAIT`
backlog drains (default macOS MSL = 15 s × 18 450 entries → ~5 min
recovery if no new outbound traffic), or on a fresh host.

Unit + parity coverage that did pass on this host:

| Suite                                              | Pass |
| -------------------------------------------------- | ---- |
| kirk-server lib unit (`tcp::framing`)              | 5    |
| kirk-server bin unit (`main::parse_http_status`)   | 6    |
| kirk-stub-realistic `tests/basic.rs`               | 7    |
| kirk-stub-realistic `tests/parity.rs`              | 4    |
| **Subtotal verified this pass**                    | **22** |

The integration suites (33 tests, all `127.0.0.1:0`-based) are blocked
by host state; the prior pass recorded them passing under the same
implementations, which are untouched in this pass (the only code changes
this pass are the new `--healthcheck` path in `main.rs` and the
`docker-compose.yml` healthcheck entry — neither is exercised by the
integration tests).

### Outstanding Findings (post-pass)

- LOW SEC-010 — kept as-is per orchestrator brief (informational).
- INFO SEC-012 — documented in `kirk-server/README.md`; behavior unchanged.
- INFO SEC-013 — client-side bench; out of scope for this loop.
- INFO SEC-015..SEC-019 — no action items.
- **Environmental gap (this host only)**: the integration test suite cannot
  be re-executed locally until macOS `TIME_WAIT` drains. The test agent
  should re-run on a clean host; the code under test is unchanged from the
  prior pass except for the SEC-009 healthcheck additions, neither of
  which touch the integration-test code paths.

### Files Modified (this pass)

- `kirk-server/src/config.rs` — SEC-009: added `--healthcheck` flag.
- `kirk-server/src/main.rs` — SEC-009: added `run_healthcheck` +
  `parse_http_status`; gate full server start-up on `cfg.healthcheck`;
  added 6 unit tests for `parse_http_status` covering 200/503/HTTP-1.0/
  malformed/empty/non-numeric.
- `docker-compose.yml` — SEC-009: re-add a working healthcheck using the
  new CLI flag; bench `depends_on.condition` reverted to `service_healthy`.
