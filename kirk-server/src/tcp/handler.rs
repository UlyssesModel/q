//! Per-connection TCP handler. Each accepted connection runs one read loop and
//! one write loop bridged by an `mpsc` so pipelined responses don't block the
//! reader on slow writers.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Semaphore};

use crate::backend::KirkBackend;
use crate::error::ServerError;
use crate::metrics::MetricsHandle;

use super::codec::{
    encode_entropy_response, encode_features_response, encode_forward_response,
    parse_forward_request, parse_forward_sample_request, parse_sample_request, peek_payload_dim,
};
use super::framing::{
    parse_header, write_error_frame, write_header, FramingError, Header, Opcode, TcpErrorCode,
    HEADER_LEN, MAX_PAYLOAD,
};

/// Runtime DoS bounds for the TCP listener — see SEC-002 + SEC-007.
#[derive(Debug, Clone, Copy)]
pub struct TcpServeLimits {
    /// Maximum number of accepted connections held simultaneously.
    pub max_connections: u32,
    /// Maximum number of in-flight frames per single connection.
    pub max_in_flight_per_conn: u32,
    /// Per-`write_all` / per-`flush` timeout for the writer task.
    pub write_timeout: Duration,
}

pub async fn serve_tcp(
    listener: TcpListener,
    backend: Arc<KirkBackend>,
    metrics: MetricsHandle,
    mut shutdown: broadcast::Receiver<()>,
    limits: TcpServeLimits,
) -> anyhow::Result<()> {
    tracing::info!(
        addr = ?listener.local_addr().ok(),
        max_connections = limits.max_connections,
        max_in_flight_per_conn = limits.max_in_flight_per_conn,
        write_timeout_ms = limits.write_timeout.as_millis() as u64,
        "tcp listener started"
    );
    // SEC-002: bound the per-listener connection count. Acquire a permit before
    // spawning the per-connection task; drop it when the connection ends.
    let connection_sem = Arc::new(Semaphore::new(limits.max_connections as usize));
    loop {
        tokio::select! {
            biased;
            _ = shutdown.recv() => {
                tracing::info!("tcp listener shutting down");
                return Ok(());
            }
            accept = listener.accept() => {
                let (stream, peer) = match accept {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(error=?e, "tcp accept failed");
                        continue;
                    }
                };
                // SEC-002: bound the connection count. We hold the permit for the
                // entire lifetime of the handler task. If the cap is saturated, log
                // and drop the new connection to avoid blocking the accept loop.
                let permit = match connection_sem.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!(?peer, max = limits.max_connections, "tcp connection cap saturated, dropping new connection");
                        drop(stream);
                        continue;
                    }
                };
                let backend = backend.clone();
                let metrics = metrics.clone();
                let mut conn_shutdown = shutdown.resubscribe();
                tokio::spawn(async move {
                    if let Err(e) = stream.set_nodelay(true) {
                        tracing::debug!(?peer, error=?e, "set_nodelay failed");
                    }
                    if let Err(e) = handle_connection(stream, backend, metrics, &mut conn_shutdown, limits).await {
                        tracing::debug!(?peer, error=?e, "tcp connection ended");
                    }
                    drop(permit);
                });
            }
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    backend: Arc<KirkBackend>,
    metrics: MetricsHandle,
    shutdown: &mut broadcast::Receiver<()>,
    limits: TcpServeLimits,
) -> anyhow::Result<()> {
    let (rd, wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    let mut writer = BufWriter::new(wr);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);

    // SEC-002: per-connection in-flight cap. Each spawned `process_frame` task
    // holds one permit for its lifetime.
    let in_flight_sem = Arc::new(Semaphore::new(limits.max_in_flight_per_conn as usize));

    // SEC-007: write timeout on the per-connection writer task. A slow / dead
    // client cannot indefinitely pin the mpsc + writer pipeline anymore.
    let write_timeout = limits.write_timeout;
    let write_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            match tokio::time::timeout(write_timeout, writer.write_all(&frame)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::debug!(error=?e, "tcp writer io error, closing connection");
                    break;
                }
                Err(_) => {
                    tracing::warn!(
                        timeout_ms = write_timeout.as_millis() as u64,
                        "tcp writer timeout on write_all, closing connection"
                    );
                    break;
                }
            }
            match tokio::time::timeout(write_timeout, writer.flush()).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::debug!(error=?e, "tcp writer flush error, closing connection");
                    break;
                }
                Err(_) => {
                    tracing::warn!(
                        timeout_ms = write_timeout.as_millis() as u64,
                        "tcp writer timeout on flush, closing connection"
                    );
                    break;
                }
            }
        }
        let _ = writer.shutdown().await;
    });

    let mut header_buf = [0u8; HEADER_LEN];
    loop {
        tokio::select! {
            biased;
            _ = shutdown.recv() => {
                let mut buf = Vec::with_capacity(HEADER_LEN + 16);
                write_error_frame(&mut buf, 0, TcpErrorCode::ShutdownInProgress, "shutdown");
                let _ = tx.send(buf).await;
                break;
            }
            r = reader.read_exact(&mut header_buf) => {
                match r {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(e.into()),
                }
                let header = match parse_header(&header_buf) {
                    Ok(h) => h,
                    Err(FramingError::BadMagic(_)) => {
                        let mut buf = Vec::new();
                        write_error_frame(&mut buf, 0, TcpErrorCode::BadMagic, "bad magic");
                        let _ = tx.send(buf).await;
                        break;
                    }
                    Err(FramingError::UnsupportedVersion(_)) => {
                        let mut buf = Vec::new();
                        write_error_frame(&mut buf, 0, TcpErrorCode::UnsupportedVersion, "version");
                        let _ = tx.send(buf).await;
                        break;
                    }
                    Err(FramingError::PayloadTooLarge(_)) => {
                        let mut buf = Vec::new();
                        write_error_frame(&mut buf, 0, TcpErrorCode::PayloadTooLarge, "frame too large");
                        let _ = tx.send(buf).await;
                        break;
                    }
                    Err(FramingError::TruncatedHeader(_)) => break,
                };

                let mut payload = vec![0u8; header.payload_len as usize];
                if payload.len() > MAX_PAYLOAD {
                    let mut buf = Vec::new();
                    write_error_frame(&mut buf, header.req_id, TcpErrorCode::PayloadTooLarge, "frame too large");
                    let _ = tx.send(buf).await;
                    break;
                }
                if !payload.is_empty() {
                    // SEC-011: emit a BAD_PAYLOAD error frame before closing if
                    // the client cuts off mid-payload.
                    if let Err(e) = reader.read_exact(&mut payload).await {
                        if e.kind() != std::io::ErrorKind::UnexpectedEof
                            && e.kind() != std::io::ErrorKind::ConnectionReset
                        {
                            tracing::debug!(error=?e, "read_exact payload error");
                        }
                        let mut buf = Vec::new();
                        write_error_frame(
                            &mut buf,
                            header.req_id,
                            TcpErrorCode::BadPayload,
                            "truncated payload",
                        );
                        let _ = tx.send(buf).await;
                        break;
                    }
                }

                // SEC-002: throttle per-connection in-flight frames. Acquire one
                // permit; release on task exit.
                let permit = match in_flight_sem.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => break, // semaphore closed — connection shutting down
                };

                let backend = backend.clone();
                let metrics = metrics.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    process_frame(backend, metrics, tx, header, payload).await;
                    drop(permit);
                });
            }
        }
    }
    drop(tx);
    let _ = write_task.await;
    Ok(())
}

