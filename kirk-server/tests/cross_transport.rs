//! Cross-transport parity test. Feeds the SAME (matrix_re, matrix_im, ts)
//! through REST, gRPC, and TCP in sequence, each on a fresh server instance
//! so that rolling-window state is identical (first call, no history).
//!
//! Asserts that all three return the same `entropy`/`confidence`/`regime`
//! within the NFR-001 float tolerance:
//!   |Δentropy| ≤ 1e-4 * max(1, entropy_ref)
//!   |Δconfidence| ≤ 1e-4

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use kirk_server::grpc::proto::{kirk_service_client::KirkServiceClient, KirkRequest, Matrix};
use kirk_server::start_server;

const MAGIC: u32 = 0x4B49_524B;
const VERSION: u8 = 1;
const HEADER_LEN: usize = 16;

// ─── Shared matrix fixture ───────────────────────────────────────────────────
// Use a simple N=4 non-trivial (non-identity) matrix: diagonal (1, 2, 3, 4).

fn build_diag4_matrix() -> (Vec<f32>, Vec<f32>) {
    let n = 4;
    let m = n * n;
    let diag = [1.0f32, 2.0, 3.0, 4.0];
    let mut re = vec![0.0f32; m];
    for i in 0..n {
        re[i * n + i] = diag[i];
    }
    let im = vec![0.0f32; m];
    (re, im)
}

fn le_f32_bytes(vals: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

fn b64(vals: &[f32]) -> String {
    B64.encode(le_f32_bytes(vals))
}

// ─── TCP helpers ─────────────────────────────────────────────────────────────

fn tcp_header(opcode: u8, req_id: u32, payload_len: u32) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    h[4] = VERSION;
    h[5] = opcode;
    h[8..12].copy_from_slice(&req_id.to_le_bytes());
    h[12..16].copy_from_slice(&payload_len.to_le_bytes());
    h
}

fn tcp_forward_payload(re: &[f32], im: &[f32], n: u32, ts: i64) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&n.to_le_bytes());
    p.extend_from_slice(&le_f32_bytes(re));
    p.extend_from_slice(&le_f32_bytes(im));
    p.extend_from_slice(&ts.to_le_bytes());
    p
}

/// Minimal forward-response decoder: returns (entropy, confidence, regime).
fn decode_tcp_forward_resp(payload: &[u8]) -> (f32, f32, u32) {
    let read_f32 = |p: &[u8], o: usize| f32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
    let read_u32 = |p: &[u8], o: usize| u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
    let _entropy_re = read_f32(payload, 0);
    let _entropy_im = read_f32(payload, 4);
    let entropy = read_f32(payload, 8);
    let _entropy_zscore = read_f32(payload, 12);
    let regime = read_u32(payload, 16);
    let confidence = read_f32(payload, 20);
    (entropy, confidence, regime)
}

// ─── REST call ───────────────────────────────────────────────────────────────

async fn rest_forward(port: u16, re: &[f32], im: &[f32], n: u32, ts: i64) -> (f32, f32, u32) {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "matrix_re": b64(re),
        "matrix_im": b64(im),
        "matrix_dim": n,
        "timestamp_us": ts
    });
    let resp: serde_json::Value = client
        .post(format!("http://127.0.0.1:{port}/v1/forward"))
        .json(&body)
        .send()
        .await
        .expect("REST forward")
        .json()
        .await
        .expect("REST JSON");
    let entropy = resp["entropy"].as_f64().unwrap() as f32;
    let confidence = resp["confidence"].as_f64().unwrap() as f32;
    let regime = resp["regime"].as_u64().unwrap() as u32;
    (entropy, confidence, regime)
}

// ─── gRPC call ───────────────────────────────────────────────────────────────

async fn grpc_forward(port: u16, re: &[f32], im: &[f32], n: u32, ts: i64) -> (f32, f32, u32) {
    let mut client = KirkServiceClient::connect(format!("http://127.0.0.1:{port}"))
        .await
        .expect("gRPC connect");
    let req = KirkRequest {
        matrix: Some(Matrix {
            dim: n,
            data_re: le_f32_bytes(re),
            data_im: le_f32_bytes(im),
        }),
        timestamp_us: ts,
    };
    let r = client
        .forward(req)
        .await
        .expect("gRPC forward")
        .into_inner();
    (r.entropy, r.confidence, r.regime)
}

// ─── TCP call ────────────────────────────────────────────────────────────────

async fn tcp_forward(port: u16, re: &[f32], im: &[f32], n: u32, ts: i64) -> (f32, f32, u32) {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("TCP connect");
    let payload = tcp_forward_payload(re, im, n, ts);
    let header = tcp_header(0x01, 1, payload.len() as u32);
    stream.write_all(&header).await.unwrap();
    stream.write_all(&payload).await.unwrap();
    stream.flush().await.unwrap();

    let mut resp_hdr = [0u8; HEADER_LEN];
    stream
        .read_exact(&mut resp_hdr)
        .await
        .expect("read resp header");
    let plen = u32::from_le_bytes([resp_hdr[12], resp_hdr[13], resp_hdr[14], resp_hdr[15]]);
    let mut resp_body = vec![0u8; plen as usize];
    stream
        .read_exact(&mut resp_body)
        .await
        .expect("read resp body");
    decode_tcp_forward_resp(&resp_body)
}

