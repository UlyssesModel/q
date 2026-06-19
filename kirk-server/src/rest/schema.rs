//! Serde envelopes for the REST surface. Field names mirror the Python
//! `to_kafka_envelope` schema. Matrices are base64-encoded little-endian f32.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use kirk_stub_realistic::output::MAX_DECODED_BYTES;
use num_complex::Complex32;
use serde::{Deserialize, Serialize};

use crate::error::ServerError;

#[derive(Debug, Deserialize)]
pub struct ForwardRequest {
    pub matrix_re: String,
    pub matrix_im: String,
    pub matrix_dim: u32,
    #[serde(default)]
    pub timestamp_us: i64,
}

#[derive(Debug, Serialize)]
pub struct ForwardResponse {
    pub entropy_re: f32,
    pub entropy_im: f32,
    pub entropy: f32,
    pub entropy_zscore: f32,
    pub regime: u32,
    pub confidence: f32,
    pub processing_time_us: u64,
    pub timestamp_us: i64,
    pub matrix_re: String,
    pub matrix_im: String,
    pub matrix_dim: u32,
}

#[derive(Debug, Deserialize)]
pub struct SampleRequest {
    pub sample_re: String,
    pub sample_im: String,
    pub matrix_dim: u32,
}

#[derive(Debug, Serialize)]
pub struct EntropyResponse {
    pub total_relative_entropy: f32,
}

#[derive(Debug, Serialize)]
pub struct FeatureScalarJson {
    pub re: f32,
    pub im: f32,
}

#[derive(Debug, Serialize)]
pub struct FeaturesResponse {
    pub feature_arr_re: String,
    pub feature_arr_im: String,
    pub feature_vec_re: String,
    pub feature_vec_im: String,
    pub feature_scalar: FeatureScalarJson,
    pub matrix_dim: u32,
}

#[derive(Debug, Serialize)]
pub struct ActiveInferenceResponse {
    pub feature_arr_re: String,
    pub feature_arr_im: String,
    pub feature_vec_re: String,
    pub feature_vec_im: String,
    pub feature_scalar: FeatureScalarJson,
    pub matrix_dim: u32,
    pub total_relative_entropy: f32,
}

#[derive(Debug, Deserialize)]
pub struct SampleSizeRequest {
    pub matrix_dim: u32,
    pub seed: u64,
}

#[derive(Debug, Serialize)]
pub struct SampleResponse {
    pub feature_array_re: String,
    pub feature_array_im: String,
    pub feature_vector_re: String,
    pub feature_vector_im: String,
    pub feature_scalar: FeatureScalarJson,
    pub matrix_dim: u32,
    pub relative_entropy: f32,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

/// Safe upper bound on the decoded byte length of a standard base64 input of
/// `b64_len` characters. Equivalent to `((b64_len + 3) / 4) * 3`. Always
/// `>=` the actual decoded length (overestimates by up to 2 bytes when
/// padding is present), so we can reject *before* allocating.
#[inline]
fn base64_decoded_upper_bound(b64_len: usize) -> usize {
    // Ceil divide by 4, multiply by 3.
    b64_len.saturating_add(3) / 4 * 3
}

/// Base64-decode + length-check + bytes → Vec<f32> (little-endian).
///
/// SEC-001: All size validation happens *before* `B64.decode` allocates, so an
/// attacker-supplied oversized base64 string cannot force a giant allocation.
///
/// Two-step check:
///   1. Reject the request before decoding if the *upper-bound* decoded size
///      already exceeds the global `MAX_DECODED_BYTES` cap. Caps the worst-case
///      allocation at ~`MAX_DECODED_BYTES` regardless of input length.
///   2. Reject the request before decoding if the upper-bound decoded size
///      is materially larger than the per-shape budget `4 * expected_len`.
///      A 2-byte slack accommodates standard-base64 padding (the upper bound
///      overestimates the actual decoded size by up to 2 bytes per padded
///      block).
///   3. Decode into a pre-sized `Vec` so the allocation is bounded by the
///      validated `expected_bytes`, not by the attacker-controlled input
///      length.
///   4. Final exact-length check on the decoded buffer.
pub fn decode_f32_matrix(b64: &str, expected_len: usize) -> Result<Vec<f32>, ServerError> {
    let expected_bytes = expected_len
        .checked_mul(4)
        .ok_or_else(|| ServerError::BadRequest("decoded length overflows usize".into()))?;
    // Upper bound on the decoded size, computed *without* touching the decoder.
    let upper = base64_decoded_upper_bound(b64.len());
    // Global cap: hard-reject if the wire-encoded payload would decode to more
    // than MAX_DECODED_BYTES even after accounting for padding.
    if upper.saturating_sub(2) > MAX_DECODED_BYTES {
        return Err(ServerError::PayloadTooLarge {
            actual: upper,
            limit: MAX_DECODED_BYTES,
        });
    }
    // Per-shape budget: hard-reject if the upper bound is materially larger
    // than the expected per-shape decode. The `+ 2` slack handles padding bias
    // (the upper bound overstates the true decoded size by at most 2 bytes).
    if upper > expected_bytes.saturating_add(2) {
        return Err(ServerError::BadRequest(format!(
            "matrix buffer base64 length {} exceeds per-shape budget {} (upper-bound decode {})",
            b64.len(),
            expected_bytes,
            upper
        )));
    }
    // Decode into a pre-sized buffer so the allocation is bounded by the
    // validated `expected_bytes`, not by the input length.
    let mut raw = Vec::<u8>::with_capacity(expected_bytes);
    // base64 0.22's `decode_vec` reuses the supplied capacity.
    B64.decode_vec(b64.as_bytes(), &mut raw)
        .map_err(|e| ServerError::BadRequest(format!("base64 decode failed: {e}")))?;
    // Final guard: in case the decoded size differs from the upper-bound
    // estimate (padding-aware), enforce the same caps strictly.
    if raw.len() > MAX_DECODED_BYTES {
        return Err(ServerError::PayloadTooLarge {
            actual: raw.len(),
            limit: MAX_DECODED_BYTES,
        });
    }
    if raw.len() != expected_bytes {
        return Err(ServerError::BadRequest(format!(
            "matrix buffer length {} != expected {} bytes",
            raw.len(),
            expected_bytes
        )));
    }
    let mut out = Vec::with_capacity(expected_len);
    for chunk in raw.chunks_exact(4) {
        let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
        out.push(f32::from_le_bytes(bytes));
    }
    Ok(out)
}

/// Vec<f32> → base64 little-endian.
pub fn encode_f32(vals: &[f32]) -> String {
    let mut buf = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    B64.encode(&buf)
}

/// Vec<Complex32> → (base64 re, base64 im).
pub fn encode_complex_split(arr: &[Complex32]) -> (String, String) {
    let mut re = Vec::with_capacity(arr.len() * 4);
    let mut im = Vec::with_capacity(arr.len() * 4);
    for c in arr {
        re.extend_from_slice(&c.re.to_le_bytes());
        im.extend_from_slice(&c.im.to_le_bytes());
    }
    (B64.encode(&re), B64.encode(&im))
}