async fn process_frame(
    backend: Arc<KirkBackend>,
    metrics: MetricsHandle,
    tx: mpsc::Sender<Vec<u8>>,
    header: Header,
    payload: Vec<u8>,
) {
    let opcode = match Opcode::from_u8(header.opcode) {
        Ok(o) => o,
        Err(_) => {
            let mut buf = Vec::new();
            write_error_frame(
                &mut buf,
                header.req_id,
                TcpErrorCode::UnknownOpcode,
                "unknown opcode",
            );
            let _ = tx.send(buf).await;
            return;
        }
    };

    let t0 = Instant::now();
    let (op_label, result) = match opcode {
        Opcode::Ping => {
            let mut frame = Vec::with_capacity(HEADER_LEN);
            write_header(&mut frame, Opcode::Ping as u8, header.req_id, 0);
            ("ping", Ok::<Vec<u8>, ServerError>(frame))
        }
        Opcode::Forward => (
            "forward",
            handle_forward(&backend, header.req_id, &payload).await,
        ),
        Opcode::InferenceEntropy => (
            "inference_entropy",
            handle_entropy(
                &backend,
                header.req_id,
                &payload,
                Opcode::InferenceEntropy,
                false,
            )
            .await,
        ),
        Opcode::InferenceFeatures => (
            "inference_features",
            handle_features(
                &backend,
                header.req_id,
                &payload,
                Opcode::InferenceFeatures,
                false,
            )
            .await,
        ),
        Opcode::ActiveInference => (
            "active_inference",
            handle_active_inference(&backend, header.req_id, &payload).await,
        ),
        Opcode::ActiveInferenceEntropy => (
            "active_inference_entropy",
            handle_entropy(
                &backend,
                header.req_id,
                &payload,
                Opcode::ActiveInferenceEntropy,
                true,
            )
            .await,
        ),
        Opcode::ActiveInferenceFeatures => (
            "active_inference_features",
            handle_features(
                &backend,
                header.req_id,
                &payload,
                Opcode::ActiveInferenceFeatures,
                true,
            )
            .await,
        ),
        Opcode::ForwardSample => (
            "forward_sample",
            handle_forward_sample(&backend, header.req_id, &payload).await,
        ),
        Opcode::Error => {
            // Clients should not send ERROR frames; treat as unknown.
            (
                "error",
                Err(ServerError::BadRequest("client sent ERROR opcode".into())),
            )
        }
    };

    let ok = result.is_ok();
    metrics.observe("tcp", op_label, t0.elapsed().as_micros() as f64, ok);

    let frame = match result {
        Ok(buf) => buf,
        Err(e) => {
            let mut buf = Vec::new();
            let code = match &e {
                ServerError::PayloadTooLarge { .. } => TcpErrorCode::PayloadTooLarge,
                ServerError::MatrixDimExceeded { .. } => TcpErrorCode::MatrixDimExceeded,
                ServerError::Shutdown => TcpErrorCode::ShutdownInProgress,
                ServerError::Compute(_) => TcpErrorCode::ComputeError,
                ServerError::BadRequest(_) => TcpErrorCode::BadPayload,
                ServerError::Internal(_) => TcpErrorCode::ComputeError,
            };
            write_error_frame(&mut buf, header.req_id, code, &e.to_string());
            buf
        }
    };
    let _ = tx.send(frame).await;
}

