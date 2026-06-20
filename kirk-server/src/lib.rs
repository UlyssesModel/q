//! Library entry point for `kirk-server`. Exposes the server infrastructure so
//! integration tests can spin up an in-process server on ephemeral ports without
//! going through `std::process::Command`.

#![forbid(unsafe_code)]

pub mod backend;
pub mod backends;
pub mod config;
pub mod error;
pub mod grpc;
pub mod metrics;
pub mod model;
pub mod rest;
pub mod shutdown;
pub mod tcp;

pub use backend::KirkBackend;
pub use config::{
    Config, Env, Model, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_IN_FLIGHT,
    DEFAULT_TCP_WRITE_TIMEOUT_MS,
};
pub use metrics::MetricsHandle;
pub use shutdown::ShutdownHandle;

use std::sync::Arc;
use std::time::Duration;

/// Ports that the server has bound to (may differ from requested when using port 0).
pub struct BoundPorts {
    pub grpc: u16,
    pub rest: u16,
    pub tcp: u16,
}

/// Handle returned by `start_server`. Drop it (or call `shutdown`) to stop the
/// server.
pub struct ServerHandle {
    pub ports: BoundPorts,
    shutdown: ShutdownHandle,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl ServerHandle {
    /// Send the shutdown signal and wait for all listener tasks to finish.
    pub async fn shutdown(mut self) {
        self.shutdown.fire();
        for t in self.tasks.drain(..) {
            let _ = t.await;
        }
    }
}

/// Maximum-size wire payload (mirrors `tcp::framing::MAX_PAYLOAD`).
/// SEC-006: aligns the gRPC message cap with REST + TCP.
const MAX_GRPC_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// Build-out parameters for `start_server_with`. Filled in by `Config` at
/// runtime; tests use the legacy `start_server(...)` shim which fills in
/// sane defaults.
#[derive(Debug, Clone)]
pub struct ServerSettings {
    pub bind: String,
    pub grpc_port: u16,
    pub rest_port: u16,
    pub tcp_port: u16,
    pub temperature: f32,
    pub window_size: usize,
    pub max_matrix_dim: u32,
    pub max_connections: u32,
    pub max_in_flight_per_conn: u32,
    pub tcp_write_timeout: Duration,
    pub model: Model,
    pub env: Env,
}

impl ServerSettings {
    /// Build from a parsed `Config`.
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            bind: cfg.bind.clone(),
            grpc_port: cfg.grpc_port,
            rest_port: cfg.rest_port,
            tcp_port: cfg.tcp_port,
            temperature: cfg.temperature,
            window_size: cfg.window_size,
            max_matrix_dim: cfg.max_matrix_dim,
            max_connections: cfg.max_connections,
            max_in_flight_per_conn: cfg.max_in_flight_per_conn,
            tcp_write_timeout: Duration::from_millis(cfg.tcp_write_timeout_ms),
            model: cfg.model,
            env: cfg.env,
        }
    }

    /// Recover a partial `Config` that's sufficient for `backend::KirkBackend::from_config`.
    fn to_config(&self) -> Config {
        Config {
            grpc_port: self.grpc_port,
            rest_port: self.rest_port,
            tcp_port: self.tcp_port,
            bind: self.bind.clone(),
            workers: 0,
            temperature: self.temperature,
            window_size: self.window_size,
            max_matrix_dim: self.max_matrix_dim,
            max_connections: self.max_connections,
            max_in_flight_per_conn: self.max_in_flight_per_conn,
            tcp_write_timeout_ms: self.tcp_write_timeout.as_millis() as u64,
            log_level: "info".to_string(),
            healthcheck: false,
            model: self.model,
            env: self.env,
        }
    }
}

