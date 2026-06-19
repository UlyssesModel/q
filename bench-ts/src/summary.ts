/** Sorted-array percentile aggregator. */

export interface Summary {
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

function quantile(sorted: Float64Array, q: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.min(sorted.length - 1, Math.max(0, Math.floor(q * (sorted.length - 1))));
  return sorted[idx];
}

export function summarize(
  latencies: Float64Array,
  count: number,
  errors: number,
  durationS: number,
  bytesIn: number,
  bytesOut: number,
  concurrency: number,
): Summary {
  const slice = latencies.slice(0, count);
  slice.sort();
  const min = slice.length ? slice[0] : 0;
  const max = slice.length ? slice[slice.length - 1] : 0;
  let sum = 0;
  for (let i = 0; i < slice.length; i++) sum += slice[i];
  const mean = slice.length ? sum / slice.length : 0;
  return {
    requests_total: count,
    errors_total: errors,
    duration_s: durationS,
    throughput_rps: durationS > 0 ? count / durationS : 0,
    latency_ns: {
      min,
      p50: quantile(slice, 0.5),
      p90: quantile(slice, 0.9),
      p95: quantile(slice, 0.95),
      p99: quantile(slice, 0.99),
      p999: quantile(slice, 0.999),
      max,
      mean,
    },
    bytes_in_total: bytesIn,
    bytes_out_total: bytesOut,
    concurrency_actual: concurrency,
  };
}

export function prettyTable(s: Summary, label: string): string {
  const fmtMs = (ns: number) => (ns / 1_000_000).toFixed(2) + "ms";
  const lines = [
    `--- ${label} ---`,
    `requests   : ${s.requests_total}`,
    `errors     : ${s.errors_total}`,
    `duration_s : ${s.duration_s.toFixed(2)}`,
    `throughput : ${s.throughput_rps.toFixed(1)} rps`,
    `latency p50: ${fmtMs(s.latency_ns.p50)}`,
    `latency p95: ${fmtMs(s.latency_ns.p95)}`,
    `latency p99: ${fmtMs(s.latency_ns.p99)}`,
    `latency max: ${fmtMs(s.latency_ns.max)}`,
    `bytes in   : ${s.bytes_in_total}`,
    `bytes out  : ${s.bytes_out_total}`,
    `users      : ${s.concurrency_actual}`,
  ];
  return lines.join("\n");
}
