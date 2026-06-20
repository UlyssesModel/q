//! axum routes for the REST surface.

use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};

/// REST request body cap (mirrors `kirk_stub_realistic::output::MAX_DECODED_BYTES`).
/// SEC-006: axum's framework default is 2 MiB which silently caps `matrix_dim`
/// well below the architect spec's documented `--max-matrix-dim = 1024`.
pub const REST_BODY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
use serde::Serialize;

use crate::backend::KirkBackend;
use crate::error::ServerError;
use crate::metrics::MetricsHandle;

use super::schema::{
    decode_f32_matrix, encode_complex_split, encode_f32, ActiveInferenceResponse, EntropyResponse,
    ErrorResponse, FeatureScalarJson, FeaturesResponse, ForwardRequest, ForwardResponse,
    SampleRequest, SampleResponse, SampleSizeRequest,
};

#[derive(Clone)]
pub struct RestState {
    pub backend: Arc<KirkBackend>,
    pub metrics: MetricsHandle,
    pub shutdown: Arc<std::sync::atomic::AtomicBool>,
}

pub fn build_router(state: RestState) -> Router {
    use super::routes_v2::{
        active_inference_entropy_v2, active_inference_features_v2, active_inference_v2,
        forward_sample_v2, forward_v2, inference_entropy_v2, inference_features_v2,
    };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics_endpoint))
        .route("/v1/forward", post(forward))
        .route("/v1/inference/entropy", post(inference_entropy))
        .route("/v1/inference/features", post(inference_features))
        .route("/v1/active-inference", post(active_inference))
        .route(
            "/v1/active-inference/entropy",
            post(active_inference_entropy),
        )
        .route(
            "/v1/active-inference/features",
            post(active_inference_features),
        )
        .route("/v1/forward-sample", post(forward_sample))
        // v2: nested JSON [re, im] envelope.
        .route("/v2/forward", post(forward_v2))
        .route("/v2/inference/entropy", post(inference_entropy_v2))
        .route("/v2/inference/features", post(inference_features_v2))
        .route("/v2/active-inference", post(active_inference_v2))
        .route(
            "/v2/active-inference/entropy",
            post(active_inference_entropy_v2),
        )
        .route(
            "/v2/active-inference/features",
            post(active_inference_features_v2),
        )
        .route("/v2/forward-sample", post(forward_sample_v2))
        // SEC-006: align the HTTP body cap with the TCP `MAX_PAYLOAD` so the
        // documented `--max-matrix-dim = 1024` is actually reachable. Without
        // this, axum's 2 MiB default silently rejects any `matrix_dim` above
        // ~325. The downstream `decode_f32_matrix` enforces a *tighter*
        // per-request shape budget so this cap is a backstop, not the only
        // bound (see SEC-001).
        .layer(DefaultBodyLimit::max(REST_BODY_LIMIT_BYTES))
        .with_state(state)
}

pub(super) fn err_response(err: ServerError) -> Response {
    let status =
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = ErrorResponse {
        error: err.code().to_string(),
        message: err.to_string(),
    };
    (status, Json(body)).into_response()
}

fn ok<T: Serialize>(v: T) -> Response {
    Json(v).into_response()
}

pub(super) fn ok_json<T: Serialize>(v: T) -> Response {
    ok(v)
}

pub(super) fn time_observe(metrics: &MetricsHandle, op: &str, t0: Instant, ok: bool) {
    let dt = t0.elapsed().as_micros() as f64;
    metrics.observe("rest", op, dt, ok);
}

async fn healthz(State(state): State<RestState>) -> Response {
    if state.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status":"shutdown"})),
        )
            .into_response()
    } else {
        Json(serde_json::json!({"status":"ok"})).into_response()
    }
}

async fn metrics_endpoint(State(state): State<RestState>) -> Response {
    let body = state.metrics.render_prometheus();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        body,
    )
        .into_response()
}