// ─── Parity test ─────────────────────────────────────────────────────────────

/// Feed the same 4×4 diagonal matrix through REST, gRPC, and TCP on separate
/// fresh server instances so rolling-window state is identical. Assert entropy,
/// confidence, and regime all agree within NFR-001 tolerance.
#[tokio::test(flavor = "multi_thread")]
async fn cross_transport_parity_diag4() {
    let (re, im) = build_diag4_matrix();
    let n = 4u32;
    let ts = 1_718_560_000_000_000i64;

    // REST — fresh server.
    let srv_rest = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start rest server");
    let (rest_entropy, rest_confidence, rest_regime) =
        rest_forward(srv_rest.ports.rest, &re, &im, n, ts).await;
    srv_rest.shutdown().await;

    // gRPC — fresh server.
    let srv_grpc = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start grpc server");
    let (grpc_entropy, grpc_confidence, grpc_regime) =
        grpc_forward(srv_grpc.ports.grpc, &re, &im, n, ts).await;
    srv_grpc.shutdown().await;

    // TCP — fresh server.
    let srv_tcp = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start tcp server");
    let (tcp_entropy, tcp_confidence, tcp_regime) =
        tcp_forward(srv_tcp.ports.tcp, &re, &im, n, ts).await;
    srv_tcp.shutdown().await;

    let rel_err = |a: f32, b: f32| -> f32 {
        let denom = b.abs().max(1.0);
        (a - b).abs() / denom
    };
    let tol = 1e-4_f32;

    // gRPC vs REST
    assert!(
        rel_err(grpc_entropy, rest_entropy) <= tol,
        "gRPC vs REST entropy: gRPC={grpc_entropy} REST={rest_entropy} err={}",
        rel_err(grpc_entropy, rest_entropy)
    );
    assert!(
        (grpc_confidence - rest_confidence).abs() <= tol,
        "gRPC vs REST confidence: gRPC={grpc_confidence} REST={rest_confidence}"
    );
    assert_eq!(grpc_regime, rest_regime, "gRPC vs REST regime mismatch");

    // TCP vs REST
    assert!(
        rel_err(tcp_entropy, rest_entropy) <= tol,
        "TCP vs REST entropy: TCP={tcp_entropy} REST={rest_entropy} err={}",
        rel_err(tcp_entropy, rest_entropy)
    );
    assert!(
        (tcp_confidence - rest_confidence).abs() <= tol,
        "TCP vs REST confidence: TCP={tcp_confidence} REST={rest_confidence}"
    );
    assert_eq!(tcp_regime, rest_regime, "TCP vs REST regime mismatch");

    // TCP vs gRPC
    assert!(
        rel_err(tcp_entropy, grpc_entropy) <= tol,
        "TCP vs gRPC entropy: TCP={tcp_entropy} gRPC={grpc_entropy} err={}",
        rel_err(tcp_entropy, grpc_entropy)
    );
}

/// Same parity check with the N=4 identity matrix.
#[tokio::test(flavor = "multi_thread")]
async fn cross_transport_parity_identity4() {
    let n = 4usize;
    let m = n * n;
    let mut re = vec![0.0f32; m];
    for i in 0..n {
        re[i * n + i] = 1.0;
    }
    let im = vec![0.0f32; m];
    let n_u32 = n as u32;
    let ts = 0i64;

    let srv_rest = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start rest server");
    let (rest_entropy, rest_confidence, rest_regime) =
        rest_forward(srv_rest.ports.rest, &re, &im, n_u32, ts).await;
    srv_rest.shutdown().await;

    let srv_grpc = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start grpc server");
    let (grpc_entropy, grpc_confidence, grpc_regime) =
        grpc_forward(srv_grpc.ports.grpc, &re, &im, n_u32, ts).await;
    srv_grpc.shutdown().await;

    let srv_tcp = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start tcp server");
    let (tcp_entropy, tcp_confidence, tcp_regime) =
        tcp_forward(srv_tcp.ports.tcp, &re, &im, n_u32, ts).await;
    srv_tcp.shutdown().await;

    let tol = 1e-4_f32;
    let rel_err = |a: f32, b: f32| -> f32 { (a - b).abs() / b.abs().max(1.0) };

    assert!(
        rel_err(grpc_entropy, rest_entropy) <= tol,
        "identity4 gRPC vs REST entropy: {grpc_entropy} vs {rest_entropy}"
    );
    assert!(
        (grpc_confidence - rest_confidence).abs() <= tol,
        "identity4 gRPC vs REST confidence: {grpc_confidence} vs {rest_confidence}"
    );
    assert_eq!(grpc_regime, rest_regime);

    assert!(
        rel_err(tcp_entropy, rest_entropy) <= tol,
        "identity4 TCP vs REST entropy: {tcp_entropy} vs {rest_entropy}"
    );
    assert!(
        (tcp_confidence - rest_confidence).abs() <= tol,
        "identity4 TCP vs REST confidence: {tcp_confidence} vs {rest_confidence}"
    );
    assert_eq!(tcp_regime, rest_regime);
}
