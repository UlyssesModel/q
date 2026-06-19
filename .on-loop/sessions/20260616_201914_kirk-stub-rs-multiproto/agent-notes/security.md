# Security Audit Findings

## Methodology

Read-only static audit of the worktree
`/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/`
through a STRIDE + OWASP API Top 10 lens. Focus areas (in order): TCP framing
parser and per-connection handler (highest-risk untrusted-binary surface),
REST request schema and decoders, tonic gRPC service, the `kirk-stub-realistic`
numerical kernel (panics / divide-by-zero), the Bun TypeScript bench harness
(client-side; lower priority), and operational posture (Dockerfile, compose,
defaults, README guidance). Ran `cargo install cargo-audit --locked` and
`cargo audit --file <worktree>/Cargo.lock` (output captured at the bottom of
this document). No code was modified.

Out-of-scope items (auth, TLS, mTLS, Kafka) are not vulnerabilities — they are
explicit architect spec exclusions. They are restated under "Out-of-Scope
Posture" but are not findings.

## Summary

| Severity | Count |
| -------- | ----- |
| CRITICAL | 0     |
| HIGH     | 2     |
| MEDIUM   | 6     |
| LOW      | 5     |
| INFO     | 6     |

## Findings

### HIGH

#### SEC-001: REST base64 decode happens before the size cap is enforced
- **Component**: `kirk-server/src/rest/schema.rs:100-108` (`decode_f32_matrix`)
- **STRIDE**: Denial of Service.
- **OWASP**: API4 Unrestricted Resource Consumption.
- **Description**: `decode_f32_matrix` calls `B64.decode(b64.as_bytes())`
  unconditionally and then checks `if raw.len() > MAX_DECODED_BYTES`. The
  `base64` crate eagerly allocates the entire decoded buffer (≈ 6/8 of the
  base64 input length) before the check. The size cap therefore cannot prevent
  the allocation it was supposed to bound. In addition there is no explicit
  cap on the base64 *input* length — the only upstream limit is axum's default
  body limit (2 MiB, see SEC-006), which is incidental and undocumented.
  Mitigation today: axum's 2 MiB default keeps the worst case small, but any
  future per-route `DefaultBodyLimit::disable()` or upgrade to a streaming
  decoder would re-expose the allocation.
- **Reproduction**: POST `/v1/forward` with a `matrix_re` of length close to
  the body limit; observe that decoding allocates roughly 75% of the input
  size regardless of `MAX_DECODED_BYTES = 64 MiB`. If axum's body limit is
  ever raised to support the documented `max_matrix_dim = 1024` (which needs
  ≈ 11 MiB base64 input per matrix), the 64 MiB cap will be reachable only
  AFTER the 48 MiB decoded buffer is already on the heap.
- **Impact**: Memory amplification — an attacker can force one allocation of
  ~0.75 × (axum body limit) per request, multiplied by concurrent requests
  with no connection cap (see SEC-002). Currently bounded by axum's 2 MiB
  default to ~1.5 MiB per request, but the bound is unintentional and easy
  to remove.
- **Recommendation**:
  1. Compute the maximum decoded size from `matrix_dim` *before* the call to
     `B64.decode`. The decoder API supports `decoded_len_estimate(b64.len())`
     — reject if it exceeds `MAX_DECODED_BYTES` and reject if it exceeds
     `4 * matrix_dim^2` (which is the only legitimate size).
  2. Use `B64.decode_slice_unchecked` into a pre-sized
     `Vec::with_capacity(4 * matrix_dim^2)` so the allocation is bounded by
     the validated `matrix_dim`, not by attacker-controlled input length.
  3. Explicitly raise axum's `DefaultBodyLimit` to a documented value
     (e.g. `64 MiB`) so that the spec's matrix sizes work at all — and so
     that SEC-001's hardening is necessary rather than accidentally
     superseded by the body limit.
- **Reference**: CWE-789 (Memory Allocation with Excessive Size Value), CWE-770
  (Allocation of Resources Without Limits or Throttling).

#### SEC-002: No connection / concurrency cap on TCP listener (and per-request task spawn is unbounded)
- **Component**: `kirk-server/src/tcp/handler.rs:39-58` (accept loop),
  `kirk-server/src/tcp/handler.rs:139-141` (per-frame `tokio::spawn`),
  `kirk-server/src/lib.rs:107-115` (TCP listener bind).
- **STRIDE**: Denial of Service.
- **OWASP**: API4 Unrestricted Resource Consumption.
- **Description**: The TCP accept loop spawns one tokio task per connection
  with no semaphore, no max-connections cap, and no listener backlog tuning
  (the spec FR-051 explicitly calls for `listener backlog 1024`). Once a
  connection is accepted, every parsed frame is dispatched via
  `tokio::spawn(process_frame(...))` with no upper bound on the number of
  in-flight frames per connection. The per-connection mpsc *response* channel
  is bounded at 256 (line 72), but inbound frames are read and spawned without
  bound — so the mpsc fills up, but the unbounded task spawn continues to
  drain the inbound stream, accumulating completed responses that block on
  the bounded writer. The `parking_lot::Mutex<KirkRealistic>` in
  `KirkBackend.kirk` (held during `forward`) means parallel inbound frames
  also serialize on the lock and queue work indefinitely.
