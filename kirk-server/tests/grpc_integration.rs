//! gRPC integration tests. Spawns the server in-process and connects with a
//! tonic-generated client. Validates that `Forward` returns finite values for
//! a known identity matrix, and that results match the REST endpoint within
//! float tolerance (NFR-001).

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use kirk_server::start_server;

// Pull in the protobuf-generated types from the library's grpc module.
use kirk_server::grpc::proto::{
    kirk_service_client::KirkServiceClient, ComplexMatrix, KirkRequest, Matrix, SampleRequest,
    SampleSizeRequest,
};

/// Build a little-endian f32 byte buffer.
fn le_f32_bytes(vals: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

/// Build an N×N identity matrix as (re_bytes, im_bytes).
fn identity_matrix_bytes(n: usize) -> (Vec<u8>, Vec<u8>) {
    let m = n * n;
    let mut re = vec![0.0f32; m];
    for i in 0..n {
        re[i * n + i] = 1.0;
    }
    let im = vec![0.0f32; m];
    (le_f32_bytes(&re), le_f32_bytes(&im))
}

/// Build an N×N identity matrix as base64 strings (for REST).
fn identity_matrix_b64(n: usize) -> (String, String) {
    let (re, im) = identity_matrix_bytes(n);
    (B64.encode(&re), B64.encode(&im))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// gRPC Forward with N=4 identity matrix: response fields are finite and valid.
#[tokio::test(flavor = "multi_thread")]
async fn grpc_forward_identity_n4() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let grpc_port = srv.ports.grpc;

    let mut client = KirkServiceClient::connect(format!("http://127.0.0.1:{grpc_port}"))
        .await
        .expect("connect gRPC");

    let (re_bytes, im_bytes) = identity_matrix_bytes(4);
    let req = KirkRequest {
        matrix: Some(Matrix {
            dim: 4,
            data_re: re_bytes,
            data_im: im_bytes,
        }),
        timestamp_us: 0,
    };

    let resp = client.forward(req).await.expect("gRPC Forward");
    let r = resp.into_inner();

    assert!(
        r.entropy_re.is_finite(),
        "entropy_re not finite: {}",
        r.entropy_re
    );
    assert!(
        r.entropy_im.is_finite(),
        "entropy_im not finite: {}",
        r.entropy_im
    );
    assert!(r.entropy.is_finite(), "entropy not finite: {}", r.entropy);
    assert!(r.entropy_zscore.is_finite(), "entropy_zscore not finite");
    assert!(
        (0.0..=1.0).contains(&r.confidence),
        "confidence {} not in [0, 1]",
        r.confidence
    );
    assert!(r.processing_time_us > 0, "processing_time_us should be > 0");
    // rho density matrix should be present
    assert!(r.rho.is_some(), "rho field missing in KirkResponse");

    srv.shutdown().await;
}

/// gRPC InferenceEntropy with N=4 identity sample: returns finite total_relative_entropy.
#[tokio::test(flavor = "multi_thread")]
async fn grpc_inference_entropy_finite() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let grpc_port = srv.ports.grpc;

    let mut client = KirkServiceClient::connect(format!("http://127.0.0.1:{grpc_port}"))
        .await
        .expect("connect gRPC");

    let (re_bytes, im_bytes) = identity_matrix_bytes(4);
    let req = SampleRequest {
        sample: Some(ComplexMatrix {
            dim: 4,
            data_re: re_bytes,
            data_im: im_bytes,
        }),
    };

    let resp = client
        .inference_entropy(req)
        .await
        .expect("gRPC InferenceEntropy");
    let r = resp.into_inner();
    assert!(
        r.total_relative_entropy.is_finite(),
        "total_relative_entropy not finite: {}",
        r.total_relative_entropy
    );

    srv.shutdown().await;
}

/// gRPC ForwardSample with dim=4 seed=42: returns a valid sample response.
#[tokio::test(flavor = "multi_thread")]
async fn grpc_forward_sample_dim4() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let grpc_port = srv.ports.grpc;

    let mut client = KirkServiceClient::connect(format!("http://127.0.0.1:{grpc_port}"))
        .await
        .expect("connect gRPC");

    let req = SampleSizeRequest { dim: 4, seed: 42 };
    let resp = client
        .forward_sample(req)
        .await
        .expect("gRPC ForwardSample");
    let r = resp.into_inner();

    assert!(
        r.relative_entropy.is_finite() && r.relative_entropy >= 0.0,
        "relative_entropy invalid: {}",
        r.relative_entropy
    );
    assert!(r.feature_array.is_some(), "feature_array missing");
    assert!(r.feature_vector.is_some(), "feature_vector missing");
    assert!(r.feature_scalar.is_some(), "feature_scalar missing");

    let fa = r.feature_array.unwrap();
    assert_eq!(fa.dim, 4);
    // feature_arr has N*N complex elements → N*N * 4 bytes each side
    assert_eq!(fa.data_re.len(), 4 * 4 * 4, "feature_arr re bytes wrong");
    assert_eq!(fa.data_im.len(), 4 * 4 * 4, "feature_arr im bytes wrong");

    srv.shutdown().await;
}

