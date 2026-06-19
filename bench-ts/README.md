# bench-ts

Bun TypeScript benchmark harness for the `kirk-server` multi-protocol surface. Drives concurrent virtual users against gRPC, REST, or the custom TCP transport, collects per-request latencies in a preallocated `Float64Array`, and computes p50/p95/p99 throughput summaries with side-by-side transport comparison.

## Requirements

- Bun 1.x — required at runtime. The `tcp` transport uses `Bun.connect()`; the harness refuses to run that transport under plain Node.
  ```bash
  curl -fsSL https://bun.sh/install | bash
  ```
- `kirk-server` running on the target host (defaults: gRPC :50051, REST :8080, TCP :9090)
- `protoc` is not required by the bench (it loads `kirk.proto` at runtime via `@grpc/proto-loader`)

## Installation

```bash
cd bench-ts
bun install
```

## `run` Subcommand

```bash
bun src/cli.ts run --transport <grpc|rest|tcp> [options]
```

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--transport` | (required) | `grpc`, `rest`, or `tcp` |
| `--host` | `localhost` | Server hostname or IP |
| `--port` | per-transport | 50051 (gRPC), 8080 (REST), 9090 (TCP) |
| `--users` | `10` | Number of concurrent virtual users |
| `--duration` | `30s` | Measurement duration (e.g. `30s`, `2m`). Mutually exclusive with `--requests`. |
| `--requests` | — | Total request count. Mutually exclusive with `--duration`. |
| `--matrix-size` | `32` | Matrix dimension N |
| `--temperature` | `1.0` | Boltzmann temperature passed to each request |
| `--seed` | `42` | 64-bit seed for the Xoshiro256** RNG (reproducible payloads) |
| `--warmup` | `2s` | Warmup duration (results discarded) |
| `--output` | `results/<UTC>.json` | JSON result file path |
| `--op` | `forward` | Operation: `forward`, `inference_entropy`, `inference_features`, `active_inference`, `forward_sample` |

### Examples

```bash
# TCP, 100 users, 30 seconds, N=32
bun src/cli.ts run --transport tcp --users 100 --duration 30s --matrix-size 32

# gRPC, 50 users, 10k total requests
bun src/cli.ts run --transport grpc --users 50 --requests 10000

# REST, 20 users, inference_entropy only
bun src/cli.ts run --transport rest --users 20 --duration 60s --op inference_entropy

# Pin to a remote server
bun src/cli.ts run --transport tcp --host 10.0.1.5 --port 9090 --users 50 --duration 30s
```

### Reproducibility

Each virtual user draws matrices from a pre-generated `MatrixPool` of 1024 matrices built from the seeded RNG at startup. Identical `--seed` values produce identical request payloads for any given `--matrix-size`, regardless of concurrency. The pool is filled during startup before the warmup phase; generation cost is excluded from latency measurements.

## `compare` Subcommand

```bash
bun src/cli.ts compare results/a.json results/b.json results/c.json
```

Prints a side-by-side table with relative deltas. Example output:

```
                     tcp           grpc          rest
requests_total       312450        285100        88320
throughput_rps       10415         9503          2944
latency p50          89us          102us         335us
latency p95          142us         178us         542us
latency p99          220us         310us         820us
latency max          5100us        4800us        12000us
  vs tcp                           +25% p95      +282% p95
```

## Output JSON Schema

Each run produces a JSON file defined by `src/results.ts`:

```typescript
interface ResultFile {
  meta: {
    transport: string;      // "grpc" | "rest" | "tcp"
    host: string;
    port: number;
    users: number;
    matrix_size: number;
    op: string;
    seed: string;           // BigInt serialized as string
    warmup: string;
    duration?: string;
    requests?: number;
    started_at: string;     // ISO 8601
    finished_at: string;
  };
  summary: Summary;
}

interface Summary {
  requests_total: number;
  errors_total: number;
  duration_s: number;
  throughput_rps: number;
  latency_ns: {
    min: number;
    p50: number;
    p90: number;
    p95: number;
    p99: number;
    p999: number;
    max: number;
    mean: number;
  };
  bytes_in_total: number;
  bytes_out_total: number;
  concurrency_actual: number;
}
```

Latencies are in nanoseconds. The source of truth for these types is `src/results.ts` and `src/summary.ts`.

## Connection-Reuse Model (FR-036)

All three transports reuse connections across requests. No transport performs a connection handshake per request. This is required for a meaningful throughput comparison.

| Transport | Connection model | Pipelining |
|-----------|-----------------|------------|
| REST | One HTTP/1.1 keepalive connection per virtual user (Bun `fetch` with `connection: keep-alive`) | Serial: one request at a time per connection |
| gRPC | One HTTP/2 channel per virtual user (`@grpc/grpc-js`; streams multiplexed on a single TCP connection) | HTTP/2 stream multiplexing |
| TCP | One persistent socket per virtual user (`Bun.connect()`); requests pipelined with `req_id` correlation | Full pipelining: multiple outstanding requests per socket |

The TCP transport maintains a `Map<req_id, resolver>` to correlate responses to outstanding requests. The bench-ts runner does not pipeline within a single `userLoop` iteration (it awaits one response before the next), but the underlying socket stays open across iterations. This is sufficient to demonstrate the transport-level latency difference without introducing client-side pipelining complexity.

The expected throughput ordering is `tcp_rps >= grpc_rps >= rest_rps` at matched concurrency. The expected p95 ordering is `tcp_p95 <= grpc_p95 <= rest_p95`.

## End-to-End Run with Docker Compose

```bash
# Start the server
docker compose up -d kirk-server

# Bench each transport
TRANSPORT=tcp  USERS=100 DURATION=30s MATRIX_SIZE=32 docker compose run --rm bench
TRANSPORT=grpc USERS=100 DURATION=30s MATRIX_SIZE=32 docker compose run --rm bench
TRANSPORT=rest USERS=100 DURATION=30s MATRIX_SIZE=32 docker compose run --rm bench

# Results land in bench-ts/results/ (bind-mounted from the bench container)
bun src/cli.ts compare bench-ts/results/*.json
```

Or via the Makefile:

```bash
make up         # docker compose up -d kirk-server
make bench-all  # runs rest, grpc, tcp back-to-back
make compare    # bun src/cli.ts compare results/*.json
```

## Notes

- The harness checks `typeof Bun !== "undefined"` at startup and throws if the `tcp` transport is requested on plain Node.
- Warmup results are discarded; only post-warmup latencies are included in the output.
- The `Float64Array` for latencies is preallocated to `max(1024, users * 1024)` entries to avoid GC pressure during measurement. Entries beyond the preallocated capacity are dropped (only relevant for very long runs at high user counts).
- The matrix pool is a circular buffer of 1024 random matrices. At pool exhaustion, indices wrap. This provides fresh-looking inputs while excluding generation cost from measurement.
