//! TCP integration tests. Spawn the full server in-process on ephemeral ports,
//! open a raw `TcpStream`, and exercise all the scenarios called out in the
//! open TODOs.

use std::time::Duration;

use kirk_server::start_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAGIC: u32 = 0x4B49_524B;
const VERSION: u8 = 1;
const HEADER_LEN: usize = 16;

// ─── Wire helpers ────────────────────────────────────────────────────────────

fn build_header(opcode: u8, req_id: u32, payload_len: u32) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    h[4] = VERSION;
    h[5] = opcode;
    // flags = 0 (bytes 6-7)
    h[8..12].copy_from_slice(&req_id.to_le_bytes());
    h[12..16].copy_from_slice(&payload_len.to_le_bytes());
    h
}

/// Build a FORWARD request payload for an N×N diagonal identity matrix.
/// Layout: [u32 N][N*N f32 re][N*N f32 im][i64 ts]
fn build_forward_payload(n: u32) -> Vec<u8> {
    let n_usize = n as usize;
    let m = n_usize * n_usize;
    let expected_len = 4 + 4 * m + 4 * m + 8;
    let mut payload = Vec::with_capacity(expected_len);
    payload.extend_from_slice(&n.to_le_bytes());
    // identity matrix_re: diag = 1.0
    let mut re = vec![0.0f32; m];
    for i in 0..n_usize {
        re[i * n_usize + i] = 1.0;
    }
    for v in &re {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    // matrix_im: zeros
    let im = vec![0.0f32; m];
    for v in &im {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    // timestamp_us = 0
    payload.extend_from_slice(&0i64.to_le_bytes());
    payload
}

async fn read_header(stream: &mut TcpStream) -> [u8; HEADER_LEN] {
    let mut buf = [0u8; HEADER_LEN];
    stream.read_exact(&mut buf).await.expect("read header");
    buf
}

async fn read_payload(stream: &mut TcpStream, len: u32) -> Vec<u8> {
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await.expect("read payload");
    buf
}

/// Parse the error frame payload: [u16 code][u16 reserved][u32 msg_len][msg bytes].
fn parse_error_payload(payload: &[u8]) -> (u16, String) {
    let code = u16::from_le_bytes([payload[0], payload[1]]);
    let msg_len = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
    let msg = String::from_utf8_lossy(&payload[8..8 + msg_len]).to_string();
    (code, msg)
}

/// Decode a FORWARD response payload into named floats for assertion.
///
/// Layout (per spec):
///   [f32 entropy_re][f32 entropy_im][f32 entropy][f32 entropy_zscore]
///   [u32 regime][f32 confidence][u64 processing_time_us][i64 timestamp_us]
///   [u32 dim][dim*dim f32 rho_re][dim*dim f32 rho_im]
struct ForwardResp {
    entropy_re: f32,
    entropy_im: f32,
    entropy: f32,
    #[allow(dead_code)]
    entropy_zscore: f32,
    #[allow(dead_code)]
    regime: u32,
    confidence: f32,
    #[allow(dead_code)]
    processing_time_us: u64,
    #[allow(dead_code)]
    timestamp_us: i64,
}

fn decode_forward_resp(payload: &[u8]) -> ForwardResp {
    let mut off = 0;
    let read_f32 = |data: &[u8], o: &mut usize| -> f32 {
        let v = f32::from_le_bytes([data[*o], data[*o + 1], data[*o + 2], data[*o + 3]]);
        *o += 4;
        v
    };
    let read_u32 = |data: &[u8], o: &mut usize| -> u32 {
        let v = u32::from_le_bytes([data[*o], data[*o + 1], data[*o + 2], data[*o + 3]]);
        *o += 4;
        v
    };
    let read_u64 = |data: &[u8], o: &mut usize| -> u64 {
        let v = u64::from_le_bytes([
            data[*o],
            data[*o + 1],
            data[*o + 2],
            data[*o + 3],
            data[*o + 4],
            data[*o + 5],
            data[*o + 6],
            data[*o + 7],
        ]);
        *o += 8;
        v
    };
    let read_i64 = |data: &[u8], o: &mut usize| -> i64 {
        let v = i64::from_le_bytes([
            data[*o],
            data[*o + 1],
            data[*o + 2],
            data[*o + 3],
            data[*o + 4],
            data[*o + 5],
            data[*o + 6],
            data[*o + 7],
        ]);
        *o += 8;
        v
    };
    let entropy_re = read_f32(payload, &mut off);
    let entropy_im = read_f32(payload, &mut off);
    let entropy = read_f32(payload, &mut off);
    let entropy_zscore = read_f32(payload, &mut off);
    let regime = read_u32(payload, &mut off);
    let confidence = read_f32(payload, &mut off);
    let processing_time_us = read_u64(payload, &mut off);
    let timestamp_us = read_i64(payload, &mut off);
    ForwardResp {
        entropy_re,
        entropy_im,
        entropy,
        entropy_zscore,
        regime,
        confidence,
        processing_time_us,
        timestamp_us,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// Happy path: FORWARD with a known N=4 identity matrix.
/// Asserts response header echoes opcode+req_id, payload carries finite
/// entropy values and confidence ∈ [0, 1].
#[tokio::test(flavor = "multi_thread")]
async fn tcp_forward_identity_n4() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");
    stream.set_nodelay(true).unwrap();

    let payload = build_forward_payload(4);
    let header = build_header(0x01 /*FORWARD*/, 99, payload.len() as u32);
    stream.write_all(&header).await.unwrap();
    stream.write_all(&payload).await.unwrap();
    stream.flush().await.unwrap();

    let resp_header = read_header(&mut stream).await;
    // Verify magic
    let magic = u32::from_le_bytes([
        resp_header[0],
        resp_header[1],
        resp_header[2],
        resp_header[3],
    ]);
    assert_eq!(magic, MAGIC, "response magic mismatch");
    // Opcode echoed
    assert_eq!(resp_header[5], 0x01, "response opcode should be FORWARD");
    // req_id echoed
    let resp_req_id = u32::from_le_bytes([
        resp_header[8],
        resp_header[9],
        resp_header[10],
        resp_header[11],
    ]);
    assert_eq!(resp_req_id, 99, "req_id not echoed");

    let payload_len = u32::from_le_bytes([
        resp_header[12],
        resp_header[13],
        resp_header[14],
        resp_header[15],
    ]);
    // Minimum payload: 4*4 (entropies) + 4 (regime) + 4 (confidence) + 8 (proc_us) + 8 (ts) + 4 (dim) + 8*4*4 (rho)
    // = 16 + 4 + 4 + 8 + 8 + 4 + 128 = 172
    assert!(payload_len >= 36, "payload too small: {payload_len}");
    let resp_payload = read_payload(&mut stream, payload_len).await;

    let resp = decode_forward_resp(&resp_payload);
    assert!(
        resp.entropy_re.is_finite(),
        "entropy_re not finite: {}",
        resp.entropy_re
    );
    assert!(
        resp.entropy_im.is_finite(),
        "entropy_im not finite: {}",
        resp.entropy_im
    );
    assert!(
        resp.entropy.is_finite(),
        "entropy not finite: {}",
        resp.entropy
    );
    assert!(
        (0.0..=1.0).contains(&resp.confidence),
        "confidence {} not in [0, 1]",
        resp.confidence
    );

    srv.shutdown().await;
}

/// PING (opcode 0xFE): empty payload → response PING with same req_id.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_ping_echoes_req_id() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");

    let header = build_header(0xFE /*PING*/, 777, 0);
    stream.write_all(&header).await.unwrap();
    stream.flush().await.unwrap();

    let resp_header = read_header(&mut stream).await;
    assert_eq!(resp_header[5], 0xFE, "expected PING opcode in response");
    let resp_req_id = u32::from_le_bytes([
        resp_header[8],
        resp_header[9],
        resp_header[10],
        resp_header[11],
    ]);
    assert_eq!(resp_req_id, 777, "req_id not echoed in PING response");
    let payload_len = u32::from_le_bytes([
        resp_header[12],
        resp_header[13],
        resp_header[14],
        resp_header[15],
    ]);
    assert_eq!(payload_len, 0, "PING response should have empty payload");

    srv.shutdown().await;
}

/// Oversized payload (payload_len > 64 MiB in header): server must respond
/// with ERROR code 0x02 (PAYLOAD_TOO_LARGE) and close the connection.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_oversized_payload_returns_error() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");

    // 64 MiB + 1 exceeds the cap
    let oversized_len: u32 = (64 * 1024 * 1024) + 1;
    let header = build_header(0x01 /*FORWARD*/, 1, oversized_len);
    stream.write_all(&header).await.unwrap();
    stream.flush().await.unwrap();

    // Server should send an ERROR frame then close.
    let resp_header = read_header(&mut stream).await;
    assert_eq!(resp_header[5], 0xFF, "expected ERROR opcode");
    let payload_len = u32::from_le_bytes([
        resp_header[12],
        resp_header[13],
        resp_header[14],
        resp_header[15],
    ]);
    let resp_payload = read_payload(&mut stream, payload_len).await;
    let (code, _msg) = parse_error_payload(&resp_payload);
    assert_eq!(
        code, 0x02,
        "expected PAYLOAD_TOO_LARGE error code 0x02, got {code:#04x}"
    );

    // Connection should be closed by server after error.
    let mut buf = [0u8; 1];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    assert_eq!(
        n, 0,
        "expected connection closed after oversized payload error"
    );

    srv.shutdown().await;
}

/// Unknown opcode (0xAA): server must respond with ERROR code 0x04 (UNKNOWN_OPCODE).
#[tokio::test(flavor = "multi_thread")]
async fn tcp_unknown_opcode_returns_error() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");

    // Send a well-formed header with payload_len=0 but opcode=0xAA (unknown).
    let header = build_header(0xAA, 2, 0);
    stream.write_all(&header).await.unwrap();
    stream.flush().await.unwrap();

    // Read the response — may come in the same frame loop (connection stays open).
    let resp_header = read_header(&mut stream).await;
    assert_eq!(resp_header[5], 0xFF, "expected ERROR opcode");
    let payload_len = u32::from_le_bytes([
        resp_header[12],
        resp_header[13],
        resp_header[14],
        resp_header[15],
    ]);
    let resp_payload = read_payload(&mut stream, payload_len).await;
    let (code, _msg) = parse_error_payload(&resp_payload);
    assert_eq!(
        code, 0x04,
        "expected UNKNOWN_OPCODE error code 0x04, got {code:#04x}"
    );

    srv.shutdown().await;
}

