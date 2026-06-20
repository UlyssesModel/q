//! REST `/v2/` route handlers. Mirrors v1 semantics with a JSON-array
//! request/response envelope and f64-on-the-wire.

use std::time::Instant;

use axum::{extract::State, response::Response, Json};

use crate::error::ServerError;

use super::routes::{err_response, ok_json, time_observe, RestState};
use super::schema_v2::{
    encode_complex_matrix_v2, encode_matrix_v2, encode_vec_v2, parse_matrix_v2,
    ActiveInferenceV2Response, EntropyV2Response, FeaturesV2Response, ForwardSampleV2Request,
    ForwardV2Request, ForwardV2Response, SampleV2Request, SampleV2Response,
};

fn request_dim(state: &RestState, m_len: usize) -> Result<u32, ServerError> {
    if m_len > u32::MAX as usize {
        return Err(ServerError::BadRequest("matrix dim overflows u32".into()));
    }
    let n = m_len as u32;
    state.backend.check_dim(n)?;
    Ok(n)
}

pub async fn forward_v2(
    State(state): State<RestState>,
    Json(req): Json<ForwardV2Request>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        let _ = request_dim(&state, req.matrix.len())?;
        let (re, im, n) = parse_matrix_v2(&req.matrix, state.backend.max_matrix_dim)?;
        let out = state
            .backend
            .clone()
            .forward(re, im, n, req.timestamp_us)
            .await?;
        let matrix = encode_matrix_v2(&out.matrix_re, &out.matrix_im, n);
        Ok::<_, ServerError>(ForwardV2Response {
            entropy_re: out.entropy_re,
            entropy_im: out.entropy_im,
            entropy: out.entropy,
            entropy_zscore: out.entropy_zscore,
            regime: out.regime,
            confidence: out.confidence,
            processing_time_us: out.processing_time_us,
            timestamp_us: out.timestamp_us,
            matrix,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "forward_v2", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

pub async fn inference_entropy_v2(
    State(state): State<RestState>,
    Json(req): Json<SampleV2Request>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        let _ = request_dim(&state, req.matrix.len())?;
        let (re, im, n) = parse_matrix_v2(&req.matrix, state.backend.max_matrix_dim)?;
        let e = state.backend.clone().inference_entropy(re, im, n).await?;
        Ok::<_, ServerError>(EntropyV2Response {
            total_relative_entropy: e,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "inference_entropy_v2", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

pub async fn inference_features_v2(
    State(state): State<RestState>,
    Json(req): Json<SampleV2Request>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        let _ = request_dim(&state, req.matrix.len())?;
        let (re, im, n) = parse_matrix_v2(&req.matrix, state.backend.max_matrix_dim)?;
        let f = state.backend.clone().inference_features(re, im, n).await?;
        Ok::<_, ServerError>(FeaturesV2Response {
            feature_arr: encode_complex_matrix_v2(&f.feature_arr, f.n),
            feature_vec: encode_vec_v2(&f.feature_vec),
            feature_scalar: [f.feature_scalar.re as f64, f.feature_scalar.im as f64],
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "inference_features_v2", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

pub async fn active_inference_v2(
    State(state): State<RestState>,
    Json(req): Json<SampleV2Request>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        let _ = request_dim(&state, req.matrix.len())?;
        let (re, im, n) = parse_matrix_v2(&req.matrix, state.backend.max_matrix_dim)?;
        let out = state.backend.clone().active_inference(re, im, n).await?;
        let f = out.features;
        Ok::<_, ServerError>(ActiveInferenceV2Response {
            feature_arr: encode_complex_matrix_v2(&f.feature_arr, f.n),
            feature_vec: encode_vec_v2(&f.feature_vec),
            feature_scalar: [f.feature_scalar.re as f64, f.feature_scalar.im as f64],
            total_relative_entropy: out.total_relative_entropy,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "active_inference_v2", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

pub async fn active_inference_entropy_v2(
    State(state): State<RestState>,
    Json(req): Json<SampleV2Request>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        let _ = request_dim(&state, req.matrix.len())?;
        let (re, im, n) = parse_matrix_v2(&req.matrix, state.backend.max_matrix_dim)?;
        let e = state
            .backend
            .clone()
            .active_inference_entropy(re, im, n)
            .await?;
        Ok::<_, ServerError>(EntropyV2Response {
            total_relative_entropy: e,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "active_inference_entropy_v2", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

pub async fn active_inference_features_v2(
    State(state): State<RestState>,
    Json(req): Json<SampleV2Request>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        let _ = request_dim(&state, req.matrix.len())?;
        let (re, im, n) = parse_matrix_v2(&req.matrix, state.backend.max_matrix_dim)?;
        let f = state
            .backend
            .clone()
            .active_inference_features(re, im, n)
            .await?;
        Ok::<_, ServerError>(FeaturesV2Response {
            feature_arr: encode_complex_matrix_v2(&f.feature_arr, f.n),
            feature_vec: encode_vec_v2(&f.feature_vec),
            feature_scalar: [f.feature_scalar.re as f64, f.feature_scalar.im as f64],
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "active_inference_features_v2", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}

pub async fn forward_sample_v2(
    State(state): State<RestState>,
    Json(req): Json<ForwardSampleV2Request>,
) -> Response {
    let t0 = Instant::now();
    let res = async {
        state.backend.check_dim(req.matrix_dim)?;
        let out = state
            .backend
            .clone()
            .forward_sample(req.matrix_dim as usize, req.seed)
            .await?;
        Ok::<_, ServerError>(SampleV2Response {
            feature_array: encode_complex_matrix_v2(&out.feature_array, out.matrix_dim),
            feature_vector: encode_vec_v2(&out.feature_vector),
            feature_scalar: [out.feature_scalar.re as f64, out.feature_scalar.im as f64],
            relative_entropy: out.relative_entropy,
        })
    }
    .await;
    let ok = res.is_ok();
    time_observe(&state.metrics, "forward_sample_v2", t0, ok);
    match res {
        Ok(r) => ok_json(r),
        Err(e) => err_response(e),
    }
}
