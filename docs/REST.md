# kirk-server REST API

The REST API exposes the full `kirk-stub-realistic` compute pipeline — a six-stage quantum-inspired kernel that hermitianizes an input matrix, diagonalizes it via eigendecomposition, applies Boltzmann weights, computes a density matrix, and returns entropy and feature observables. Nine endpoints cover the stateful forward pass (`/v1/forward`), stateless inference variants, active-inference variants, and a convenience sampler.

All requests and responses use JSON. Complex matrices are transmitted as two parallel base64-encoded blobs (real part and imaginary part), each containing little-endian 32-bit floats in row-major order. This encoding keeps the wire format human-readable and compatible with every HTTP client; see [Matrix Encoding](#matrix-encoding) for the step-by-step details.

The server runs a single instance with no built-in authentication or TLS. It is designed to sit behind a reverse proxy (nginx, Envoy, or similar) that handles TLS termination and access control. For high-throughput use cases, consider the gRPC transport (port 50051) or the custom binary TCP protocol (port 9090), which avoid the base64 overhead; see [ARCHITECTURE.md](ARCHITECTURE.md) and [../kirk-server/README.md](../kirk-server/README.md) for those wire formats.

## Quick Start

**Start the server**

```bash
# With Docker Compose (recommended)
make up

# Or from source
cargo run --release -p kirk-server -- --bind 127.0.0.1
```

The REST listener binds to port 8080 by default. Verify it is up:

```bash
curl http://localhost:8080/healthz
```

Expected response:

```json
{"status":"ok"}
```

**Send a forward request with a tiny N=2 matrix**

The N=2 diagonal matrix `H = diag(1, -1)` from the hand-computed fixture encodes the four real-part elements `[1.0, 0.0, 0.0, -1.0]` and an all-zero imaginary part. Encode them as little-endian f32:

```bash
python3 -c "
import base64, struct
re = struct.pack('<4f', 1.0, 0.0, 0.0, -1.0)
im = struct.pack('<4f', 0.0, 0.0, 0.0,  0.0)
print('matrix_re:', base64.b64encode(re).decode())
print('matrix_im:', base64.b64encode(im).decode())
"
```

Output:

```plaintext
matrix_re: AACAPwAAAAAAAAAAAAAAwL8=
matrix_im: AAAAAAAAAAAAAAAAAAAAAAA=
```

Send the request:

```bash
curl -s -X POST http://localhost:8080/v1/forward \
  -H "Content-Type: application/json" \
  -d '{
    "matrix_re": "AACAPwAAAAAAAAAAAAAAwL8=",
    "matrix_im": "AAAAAAAAAAAAAAAAAAAAAAA=",
    "matrix_dim": 2,
    "timestamp_us": 0
  }' | python3 -m json.tool
```

**Decode the response**

| Field | Type | Meaning |
|-------|------|---------|
| `entropy` | f32 | Shannon entropy of the density matrix (nats). For N=2 with H=diag(1,-1) at T=1, expect ~0.365. |
| `entropy_re` | f32 | Entropy computed from real eigenvalues only |
| `entropy_im` | f32 | Entropy computed from imaginary eigenvalues only |
| `entropy_zscore` | f32 | Z-score against the rolling window (0 until the window fills) |
| `regime` | u32 | Discretized market-regime label derived from the z-score |
| `confidence` | f32 | `1 - entropy / ln(N)` — how far from maximum entropy (0=max entropy, 1=pure state) |
| `processing_time_us` | u64 | Server-side wall time for the compute call, in microseconds |
| `timestamp_us` | i64 | Echo of the request `timestamp_us` |
| `matrix_re` | string | Base64 little-endian f32: real part of the output density matrix rho (N*N elements) |
| `matrix_im` | string | Base64 little-endian f32: imaginary part of rho (N*N elements) |
| `matrix_dim` | u32 | Echo of the input dimension N |

---

## Endpoint Reference

### GET /healthz

Returns the server's readiness state.

**Response — 200 OK**

```json
{"status":"ok"}
```

**Response — 503 Service Unavailable** (during graceful shutdown)

```json
{"status":"shutdown"}
```

**Example**

```bash
curl http://localhost:8080/healthz
```

---

### GET /metrics

Returns Prometheus text exposition (format version 0.0.4). Suitable for scraping by a Prometheus server.

**Response** — `200 OK`, `Content-Type: text/plain; version=0.0.4`

Counters and histograms emitted:

| Metric | Labels | Description |
|--------|--------|-------------|
| `kirk_requests_total` | `transport`, `op` | Total request count per operation |
| `kirk_request_duration_seconds` | `transport`, `op`, `le` | 15-bucket latency histogram |

**Example**

```bash
curl http://localhost:8080/metrics
```

---

### POST /v1/forward

Runs the full stateful pipeline: hermitianize → eigh → Boltzmann weights → density matrix → entropy + z-score update. The rolling window state (for z-score) is shared across all callers on all transports.

**Request body**

```json
{
  "matrix_re": "<base64-f32>",
  "matrix_im": "<base64-f32>",
  "matrix_dim": 32,
  "timestamp_us": 1718560000000000
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `matrix_re` | string | yes | Base64 standard-encoding of N*N little-endian f32 values (real part, row-major) |
| `matrix_im` | string | yes | Base64 standard-encoding of N*N little-endian f32 values (imaginary part, row-major) |
| `matrix_dim` | u32 | yes | Matrix dimension N. Must satisfy 2 <= N <= `--max-matrix-dim` (default 1024) |
| `timestamp_us` | i64 | no | Caller-supplied Unix timestamp in microseconds; echoed in the response. Defaults to 0 |

**Response — 200 OK**

```json
{
  "entropy_re": 0.36533208,
  "entropy_im": 0.6931472,
  "entropy": 0.36533208,
  "entropy_zscore": 0.0,
  "regime": 0,
  "confidence": 0.47296384,
  "processing_time_us": 47,
  "timestamp_us": 1718560000000000,
  "matrix_re": "<base64-f32>",
  "matrix_im": "<base64-f32>",
  "matrix_dim": 2
}
```

| Field | Type | Description |
|-------|------|-------------|
| `entropy_re` | f32 | Shannon entropy of the real-part eigenvalue distribution |
| `entropy_im` | f32 | Shannon entropy of the imaginary-part eigenvalue distribution |
| `entropy` | f32 | Combined entropy scalar |
| `entropy_zscore` | f32 | Z-score against the rolling window (`--window-size`, default 256) |
| `regime` | u32 | Discretized regime label |
| `confidence` | f32 | `1 - entropy / ln(N)` |
| `processing_time_us` | u64 | Server-side compute time in microseconds |
| `timestamp_us` | i64 | Echo of the request timestamp |
| `matrix_re` | string | Base64 little-endian f32: real part of output density matrix rho (N*N elements) |
| `matrix_im` | string | Base64 little-endian f32: imaginary part of rho (N*N elements) |
| `matrix_dim` | u32 | Echo of N |

**Error responses**: 400, 413, 422, 500, 503 — see [Error Envelope](#error-envelope).

**Example**

```bash
curl -s -X POST http://localhost:8080/v1/forward \
  -H "Content-Type: application/json" \
  -d '{
    "matrix_re": "AACAPwAAAAAAAAAAAAAAwL8=",
    "matrix_im": "AAAAAAAAAAAAAAAAAAAAAAA=",
    "matrix_dim": 2,
    "timestamp_us": 1718560000000000
  }'