/// gRPC Forward with missing matrix field returns error status.
#[tokio::test(flavor = "multi_thread")]
async fn grpc_forward_missing_matrix_returns_error() {
    let srv = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start server");
    let grpc_port = srv.ports.grpc;

    let mut client = KirkServiceClient::connect(format!("http://127.0.0.1:{grpc_port}"))
        .await
        .expect("connect gRPC");

    let req = KirkRequest {
        matrix: None, // missing
        timestamp_us: 0,
    };

    let result = client.forward(req).await;
    assert!(result.is_err(), "expected error when matrix is missing");
    let status = result.unwrap_err();
    // Should be INVALID_ARGUMENT
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "expected INVALID_ARGUMENT, got {:?}",
        status.code()
    );

    srv.shutdown().await;
}

/// gRPC result agrees with REST result for the same N=4 identity input.
///
/// Since the KirkRealistic state is shared across transports (one backend),
/// we use a fresh server for each call so that the rolling-window history is
/// identical (first call → no window → zscore = 0). Then we verify that the
/// entropy values agree within the NFR-001 tolerance (1e-4 relative).
#[tokio::test(flavor = "multi_thread")]
async fn grpc_and_rest_agree_on_identity_n4() {
    // Fresh server for gRPC call.
    let srv_grpc = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start grpc server");

    let (re_bytes, im_bytes) = identity_matrix_bytes(4);
    let mut grpc_client =
        KirkServiceClient::connect(format!("http://127.0.0.1:{}", srv_grpc.ports.grpc))
            .await
            .expect("connect gRPC");

    let grpc_req = KirkRequest {
        matrix: Some(Matrix {
            dim: 4,
            data_re: re_bytes.clone(),
            data_im: im_bytes.clone(),
        }),
        timestamp_us: 1718560000000000,
    };
    let grpc_resp = grpc_client
        .forward(grpc_req)
        .await
        .expect("gRPC Forward")
        .into_inner();
    srv_grpc.shutdown().await;

    // Fresh server for REST call.
    let srv_rest = start_server(0, 0, 0, 1.0, 256, 1024)
        .await
        .expect("start rest server");

    let (re_b64, im_b64) = identity_matrix_b64(4);
    let rest_client = reqwest::Client::new();
    let rest_body = serde_json::json!({
        "matrix_re": re_b64,
        "matrix_im": im_b64,
        "matrix_dim": 4,
        "timestamp_us": 1718560000000000i64
    });
    let rest_json: serde_json::Value = rest_client
        .post(format!(
            "http://127.0.0.1:{}/v1/forward",
            srv_rest.ports.rest
        ))
        .json(&rest_body)
        .send()
        .await
        .expect("REST forward")
        .json()
        .await
        .expect("REST JSON");
    srv_rest.shutdown().await;

    let rest_entropy = rest_json["entropy"].as_f64().expect("rest entropy") as f32;
    let rest_entropy_re = rest_json["entropy_re"].as_f64().expect("rest entropy_re") as f32;
    let rest_entropy_im = rest_json["entropy_im"].as_f64().expect("rest entropy_im") as f32;
    let rest_confidence = rest_json["confidence"].as_f64().expect("rest confidence") as f32;

    let tol = 1e-4_f32;

    let rel_err = |a: f32, b: f32| -> f32 {
        let denom = b.abs().max(1.0);
        (a - b).abs() / denom
    };

    assert!(
        rel_err(grpc_resp.entropy, rest_entropy) <= tol,
        "entropy mismatch: gRPC={} REST={} rel_err={}",
        grpc_resp.entropy,
        rest_entropy,
        rel_err(grpc_resp.entropy, rest_entropy)
    );
    assert!(
        rel_err(grpc_resp.entropy_re, rest_entropy_re) <= tol,
        "entropy_re mismatch: gRPC={} REST={}",
        grpc_resp.entropy_re,
        rest_entropy_re
    );
    assert!(
        rel_err(grpc_resp.entropy_im, rest_entropy_im) <= tol,
        "entropy_im mismatch: gRPC={} REST={}",
        grpc_resp.entropy_im,
        rest_entropy_im
    );
    assert!(
        (grpc_resp.confidence - rest_confidence).abs() <= tol,
        "confidence mismatch: gRPC={} REST={}",
        grpc_resp.confidence,
        rest_confidence
    );
}