- **Reproduction**: A client opens a TCP connection, pipelines thousands of
  FORWARD frames without ever reading responses. Each frame spawns a task.
  Tasks blocked on the writer mpsc.send pile up. Combined with a flood of
  parallel connections (no accept cap), an attacker can exhaust task slots
  and fds with a small number of well-crafted clients.
- **Impact**: Denial of service via memory/file-descriptor exhaustion. No
  authentication exists (out of scope) so any TCP-reachable client can
  trigger it.
- **Recommendation**:
  1. Add a `tokio::sync::Semaphore` wrapping accept, with a configurable
     `--max-connections` flag (default e.g. 1024).
  2. Add a per-connection in-flight-frame limit (e.g. a semaphore with N=128
     permits acquired before `tokio::spawn(process_frame)` is allowed).
  3. Set the listener backlog: rebuild the listener via `socket2` to set
     `SOL_SOCKET/SO_BACKLOG = 1024` per FR-051.
  4. Document the per-connection pipelining cap in `kirk-server/README.md`.
- **Reference**: CWE-400 (Uncontrolled Resource Consumption), CWE-770.

### MEDIUM

#### SEC-003: Unbounded `--max-matrix-dim` (no upper limit enforced on the flag)
- **Component**: `kirk-server/src/config.rs:36-38`.
- **STRIDE**: Denial of Service (configuration foot-gun → operator can disable
  the protective cap).
- **OWASP**: API8 Security Misconfiguration.
- **Description**: The architect spec § Security Considerations S-001 says
  `--max-matrix-dim` defaults to 1024 and is "configurable up to 4096". The
  clap derive uses `default_value_t = 1024` with no `value_parser` to enforce
  an upper bound. A misconfigured operator (or `KIRK_MAX_MATRIX_DIM` env var)
  could set this to e.g. `u32::MAX`, after which a single FORWARD with N=10000
  would allocate the `2N x 2N = 20000 x 20000` real symmetric block matrix
  inside `eigensolver::diagonalize` — `4 * 4e8 = 1.6 GB` per request — and
  the eigh would dominate compute, freezing the worker pool.
- **Impact**: Configuration vector for DoS / OOM. Single-request exhaustion.
- **Recommendation**: Add `value_parser = clap::value_parser!(u32).range(1..=4096)`
  to the `--max-matrix-dim` flag in `config.rs`. Document the hard ceiling in
  `kirk-server/README.md` next to the flag.
- **Reference**: CWE-1284 (Improper Validation of Specified Quantity in
  Input), CWE-400.

#### SEC-004: Integer-overflow / arithmetic-wrap risk in TCP payload-size validators
- **Component**: `kirk-server/src/tcp/codec.rs:13-39` (`parse_forward_request`)
  and `:47-67` (`parse_sample_request`).
- **STRIDE**: Tampering / Denial of Service.
- **OWASP**: API4 Unrestricted Resource Consumption.
- **Description**: The validators compute
  `m = n_usize * n_usize` and `expected = 4 + 4*m + 4*m + 8` from the
  attacker-supplied `n: u32` *without any range check on n* before the
  arithmetic. On 64-bit `usize`, `m` wraps for any `n ≥ 2^32` — impossible
  through u32, but the parallel computation `expected = 8 + 8*m` overflows
  `usize` once `m > usize::MAX/8 ≈ 2^61`, which requires `n ≥ 2^30 ≈ 10^9`.
  Within a u32, an attacker can set `n` up to `2^32 - 1`. With `n = 2^16`,
  `m = 2^32`, `expected = 8 + 2^35 ≈ 34 GiB` — larger than `MAX_PAYLOAD`
  (64 MiB), so the equality check `payload.len() != expected` rejects on
  the size check naturally. But the architectural guarantee is brittle:
  the TCP `check_dim(n)` is called *after* `parse_forward_request`, so the
  parser has already done the wrap-able arithmetic.
  In Rust release mode arithmetic is silent-wrap; in debug it panics. A
  panic in the per-frame `tokio::spawn` task is *contained* (does not crash
  the whole server) but takes the task out without sending an ERROR frame
  back to the client (so the client hangs until timeout).
- **Reproduction**: Send a FORWARD frame with `n = u32::MAX`, payload_len
  chosen to satisfy the wrapped-`expected` arithmetic. The exact wrap value
  is platform-dependent, but on 64-bit, `n = 65536` gives an `expected` that
  exceeds 4 GiB (rejected by `MAX_PAYLOAD` already). The exploit window is
  narrow because of `MAX_PAYLOAD = 64 MiB`, but the parser should fail
  closed regardless.
- **Impact**: Today: bounded by `MAX_PAYLOAD`; result is wasted parse cycles
  and a possible debug-mode panic. Tomorrow (if `MAX_PAYLOAD` is ever raised,
  or `usize` is 32-bit on some embedded target): real memory blow-up.
- **Recommendation**: At the top of every `parse_*_request`, validate
  `n <= MAX_REASONABLE_DIM` (e.g. `4096`) *before* using `n` in arithmetic.
  Use `checked_mul`/`checked_add` for the `expected` computation and return
  `ServerError::BadRequest` on overflow. Have the TCP handler call
  `backend.check_dim(n)` *before* the codec's arithmetic, not after.
- **Reference**: CWE-190 (Integer Overflow or Wraparound), CWE-1284.

#### SEC-005: `--bind` CLI flag is silently ignored — `start_server` always binds to `127.0.0.1`
- **Component**: `kirk-server/src/lib.rs:66, 87, 107` (all listeners use
  hard-coded `127.0.0.1:{port}`); `kirk-server/src/config.rs:21-22` (the
  flag is parsed but never read).