```

---

### POST /v1/inference/entropy

Stateless entropy computation on a caller-supplied sample matrix. Does not update the rolling window.

**Request body**

```json
{
  "sample_re": "<base64-f32>",
  "sample_im": "<base64-f32>",
  "matrix_dim": 32
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `sample_re` | string | yes | Base64 little-endian f32: real part of N*N input matrix |
| `sample_im` | string | yes | Base64 little-endian f32: imaginary part of N*N input matrix |
| `matrix_dim` | u32 | yes | Dimension N. Must satisfy 2 <= N <= `--max-matrix-dim` |

**Response — 200 OK**

```json
{"total_relative_entropy": 2.1403}
```

| Field | Type | Description |
|-------|------|-------------|
| `total_relative_entropy` | f32 | Shannon entropy of the density matrix derived from the sample |

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v1/inference/entropy \
  -H "Content-Type: application/json" \
  -d '{
    "sample_re": "AACAPwAAAAAAAAAAAAAAwL8=",
    "sample_im": "AAAAAAAAAAAAAAAAAAAAAAA=",
    "matrix_dim": 2
  }'
```

---

### POST /v1/inference/features

Stateless extraction of the feature array, feature vector, and feature scalar from a caller-supplied sample matrix.

**Request body** — same shape as `/v1/inference/entropy`.

**Response — 200 OK**

```json
{
  "feature_arr_re": "<base64-f32>",
  "feature_arr_im": "<base64-f32>",
  "feature_vec_re": "<base64-f32>",
  "feature_vec_im": "<base64-f32>",
  "feature_scalar": {"re": 0.12, "im": -0.04},
  "matrix_dim": 32
}
```

| Field | Type | Description |
|-------|------|-------------|
| `feature_arr_re` | string | Base64 little-endian f32: real part of the N*N feature array |
| `feature_arr_im` | string | Base64 little-endian f32: imaginary part of the N*N feature array |
| `feature_vec_re` | string | Base64 little-endian f32: real part of the 2N feature vector |
| `feature_vec_im` | string | Base64 little-endian f32: imaginary part of the 2N feature vector |
| `feature_scalar` | object | Complex scalar `{"re": f32, "im": f32}` |
| `matrix_dim` | u32 | Echo of N |

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v1/inference/features \
  -H "Content-Type: application/json" \
  -d '{
    "sample_re": "AACAPwAAAAAAAAAAAAAAwL8=",
    "sample_im": "AAAAAAAAAAAAAAAAAAAAAAA=",
    "matrix_dim": 2
  }'
```

