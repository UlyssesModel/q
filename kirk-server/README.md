# kirk-server

Single Rust binary that exposes the `kirk-stub-realistic` pipeline on three transports simultaneously, sharing one `Arc<KirkBackend>` so the rolling-window state is global.

## Overview

```
Arc<KirkBackend>
  parking_lot::Mutex<KirkRealistic>
      |
      +--- tonic gRPC listener  :50051
      +--- axum REST listener   :8080
      +--- custom TCP listener  :9090
```

The mutex is held only for the rolling-window update (a few microseconds). Eigendecomposition runs inline for `N < 128` and on a `spawn_blocking` thread for `N >= 128`.

## Quick Start

```bash
# Build
cargo build --release -p kirk-server

# Run with defaults
cargo run --release -p kirk-server

# Or via Makefile
make run

# Health check
curl http://localhost:8080/healthz
```

## CLI Flags

All flags are also configurable via environment variables (`KIRK_<FLAG_NAME_UPPERCASE>`).

| Flag | Default | Range | Env var | Description |
|------|---------|-------|---------|-------------|
| `--grpc-port` | `50051` | 1..65535 | `KIRK_GRPC_PORT` | gRPC listener port |
| `--rest-port` | `8080` | 1..65535 | `KIRK_REST_PORT` | REST listener port |
| `--tcp-port` | `9090` | 1..65535 | `KIRK_TCP_PORT` | Custom TCP listener port |
| `--bind` | `0.0.0.0` | any IP | `KIRK_BIND` | Bind address for all three listeners |
| `--workers` | `0` (= num_cpus) | >= 0 | `KIRK_WORKERS` | Tokio worker threads; 0 = num_cpus |
| `--temperature` | `1.0` | > 0 | `KIRK_TEMPERATURE` | Boltzmann temperature |
| `--window-size` | `256` | >= 1 | `KIRK_WINDOW_SIZE` | Rolling-window size for z-score |
| `--max-matrix-dim` | `1024` | 1..=4096 | `KIRK_MAX_MATRIX_DIM` | Hard cap on matrix dimension N (clamped by clap; see Security) |
| `--max-connections` | `1024` | 1..=65535 | `KIRK_MAX_CONNECTIONS` | Max concurrent TCP connections (semaphore) |
| `--max-in-flight-per-conn` | `128` | 1..=65535 | `KIRK_MAX_IN_FLIGHT_PER_CONN` | Max in-flight frames per TCP connection |
| `--tcp-write-timeout-ms` | `10000` | 100..=600000 | `KIRK_TCP_WRITE_TIMEOUT_MS` | Per-write timeout (ms) for TCP writer task |
| `--log-level` | `info` | env_filter | `KIRK_LOG_LEVEL` | Tracing filter string |
| `--healthcheck` | (flag only, no env var) | — | — | One-shot health probe: GET /healthz, exit 0 on HTTP 200 |

### Dimension constraints

`N` must satisfy `2 <= N <= --max-matrix-dim`. `N = 1` is rejected by `KirkBackend::check_dim` with a `400 Bad Request` (REST), `INVALID_ARGUMENT` (gRPC), or `MATRIX_DIM_EXCEEDED` ERROR frame (TCP). The confidence formula `1 - H/ln(N)` and the rolling-window z-score are mathematically degenerate at N=1.

### Wire-level payload caps

The following caps are aligned across all three transports at 64 MiB:

- REST: `DefaultBodyLimit::max(64 MiB)` on the axum router
- gRPC: `max_decoding_message_size(64 MiB)` and `max_encoding_message_size(64 MiB)` on the tonic server
- TCP: hard `payload_len` cap in the framing parser; per-shape budget checked before any per-N allocation (defense against base64-amplification equivalent at the binary level)

## REST Endpoints

### Overview

JSON in / out. Matrices are base64-encoded little-endian f32. Field names mirror the Python `to_kafka_envelope` convention (`matrix_re`, `matrix_im`, `matrix_dim`).

Error envelope (4xx/5xx):
```json
{"error": "<code>", "message": "<human-readable>", "detail": {}}
```