/// Bad magic: server must respond with ERROR code 0x01 (BAD_MAGIC) and close.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_bad_magic_returns_error_and_closes() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");

    // Build a header with wrong magic.
    let mut header = [0u8; HEADER_LEN];
    header[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // wrong magic
    header[4] = VERSION;
    header[5] = 0x01; // FORWARD
                      // rest zeros
    stream.write_all(&header).await.unwrap();
    stream.flush().await.unwrap();

    let resp_header = read_header(&mut stream).await;
    assert_eq!(resp_header[5], 0xFF, "expected ERROR opcode");
    let payload_len = u32::from_le_bytes([
        resp_header[12],
        resp_header[13],
        resp_header[14],
        resp_header[15],
    ]);
    let resp_payload = read_payload(&mut stream, payload_len).await;
    let (code, _msg) = parse_error_payload(&resp_payload);
    assert_eq!(
        code, 0x01,
        "expected BAD_MAGIC error code 0x01, got {code:#04x}"
    );

    // Connection should close after bad magic.
    let mut buf = [0u8; 1];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    assert_eq!(n, 0, "expected connection closed after bad magic error");

    srv.shutdown().await;
}

/// Pipelined FORWARD+FORWARD with two different req_ids: both responses must
/// come back with their respective req_ids correlated correctly.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_pipelined_forward_correlation() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");
    stream.set_nodelay(true).unwrap();

    let payload_a = build_forward_payload(4);
    let payload_b = build_forward_payload(4);

    // Send both requests back-to-back without waiting for responses.
    let header_a = build_header(0x01, 111, payload_a.len() as u32);
    let header_b = build_header(0x01, 222, payload_b.len() as u32);
    stream.write_all(&header_a).await.unwrap();
    stream.write_all(&payload_a).await.unwrap();
    stream.write_all(&header_b).await.unwrap();
    stream.write_all(&payload_b).await.unwrap();
    stream.flush().await.unwrap();

    // Read both responses. The server processes in arrival order so req 111 comes first.
    let mut seen_req_ids = std::collections::HashSet::new();
    for _ in 0..2 {
        let resp_header = read_header(&mut stream).await;
        assert_eq!(
            resp_header[5], 0x01,
            "expected FORWARD opcode in pipelined response"
        );
        let resp_req_id = u32::from_le_bytes([
            resp_header[8],
            resp_header[9],
            resp_header[10],
            resp_header[11],
        ]);
        assert!(
            resp_req_id == 111 || resp_req_id == 222,
            "unexpected req_id {resp_req_id}"
        );
        seen_req_ids.insert(resp_req_id);
        let payload_len = u32::from_le_bytes([
            resp_header[12],
            resp_header[13],
            resp_header[14],
            resp_header[15],
        ]);
        let _resp_payload = read_payload(&mut stream, payload_len).await;
    }
    assert!(
        seen_req_ids.contains(&111),
        "req_id 111 missing from responses"
    );
    assert!(
        seen_req_ids.contains(&222),
        "req_id 222 missing from responses"
    );

    srv.shutdown().await;
}

