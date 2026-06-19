//! kirk-server entry point. Spawns one tokio task per transport sharing a
//! single `Arc<KirkBackend>`.

#![forbid(unsafe_code)]

use std::time::Duration;

use clap::Parser;
use kirk_server::{start_server_with, Config, ServerSettings};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    let cfg = Config::parse();

    // SEC-009: `--healthcheck` is a one-shot HTTP GET probe used by the
    // docker-compose healthcheck. Bypass the full server bring-up.
    if cfg.healthcheck {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let code = rt.block_on(run_healthcheck(cfg.rest_port));
        std::process::exit(code);
    }

    let workers = cfg.worker_threads();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;
    rt.block_on(async_main(cfg))
}

/// Minimal HTTP/1.1 healthcheck client. Returns the process exit code: 0 on
/// HTTP 200, 1 on any other status or io failure. Hand-rolled to avoid pulling
/// in a runtime HTTP client just for the probe.
async fn run_healthcheck(rest_port: u16) -> i32 {
    let target = format!("127.0.0.1:{rest_port}");
    let request = b"GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = TcpStream::connect(&target).await?;
        stream.write_all(request).await?;
        stream.flush().await?;
        let mut buf = Vec::with_capacity(256);
        // Read up to 4 KiB — `/healthz` body is tiny; we only need the status line.
        let _ = stream.take(4096).read_to_end(&mut buf).await?;
        Ok::<Vec<u8>, std::io::Error>(buf)
    })
    .await;
    let body = match result {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            eprintln!("healthcheck: connect/io error: {e}");
            return 1;
        }
        Err(_) => {
            eprintln!("healthcheck: timeout after 5s connecting to {target}");
            return 1;
        }
    };
    // Parse the status line: "HTTP/1.1 200 OK\r\n..."
    let status = parse_http_status(&body);
    match status {
        Some(200) => {
            println!("healthcheck: 200 OK");
            0
        }
        Some(code) => {
            eprintln!("healthcheck: status {code}");
            1
        }
        None => {
            eprintln!(
                "healthcheck: malformed HTTP response ({} bytes)",
                body.len()
            );
            1
        }
    }
}

/// Parse the numeric status code from an HTTP/1.x response. Returns `None` if
/// the response is malformed.
fn parse_http_status(buf: &[u8]) -> Option<u16> {
    // First line is "HTTP/1.x <code> <reason>\r\n".
    let end = buf.iter().position(|&b| b == b'\r' || b == b'\n')?;
    let line = std::str::from_utf8(&buf[..end]).ok()?;
    let mut parts = line.split_whitespace();
    let _version = parts.next()?;
    let code = parts.next()?;
    code.parse().ok()
}

async fn async_main(cfg: Config) -> anyhow::Result<()> {
    let workers = cfg.worker_threads();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&cfg.log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .with_target(false)
        .init();

    let settings = ServerSettings::from_config(&cfg);
    let handle = start_server_with(settings).await?;

    tracing::info!(
        bind = %cfg.bind,
        grpc = handle.ports.grpc,
        rest = handle.ports.rest,
        tcp = handle.ports.tcp,
        workers,
        max_matrix_dim = cfg.max_matrix_dim,
        max_connections = cfg.max_connections,
        max_in_flight_per_conn = cfg.max_in_flight_per_conn,
        tcp_write_timeout_ms = cfg.tcp_write_timeout_ms,
        "kirk-server listening on all transports"
    );

    wait_for_signal().await;
    tracing::info!("shutdown signal received, draining");
    handle.shutdown().await;
    Ok(())
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM");
        let mut int = signal(SignalKind::interrupt()).expect("install SIGINT");
        tokio::select! {
            _ = term.recv() => {},
            _ = int.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::parse_http_status;

    #[test]
    fn parses_200_ok() {
        let buf = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}";
        assert_eq!(parse_http_status(buf), Some(200));
    }

    #[test]
    fn parses_503_service_unavailable() {
        let buf = b"HTTP/1.1 503 Service Unavailable\r\n\r\n";
        assert_eq!(parse_http_status(buf), Some(503));
    }

    #[test]
    fn parses_http10() {
        let buf = b"HTTP/1.0 404 Not Found\r\n";
        assert_eq!(parse_http_status(buf), Some(404));
    }

    #[test]
    fn rejects_malformed_no_status() {
        let buf = b"HTTP/1.1\r\n";
        assert!(parse_http_status(buf).is_none());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_http_status(b"").is_none());
    }

    #[test]
    fn rejects_non_numeric() {
        let buf = b"HTTP/1.1 OK 200\r\n";
        assert!(parse_http_status(buf).is_none());
    }
}
