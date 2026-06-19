//! CLI configuration via clap.

use clap::Parser;

/// Hard upper bound on `--max-matrix-dim` (architect spec § Security Considerations S-001).
pub const MAX_ALLOWED_MATRIX_DIM: u32 = 4096;

/// Default TCP accept-loop semaphore cap.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 1024;

/// Default per-connection in-flight frame semaphore cap.
pub const DEFAULT_MAX_IN_FLIGHT: u32 = 128;

/// Default TCP write timeout (ms) for the per-connection writer task.
pub const DEFAULT_TCP_WRITE_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "kirk-server",
    about = "Kirk realistic stub — multi-protocol server",
    version
)]
pub struct Config {
    /// gRPC listener port.
    #[arg(long, env = "KIRK_GRPC_PORT", default_value_t = 50051)]
    pub grpc_port: u16,

    /// REST listener port.
    #[arg(long, env = "KIRK_REST_PORT", default_value_t = 8080)]
    pub rest_port: u16,

    /// Custom TCP listener port.
    #[arg(long, env = "KIRK_TCP_PORT", default_value_t = 9090)]
    pub tcp_port: u16,

    /// Listener bind address. Honored by all three transports.
    #[arg(long, env = "KIRK_BIND", default_value = "0.0.0.0")]
    pub bind: String,

    /// Tokio worker threads. 0 = num_cpus.
    #[arg(long, env = "KIRK_WORKERS", default_value_t = 0)]
    pub workers: usize,

    /// Boltzmann temperature.
    #[arg(long, env = "KIRK_TEMPERATURE", default_value_t = 1.0)]
    pub temperature: f32,

    /// Rolling-window size for z-score.
    #[arg(long, env = "KIRK_WINDOW_SIZE", default_value_t = 256)]
    pub window_size: usize,

    /// Hard cap on matrix dimension N. Clamped to `[2, 4096]` per spec S-001.
    /// (N must be >= 2 — N=1 is rejected upstream by `KirkBackend::check_dim`.)
    #[arg(
        long,
        env = "KIRK_MAX_MATRIX_DIM",
        default_value_t = 1024,
        value_parser = clap::value_parser!(u32).range(1..=4096),
    )]
    pub max_matrix_dim: u32,

    /// Maximum concurrent TCP connections accepted by the custom-TCP listener.
    /// Bounds the per-listener resource consumption (file descriptors + tasks).
    #[arg(
        long,
        env = "KIRK_MAX_CONNECTIONS",
        default_value_t = DEFAULT_MAX_CONNECTIONS,
        value_parser = clap::value_parser!(u32).range(1..=65535),
    )]
    pub max_connections: u32,

    /// Maximum in-flight frames per TCP connection. Bounds the per-connection
    /// task-spawn explosion when a client pipelines without reading responses.
    #[arg(
        long,
        env = "KIRK_MAX_IN_FLIGHT_PER_CONN",
        default_value_t = DEFAULT_MAX_IN_FLIGHT,
        value_parser = clap::value_parser!(u32).range(1..=65535),
    )]
    pub max_in_flight_per_conn: u32,

    /// Per-write timeout (ms) on the TCP writer task. Defends against slow-reader
    /// / slowloris-style clients.
    #[arg(
        long,
        env = "KIRK_TCP_WRITE_TIMEOUT_MS",
        default_value_t = DEFAULT_TCP_WRITE_TIMEOUT_MS,
        value_parser = clap::value_parser!(u64).range(100..=600_000),
    )]
    pub tcp_write_timeout_ms: u64,

    /// Log level (env_filter compatible: `info`, `debug`, `kirk_server=debug,info`, ...).
    #[arg(long, env = "KIRK_LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// One-shot health probe: GET `http://127.0.0.1:<rest-port>/healthz`, print
    /// the response status, exit 0 on HTTP 200 else 1. Bypasses the full
    /// server startup. Used by the docker-compose healthcheck because the
    /// distroless runtime image has neither `wget` nor a shell — see SEC-009.
    #[arg(long)]
    pub healthcheck: bool,
}

impl Config {
    pub fn worker_threads(&self) -> usize {
        if self.workers == 0 {
            num_cpus::get()
        } else {
            self.workers
        }
    }
}