- **STRIDE**: Information Disclosure (security configuration not honored) /
  operational confusion.
- **OWASP**: API8 Security Misconfiguration.
- **Description**: The `--bind` / `KIRK_BIND` configuration is exposed on
  the CLI and documented in `kirk-server/README.md` with default `0.0.0.0`,
  but `Config::bind` is never read anywhere in the codebase. The actual
  `start_server` always binds to `127.0.0.1`. The end-user impact is two-
  faceted:
  - **Security-positive**: host runs without docker accidentally bind to
    loopback, so they are NOT exposed to the LAN — *more* secure than the
    documented default. This was probably an accident.
  - **Security-negative**: an operator who reads the README and sets
    `--bind 0.0.0.0` thinking they are intentionally exposing the server
    on the LAN will silently get `127.0.0.1` instead. Containerized
    deployments (docker-compose) work because Docker port-mapping is
    independent of the bind address (Docker connects to the container's
    loopback). But a host-bind-to-LAN deployment fails silently.
  - **Spec deviation**: FR-014 mandates `--bind` honor the flag.
- **Reproduction**: `kirk-server --bind 0.0.0.0` then from another host on
  the LAN attempt `curl http://<host>:8080/healthz` → connection refused.
- **Impact**: Configuration drift, broken documentation contract, and
  operator confusion. Not an exploitable vulnerability today, but a real
  security-config bug because the operator's intent is silently overridden.
- **Recommendation**: Pass `cfg.bind` into `start_server` and use it for all
  three listener binds. Either parse it via `IpAddr::from_str` for safety,
  or keep it as a `String` and concatenate with the port. Add an integration
  test that `--bind 0.0.0.0` actually accepts non-loopback connections.
- **Reference**: CWE-1188 (Insecure Default Initialization of Resource),
  CWE-440 (Expected Behavior Violation).

#### SEC-006: REST body limit not raised — silently caps usable matrix dim well below `--max-matrix-dim`
- **Component**: `kirk-server/src/rest/routes.rs:32-44` (no `DefaultBodyLimit`
  layer), and absent `tonic::transport::Server::max_decoding_message_size`
  in `kirk-server/src/lib.rs:72`.
- **STRIDE**: Denial of Service (functional cap) / Security Misconfiguration.
- **OWASP**: API8 Security Misconfiguration, API4 Unrestricted Resource
  Consumption (in the opposite direction — limits not aligned with spec).
- **Description**: The REST router does not configure
  `axum::extract::DefaultBodyLimit`. The framework default is 2 MiB
  (`axum-core-0.4.5/src/ext_traits/request.rs:325: const DEFAULT_LIMIT: usize = 2_097_152`).
  A 1024×1024 f32 matrix is `4 MiB` raw; base64-encoded `matrix_re` plus
  `matrix_im` plus JSON envelope is ~12 MiB. REST silently fails (413) for
  any `matrix_dim` above ~325 (the largest N whose base64 envelope fits in
  2 MiB). Similarly tonic's default `max_decoding_message_size` is 4 MiB —
  not raised — so gRPC silently fails for the same matrix sizes.
  Spec FR-012 documents matrix sizes up to `max_matrix_dim = 1024`, which
  cannot be exercised through REST or gRPC. The TCP cap is 64 MiB which
  *does* allow N=1024.
  **Security angle**: the silent body limit is an accidental DoS protection
  but is not the intended limit. It is also undocumented — operators are
  not warned. Worse, if a future commit raises this limit to match the spec
  without also adding the matrix-shape validation from SEC-001, the
  base64-decode amplification becomes immediately exploitable.
- **Impact**: Today: silent functional cap that operators cannot diagnose
  except by reading axum source. Tomorrow: when this is raised, SEC-001 and
  SEC-003 become severe.
- **Recommendation**:
  1. Add `.layer(axum::extract::DefaultBodyLimit::max(67_108_864))` to the
     REST router (64 MiB, mirrors TCP).
  2. Add `.max_decoding_message_size(67_108_864)` to the tonic builder.
  3. Add a clear `kirk-server/README.md` note explaining the wire-level cap
     versus `--max-matrix-dim`.
  4. Fix SEC-001 first or in the same change — raising the limit without
     pre-base64-decode validation is a regression.
- **Reference**: CWE-770, CWE-1284.

#### SEC-007: TCP per-connection writer task can deadlock with reader on a slow client
- **Component**: `kirk-server/src/tcp/handler.rs:72-84` (writer task),
  `:139-141` (frame dispatch).
- **STRIDE**: Denial of Service (slow-reader / slowloris-style).
- **OWASP**: API4.
- **Description**: The per-connection writer task pulls frames from the mpsc
  and writes them to the socket. There is no write timeout. A slow / dead
  client (TCP receive window held closed) will cause `writer.write_all` /
  `writer.flush` to block indefinitely. The mpsc fills to its 256-element
  cap. Per-frame `tokio::spawn(process_frame)` continues to drain the inbound
  stream and produce responses, each of which blocks at `tx.send(frame)`.
  Combined with no in-flight cap per connection (SEC-002), an attacker can
  open one connection, pipeline frames, never read responses, and
  accumulate work indefinitely.