/// Confirm that entropy values for the identity-4 matrix are positive and finite.
/// The identity matrix is Hermitian so the pipeline should complete without error.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_forward_response_numerical_sanity() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");

    let payload = build_forward_payload(4);
    let header = build_header(0x01, 5, payload.len() as u32);
    stream.write_all(&header).await.unwrap();
    stream.write_all(&payload).await.unwrap();
    stream.flush().await.unwrap();

    let resp_header = read_header(&mut stream).await;
    let payload_len = u32::from_le_bytes([
        resp_header[12],
        resp_header[13],
        resp_header[14],
        resp_header[15],
    ]);
    let resp_payload = read_payload(&mut stream, payload_len).await;
    let resp = decode_forward_resp(&resp_payload);

    // For the N=4 identity matrix all eigenvalues are 1.0 — uniform Boltzmann
    // → rho = I/N → entropy = ln(N). Should be positive.
    assert!(
        resp.entropy > 0.0,
        "expected positive entropy for identity matrix, got {}",
        resp.entropy
    );
    assert!(resp.entropy_re > 0.0, "expected positive entropy_re");
    assert!(resp.entropy_im > 0.0, "expected positive entropy_im");
    assert!(
        resp.confidence >= 0.0 && resp.confidence <= 1.0,
        "confidence out of range"
    );

    srv.shutdown().await;
}