---

### POST /v1/active-inference

Stateless combined operation: returns both features and total relative entropy in one call.

**Request body** — same shape as `/v1/inference/entropy`.

**Response — 200 OK**

```json
{
  "feature_arr_re": "<base64-f32>",
  "feature_arr_im": "<base64-f32>",
  "feature_vec_re": "<base64-f32>",
  "feature_vec_im": "<base64-f32>",
  "feature_scalar": {"re": 0.12, "im": -0.04},
  "matrix_dim": 32,
  "total_relative_entropy": 2.1403
}
```

Same fields as `/v1/inference/features` plus `total_relative_entropy` (f32).

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v1/active-inference \
  -H "Content-Type: application/json" \
  -d '{
    "sample_re": "AACAPwAAAAAAAAAAAAAAwL8=",
    "sample_im": "AAAAAAAAAAAAAAAAAAAAAAA=",
    "matrix_dim": 2
  }'
```

---

### POST /v1/active-inference/entropy

Stateless entropy scalar using the active-inference variant. Identical request/response shape to `/v1/inference/entropy`.

**Request body** — same shape as `/v1/inference/entropy`.

**Response — 200 OK**

```json
{"total_relative_entropy": 2.1403}
```

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v1/active-inference/entropy \
  -H "Content-Type: application/json" \
  -d '{
    "sample_re": "AACAPwAAAAAAAAAAAAAAwL8=",
    "sample_im": "AAAAAAAAAAAAAAAAAAAAAAA=",
    "matrix_dim": 2
  }'
```

---

### POST /v1/active-inference/features

Stateless feature extraction using the active-inference variant. Identical request/response shape to `/v1/inference/features`.

**Request body** — same shape as `/v1/inference/entropy`.

**Response — 200 OK** — same shape as `/v1/inference/features`.

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v1/active-inference/features \
  -H "Content-Type: application/json" \
  -d '{
    "sample_re": "AACAPwAAAAAAAAAAAAAAwL8=",
    "sample_im": "AAAAAAAAAAAAAAAAAAAAAAA=",
    "matrix_dim": 2
  }'
