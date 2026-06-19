//! Per-opcode payload encoders/decoders for the custom TCP protocol.

use crate::config::MAX_ALLOWED_MATRIX_DIM;
use crate::error::ServerError;
use num_complex::Complex32;

pub struct ForwardRequest {
    pub n: u32,
    pub re: Vec<f32>,
    pub im: Vec<f32>,
    pub timestamp_us: i64,
}

/// Peek at the N=u32 in the first four bytes of any FORWARD / SAMPLE payload
/// without performing any arithmetic. The handler validates this against the
/// backend's `max_matrix_dim` *before* dispatching the (potentially huge)
/// per-shape calculations in the parsers below.
pub fn peek_payload_dim(payload: &[u8]) -> Result<u32, ServerError> {
    if payload.len() < 4 {
        return Err(ServerError::BadRequest(
            "payload too short for N prefix".into(),
        ));
    }
    Ok(u32::from_le_bytes([
        payload[0], payload[1], payload[2], payload[3],
    ]))
}

/// Compute `4 + 4*N*N + 4*N*N + 8` with overflow protection. Returns
/// `BadRequest` on overflow rather than wrapping. `n` is also bounded by
/// `MAX_ALLOWED_MATRIX_DIM` so the arithmetic never gets close to wrapping.
fn forward_expected_len(n: u32) -> Result<usize, ServerError> {
    if n > MAX_ALLOWED_MATRIX_DIM {
        return Err(ServerError::BadRequest(format!(
            "dim {n} exceeds arithmetic ceiling {MAX_ALLOWED_MATRIX_DIM}"
        )));
    }
    let n_usize = n as usize;
    let m = n_usize
        .checked_mul(n_usize)
        .ok_or_else(|| ServerError::BadRequest("dim too large for arithmetic".into()))?;
    let bytes_per_part = m
        .checked_mul(4)
        .ok_or_else(|| ServerError::BadRequest("dim too large for arithmetic".into()))?;
    // 4 (N) + bytes_per_part (re) + bytes_per_part (im) + 8 (ts)
    bytes_per_part
        .checked_mul(2)
        .and_then(|v| v.checked_add(4 + 8))
        .ok_or_else(|| ServerError::BadRequest("dim too large for arithmetic".into()))
}

fn sample_expected_len(n: u32) -> Result<usize, ServerError> {
    if n > MAX_ALLOWED_MATRIX_DIM {
        return Err(ServerError::BadRequest(format!(
            "dim {n} exceeds arithmetic ceiling {MAX_ALLOWED_MATRIX_DIM}"
        )));
    }
    let n_usize = n as usize;
    let m = n_usize
        .checked_mul(n_usize)
        .ok_or_else(|| ServerError::BadRequest("dim too large for arithmetic".into()))?;
    let bytes_per_part = m
        .checked_mul(4)
        .ok_or_else(|| ServerError::BadRequest("dim too large for arithmetic".into()))?;
    // 4 (N) + bytes_per_part (re) + bytes_per_part (im)
    bytes_per_part
        .checked_mul(2)
        .and_then(|v| v.checked_add(4))
        .ok_or_else(|| ServerError::BadRequest("dim too large for arithmetic".into()))
}

pub fn parse_forward_request(payload: &[u8]) -> Result<ForwardRequest, ServerError> {
    if payload.len() < 4 {
        return Err(ServerError::BadRequest("FORWARD payload too short".into()));
    }
    let n = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let expected = forward_expected_len(n)?;
    if payload.len() != expected {
        return Err(ServerError::BadRequest(format!(
            "FORWARD payload length {} != expected {}",
            payload.len(),
            expected
        )));
    }
    let n_usize = n as usize;
    let m = n_usize * n_usize; // safe: validated by forward_expected_len
    let mut off = 4usize;
    let re = read_f32_slice(payload, off, m);
    off += 4 * m;
    let im = read_f32_slice(payload, off, m);
    off += 4 * m;
    let ts_bytes = [
        payload[off],
        payload[off + 1],
        payload[off + 2],
        payload[off + 3],
        payload[off + 4],
        payload[off + 5],
        payload[off + 6],
        payload[off + 7],
    ];
    let timestamp_us = i64::from_le_bytes(ts_bytes);
    Ok(ForwardRequest {
        n,
        re,
        im,
        timestamp_us,
    })
}

