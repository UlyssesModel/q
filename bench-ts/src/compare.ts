import { readFile } from "node:fs/promises";
import type { ResultFile } from "./results.ts";

function pct(x: number): string {
  const sign = x >= 0 ? "+" : "";
  return `${sign}${x.toFixed(0)}%`;
}

export async function compare(paths: string[]): Promise<void> {
  if (paths.length === 0) {
    console.error("usage: bench compare <result.json> [...]");
    process.exit(2);
  }
  const results: ResultFile[] = [];
  for (const p of paths) {
    const text = await readFile(p, "utf-8");
    results.push(JSON.parse(text) as ResultFile);
  }
  // Baseline = REST if present, else first.
  const baselineIdx = results.findIndex((r) => r.meta.transport === "rest");
  const baseline = results[baselineIdx >= 0 ? baselineIdx : 0];

  const rows: string[][] = [];
  const headers = ["transport", "users", "N", "rps", "p50ms", "p95ms", "p99ms", "vs rest p95"];
  rows.push(headers);
  for (const r of results) {
    const s = r.summary;
    const dp95 = baseline.summary.latency_ns.p95
      ? ((s.latency_ns.p95 - baseline.summary.latency_ns.p95) / baseline.summary.latency_ns.p95) * 100
      : 0;
    rows.push([
      r.meta.transport,
      String(r.meta.users),
      String(r.meta.matrix_size),
      s.throughput_rps.toFixed(1),
      (s.latency_ns.p50 / 1_000_000).toFixed(2),
      (s.latency_ns.p95 / 1_000_000).toFixed(2),
      (s.latency_ns.p99 / 1_000_000).toFixed(2),
      pct(dp95),
    ]);
  }
  const widths = headers.map((_, c) => Math.max(...rows.map((r) => r[c].length)));
  for (const r of rows) {
    console.log(r.map((cell, c) => cell.padEnd(widths[c])).join("  "));
  }
}