```

---

### POST /v1/forward-sample

Generates a random shape-correct sample of dimension N using a caller-supplied seed, then runs the inference pipeline on it. Useful for testing and benchmarking without needing to supply a real input matrix.

**Request body**

```json
{
  "matrix_dim": 32,
  "seed": 42
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `matrix_dim` | u32 | yes | Dimension N. Must satisfy 2 <= N <= `--max-matrix-dim` |
| `seed` | u64 | yes | RNG seed for reproducible sample generation |

**Response — 200 OK**

```json
{
  "feature_array_re": "<base64-f32>",
  "feature_array_im": "<base64-f32>",
  "feature_vector_re": "<base64-f32>",
  "feature_vector_im": "<base64-f32>",
  "feature_scalar": {"re": 0.07, "im": -0.02},
  "matrix_dim": 32,
  "relative_entropy": 3.4102
}
```

| Field | Type | Description |
|-------|------|-------------|
| `feature_array_re` | string | Base64 little-endian f32: real part of N*N feature array |
| `feature_array_im` | string | Base64 little-endian f32: imaginary part of N*N feature array |
| `feature_vector_re` | string | Base64 little-endian f32: real part of 2N feature vector |
| `feature_vector_im` | string | Base64 little-endian f32: imaginary part of 2N feature vector |
| `feature_scalar` | object | Complex scalar `{"re": f32, "im": f32}` |
| `matrix_dim` | u32 | Echo of N |
| `relative_entropy` | f32 | Shannon entropy of the generated sample's density matrix |

Note: this endpoint uses `feature_array_re` / `feature_vector_re` (full words), unlike the `/v1/inference/features` family which uses `feature_arr_re` / `feature_vec_re` (abbreviated). Match field names exactly.

**Error responses**: 400, 413, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v1/forward-sample \
  -H "Content-Type: application/json" \
  -d '{"matrix_dim": 4, "seed": 42}'
```

---

## Matrix Encoding

### Why base64?

JSON cannot represent raw binary data. Encoding each f32 as a decimal string (e.g., `"1.0000001192"`) inflates the payload and introduces rounding at the JSON layer. Instead, the API uses standard base64 (RFC 4648) applied to the raw IEEE 754 little-endian byte representation of each float. A 32x32 matrix (1024 elements) becomes 5120 base64 characters — compact and lossless.

### Encoding rules

1. Start with N*N `f32` values in **row-major** order (row 0 left to right, then row 1, etc.).
2. Serialize each `f32` as **4 bytes, little-endian** (least significant byte first).
3. Concatenate all byte sequences into one buffer of `N * N * 4` bytes.
4. Apply **standard base64 encoding** (alphabet `A-Za-z0-9+/`, `=` padding).
5. Repeat for real and imaginary parts independently, producing two strings.

### Python helper

```python
import base64
import struct

def encode_matrix(values: list[float]) -> str:
    """Encode a flat row-major list of floats as base64 little-endian f32."""
    raw = struct.pack(f"<{len(values)}f", *values)
    return base64.b64encode(raw).decode()

def decode_matrix(b64: str, n: int) -> list[float]:
    """Decode a base64 little-endian f32 blob into a flat list of n*n floats."""
    raw = base64.b64decode(b64)
    return list(struct.unpack(f"<{n * n}f", raw))

# Example: 2x2 identity matrix (real part)
matrix_re = encode_matrix([1.0, 0.0, 0.0, 1.0])
print(matrix_re)  # AACAPwAAAAAAAAAAAACAPw==
```

### JavaScript / Bun helper

```javascript
function encodeMatrix(values) {
  const buf = new ArrayBuffer(values.length * 4);
  const view = new DataView(buf);
  for (let i = 0; i < values.length; i++) {
    view.setFloat32(i * 4, values[i], true); // true = little-endian
  }
  return Buffer.from(buf).toString("base64");
}

function decodeMatrix(b64, n) {
  const buf = Buffer.from(b64, "base64");
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const out = [];
  for (let i = 0; i < n * n; i++) {
    out.push(view.getFloat32(i * 4, true)); // true = little-endian
  }
  return out;
}

// Example: 2x2 identity matrix (real part)
const matrixRe = encodeMatrix([1.0, 0.0, 0.0, 1.0]);
console.log(matrixRe); // AACAPwAAAAAAAAAAAACAPw==
```

### Size limits

| Limit | Value | Configurable |
|-------|-------|--------------|
| Maximum matrix dimension N | 1024 (default) | `--max-matrix-dim` / `KIRK_MAX_MATRIX_DIM` (range 1..=4096) |
| Maximum HTTP request body | 64 MiB | No (hard-coded) |

The base64 decode is validated against `matrix_dim` before any allocation: if the encoded length implies more bytes than `N * N * 4`, the server returns `413` without allocating. A 1024x1024 matrix encodes to approximately 5.6 MiB of base64, well within the 64 MiB body limit.

---

## Error Envelope

All 4xx and 5xx responses share the same JSON shape:

```json
{
  "error": "<code>",
  "message": "<human-readable description>"
}
```

| `error` code | HTTP status | Trigger |
|--------------|-------------|---------|
| `bad_request` | 400 | Malformed JSON, invalid base64, wrong decoded byte length, N < 2 |
| `payload_too_large` | 413 | Decoded matrix bytes exceed the global `MAX_DECODED_BYTES` cap |
| `matrix_dim_exceeded` | 413 | `matrix_dim` exceeds `--max-matrix-dim` (default 1024) |
| `compute_error` | 422 | Eigendecomposition failed, NaN in output, or other kernel error |
| `internal` | 500 | Unexpected server-side error |
| `shutdown_in_progress` | 503 | Server is draining in-flight requests before exit |

**Example error response**

```json
{
  "error": "matrix_dim_exceeded",
  "message": "matrix dim 2048 exceeds max 1024"
}
```

---

## Limits and Performance Notes

**Matrix dimension cap** — `--max-matrix-dim` (default 1024, range 1..=4096). Clap rejects values outside this range at startup. The server enforces the cap per-request before any per-N allocation.

**HTTP body limit** — 64 MiB hard cap on the axum router (`DefaultBodyLimit`). Requests exceeding this are rejected by the framework before reaching handler code.

**Blocking threshold** — For N < 128, eigendecomposition runs inline on the tokio async thread (sub-millisecond). For N >= 128, it is dispatched to `tokio::task::spawn_blocking` to avoid stalling the reactor. Expect one extra task-switch per large request.

**Global mutex** — All three transports share one `parking_lot::Mutex<KirkRealistic>`. The mutex is held only for the rolling-window state update (a few microseconds after eigendecomposition). High REST concurrency at small N (e.g., N=32) will queue at this mutex. For throughput-critical workloads at small N, prefer the gRPC or TCP transports, which share the same bottleneck but have lower per-request overhead.

**Prometheus metrics** — Scrape `GET /metrics` to observe per-operation request counts and latency histograms. Labels: `transport=rest`, `op=<forward|inference_entropy|...>`.

**When to use gRPC or TCP instead** — The REST transport encodes matrices as base64, adding ~33% wire overhead and a CPU decode step on the server. If you need maximum throughput (especially at N >= 32), use the gRPC transport (port 50051, raw bytes in protobuf) or the custom TCP transport (port 9090, zero-copy binary framing). See [ARCHITECTURE.md](ARCHITECTURE.md) for a benchmark comparison.

---

## Configuration Cheat-Sheet

All CLI flags accept a corresponding environment variable of the form `KIRK_<UPPERCASE_FLAG>`.

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--rest-port` | `KIRK_REST_PORT` | `8080` | REST listener port |
| `--bind` | `KIRK_BIND` | `0.0.0.0` | Bind address for all listeners. Use `127.0.0.1` on non-Docker hosts |
| `--max-matrix-dim` | `KIRK_MAX_MATRIX_DIM` | `1024` | Hard cap on N (1..=4096) |
| `--temperature` | `KIRK_TEMPERATURE` | `1.0` | Boltzmann temperature for softmax weighting |
| `--window-size` | `KIRK_WINDOW_SIZE` | `256` | Rolling-window depth for entropy z-score |
| `--workers` | `KIRK_WORKERS` | `0` (= num_cpus) | Tokio worker threads |
| `--log-level` | `KIRK_LOG_LEVEL` | `info` | tracing-subscriber filter string |
| `--grpc-port` | `KIRK_GRPC_PORT` | `50051` | gRPC listener port (not REST-specific but shares `--bind`) |
| `--tcp-port` | `KIRK_TCP_PORT` | `9090` | Custom TCP listener port |

**Common Docker Compose overrides**

```bash
# Expose REST on a non-default port
KIRK_REST_PORT=9080 docker compose up -d kirk-server

# Smaller matrix dimension cap in a constrained environment
KIRK_MAX_MATRIX_DIM=256 docker compose up -d kirk-server

# Increase worker threads for a high-core machine
KIRK_WORKERS=16 docker compose up -d kirk-server
```

**Restrict to loopback on a non-Docker host**

```bash
cargo run --release -p kirk-server -- --bind 127.0.0.1
```

---

## Versioning and Stability

All compute endpoints are prefixed with `/v1/`. Breaking changes (field removals, type changes) will bump the prefix to `/v2/`. Additive changes (new optional request fields, new response fields) may be introduced within a version without notice — clients should ignore unknown JSON fields.

`/healthz` and `/metrics` are unversioned utility endpoints and will not be moved.

For the full API changelog and architecture decisions, see [ARCHITECTURE.md](ARCHITECTURE.md) and [../kirk-server/README.md](../kirk-server/README.md).