async fn forward(State(state): State<RestState>, Json(req): Json<ForwardRequest>) -> Response {
    let t0 = Instant::now();
    let res = forward_inner(state.clone(), req).await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "forward", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

async fn forward_inner(
    state: RestState,
    req: ForwardRequest,
) -> Result<ForwardResponse, ServerError> {
    let n = req.matrix_dim;
    state.backend.check_dim(n)?;
    let n_usize = n as usize;
    let len = n_usize * n_usize;
    let matrix_re = decode_f32_matrix(&req.matrix_re, len)?;
    let matrix_im = decode_f32_matrix(&req.matrix_im, len)?;
    let out = state
        .backend
        .clone()
        .forward(matrix_re, matrix_im, n_usize, req.timestamp_us)
        .await?;
    Ok(ForwardResponse {
        entropy_re: out.entropy_re,
        entropy_im: out.entropy_im,
        entropy: out.entropy,
        entropy_zscore: out.entropy_zscore,
        regime: out.regime,
        confidence: out.confidence,
        processing_time_us: out.processing_time_us,
        timestamp_us: out.timestamp_us,
        matrix_re: encode_f32(&out.matrix_re),
        matrix_im: encode_f32(&out.matrix_im),
        matrix_dim: n,
    })
}

fn decode_sample(req: &SampleRequest) -> Result<(Vec<f32>, Vec<f32>, usize), ServerError> {
    let n = req.matrix_dim as usize;
    let len = n * n;
    let re = decode_f32_matrix(&req.sample_re, len)?;
    let im = decode_f32_matrix(&req.sample_im, len)?;
    Ok((re, im, n))
}

async fn inference_entropy(
    State(state): State<RestState>,
    Json(req): Json<SampleRequest>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        state.backend.check_dim(req.matrix_dim)?;
        let (re, im, n) = decode_sample(&req)?;
        let e = state.backend.clone().inference_entropy(re, im, n).await?;
        Ok::<_, ServerError>(EntropyResponse {
            total_relative_entropy: e,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "inference_entropy", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

fn features_to_response(features: kirk_stub_realistic::variants::Features) -> FeaturesResponse {
    let n = features.n as u32;
    let (arr_re, arr_im) = encode_complex_split(&features.feature_arr);
    let (vec_re, vec_im) = encode_complex_split(&features.feature_vec);
    FeaturesResponse {
        feature_arr_re: arr_re,
        feature_arr_im: arr_im,
        feature_vec_re: vec_re,
        feature_vec_im: vec_im,
        feature_scalar: FeatureScalarJson {
            re: features.feature_scalar.re,
            im: features.feature_scalar.im,
        },
        matrix_dim: n,
    }
}

async fn inference_features(
    State(state): State<RestState>,
    Json(req): Json<SampleRequest>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        state.backend.check_dim(req.matrix_dim)?;
        let (re, im, n) = decode_sample(&req)?;
        let f = state.backend.clone().inference_features(re, im, n).await?;
        Ok::<_, ServerError>(features_to_response(f))
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "inference_features", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

async fn active_inference(
    State(state): State<RestState>,
    Json(req): Json<SampleRequest>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        state.backend.check_dim(req.matrix_dim)?;
        let (re, im, n) = decode_sample(&req)?;
        let out = state.backend.clone().active_inference(re, im, n).await?;
        let f = out.features;
        let n_u32 = f.n as u32;
        let (arr_re, arr_im) = encode_complex_split(&f.feature_arr);
        let (vec_re, vec_im) = encode_complex_split(&f.feature_vec);
        Ok::<_, ServerError>(ActiveInferenceResponse {
            feature_arr_re: arr_re,
            feature_arr_im: arr_im,
            feature_vec_re: vec_re,
            feature_vec_im: vec_im,
            feature_scalar: FeatureScalarJson {
                re: f.feature_scalar.re,
                im: f.feature_scalar.im,
            },
            matrix_dim: n_u32,
            total_relative_entropy: out.total_relative_entropy,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "active_inference", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

async fn active_inference_entropy(
    State(state): State<RestState>,
    Json(req): Json<SampleRequest>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        state.backend.check_dim(req.matrix_dim)?;
        let (re, im, n) = decode_sample(&req)?;
        let e = state
            .backend
            .clone()
            .active_inference_entropy(re, im, n)
            .await?;
        Ok::<_, ServerError>(EntropyResponse {
            total_relative_entropy: e,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "active_inference_entropy", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

async fn active_inference_features(
    State(state): State<RestState>,
    Json(req): Json<SampleRequest>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        state.backend.check_dim(req.matrix_dim)?;
        let (re, im, n) = decode_sample(&req)?;
        let f = state
            .backend
            .clone()
            .active_inference_features(re, im, n)
            .await?;
        Ok::<_, ServerError>(features_to_response(f))
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "active_inference_features", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

async fn forward_sample(
    State(state): State<RestState>,
    Json(req): Json<SampleSizeRequest>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        state.backend.check_dim(req.matrix_dim)?;
        let out = state
            .backend
            .clone()
            .forward_sample(req.matrix_dim as usize, req.seed)
            .await?;
        let n_u32 = out.matrix_dim as u32;
        let (arr_re, arr_im) = encode_complex_split(&out.feature_array);
        let (vec_re, vec_im) = encode_complex_split(&out.feature_vector);
        Ok::<_, ServerError>(SampleResponse {
            feature_array_re: arr_re,
            feature_array_im: arr_im,
            feature_vector_re: vec_re,
            feature_vector_im: vec_im,
            feature_scalar: FeatureScalarJson {
                re: out.feature_scalar.re,
                im: out.feature_scalar.im,
            },
            matrix_dim: n_u32,
            relative_entropy: out.relative_entropy,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "forward_sample", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}