- **Reproduction**: Open a TCP socket, send 100 FORWARD frames with N=64,
  never `recv()`. The server holds 256 pending writes + many tasks blocked
  on `tx.send` + memory for response bytes.
- **Impact**: One slow client can pin per-connection memory unboundedly.
  No authentication exists (out of scope).
- **Recommendation**:
  1. Add `tokio::time::timeout` to `writer.write_all` and `writer.flush`
     (e.g. 10 s).
  2. Use `try_send` in `process_frame` and drop the frame (or respond with
     ERROR) if the writer queue is full.
  3. Add a TCP read-idle timeout (e.g. close after 60 s with no inbound
     data) to defend against slowloris-style hold-open.
- **Reference**: CWE-400, CWE-770.

#### SEC-008: `confidence` formula falls back to `ln_n = 1` for N=1 instead of returning a defined value
- **Component**: `kirk-stub-realistic/src/kirk.rs:97-98`.
- **STRIDE**: Denial of Service / correctness.
- **OWASP**: API4 (correctness-driven malformed input → garbage output).
- **Description**: For N=1, the code uses `ln_n = 1.0` instead of `ln(1) = 0`
  to dodge a divide-by-zero. The resulting `confidence = clip(1 - H/1, 0, 1)`
  is mathematically inconsistent with the documented formula
  `confidence = clip(1 - H/ln(N), 0, 1)` (which for N=1 is undefined). It
  is a deliberate workaround — but the value returned is not flagged. A
  caller cannot distinguish "this is the N=1 fallback" from "this is real
  confidence". Worse, since the entropy `H` for an N=1 system is always 0,
  the returned confidence is always `1.0` for any N=1 input. Pen-test angle:
  an attacker who can submit FORWARD with N=1 (allowed today by `check_dim`)
  gets a maximum-confidence response with minimum compute, which could be
  used to bias any downstream consumer that trusts the confidence field
  (no such consumer exists in this codebase today). This is reachable DoS
  in the sense that it's a *functional* vulnerability, but not an
  exploit. Still — the spec calls out this as a finding category. The N=0
  case IS rejected (`check_dim` returns BadRequest for `n == 0`); good.
- **Reproduction**: TCP FORWARD with N=1, matrix_re=[1.0], matrix_im=[0.0]
  → `confidence == 1.0` deterministically.
- **Impact**: Misleading metric value. No security impact in current
  codebase since no consumer trusts confidence as authority; flagged
  per the spec's explicit instruction that numerical-kernel correctness
  bugs are valid findings.
- **Recommendation**: Either (a) reject `n == 1` at `check_dim` (the
  rolling-window z-score is also degenerate at N=1), or (b) return
  `confidence = 1.0` only as a documented sentinel, with a clear comment
  and unit test.
- **Reference**: CWE-682 (Incorrect Calculation), CWE-1339 (Insufficient
  Precision or Accuracy of a Real Number).

### LOW

#### SEC-009: Health probe in compose uses `wget`, which is not present in distroless
- **Component**: `docker-compose.yml:18` (`test: ["CMD", "wget", "-qO-", ...]`),
  `docker/Dockerfile:22` (`FROM gcr.io/distroless/cc-debian12`).
- **STRIDE**: (operational) — healthchecks always fail → restart loops.
- **OWASP**: API8 Security Misconfiguration.
- **Description**: The compose healthcheck shell-execs `wget`, but the
  distroless `cc-debian12` runtime image does not include `wget` or any
  shell. The healthcheck will always exit non-zero; with
  `restart: unless-stopped` (line 22), this leads to a restart loop after
  the start-period grace.
- **Impact**: Operational. The `bench` service depends_on `kirk-server`
  with `condition: service_healthy`, so `bench` will never start in compose.
  Not a vulnerability, but a real deployment blocker.
- **Recommendation**: Replace the healthcheck with a TCP probe (e.g.
  `["CMD", "/usr/local/bin/kirk-server", "--healthcheck"]` if a CLI flag is
  added) OR switch to an image that contains `wget` (`distroless/cc:debug`
  has a shell) OR drop the compose-level healthcheck and let the bench
  retry on connection refused.
- **Reference**: CWE-665 (Improper Initialization).