/// Backwards-compatible shim used by integration tests. Always binds to
/// `127.0.0.1` (loopback) and uses the default DoS caps.
pub async fn start_server(
    grpc_port: u16,
    rest_port: u16,
    tcp_port: u16,
    temperature: f32,
    window_size: usize,
    max_matrix_dim: u32,
) -> anyhow::Result<ServerHandle> {
    let settings = ServerSettings {
        bind: "127.0.0.1".to_string(),
        grpc_port,
        rest_port,
        tcp_port,
        temperature,
        window_size,
        max_matrix_dim,
        max_connections: DEFAULT_MAX_CONNECTIONS,
        max_in_flight_per_conn: DEFAULT_MAX_IN_FLIGHT,
        tcp_write_timeout: Duration::from_millis(DEFAULT_TCP_WRITE_TIMEOUT_MS),
        model: Model::default(),
        env: Env::default(),
    };
    start_server_with(settings).await
}

/// Spawn a full three-transport server using the given `settings`.
pub async fn start_server_with(settings: ServerSettings) -> anyhow::Result<ServerHandle> {
    let cfg = settings.to_config();
    let backend =
        KirkBackend::from_config(&cfg).map_err(|e| anyhow::anyhow!("backend init failed: {e}"))?;
    tracing::info!(backend = backend.name(), "backend selected");
    let metrics = MetricsHandle::new();
    let shutdown = ShutdownHandle::new();
    let shutdown_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let bind = settings.bind.as_str();

    // --- gRPC ---
    let grpc_listener =
        tokio::net::TcpListener::bind(format!("{bind}:{}", settings.grpc_port)).await?;
    let grpc_bound = grpc_listener.local_addr()?.port();
    let svc = grpc::KirkSvc {
        backend: backend.clone(),
        metrics: metrics.clone(),
    };
    let grpc_svc = grpc::service::export::KirkServiceServer::new(svc)
        // SEC-006: align decoding cap with REST + TCP. Default tonic cap is 4 MiB.
        .max_decoding_message_size(MAX_GRPC_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_GRPC_MESSAGE_BYTES);
    let mut grpc_shutdown = shutdown.subscribe();
    let grpc_handle = tokio::spawn(async move {
        let serve = tonic::transport::Server::builder()
            .tcp_nodelay(true)
            .add_service(grpc_svc)
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(grpc_listener),
                async move {
                    let _ = grpc_shutdown.recv().await;
                },
            );
        if let Err(e) = serve.await {
            tracing::debug!(error=?e, "grpc server exited");
        }
    });

    // --- REST ---
    let rest_listener =
        tokio::net::TcpListener::bind(format!("{bind}:{}", settings.rest_port)).await?;
    let rest_bound = rest_listener.local_addr()?.port();
    let rest_state = rest::routes::RestState {
        backend: backend.clone(),
        metrics: metrics.clone(),
        shutdown: shutdown_flag.clone(),
    };
    let app = rest::build_router(rest_state);
    let mut rest_shutdown = shutdown.subscribe();
    let rest_handle = tokio::spawn(async move {
        let serve = axum::serve(rest_listener, app).with_graceful_shutdown(async move {
            let _ = rest_shutdown.recv().await;
        });
        if let Err(e) = serve.await {
            tracing::debug!(error=?e, "rest server exited");
        }
    });

    // --- TCP ---
    let tcp_listener =
        tokio::net::TcpListener::bind(format!("{bind}:{}", settings.tcp_port)).await?;
    let tcp_bound = tcp_listener.local_addr()?.port();
    let tcp_shutdown = shutdown.subscribe();
    let tcp_limits = tcp::TcpServeLimits {
        max_connections: settings.max_connections,
        max_in_flight_per_conn: settings.max_in_flight_per_conn,
        write_timeout: settings.tcp_write_timeout,
    };
    let tcp_handle = tokio::spawn(async move {
        if let Err(e) =
            tcp::serve_tcp(tcp_listener, backend, metrics, tcp_shutdown, tcp_limits).await
        {
            tracing::debug!(error=?e, "tcp listener exited");
        }
    });

    Ok(ServerHandle {
        ports: BoundPorts {
            grpc: grpc_bound,
            rest: rest_bound,
            tcp: tcp_bound,
        },
        shutdown,
        tasks: vec![grpc_handle, rest_handle, tcp_handle],
    })
}
