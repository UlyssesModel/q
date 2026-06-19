//! Minimal Prometheus text-format counters/histograms. Pure stdlib; no extra
//! dependencies needed for the bench scope.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Histogram buckets in microseconds.
const BUCKETS_US: &[f64] = &[
    50.0,
    100.0,
    200.0,
    500.0,
    1_000.0,
    2_000.0,
    5_000.0,
    10_000.0,
    25_000.0,
    50_000.0,
    100_000.0,
    250_000.0,
    500_000.0,
    1_000_000.0,
];

#[derive(Default)]
struct Histogram {
    counts: [u64; 15], // 14 buckets + inf
    sum_us: f64,
    total: u64,
}

#[derive(Default)]
pub struct Metrics {
    requests_total: HashMap<String, AtomicU64>,
    errors_total: HashMap<String, AtomicU64>,
    histograms: HashMap<String, Histogram>,
}

pub struct MetricsHandle {
    inner: Arc<Mutex<Metrics>>,
}

impl Default for MetricsHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MetricsHandle {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl MetricsHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Metrics::default())),
        }
    }

    pub fn observe(&self, transport: &str, op: &str, latency_us: f64, ok: bool) {
        let label = format!("{transport}|{op}");
        let mut g = self.inner.lock();
        g.requests_total
            .entry(label.clone())
            .or_insert_with(|| AtomicU64::new(0));
        g.requests_total
            .get(&label)
            .unwrap()
            .fetch_add(1, Ordering::Relaxed);
        if !ok {
            g.errors_total
                .entry(label.clone())
                .or_insert_with(|| AtomicU64::new(0));
            g.errors_total
                .get(&label)
                .unwrap()
                .fetch_add(1, Ordering::Relaxed);
        }
        let h = g.histograms.entry(label).or_default();
        h.sum_us += latency_us;
        h.total += 1;
        let mut placed = false;
        for (i, &b) in BUCKETS_US.iter().enumerate() {
            if latency_us <= b {
                h.counts[i] += 1;
                placed = true;
                break;
            }
        }
        if !placed {
            h.counts[BUCKETS_US.len()] += 1;
        }
    }

    pub fn render_prometheus(&self) -> String {
        let g = self.inner.lock();
        let mut s = String::new();
        s.push_str("# HELP kirk_requests_total Number of requests processed.\n");
        s.push_str("# TYPE kirk_requests_total counter\n");
        for (label, count) in g.requests_total.iter() {
            let (t, op) = label.split_once('|').unwrap_or(("?", "?"));
            s.push_str(&format!(
                "kirk_requests_total{{transport=\"{t}\",op=\"{op}\"}} {}\n",
                count.load(Ordering::Relaxed)
            ));
        }
        s.push_str("# HELP kirk_errors_total Number of failed requests.\n");
        s.push_str("# TYPE kirk_errors_total counter\n");
        for (label, count) in g.errors_total.iter() {
            let (t, op) = label.split_once('|').unwrap_or(("?", "?"));
            s.push_str(&format!(
                "kirk_errors_total{{transport=\"{t}\",op=\"{op}\"}} {}\n",
                count.load(Ordering::Relaxed)
            ));
        }
        s.push_str("# HELP kirk_request_latency_us Per-request latency in microseconds.\n");
        s.push_str("# TYPE kirk_request_latency_us histogram\n");
        for (label, h) in g.histograms.iter() {
            let (t, op) = label.split_once('|').unwrap_or(("?", "?"));
            let mut cumulative = 0u64;
            for (i, &b) in BUCKETS_US.iter().enumerate() {
                cumulative += h.counts[i];
                s.push_str(&format!(
                    "kirk_request_latency_us_bucket{{transport=\"{t}\",op=\"{op}\",le=\"{b}\"}} {cumulative}\n",
                ));
            }
            cumulative += h.counts[BUCKETS_US.len()];
            s.push_str(&format!(
                "kirk_request_latency_us_bucket{{transport=\"{t}\",op=\"{op}\",le=\"+Inf\"}} {cumulative}\n",
            ));
            s.push_str(&format!(
                "kirk_request_latency_us_sum{{transport=\"{t}\",op=\"{op}\"}} {}\n",
                h.sum_us
            ));
            s.push_str(&format!(
                "kirk_request_latency_us_count{{transport=\"{t}\",op=\"{op}\"}} {}\n",
                h.total
            ));
        }
        s
    }
}