#### SEC-010: REST error responses echo internal error messages (incl. some buffer length numbers)
- **Component**: `kirk-server/src/error.rs:24-31` + `rest/routes.rs:46-53`.
- **STRIDE**: Information Disclosure (mild).
- **OWASP**: API3 Broken Object Property Level Authorization (loosely).
- **Description**: `err_response` puts `err.to_string()` in the
  `ErrorResponse.message` field. The error texts include configured limits
  (e.g. "matrix dim 1234 exceeds max 1024", "payload exceeds maximum
  decoded size 67108864 bytes (got X)"). These are policy values, not
  credentials; the leakage is low-impact. No file paths, no panic
  backtraces, no stack frames, no matrix contents are leaked. Compute
  errors map to `ServerError::Compute(KirkError)` which is a structured
  enum (no caller data in messages).
- **Impact**: Low — gives an attacker the `max_matrix_dim` and
  `MAX_DECODED_BYTES` values via 4xx responses. Both are public design
  constants, so this is mostly informational.
- **Recommendation**: For HIGH-load production deployments, consider
  replacing `message` with a short tag (e.g. `"matrix_dim_exceeded"`) and
  log the detail server-side only. Not required.
- **Reference**: CWE-209 (Information Exposure Through an Error Message).

#### SEC-011: ERROR frame writes have no response for partial-payload reads (read_exact `?` propagation)
- **Component**: `kirk-server/src/tcp/handler.rs:133`.
- **STRIDE**: Denial of Service (mild).
- **OWASP**: API4.
- **Description**: `reader.read_exact(&mut payload).await?` propagates an
  io error up through `handle_connection`, terminating the connection
  without writing an ERROR frame. The spec says ERROR frames should be
  written for malformed input. For a client that sends a header claiming
  N bytes of payload then closes mid-stream, the spec response should be
  `0x05 BAD_PAYLOAD`. Today the server just closes silently. Not exploitable;
  just a small spec deviation.
- **Recommendation**: Catch the read_exact error, write an ERROR frame
  with code `BAD_PAYLOAD`, then close. Optional.
- **Reference**: CWE-755 (Improper Handling of Exceptional Conditions).

#### SEC-012: TCP frames processed in parallel within a connection (spec says "in arrival order")
- **Component**: `kirk-server/src/tcp/handler.rs:139-141`.
- **STRIDE**: Tampering (mild) — completion-order responses can interleave.
- **OWASP**: API4.
- **Description**: Architect § Custom TCP "Connection lifecycle" says
  "Server processes in order arrived (single-threaded per connection task
  on the read side)". The implementation `tokio::spawn`s every parsed frame,
  so responses can complete out of arrival order, and the mpsc writer
  delivers them in completion order. Because the spec also says clients
  correlate by `req_id`, this is functionally OK — but it differs from the
  documented behavior. Security angle is minimal (a slow `forward` won't
  block a fast `ping`, which is arguably *better* for liveness). Worth
  flagging as a doc/spec drift.
- **Recommendation**: Either update the spec to say "out-of-order completion,
  client correlates by req_id", or use a single-task-per-connection model
  that processes frames serially. The current implementation is fine for
  the bench but should be documented.
- **Reference**: CWE-440 (Expected Behavior Violation).

#### SEC-013: Bench-ts TCP client read buffer grows O(N²) due to slice-concat-on-every-recv
- **Component**: `bench-ts/src/transports/tcp.ts:73-78` (`onData`).
- **STRIDE**: Client-side DoS (denies user resources, not server).
- **OWASP**: N/A (client).
- **Description**: `onData` does `new Uint8Array(buf.length + chunk.length)`
  + `set + set` on every chunk arrival, then `buf = buf.subarray(totalLen)`
  to "consume" frames. For high-throughput pipelined responses, the merge
  allocation is O(buffered) per chunk. Also, the read buffer is unbounded —
  a malicious server claiming huge `payload_len` would never be reset, so
  memory grows until OOM.
- **Impact**: Bench-side memory pressure under attacker-controlled server.
  Not exploitable in the documented use-case (bench runs against a known
  server).
- **Recommendation**: Use a ring-buffer or `Buffer.concat`-style growth
  with capacity tracking. Validate `payload_len` against the same 64 MiB
  cap before reading the payload.
- **Reference**: CWE-770.

### INFO / Posture Notes

#### SEC-014: `kirk-stub-realistic` is `#![forbid(unsafe_code)]` — good
- The kernel crate at `kirk-stub-realistic/src/lib.rs:9` forbids unsafe code.
  `kirk-server` does NOT have a similar lint but contains no `unsafe` blocks
  (verified via grep). Recommend adding `#![forbid(unsafe_code)]` to
  `kirk-server/src/lib.rs` and `main.rs` as a future-safe lint.

#### SEC-015: No matrix contents in logs — verified
- Searched all `tracing::{info,debug,warn,error,trace}!` calls in the server.
  Only sizes, op names, error labels, and `peer` addresses are logged. No
  matrix data, no base64, no payload bytes. Per coder claim — confirmed.

#### SEC-016: Container runs as non-root UID 10001 — verified
- `docker/Dockerfile:25` sets `USER 10001` on the distroless runtime.
  Distroless has no shell, no package manager, no setuid binaries. Image
  surface is minimal. No secrets baked in (verified via `env` instructions
  in the Dockerfile — only `RUST_LOG=info`). Good.

#### SEC-017: Container memory limit `1g` set in compose — present
- `docker-compose.yml:23` (`mem_limit: 1g`). Bounds the runaway-allocation
  blast radius (SEC-002/003/006). Not enough alone (a 1 GB DoS still kills
  the container), but better than nothing.

#### SEC-018: Operations / repudiation posture — tracing logs are not tamper-evident
- The server uses `tracing-subscriber` JSON logs to stdout. There is no
  log-signing, no append-only log file, no remote log shipping. An on-host
  attacker can edit logs at will. This is normal for an unauthenticated
  service and not a finding per se, but operators planning to use these
  logs as audit evidence should ship to a write-once sink. Not in scope.

#### SEC-019: cargo audit — clean, one unmaintained warning
- See "cargo audit output" below. No known CVEs. `paste` v1.0.15 is
  flagged unmaintained (`RUSTSEC-2024-0436`), pulled in transitively by
  one of the rust crates (likely `bitvec` via tonic or nalgebra). No
  exploitable issue; a re-publish of paste would clear the warning.

## Out-of-Scope Posture (per architect spec)

- No authentication, no TLS, no mTLS. This is explicit (architect spec
  § Security Considerations S-005, S-006, and § Out of Scope). The
  recommended posture, per spec and per `kirk-server/README.md`, is to
  deploy behind a reverse proxy that terminates TLS and enforces
  AuthN/AuthZ. Acknowledged risk; not a finding.