async fn handle_forward(
    backend: &Arc<KirkBackend>,
    req_id: u32,
    payload: &[u8],
) -> Result<Vec<u8>, ServerError> {
    // SEC-004: dim-check BEFORE the codec touches `n * n` arithmetic. Peek at
    // the leading u32 only.
    let dim = peek_payload_dim(payload)?;
    backend.check_dim(dim)?;
    let r = parse_forward_request(payload)?;
    let n_usize = r.n as usize;
    let out = backend
        .clone()
        .forward(r.re, r.im, n_usize, r.timestamp_us)
        .await?;
    // payload layout: 4*4 (4 x f32 entropy) + 4 (regime) + 4 (confidence)
    //   + 8 (processing_us) + 8 (ts) + 4 (n) + 4*N*N (rho_re) + 4*N*N (rho_im)
    let payload_size = 16 + 4 + 4 + 8 + 8 + 4 + 8 * n_usize * n_usize;
    let mut frame = Vec::with_capacity(HEADER_LEN + payload_size);
    write_header(
        &mut frame,
        Opcode::Forward as u8,
        req_id,
        payload_size as u32,
    );
    encode_forward_response(
        &mut frame,
        out.entropy_re,
        out.entropy_im,
        out.entropy,
        out.entropy_zscore,
        out.regime,
        out.confidence,
        out.processing_time_us,
        out.timestamp_us,
        r.n,
        &out.matrix_re,
        &out.matrix_im,
    );
    debug_assert_eq!(frame.len(), HEADER_LEN + payload_size);
    Ok(frame)
}