/// SEC-008: TCP FORWARD with N=1 must be rejected with ERROR
/// code 0x06 (MATRIX_DIM_EXCEEDED ... not quite; we use BAD_PAYLOAD because
/// `check_dim` returns BadRequest for N<2, not MatrixDimExceeded). Either
/// is fine as long as the request is rejected with an ERROR frame.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_forward_n1_rejected() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect");
    stream.set_nodelay(true).unwrap();

    let payload = build_forward_payload(1);
    let header = build_header(0x01, 4242, payload.len() as u32);
    stream.write_all(&header).await.unwrap();
    stream.write_all(&payload).await.unwrap();
    stream.flush().await.unwrap();

    let resp_header = read_header(&mut stream).await;
    // ERROR opcode (0xFF) is expected
    assert_eq!(
        resp_header[5], 0xFF,
        "expected ERROR opcode for N=1, got 0x{:02X}",
        resp_header[5]
    );
    let payload_len = u32::from_le_bytes([
        resp_header[12],
        resp_header[13],
        resp_header[14],
        resp_header[15],
    ]);
    let resp_payload = read_payload(&mut stream, payload_len).await;
    let (code, _msg) = parse_error_payload(&resp_payload);
    // BAD_PAYLOAD (0x05) is what `BadRequest` maps to in the handler.
    assert_eq!(
        code, 0x05,
        "expected BAD_PAYLOAD error code 0x05 for N=1, got 0x{code:#04x}"
    );

    srv.shutdown().await;
}