pub struct SampleRequest {
    pub n: u32,
    pub re: Vec<f32>,
    pub im: Vec<f32>,
}

pub fn parse_sample_request(payload: &[u8]) -> Result<SampleRequest, ServerError> {
    if payload.len() < 4 {
        return Err(ServerError::BadRequest("sample payload too short".into()));
    }
    let n = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let expected = sample_expected_len(n)?;
    if payload.len() != expected {
        return Err(ServerError::BadRequest(format!(
            "sample payload length {} != expected {}",
            payload.len(),
            expected
        )));
    }
    let n_usize = n as usize;
    let m = n_usize * n_usize; // safe: validated by sample_expected_len
    let mut off = 4usize;
    let re = read_f32_slice(payload, off, m);
    off += 4 * m;
    let im = read_f32_slice(payload, off, m);
    Ok(SampleRequest { n, re, im })
}

pub struct ForwardSampleRequest {
    pub n: u32,
    pub seed: u64,
}

pub fn parse_forward_sample_request(payload: &[u8]) -> Result<ForwardSampleRequest, ServerError> {
    if payload.len() != 12 {
        return Err(ServerError::BadRequest(format!(
            "FORWARD_SAMPLE payload length {} != 12",
            payload.len()
        )));
    }
    let n = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let seed = u64::from_le_bytes([
        payload[4],
        payload[5],
        payload[6],
        payload[7],
        payload[8],
        payload[9],
        payload[10],
        payload[11],
    ]);
    Ok(ForwardSampleRequest { n, seed })
}

#[inline]
fn read_f32_slice(buf: &[u8], offset: usize, count: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let p = offset + i * 4;
        out.push(f32::from_le_bytes([
            buf[p],
            buf[p + 1],
            buf[p + 2],
            buf[p + 3],
        ]));
    }
    out
}

#[allow(clippy::too_many_arguments)] // all args are semantically distinct fields in the wire format
pub fn encode_forward_response(
    out: &mut Vec<u8>,
    entropy_re: f32,
    entropy_im: f32,
    entropy: f32,
    entropy_zscore: f32,
    regime: u32,
    confidence: f32,
    processing_time_us: u64,
    timestamp_us: i64,
    n: u32,
    rho_re: &[f32],
    rho_im: &[f32],
) {
    out.extend_from_slice(&entropy_re.to_le_bytes());
    out.extend_from_slice(&entropy_im.to_le_bytes());
    out.extend_from_slice(&entropy.to_le_bytes());
    out.extend_from_slice(&entropy_zscore.to_le_bytes());
    out.extend_from_slice(&regime.to_le_bytes());
    out.extend_from_slice(&confidence.to_le_bytes());
    out.extend_from_slice(&processing_time_us.to_le_bytes());
    out.extend_from_slice(&timestamp_us.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    for v in rho_re {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in rho_im {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

pub fn encode_entropy_response(out: &mut Vec<u8>, entropy: f32) {
    out.extend_from_slice(&entropy.to_le_bytes());
}

pub fn encode_features_response(
    out: &mut Vec<u8>,
    n: u32,
    feature_arr: &[Complex32],
    feature_vec: &[Complex32],
    feature_scalar: Complex32,
) {
    out.extend_from_slice(&n.to_le_bytes());
    for c in feature_arr {
        out.extend_from_slice(&c.re.to_le_bytes());
    }
    for c in feature_arr {
        out.extend_from_slice(&c.im.to_le_bytes());
    }
    for c in feature_vec {
        out.extend_from_slice(&c.re.to_le_bytes());
    }
    for c in feature_vec {
        out.extend_from_slice(&c.im.to_le_bytes());
    }
    out.extend_from_slice(&feature_scalar.re.to_le_bytes());
    out.extend_from_slice(&feature_scalar.im.to_le_bytes());
}