async fn handle_entropy(
    backend: &Arc<KirkBackend>,
    req_id: u32,
    payload: &[u8],
    opcode: Opcode,
    active: bool,
) -> Result<Vec<u8>, ServerError> {
    let dim = peek_payload_dim(payload)?;
    backend.check_dim(dim)?;
    let r = parse_sample_request(payload)?;
    let n_usize = r.n as usize;
    let entropy = if active {
        backend
            .clone()
            .active_inference_entropy(r.re, r.im, n_usize)
            .await?
    } else {
        backend
            .clone()
            .inference_entropy(r.re, r.im, n_usize)
            .await?
    };
    let mut frame = Vec::with_capacity(HEADER_LEN + 4);
    write_header(&mut frame, opcode as u8, req_id, 4);
    encode_entropy_response(&mut frame, entropy);
    Ok(frame)
}

async fn handle_features(
    backend: &Arc<KirkBackend>,
    req_id: u32,
    payload: &[u8],
    opcode: Opcode,
    active: bool,
) -> Result<Vec<u8>, ServerError> {
    let dim = peek_payload_dim(payload)?;
    backend.check_dim(dim)?;
    let r = parse_sample_request(payload)?;
    let n_usize = r.n as usize;
    let f = if active {
        backend
            .clone()
            .active_inference_features(r.re, r.im, n_usize)
            .await?
    } else {
        backend
            .clone()
            .inference_features(r.re, r.im, n_usize)
            .await?
    };
    let payload_size = 4 + 8 * n_usize * n_usize + 8 * 2 * n_usize + 8;
    let mut frame = Vec::with_capacity(HEADER_LEN + payload_size);
    write_header(&mut frame, opcode as u8, req_id, payload_size as u32);
    encode_features_response(
        &mut frame,
        r.n,
        &f.feature_arr,
        &f.feature_vec,
        f.feature_scalar,
    );
    debug_assert_eq!(frame.len(), HEADER_LEN + payload_size);
    Ok(frame)
}

async fn handle_active_inference(
    backend: &Arc<KirkBackend>,
    req_id: u32,
    payload: &[u8],
) -> Result<Vec<u8>, ServerError> {
    let dim = peek_payload_dim(payload)?;
    backend.check_dim(dim)?;
    let r = parse_sample_request(payload)?;
    let n_usize = r.n as usize;
    let out = backend
        .clone()
        .active_inference(r.re, r.im, n_usize)
        .await?;
    let payload_size = 4 + 8 * n_usize * n_usize + 8 * 2 * n_usize + 8 + 4;
    let mut frame = Vec::with_capacity(HEADER_LEN + payload_size);
    write_header(
        &mut frame,
        Opcode::ActiveInference as u8,
        req_id,
        payload_size as u32,
    );
    encode_features_response(
        &mut frame,
        r.n,
        &out.features.feature_arr,
        &out.features.feature_vec,
        out.features.feature_scalar,
    );
    frame.extend_from_slice(&out.total_relative_entropy.to_le_bytes());
    debug_assert_eq!(frame.len(), HEADER_LEN + payload_size);
    Ok(frame)
}

async fn handle_forward_sample(
    backend: &Arc<KirkBackend>,
    req_id: u32,
    payload: &[u8],
) -> Result<Vec<u8>, ServerError> {
    let r = parse_forward_sample_request(payload)?;
    backend.check_dim(r.n)?;
    let n_usize = r.n as usize;
    let out = backend.clone().forward_sample(n_usize, r.seed).await?;
    let payload_size = 4 + 8 * n_usize * n_usize + 8 * 2 * n_usize + 8 + 4;
    let mut frame = Vec::with_capacity(HEADER_LEN + payload_size);
    write_header(
        &mut frame,
        Opcode::ForwardSample as u8,
        req_id,
        payload_size as u32,
    );
    encode_features_response(
        &mut frame,
        r.n,
        &out.feature_array,
        &out.feature_vector,
        out.feature_scalar,
    );
    frame.extend_from_slice(&out.relative_entropy.to_le_bytes());
    debug_assert_eq!(frame.len(), HEADER_LEN + payload_size);
    Ok(frame)
}
