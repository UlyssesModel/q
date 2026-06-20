import { Xoshiro256ss } from "./rng.ts";
import { MatrixPool } from "./matrix.ts";
import type { TransportClient } from "./transports/types.ts";
import { RestClient } from "./transports/rest.ts";
import { GrpcClient } from "./transports/grpc.ts";
import { TcpClient } from "./transports/tcp.ts";
import { summarize, prettyTable } from "./summary.ts";
import { writeResult, type ResultFile, type RunOptions } from "./results.ts";

function parseDurationMs(s: string): number {
  const m = s.match(/^(\d+(?:\.\d+)?)(ms|s|m)$/i);
  if (!m) throw new Error(`bad duration ${s}`);
  const n = Number(m[1]);
  switch (m[2].toLowerCase()) {
    case "ms": return n;
    case "s":  return n * 1000;
    case "m":  return n * 60_000;
    default: throw new Error(`bad duration ${s}`);
  }
}

function defaultPort(transport: string): number {
  return transport === "grpc" ? 50051 : transport === "rest" ? 8080 : 9090;
}

function makeTransport(opts: RunOptions): TransportClient {
  switch (opts.transport) {
    case "rest": return new RestClient(opts.host, opts.port, opts.apiVersion);
    case "grpc": return new GrpcClient(opts.host, opts.port);
    case "tcp":  return new TcpClient(opts.host, opts.port);
  }
}

export async function run(rawOpts: Partial<RunOptions>): Promise<void> {
  const opts: RunOptions = {
    transport: rawOpts.transport ?? "rest",
    apiVersion: rawOpts.apiVersion ?? "v1",
    host: rawOpts.host ?? "localhost",
    port: rawOpts.port ?? defaultPort(rawOpts.transport ?? "rest"),
    users: rawOpts.users ?? 10,
    duration: rawOpts.duration,
    requests: rawOpts.requests,
    matrixSize: rawOpts.matrixSize ?? 32,
    temperature: rawOpts.temperature ?? 1.0,
    seed: rawOpts.seed ?? 42n,
    warmup: rawOpts.warmup ?? "2s",
    output: rawOpts.output ?? `results/${new Date().toISOString().replace(/[:.]/g, "-")}-${rawOpts.transport ?? "rest"}.json`,
    op: rawOpts.op ?? "forward",
  };

  if (!opts.duration && !opts.requests) {
    opts.duration = "30s";
  }

  if (opts.transport === "tcp" && typeof (globalThis as { Bun?: unknown }).Bun === "undefined") {
    throw new Error("tcp transport requires Bun runtime (process.versions.bun)");
  }

  const rng = new Xoshiro256ss(opts.seed);
  const pool = new MatrixPool(rng, opts.matrixSize, 1024);

  const startedAt = new Date().toISOString();
  const capacity = opts.requests ?? Math.max(1024, opts.users * 1024);
  const latencies = new Float64Array(capacity * 4);
  let count = 0;
  let errors = 0;
  let bytesIn = 0;
  let bytesOut = 0;

  // Warmup: open transports, run a single forward per user, discard.
  const transports: TransportClient[] = [];
  for (let i = 0; i < opts.users; i++) {
    const t = makeTransport(opts);
    await t.open();
    transports.push(t);
  }

  const warmupMs = parseDurationMs(opts.warmup);
  const warmupDeadline = Date.now() + warmupMs;
  await Promise.all(transports.map(async (t) => {
    while (Date.now() < warmupDeadline) {
      try {
        await t.forward(pool.next(), BigInt(Date.now() * 1000));
      } catch {
        // ignore during warmup
      }
    }
  }));

  // Measurement loop.
  const measureStart = process.hrtime.bigint();
  const deadlineNs = opts.duration ? measureStart + BigInt(parseDurationMs(opts.duration) * 1_000_000) : undefined;
  const requestsCap = opts.requests;

  async function userLoop(t: TransportClient): Promise<void> {
    while (true) {
      const now = process.hrtime.bigint();
      if (deadlineNs && now >= deadlineNs) return;
      if (requestsCap && count >= requestsCap) return;
      const m = pool.next();
      const t0 = process.hrtime.bigint();
      try {
        const out = await t.forward(m, BigInt(Date.now() * 1000));
        const dt = Number(process.hrtime.bigint() - t0);
        const idx = count;
        count += 1;
        if (idx < latencies.length) {
          latencies[idx] = dt;
        }
        bytesIn += out.bytesIn;
        bytesOut += out.bytesOut;
      } catch {
        errors += 1;
      }
    }
  }

  await Promise.all(transports.map(userLoop));
  const durationS = Number(process.hrtime.bigint() - measureStart) / 1e9;

  await Promise.all(transports.map((t) => t.close()));

  const summary = summarize(latencies, Math.min(count, latencies.length), errors, durationS, bytesIn, bytesOut, opts.users);
  const file: ResultFile = {
    meta: {
      transport: opts.transport,
      api_version: opts.apiVersion,
      host: opts.host,
      port: opts.port,
      users: opts.users,
      matrix_size: opts.matrixSize,
      op: opts.op,
      seed: opts.seed.toString(),
      warmup: opts.warmup,
      duration: opts.duration,
      requests: opts.requests,
      started_at: startedAt,
      finished_at: new Date().toISOString(),
    },
    summary,
  };
  await writeResult(opts.output, file);
  console.log(prettyTable(summary, `${opts.transport} @ ${opts.host}:${opts.port} N=${opts.matrixSize}`));
  console.log(`wrote ${opts.output}`);
}