/// SEC-002: TCP connection cap. Spawn a server with `max_connections = 2`,
/// open two long-lived connections, then verify a third connection attempt
/// either succeeds-and-drops or is otherwise prevented from accumulating
/// unbounded resources.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_connection_cap_enforced() {
    use kirk_server::{start_server_with, ServerSettings};

    let settings = ServerSettings {
        bind: "127.0.0.1".to_string(),
        grpc_port: 0,
        rest_port: 0,
        tcp_port: 0,
        temperature: 1.0,
        window_size: 256,
        max_matrix_dim: 1024,
        max_connections: 2,
        max_in_flight_per_conn: 8,
        tcp_write_timeout: Duration::from_secs(10),
    };
    let srv = start_server_with(settings).await.expect("start server");
    let tcp_port = srv.ports.tcp;

    // Hold two connections open. We never write anything so they stay alive
    // — the server's accept loop has consumed both permits.
    let s1 = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect 1");
    let s2 = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect 2");

    // Give the server a moment to register both connections + saturate the semaphore.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A third connection should be accepted at the TCP layer (the kernel
    // backlog accepts it) but the server drops it immediately because the cap
    // is saturated. We verify by issuing a PING and observing that we get NO
    // response within a short window — the connection was dropped before any
    // handler picked it up.
    let mut s3 = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
        .await
        .expect("connect 3");
    let header = build_header(0xFE /*PING*/, 9999, 0);
    let _ = s3.write_all(&header).await; // may succeed (buffered) before drop
    let _ = s3.flush().await;

    let mut buf = [0u8; HEADER_LEN];
    let read_result =
        tokio::time::timeout(Duration::from_millis(500), s3.read_exact(&mut buf)).await;
    // We expect either: the read returns EOF (0 bytes / err) because the
    // server dropped, or the timeout elapses (no handler attached).
    assert!(
        read_result.is_err() || read_result.as_ref().unwrap().is_err(),
        "third connection should be dropped (no PING response within 500ms), got {read_result:?}"
    );

    drop(s1);
    drop(s2);
    drop(s3);
    srv.shutdown().await;
}

/// Test with a timeout to catch connection hangs.
#[tokio::test(flavor = "multi_thread")]
async fn tcp_connect_and_ping_completes_within_timeout() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let tcp_port = srv.ports.tcp;

    let test = async {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{tcp_port}"))
            .await
            .expect("connect");
        let header = build_header(0xFE, 1, 0);
        stream.write_all(&header).await.unwrap();
        stream.flush().await.unwrap();
        read_header(&mut stream).await
    };

    tokio::time::timeout(Duration::from_secs(5), test)
        .await
        .expect("ping timed out");

    srv.shutdown().await;
}