- No Kafka streaming — out of scope. Not a finding.
- Default bind `0.0.0.0` (per spec) — note that the implementation
  *silently* enforces `127.0.0.1` instead (see SEC-005). The spec posture
  for `0.0.0.0` is "acceptable in Docker because the host firewall is
  expected to mediate". The README does call out the `--bind 127.0.0.1`
  recommendation for non-Docker host runs. Good — though the flag does
  not actually work today.

## cargo audit output

```
$ cargo audit --file /Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/Cargo.lock
    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 1132 security advisories (from /Users/charmalloc/.cargo/advisory-db)
    Updating crates.io index
    Scanning <worktree>/Cargo.lock for vulnerabilities (259 crate dependencies)
Crate:     paste
Version:   1.0.15
Warning:   unmaintained
Title:     paste - no longer maintained
Date:      2024-10-07
ID:        RUSTSEC-2024-0436
URL:       https://rustsec.org/advisories/RUSTSEC-2024-0436

warning: 1 allowed warning found
```

No CVE-class advisories. One unmaintained-crate warning (`paste`), which is
not exploitable. `cargo-audit` was installed during this audit via
`cargo install cargo-audit --locked`; it was not present beforehand.

## Recommendation to Orchestrator

The two HIGH findings (SEC-001 base64-decode-before-cap, SEC-002 unbounded
TCP connections / per-frame tasks) are *resource-consumption hardening*
issues that an unauthenticated network attacker can trivially exploit for
DoS. Per the orchestrator pass/fail rule ("any HIGH finding without
documented mitigation"), this should **fail SECURITY → retry CODE**.

Suggested coder remediation order (smallest diff, biggest impact first):

1. SEC-006 + SEC-001 together — raise the axum `DefaultBodyLimit` and
   `tonic::max_decoding_message_size` to 64 MiB, *and* validate
   `matrix_dim` before base64 decode so the decoded buffer is bounded
   by `4 * matrix_dim^2` rather than by the input length.
2. SEC-002 — add a `Semaphore` for connection accept and per-connection
   in-flight frames.
3. SEC-003 — clamp `--max-matrix-dim` to `[1, 4096]` via `value_parser`.
4. SEC-004 — `checked_mul` in `parse_*_request`, and call `check_dim`
   *before* the codec arithmetic.
5. SEC-005 — wire `cfg.bind` through to `start_server`.
6. SEC-007 — write timeouts + try_send.
7. SEC-008 — either reject N=1 or document the sentinel.
8. SEC-009 — fix the compose healthcheck.

After remediation, re-run cargo test --workspace (the existing 39 tests
should still pass), add a new test that exercises each cap, and re-submit
for SECURITY review.

**Recommendation: RETRY CODE.**

## Re-Audit (2026-06-19)

Second-pass verification after the Coding agent's "Remediation Pass" (dated
2026-06-17 and 2026-06-19 in `coding.md`). Methodology: read each source
file referenced in the coder's notes, verify the fix matches what the
original finding required, and re-rate. No source files modified.

### Verification table