### Endpoint Table

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/forward` | Full KirkRealistic pipeline (stateful, rolling window) |
| `POST` | `/v1/inference/entropy` | Stateless entropy scalar |
| `POST` | `/v1/inference/features` | Stateless feature array/vector/scalar |
| `POST` | `/v1/active-inference` | Stateless features + entropy |
| `POST` | `/v1/active-inference/entropy` | Stateless entropy scalar (active variant) |
| `POST` | `/v1/active-inference/features` | Stateless features (active variant) |
| `POST` | `/v1/forward-sample` | Shape-correct random sample |
| `GET` | `/healthz` | Health check |
| `GET` | `/metrics` | Prometheus text exposition |

### POST /v1/forward

Request:
```json
{
  "matrix_re": "QUJDR...==",
  "matrix_im": "QUJDR...==",
  "matrix_dim": 32,
  "timestamp_us": 1718560000000000
}
```

`matrix_re` and `matrix_im` are standard base64-encoded little-endian f32 arrays of length `matrix_dim * matrix_dim`.

Response (200 OK):
```json
{
  "entropy_re":        3.41,
  "entropy_im":        3.42,
  "entropy":           3.40,
  "entropy_zscore":    0.0,
  "regime":            1,
  "confidence":        0.018,
  "processing_time_us": 124,
  "timestamp_us":      1718560000000000,
  "matrix_re":         "...",
  "matrix_im":         "...",
  "matrix_dim":        32
}
```

### POST /v1/inference/entropy, /v1/active-inference/entropy

Request:
```json
{"sample_re": "...", "sample_im": "...", "matrix_dim": 32}
```

Response:
```json
{"total_relative_entropy": 2.14}
```

### POST /v1/inference/features, /v1/active-inference/features

Response:
```json
{
  "feature_arr_re": "...", "feature_arr_im": "...", "feature_arr_dim": 32,
  "feature_vec_re": "...", "feature_vec_im": "...",
  "feature_scalar_re": 0.12, "feature_scalar_im": -0.04
}
```

### POST /v1/active-inference

Response: features fields (above) plus `"total_relative_entropy": 2.14`.

### POST /v1/forward-sample

Request:
```json
{"matrix_dim": 32, "seed": 42}
```

Response: same shape as `/v1/active-inference` with `"relative_entropy"` instead of `"total_relative_entropy"`.

### GET /healthz

```
200 OK
{"status":"ok"}
```

Returns `503` while shutdown is in progress.

### GET /metrics

Prometheus text format. Includes `kirk_requests_total{transport,op}` and `kirk_request_duration_seconds{transport,op,le}` histograms per route.

## gRPC RPCs

Service: `kirk.v1.KirkService` on port 50051.

| RPC | Request | Response |
|-----|---------|----------|
| `Forward` | `KirkRequest` | `KirkResponse` |
| `InferenceEntropy` | `SampleRequest` | `EntropyResponse` |
| `InferenceFeatures` | `SampleRequest` | `FeaturesResponse` |
| `ActiveInference` | `SampleRequest` | `ActiveInferenceResponse` |
| `ActiveInferenceEntropy` | `SampleRequest` | `EntropyResponse` |
| `ActiveInferenceFeatures` | `SampleRequest` | `FeaturesResponse` |
| `ForwardSample` | `SampleSizeRequest` | `SampleResponse` |

Schema is defined in `proto/kirk.proto` (single source of truth). Matrices are transmitted as `bytes data_re` / `bytes data_im` — raw little-endian f32, no base64.

tonic settings: `max_concurrent_streams = 1024`, `tcp_nodelay(true)`, `tcp_keepalive(30 s)`.

## Custom TCP Wire Format

All multi-byte integers and floats are **little-endian**. Implemented with no alignment padding; all reads use `read_exact`.

### Frame Layout

```
+----------+-------+--------+-------+---------+-------------+
| magic    | ver   | opcode | flags | req_id  | payload_len |  <- 16-byte header
| u32      | u8    | u8     | u16   | u32     | u32         |
+----------+-------+--------+-------+---------+-------------+
| payload bytes (payload_len bytes)                         |
+-----------------------------------------------------------+
```

| Field | Size | Value |
|-------|------|-------|
| `magic` | 4 bytes | `0x4B49524B` ("KIRK" in LE: bytes `[0x4B, 0x49, 0x52, 0x4B]`) |
| `version` | 1 byte | `1` |
| `opcode` | 1 byte | see table below |
| `flags` | 2 bytes | reserved = 0 |
| `req_id` | 4 bytes | client-chosen u32; server echoes in response |
| `payload_len` | 4 bytes | payload size in bytes; hard cap 64 MiB |

Frames with a mismatched magic, wrong version, unknown opcode, or `payload_len` exceeding the 64 MiB cap result in an ERROR response (see Error Codes). Frames claiming `N > --max-matrix-dim` result in a `MATRIX_DIM_EXCEEDED` error before any per-N allocation.

### Opcodes

| Opcode | Name | Direction | Payload |
|--------|------|-----------|---------|
| `0x01` | `FORWARD` | request | `[u32 N][N*N f32 re][N*N f32 im][i64 ts_us]` |
| `0x02` | `INFERENCE_ENTROPY` | request | `[u32 N][N*N f32 re][N*N f32 im]` |
| `0x03` | `INFERENCE_FEATURES` | request | same as `INFERENCE_ENTROPY` |
| `0x04` | `ACTIVE_INFERENCE` | request | same as `INFERENCE_ENTROPY` |
| `0x05` | `ACTIVE_INFERENCE_ENTROPY` | request | same as `INFERENCE_ENTROPY` |
| `0x06` | `ACTIVE_INFERENCE_FEATURES` | request | same as `INFERENCE_ENTROPY` |
| `0x07` | `FORWARD_SAMPLE` | request | `[u32 N][u64 seed]` |
| `0xFE` | `PING` | request/response | empty payload; server echoes with same `req_id` |
| `0xFF` | `ERROR` | response only | see Error payload below |

### Request Payload Sizes

| Opcode | Payload bytes |
|--------|--------------|
| `FORWARD` | `4 + 8*N*N + 8` = `12 + 8N²` |
| `INFERENCE_*` / `ACTIVE_*` | `4 + 8*N*N` = `4 + 8N²` |
| `FORWARD_SAMPLE` | `4 + 8` = `12` (fixed; no per-N data) |
| `PING` | `0` |

### Response Payloads

**FORWARD** (`0x01` echoed):
```
[f32 entropy_re][f32 entropy_im][f32 entropy][f32 entropy_zscore]
[u32 regime][f32 confidence][u64 processing_time_us][i64 timestamp_us]
[u32 N][N*N f32 rho_re][N*N f32 rho_im]
```
Fixed prefix: 36 bytes. Total: `36 + 4 + 8N²`.

**INFERENCE_ENTROPY / ACTIVE_INFERENCE_ENTROPY**:
```
[f32 total_relative_entropy]
```
Total: 4 bytes.

**INFERENCE_FEATURES / ACTIVE_INFERENCE_FEATURES**:
```
[u32 N]
[N*N f32 feature_arr_re][N*N f32 feature_arr_im]
[2N  f32 feature_vec_re][2N  f32 feature_vec_im]
[f32 feature_scalar_re][f32 feature_scalar_im]
```
Total: `4 + 8N² + 16N + 8`.

**ACTIVE_INFERENCE**:
Features payload (above) + trailing `[f32 total_relative_entropy]`.

**FORWARD_SAMPLE**:
Features payload (above) + trailing `[f32 relative_entropy]`.

### Error Payload (`0xFF`)

```
[u16 error_code][u16 reserved][u32 msg_len][msg_len UTF-8 bytes]
```

### Error Codes

| Code | Name | Meaning |
|------|------|---------|
| `0x01` | `BAD_MAGIC` | Magic bytes mismatch; connection closed after response |
| `0x02` | `PAYLOAD_TOO_LARGE` | `payload_len > 64 MiB`; connection closed |
| `0x03` | `UNSUPPORTED_VERSION` | `version != 1` |
| `0x04` | `UNKNOWN_OPCODE` | Opcode not in the table above |
| `0x05` | `BAD_PAYLOAD` | Length or shape mismatch (e.g. truncated payload) |
| `0x06` | `MATRIX_DIM_EXCEEDED` | `N > --max-matrix-dim` |
| `0x07` | `COMPUTE_ERROR` | Eigendecomposition failed, NaN, or other kernel error |
| `0x08` | `SHUTDOWN_IN_PROGRESS` | Server is draining; connection will close after response |

### TCP Connection Lifecycle

A client opens a persistent TCP connection and may pipeline multiple requests with different `req_id` values. The server processes pipelined frames in parallel within the connection (each frame spawned as a separate tokio task) and writes responses to the client through a per-connection mpsc channel in **completion order** — not arrival order. Clients must correlate responses by `req_id`.

Bounds:
- Up to `--max-connections` concurrent connections (accept-loop semaphore)
- Up to `--max-in-flight-per-conn` frames in flight per connection
- Write timeout: `--tcp-write-timeout-ms` (default 10 s) — slow or dead clients are disconnected

A slow `forward` will not block a fast `ping` on the same connection. This is intentional; see the security notes (SEC-012) for the rationale.

## Operational Notes

### Graceful Shutdown

On `SIGINT` or `SIGTERM`:
1. All three listeners stop accepting new connections.
2. In-flight requests are given 10 seconds to complete.
3. Server exits with code 0 on clean drain, 1 if the deadline is exceeded.

During shutdown, `/healthz` returns `503 Service Unavailable` and the TCP server sends `SHUTDOWN_IN_PROGRESS` (0x08) ERROR frames on new requests.

### Healthcheck Flag

The `--healthcheck` flag performs a one-shot HTTP GET to `http://127.0.0.1:<rest-port>/healthz` (5 s timeout), prints the status, and exits 0 on HTTP 200 or 1 otherwise. This is used by the docker-compose healthcheck because the distroless runtime image has neither `wget` nor a shell.

