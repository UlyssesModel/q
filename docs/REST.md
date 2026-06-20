# kirk-server REST API

The REST API exposes the full `kirk-stub-realistic` compute pipeline — a six-stage quantum-inspired kernel that hermitianizes an input matrix, diagonalizes it via eigendecomposition, applies Boltzmann weights, computes a density matrix, and returns entropy and feature observables. Nine endpoints cover the stateful forward pass (`/v1/forward`), stateless inference variants, active-inference variants, and a convenience sampler.

All requests and responses use JSON. The server exposes two API versions:

- **v1** — complex matrices are transmitted as two parallel base64-encoded blobs (real part and imaginary part), each containing little-endian 32-bit floats in row-major order. This is the production-grade encoding. See [Matrix Encoding](#matrix-encoding) for the step-by-step details.
- **v2** — complex matrices are transmitted as nested JSON arrays of `[re, im]` pairs (f64). This encoding is designed for interactive debugging with `curl` and `jq`. See [API Versions: v1 vs v2](#api-versions-v1-vs-v2) for when to use each.

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

## API Versions: v1 vs v2

Both API versions are registered and equally supported. The server exposes both simultaneously; you can mix them across requests. They differ only in the wire encoding of complex matrices.

### Why does v1 use base64 at all?

JSON has no native binary type. The straightforward alternative — encoding each f32 as a decimal string like `"0.36533208"` — introduces two problems:

1. **Precision loss.** JSON parsers are not required to preserve more than 17 significant decimal digits. Converting an IEEE 754 f32 to decimal and back does not guarantee bit-exact round-tripping when the number is later consumed by a different language runtime.
2. **Size and parse cost.** A decimal string for a typical f32 is 8–12 bytes. The base64 encoding of the same f32's raw 4-byte IEEE 754 representation is approximately 5.33 bytes. For a 1024×1024 matrix (1,048,576 elements), the difference is roughly 3–7 MiB per blob.

Base64 over little-endian f32 bytes solves both problems: values are bit-exact across all clients that decode the same bytes, and the encoding is compact. The decode is a single allocation (one contiguous `Vec<u8>`) with no per-element parsing.

### v2: when is JSON-array encoding better?

v2 exists because base64 blobs are opaque to human eyes and to generic HTTP tools. With v2 you can inspect a matrix element directly:

```bash
curl -s -X POST http://localhost:8080/v2/forward \
  -H "Content-Type: application/json" \
  -d '{"matrix": [[[1.0, 0.0], [0.0, 0.0]], [[0.0, 0.0], [-1.0, 0.0]]], "timestamp_us": 0}' \
  | jq '.matrix[0][0]'
# => [0.731, 0]
```

No helper script, no decode step.

### Comparison table

| Aspect | v1 base64 LE f32 | v2 nested JSON arrays f64 |
|--------|------------------|---------------------------|
| Wire format | two base64 strings (`matrix_re`, `matrix_im`) | one nested array `matrix: [[[re, im], ...], ...]` |
| Bytes per complex element (wire) | ~10.7 bytes (5.33 per blob × 2) | ~30–50 bytes depending on magnitude and compact/pretty encoding |
| Payload size — 1024×1024 f32 matrix (4 MiB raw) | ~11 MiB base64 envelope | ~25–30 MiB JSON envelope |
| Parser per-element cost | single base64 chunk decode, then `from_le_bytes` | one serde walk + finite check per float |
| Allocations per request | 2 (one contiguous blob per re/im) | N+1 (one row `Vec` per row, plus outer `Vec`) |
| Max N at 64 MiB body cap | ~1024 (default; configurable to 4096) | ~300 (compact JSON; above this the body cap is hit before the dim cap) |
| Precision on the wire | f32 native (bit-exact) | f64 (truncated to f32 at the trait boundary for the tiberius backend; see note below) |
| Debuggability | none — opaque blob; requires a decode script | direct — `jq .matrix[0][0]` works out of the box |
| Prometheus op label | `op="forward"`, `op="inference_entropy"`, ... | `op="forward_v2"`, `op="inference_entropy_v2"`, ... |
| Recommended use | production, high-throughput, bench harnesses, N > 300 | interactive debugging, `curl`/Postman exploration, small matrices |

**Precision note for v2:** The JSON envelope carries f64 values, but the server's internal trait surface is f32. For the tiberius backend, v2 input values are truncated f64 → f32 at the handler boundary before being passed to the kernel. Output matrix values are upcast f32 → f64 before serialization. As a result, v2 and v1 responses for the same logical matrix are numerically equal within f32 epsilon (~1e-7 relative), not bit-exact f64. The kirk backend is f64 internally but also receives input that was first truncated to f32 by the trait surface, for the same reason. See [ADR-005 in the architect spec](../agent-notes/architect.md) for the rationale.

### When to use v1 (base64)

- Production traffic at any N, especially N > 300.
- Bench harnesses and programmatic clients — use the Python or JavaScript helpers in [Matrix Encoding](#matrix-encoding).
- Any case where you want the maximum supported matrix size (N up to 1024 by default; up to 4096 with `--max-matrix-dim`).
- High-throughput workloads where JSON parse cost is a bottleneck.

### When to use v2 (JSON arrays)

- Debugging with `curl` or Postman — you can construct a small matrix by hand and inspect the response without a decode step.
- One-off exploration and quick experiments.
- Matrices with N ≤ 300 where the larger wire size fits comfortably within the 64 MiB body cap.
- Client environments where base64 encode/decode helpers are inconvenient to set up.

---

## v1 Endpoint Reference

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

## v2 Endpoint Reference

v2 endpoints accept and return complex matrices as nested JSON arrays of `[re, im]` pairs. The request body is always `Content-Type: application/json`. The practical maximum N is approximately 300 at the 64 MiB body cap; see the comparison table above.

All v2 validation failures use the same `ErrorResponse` envelope as v1 (`{"error": "<code>", "message": "<description>"}`). v2-specific validation errors (jagged rows, wrong inner array length, non-finite values) map to `bad_request` (400).

The `regime` field in v2 forward responses follows the backend:

- **tiberius backend**: same rolling-window z-score logic as v1; regime is meaningful.
- **kirk backend**: `regime` is always `1`, `entropy_zscore` is always `0.0` (the kirk stub has no rolling window). These values are present in the response for schema consistency.

### Common matrix shape

Matrices are represented as a three-level nested array:

```json
[
  [[re00, im00], [re01, im01], ...],
  [[re10, im10], [re11, im11], ...],
  ...
]
```

The server validates:

- The outer array has length N (number of rows).
- Each inner row array has length N (square matrix).
- Each innermost array has exactly 2 elements (serde enforces this via the `[f64; 2]` type).
- Every float is finite (not NaN, not ±Inf).
- N is in the range `[2, --max-matrix-dim]`.

Validation is applied after JSON parsing but before any compute. Row-length mismatches and non-finite values return HTTP 400. Oversized N returns HTTP 413.

---

### POST /v2/forward

Runs the same stateful pipeline as `/v1/forward`. Returns the output density matrix as a nested array instead of base64 blobs. Updates the rolling window state (tiberius backend only).

**Request body**

```json
{
  "matrix": [
    [[1.0, 0.0], [0.0, 0.0]],
    [[0.0, 0.0], [-1.0, 0.0]]
  ],
  "timestamp_us": 0
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `matrix` | `[[[f64; 2]]]` | yes | N×N complex matrix as nested `[re, im]` pairs |
| `timestamp_us` | i64 | no | Caller-supplied Unix timestamp in microseconds. Defaults to 0 |

**Response — 200 OK**

```json
{
  "entropy_re": 0.36533208,
  "entropy_im": 0.6931472,
  "entropy": 0.36533208,
  "entropy_zscore": 0.0,
  "regime": 1,
  "confidence": 0.47296384,
  "processing_time_us": 52,
  "timestamp_us": 0,
  "matrix": [
    [[0.731, 0.0], [0.0, 0.0]],
    [[0.0, 0.0], [0.269, 0.0]]
  ]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `entropy_re` | f32 | Shannon entropy of the real-part eigenvalue distribution |
| `entropy_im` | f32 | Shannon entropy of the imaginary-part eigenvalue distribution |
| `entropy` | f32 | Combined entropy scalar |
| `entropy_zscore` | f32 | Z-score against the rolling window; 0.0 for the kirk backend |
| `regime` | u32 | Discretized regime label; 1 for the kirk backend |
| `confidence` | f32 | `1 - entropy / ln(N)`; 0.0 for the kirk backend |
| `processing_time_us` | u64 | Server-side compute time in microseconds |
| `timestamp_us` | i64 | Echo of the request timestamp |
| `matrix` | `[[[f64; 2]]]` | N×N output density matrix as nested `[re, im]` pairs |

Note: `matrix_dim` is NOT present in v2 responses. The dimension is derivable from `matrix.length`.

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v2/forward \
  -H "Content-Type: application/json" \
  -d '{
    "matrix": [
      [[1.0, 0.0], [0.0, 0.0]],
      [[0.0, 0.0], [-1.0, 0.0]]
    ],
    "timestamp_us": 0
  }' | jq '.matrix[0][0]'
```

---

### POST /v2/inference/entropy

Stateless entropy computation. Does not update the rolling window.

**Request body**

```json
{
  "matrix": [
    [[1.0, 0.0], [0.0, 0.0]],
    [[0.0, 0.0], [-1.0, 0.0]]
  ]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `matrix` | `[[[f64; 2]]]` | yes | N×N complex matrix as nested `[re, im]` pairs |

**Response — 200 OK**

```json
{"total_relative_entropy": 2.1403}
```

| Field | Type | Description |
|-------|------|-------------|
| `total_relative_entropy` | f32 | Shannon entropy scalar |

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v2/inference/entropy \
  -H "Content-Type: application/json" \
  -d '{
    "matrix": [
      [[1.0, 0.0], [0.0, 0.0]],
      [[0.0, 0.0], [-1.0, 0.0]]
    ]
  }'
```

---

### POST /v2/inference/features

Stateless feature extraction.

**Request body** — same shape as `/v2/inference/entropy`.

**Response — 200 OK**

```json
{
  "feature_arr": [
    [[0.731, 0.0], [0.0, 0.0]],
    [[0.0, 0.0], [0.269, 0.0]]
  ],
  "feature_vec": [[0.365, 0.0], [0.135, 0.0], [0.365, 0.0], [0.135, 0.0]],
  "feature_scalar": [0.25, 0.0]
}
```

| Field | Type | Description |
|-------|------|-------------|
| `feature_arr` | `[[[f64; 2]]]` | N×N feature array as nested `[re, im]` pairs |
| `feature_vec` | `[[f64; 2]]` | 2N feature vector as `[re, im]` pairs |
| `feature_scalar` | `[f64; 2]` | Complex scalar as `[re, im]` |

Note: `matrix_dim` is NOT present. The dimension is derivable from `feature_arr.length`.

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v2/inference/features \
  -H "Content-Type: application/json" \
  -d '{
    "matrix": [
      [[1.0, 0.0], [0.0, 0.0]],
      [[0.0, 0.0], [-1.0, 0.0]]
    ]
  }' | jq '.feature_scalar'
```

---

### POST /v2/active-inference

Stateless combined operation: returns features and total relative entropy in one call.

**Request body** — same shape as `/v2/inference/entropy`.

**Response — 200 OK**

```json
{
  "feature_arr": [[[0.731, 0.0], [0.0, 0.0]], [[0.0, 0.0], [0.269, 0.0]]],
  "feature_vec": [[0.365, 0.0], [0.135, 0.0], [0.365, 0.0], [0.135, 0.0]],
  "feature_scalar": [0.25, 0.0],
  "total_relative_entropy": 2.1403
}
```

| Field | Type | Description |
|-------|------|-------------|
| `feature_arr` | `[[[f64; 2]]]` | N×N feature array |
| `feature_vec` | `[[f64; 2]]` | 2N feature vector |
| `feature_scalar` | `[f64; 2]` | Complex scalar |
| `total_relative_entropy` | f32 | Shannon entropy scalar |

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v2/active-inference \
  -H "Content-Type: application/json" \
  -d '{
    "matrix": [
      [[1.0, 0.0], [0.0, 0.0]],
      [[0.0, 0.0], [-1.0, 0.0]]
    ]
  }'
```

---

### POST /v2/active-inference/entropy

Stateless entropy using the active-inference variant.

**Request body** — same shape as `/v2/inference/entropy`.

**Response — 200 OK**

```json
{"total_relative_entropy": 2.1403}
```

| Field | Type | Description |
|-------|------|-------------|
| `total_relative_entropy` | f32 | Shannon entropy scalar |

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v2/active-inference/entropy \
  -H "Content-Type: application/json" \
  -d '{
    "matrix": [
      [[1.0, 0.0], [0.0, 0.0]],
      [[0.0, 0.0], [-1.0, 0.0]]
    ]
  }'
```

---

### POST /v2/active-inference/features

Stateless feature extraction using the active-inference variant.

**Request body** — same shape as `/v2/inference/entropy`.

**Response — 200 OK** — same shape as `/v2/inference/features`.

```json
{
  "feature_arr": [[[0.731, 0.0], [0.0, 0.0]], [[0.0, 0.0], [0.269, 0.0]]],
  "feature_vec": [[0.365, 0.0], [0.135, 0.0], [0.365, 0.0], [0.135, 0.0]],
  "feature_scalar": [0.25, 0.0]
}
```

**Error responses**: 400, 413, 422, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v2/active-inference/features \
  -H "Content-Type: application/json" \
  -d '{
    "matrix": [
      [[1.0, 0.0], [0.0, 0.0]],
      [[0.0, 0.0], [-1.0, 0.0]]
    ]
  }'
```

---

### POST /v2/forward-sample

Generates a reproducible random sample of dimension N and runs the inference pipeline. No matrix is sent by the caller.

**Request body**

```json
{
  "matrix_dim": 4,
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
  "feature_array": [
    [[0.12, 0.03], [0.07, -0.01]],
    [[0.07, -0.01], [0.11, 0.02]]
  ],
  "feature_vector": [[0.095, 0.01], [0.09, 0.005], [0.095, 0.01], [0.09, 0.005]],
  "feature_scalar": [0.0925, 0.0075],
  "relative_entropy": 3.4102
}
```

| Field | Type | Description |
|-------|------|-------------|
| `feature_array` | `[[[f64; 2]]]` | N×N feature array as nested `[re, im]` pairs |
| `feature_vector` | `[[f64; 2]]` | 2N feature vector as `[re, im]` pairs |
| `feature_scalar` | `[f64; 2]` | Complex scalar as `[re, im]` |
| `relative_entropy` | f32 | Shannon entropy of the generated sample |

Note: the field names are `feature_array` and `feature_vector` (full words), not `feature_arr` / `feature_vec`. This matches the v1 `/v1/forward-sample` naming convention.

**Error responses**: 400, 413, 500, 503.

**Example**

```bash
curl -s -X POST http://localhost:8080/v2/forward-sample \
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
| Maximum matrix dimension N (v1) | 1024 (default) | `--max-matrix-dim` / `KIRK_MAX_MATRIX_DIM` (range 1..=4096) |
| Maximum matrix dimension N (v2, practical) | ~300 | Same flag; limited by JSON payload size at the 64 MiB body cap |
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
| `bad_request` | 400 | Malformed JSON, invalid base64, wrong decoded byte length, N < 2, jagged v2 rows, non-finite v2 element |
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

**v2 practical ceiling** — At the 64 MiB body cap, a compact JSON encoding of a v2 matrix with N=300 is approximately 54 MiB (300×300×2 floats × ~30 bytes per `[re, im]` pair). N=300 is a safe working ceiling for v2. For larger matrices, use v1.

**Blocking threshold** — For N < 128, eigendecomposition runs inline on the tokio async thread (sub-millisecond). For N >= 128, it is dispatched to `tokio::task::spawn_blocking` to avoid stalling the reactor. Expect one extra task-switch per large request.

**Global mutex** — All three transports share one `parking_lot::Mutex` over the model handle. The mutex is held only for the compute call. High REST concurrency at small N (e.g., N=32) will queue at this mutex. For throughput-critical workloads at small N, prefer the gRPC or TCP transports, which share the same bottleneck but have lower per-request overhead.

**Prometheus metrics** — Scrape `GET /metrics` to observe per-operation request counts and latency histograms. v1 labels: `transport=rest`, `op=<forward|inference_entropy|...>`. v2 labels: `transport=rest`, `op=<forward_v2|inference_entropy_v2|...>`. The distinct `_v2` suffix allows operators to split v1 and v2 traffic in dashboards.

**When to use gRPC or TCP instead** — The REST transport encodes matrices as base64 (v1) or JSON (v2), adding wire overhead and a CPU decode step on the server. If you need maximum throughput (especially at N >= 32), use the gRPC transport (port 50051, raw bytes in protobuf) or the custom TCP transport (port 9090, zero-copy binary framing). See [ARCHITECTURE.md](ARCHITECTURE.md) for a benchmark comparison.

---

## Configuration Cheat-Sheet

All CLI flags accept a corresponding environment variable of the form `KIRK_<UPPERCASE_FLAG>`.

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--rest-port` | `KIRK_REST_PORT` | `8080` | REST listener port |
| `--bind` | `KIRK_BIND` | `0.0.0.0` | Bind address for all listeners. Use `127.0.0.1` on non-Docker hosts |
| `--max-matrix-dim` | `KIRK_MAX_MATRIX_DIM` | `1024` | Hard cap on N (1..=4096) |
| `--temperature` | `KIRK_TEMPERATURE` | `1.0` | Boltzmann temperature for softmax weighting (tiberius backend only) |
| `--window-size` | `KIRK_WINDOW_SIZE` | `256` | Rolling-window depth for entropy z-score (tiberius backend only) |
| `--workers` | `KIRK_WORKERS` | `0` (= num_cpus) | Tokio worker threads |
| `--log-level` | `KIRK_LOG_LEVEL` | `info` | tracing-subscriber filter string |
| `--grpc-port` | `KIRK_GRPC_PORT` | `50051` | gRPC listener port (not REST-specific but shares `--bind`) |
| `--tcp-port` | `KIRK_TCP_PORT` | `9090` | Custom TCP listener port |
| `--model` | `KIRK_MODEL` | `tiberius` | Model selector: `tiberius` or `kirk`. See [MODELS.md](MODELS.md) |
| `--env` | `KIRK_ENV` | `local` | Runtime environment: `local` or `prod`. See [SECURE_BUILD.md](SECURE_BUILD.md) |

**Common Docker Compose overrides**

```bash
# Expose REST on a non-default port
KIRK_REST_PORT=9080 docker compose up -d kirk-server

# Smaller matrix dimension cap in a constrained environment
KIRK_MAX_MATRIX_DIM=256 docker compose up -d kirk-server

# Increase worker threads for a high-core machine
KIRK_WORKERS=16 docker compose up -d kirk-server

# Use the kirk model backend
KIRK_MODEL=kirk docker compose up -d kirk-server
```

**Restrict to loopback on a non-Docker host**

```bash
cargo run --release -p kirk-server -- --bind 127.0.0.1
```

---

## Versioning and Stability

Compute endpoints are prefixed with `/v1/` or `/v2/`. Both versions are stable. Breaking changes (field removals, type changes) will bump the prefix. Additive changes (new optional request fields, new response fields) may be introduced within a version without notice — clients should ignore unknown JSON fields.

`/healthz` and `/metrics` are unversioned utility endpoints and will not be moved.

For the full API changelog and architecture decisions, see [ARCHITECTURE.md](ARCHITECTURE.md) and [../kirk-server/README.md](../kirk-server/README.md).