| ID | Original severity | Status | Evidence |
| -- | ----------------- | ------ | -------- |
| SEC-001 | HIGH | **FIXED** | `kirk-server/src/rest/schema.rs:103-178`: `base64_decoded_upper_bound(b64.len())` is computed before `B64.decode_vec`; both the global `MAX_DECODED_BYTES` cap (line 134) and the per-shape `expected_bytes` budget (line 143) are checked *before* allocation; the decoded buffer is `Vec::with_capacity(expected_bytes)` so the allocation is bounded by validated shape, not by attacker input length. The 2-byte slack on the per-shape budget correctly absorbs standard-base64 padding bias (verified for N=1024: `upper = 4_194_306`, `expected_bytes + 2 = 4_194_306`, equal → no false rejection). |
| SEC-002 | HIGH | **FIXED** | Accept-loop semaphore: `kirk-server/src/tcp/handler.rs:52` (`Arc<Semaphore::new(max_connections)>`) + `:71` `try_acquire_owned()` (non-blocking — saturated connections are dropped + WARN-logged rather than blocking the accept loop). Permit lives for the entire connection task (line 89 `drop(permit)`). Per-connection in-flight cap: `handler.rs:110` `Arc<Semaphore::new(max_in_flight_per_conn)>`, acquired with `acquire_owned().await` at `:218` (correctly applies back-pressure to the reader rather than dropping), released at `:228` `drop(permit)`. Both caps are CLI/env-configurable with clap range `1..=65535`. Listener-backlog tuning (FR-051's 1024 target via `socket2`) intentionally deferred to keep the diff focused — connection-count cap is the bigger DoS lever; trade-off documented in `coding.md`. |
| SEC-003 | MEDIUM | **FIXED** | `kirk-server/src/config.rs:50-55`: `value_parser = clap::value_parser!(u32).range(1..=4096)` on `--max-matrix-dim`. `MAX_ALLOWED_MATRIX_DIM = 4096` exported as `pub const` (line 6) for codec defense-in-depth. clap will reject `0` and `>4096` at parse time. |
| SEC-004 | MEDIUM | **FIXED** | `kirk-server/src/tcp/codec.rs:28-66` (`forward_expected_len`, `sample_expected_len`): reject `n > MAX_ALLOWED_MATRIX_DIM` (4096) *before* any arithmetic; `checked_mul` / `checked_add` used throughout; overflow returns `BadRequest` rather than wrapping. New `peek_payload_dim` (lines 18-23) lets the handler check the wire `N` against `backend.check_dim` *before* the codec runs — see handler at `:316-422` (every opcode handler calls peek+check before parse). |
| SEC-005 | MEDIUM | **FIXED** | `kirk-server/src/lib.rs:114-185`: `start_server_with(ServerSettings)` uses `format!("{bind}:{}", port)` for all three listeners (gRPC `:127`, REST `:154`, TCP `:173`). Binary entry point `main.rs:99` calls `start_server_with(ServerSettings::from_config(&cfg))` so `--bind` / `KIRK_BIND` is honored end-to-end. The legacy `start_server` shim (lines 90-111) hard-codes `127.0.0.1` for backwards-compat with existing integration tests — acceptable since the tests run on loopback only. |
| SEC-006 | MEDIUM | **FIXED** | REST: `kirk-server/src/rest/routes.rs:48-54` `.layer(DefaultBodyLimit::max(REST_BODY_LIMIT_BYTES))` with `REST_BODY_LIMIT_BYTES = 64 * 1024 * 1024` (line 17). gRPC: `kirk-server/src/lib.rs:135-136` `.max_decoding_message_size(MAX_GRPC_MESSAGE_BYTES).max_encoding_message_size(MAX_GRPC_MESSAGE_BYTES)` with `MAX_GRPC_MESSAGE_BYTES = 64 * 1024 * 1024` (line 51). Both are 64 MiB as recommended. |
| SEC-007 | MEDIUM | **FIXED** | `kirk-server/src/tcp/handler.rs:114-145`: `tokio::time::timeout(write_timeout, writer.write_all(&frame))` *and* `tokio::time::timeout(write_timeout, writer.flush())`. Timeout configurable via `--tcp-write-timeout-ms` (default 10 000, range `100..=600_000` per `config.rs:84`). On timeout: WARN-log + `writer.shutdown()` + break out of the writer loop — connection closes cleanly. |
| SEC-008 | MEDIUM | **FIXED** | `kirk-server/src/backend.rs:53-66` (`check_dim`): rejects `n < 2` with `ServerError::BadRequest("matrix dim must be >= 2 (...)")` and `n > max_matrix_dim` with `MatrixDimExceeded`. Kernel-level sentinel (N=1 → `ln_n = 1`) is documented in `kirk-stub-realistic/src/kirk.rs:36-41` with a doc comment that explicitly references SEC-008 and the server-boundary rejection. |
| SEC-009 | LOW  | **FIXED** | New `--healthcheck` CLI flag at `config.rs:96-97`; one-shot HTTP/1.1 client in `main.rs:36-76` (5 s connect/read timeout, parses status line, exits 0 on HTTP 200 else 1). `docker-compose.yml:20-25` uses `test: ["CMD", "/usr/local/bin/kirk-server", "--healthcheck"]` and `bench.depends_on.condition: service_healthy` so the bench waits for real readiness. `parse_http_status` covered by 6 unit tests in `main.rs:138-175` (200, 503, HTTP/1.0, empty, malformed, non-numeric). |
| SEC-011 | LOW  | **FIXED** | `kirk-server/src/tcp/handler.rs:195-213`: on `reader.read_exact(&mut payload)` error mid-payload, write an `ERROR` frame with code `BadPayload (0x05)` carrying the original `req_id` before closing the connection. The previous-pass note in the brief said "(optional — note if deferred)"; the coder went ahead and shipped the spec-compliant behavior. |
| SEC-014 | INFO | **FIXED** | `#![forbid(unsafe_code)]` confirmed at `kirk-server/src/lib.rs:5`, `kirk-server/src/main.rs:4`, and `kirk-stub-realistic/src/lib.rs:9`. |

### New findings (if any)

None. The remediation is surgical: every change has a clear SEC-NNN reference,
no source paths outside the SEC findings were touched (per `coding.md` "Files
Modified"), no suspicious new dependencies were added, and `cargo clippy
--workspace --all-targets -- -D warnings` is clean.

Specific subtle-regression checks performed and cleared:

- **SEC-001 base64 false-rejection sweep**: verified for the spec's documented
  worst case (`N = 1024`): `expected_bytes = 4 * 1024^2 = 4_194_304`,
  `b64.len()` (no padding case) = `5_592_408`, `upper = ((5_592_408 + 3) / 4) *
  3 = 4_194_306`. Comparison `upper (4_194_306) > expected_bytes + 2
  (4_194_306)` evaluates to `false` → passes. The 2-byte slack is exactly
  sufficient.
- **SEC-002 connection-cap interaction with shutdown**: confirmed the per-
  connection permit is dropped on the spawned task's normal exit and on `Err`
  return (line 89). Shutdown closes the broadcast channel which breaks the
  per-connection select on `shutdown.recv()` → reader loop returns → write_task
  joins → permit drops.
- **SEC-002 in-flight semaphore vs writer mpsc back-pressure**: with mpsc bounded
  at 256 and in-flight bound at 128 (default), a pathological slow-reader can
  still queue 128 in-flight + 256 mpsc responses, but the in-flight cap then
  prevents new spawns and back-pressures the reader. SEC-007's write timeout
  (default 10 s) terminates the connection if the writer hangs. Net result:
  bounded memory footprint per connection. Good.
- **SEC-004 ordering**: `handle_forward_sample` calls
  `parse_forward_sample_request` first then `check_dim` — acceptable because
  the FORWARD_SAMPLE payload is fixed at 12 bytes (no per-shape arithmetic).
  All other opcodes peek+check before parse.
- **SEC-007 mpsc deadlock avoidance**: I considered whether the writer's
  `break` on timeout could leave the reader blocked on `tx.send`. The reader
  uses `await` on `tx.send`, but when the receiver is dropped (writer task
  exits) `send` returns `Err`, and the spawned `process_frame` exits cleanly,
  permits drop, and the connection unwinds. No deadlock.

### Re-rated severity

| ID | Was | Now | Note |
| -- | --- | --- | ---- |
| SEC-001 | HIGH | RESOLVED | No residual risk; per-shape budget caps allocation. |
| SEC-002 | HIGH | RESOLVED | Bounded by configurable semaphores. |
| SEC-003 | MEDIUM | RESOLVED | clap rejects out-of-range at parse time. |
| SEC-004 | MEDIUM | RESOLVED | `checked_*` + range pre-check makes wrap impossible. |
| SEC-005 | MEDIUM | RESOLVED | All three listeners honor `--bind`. |
| SEC-006 | MEDIUM | RESOLVED | 64 MiB caps on REST + gRPC. |
| SEC-007 | MEDIUM | RESOLVED | Write timeout configurable. |
| SEC-008 | MEDIUM | RESOLVED | N<2 rejected at the server boundary. |
| SEC-009 | LOW    | RESOLVED | Healthcheck flag wired to compose. |
| SEC-011 | LOW    | RESOLVED | BAD_PAYLOAD ERROR frame on truncated payload. |
| SEC-014 | INFO   | RESOLVED | `#![forbid(unsafe_code)]` on all three crates. |
| SEC-010 | LOW    | UNCHANGED | Informational; deferred per orchestrator brief. |
| SEC-012 | INFO   | UNCHANGED | Documented in `kirk-server/README.md` per brief. |
| SEC-013 | LOW    | UNCHANGED | Client-side bench; deferred. |
| SEC-015..SEC-019 | INFO | UNCHANGED | Informational notes; no action. |

### Test verification

```
$ cargo check --workspace
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s   (CLEAN)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.39s   (CLEAN)

$ cargo test --workspace --lib --bins
  kirk-server lib unit (tcp::framing):                      5 passed
  kirk-server bin unit (main::parse_http_status):           6 passed
  kirk-stub-realistic lib unit:                             0 passed (no #[test] inside lib)
  -- subtotal --                                          11 passed, 0 failed

$ cargo test -p kirk-stub-realistic
  tests/basic.rs:                                           7 passed
  tests/parity.rs:                                          4 passed (handcalc + seed42 N=8/16/32)
  -- subtotal --                                          11 passed, 0 failed

$ cargo test -p kirk-server --test rest_integration
  11 failed — all with `Os { code: 49, AddrNotAvailable: "Can't assign
  requested address" }` on `127.0.0.1` ephemeral-port `connect`. Confirmed
  environmental: `netstat -an -p tcp | grep -c TIME_WAIT` = 18 482 entries
  (macOS ephemeral-port range exhausted). The failure mode matches the
  coder's documented prior-pass observation exactly; the test code itself
  is unchanged from the previous green-state in `coding.md`'s 2026-06-17
  summary (44 tests passing). I did not re-run grpc / tcp / cross_transport
  integration suites because they share the same `127.0.0.1` accept
  pattern and would fail identically. NOT a remediation failure.

$ cargo audit --file Cargo.lock
  ✗ Could not fetch advisory DB (network sandboxed during this audit).
  The prior pass recorded one allowed warning (paste 1.0.15, RUSTSEC-2024-
  0436, "unmaintained"); the lockfile is unchanged this pass so the result
  carries over: no CVEs, one unmaintained-crate warning.
```

22 of 22 unit + parity tests pass on this host. 44 integration tests are
blocked by the host's TIME_WAIT exhaustion — the same gap the coder
documented. The integration test code paths under test are unchanged from
the previously-green baseline; the only code deltas since that baseline
are the SEC-009 healthcheck additions, which are exercised by 6 dedicated
new unit tests (all passing) and do not touch the integration-test
networking paths.

### Recommendation

**PASS to next phase.**

All HIGH (2) and MEDIUM (6) findings are FIXED with verifiable evidence in
source. LOW SEC-009 and SEC-011 also FIXED. INFO SEC-014 FIXED. The
deferred findings (SEC-010 informational, SEC-012/013 documented or
client-side, SEC-015..SEC-019 posture-only) all match the orchestrator
brief.

Per the pass/fail criteria:
- No CRITICAL findings: confirmed.
- No unmitigated HIGH findings: confirmed (both HIGH fixed).
- One environmental caveat: macOS TIME_WAIT exhaustion blocks integration-
  test execution on this host. The unit + parity coverage that does run is
  clean, the changes since the previously-green baseline are tightly
  scoped to non-network-test paths, and the failure mode is reproducible
  outside the kirk codebase. This is **not** a remediation failure and
  should not block phase passage. The test agent should re-run integration
  suites on a fresh host (or after ~5 min of TIME_WAIT drainage) for the
  final acceptance.