```bash
# Manually probe a running server
./kirk-server --healthcheck
```

### Tracing / Logs

Logs are structured JSON on stdout (via `tracing-subscriber`). Filter with `--log-level` or `KIRK_LOG_LEVEL`. Spans carry `transport={grpc|rest|tcp}` and `op=<opname>` attributes. Matrix contents are never logged; only sizes and timing.

### Metrics

`GET /metrics` (REST port) — Prometheus text exposition:
- `kirk_requests_total{transport, op}` — request counter
- `kirk_request_duration_seconds{transport, op, le}` — 15-bucket histogram

## Security Posture

This server has no authentication and no TLS. Intended to run behind a reverse proxy (nginx, envoy, or similar) for any network-exposed deployment.

Recommendations:
- **Non-Docker host runs**: `--bind 127.0.0.1` to prevent LAN exposure
- **Docker runs**: the default `--bind 0.0.0.0` is fine because Docker's port mapping is controlled by the host firewall
- **Public networks**: add TLS termination and AuthN/AuthZ at the proxy layer

Resource protection:
- `--max-matrix-dim` clamp prevents `O(N^4)` eigh memory allocation (`value_parser` rejects values outside `[1, 4096]` at parse time)
- `--max-connections` prevents file-descriptor exhaustion
- `--max-in-flight-per-conn` prevents per-connection task-spawn explosion
- `--tcp-write-timeout-ms` disconnects slow/dead clients
- REST body limit 64 MiB; base64 is pre-validated against `matrix_dim` before any decode allocation

Findings from the security audit and their dispositions:
- SEC-001 through SEC-009, SEC-011, SEC-014: resolved (see `coding.md` remediation pass)
- SEC-010 (mild error message info disclosure): accepted, low impact
- SEC-012 (out-of-order TCP completion): documented behavior, not a bug
- SEC-013 (bench-ts client buffer growth): client-side, deferred

## Out of Scope

- TLS / mTLS — terminate at a proxy
- Authentication / authorization
- Multi-tenant rolling-window state (future: `DashMap<TenantId, Mutex<KirkRealistic>>`)
- GPU acceleration
- Listener backlog tuning via socket2 (tokio's default backlog + connection semaphore is sufficient)
