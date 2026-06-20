//! REST `/v2/` JSON-array serde envelopes.
//!
//! Complex matrices, vectors, and scalars are represented as nested `[re, im]`
//! pairs of f64. Internally the trait surface still uses f32; the v2 handler
//! converts at the boundary.

use serde::{Deserialize, Serialize};

use crate::error::ServerError;

/// 3-D nested array: rows x cols x [re, im].
pub type ComplexMatrixJson = Vec<Vec<[f64; 2]>>;
/// 1-D vector of [re, im].
pub type ComplexVecJson = Vec<[f64; 2]>;
/// Scalar [re, im].
pub type ComplexScalarJson = [f64; 2];

#[derive(Debug, Deserialize)]
pub struct ForwardV2Request {
    pub matrix: ComplexMatrixJson,
    #[serde(default)]
    pub timestamp_us: i64,
}

#[derive(Debug, Deserialize)]
pub struct SampleV2Request {
    pub matrix: ComplexMatrixJson,
}

#[derive(Debug, Deserialize)]
pub struct ForwardSampleV2Request {
    pub matrix_dim: u32,
    pub seed: u64,
}

#[derive(Debug, Serialize)]
pub struct ForwardV2Response {
    pub entropy_re: f32,
    pub entropy_im: f32,
    pub entropy: f32,
    pub entropy_zscore: f32,
    pub regime: u32,
    pub confidence: f32,
    pub processing_time_us: u64,
    pub timestamp_us: i64,
    pub matrix: ComplexMatrixJson,
}

#[derive(Debug, Serialize)]
pub struct EntropyV2Response {
    pub total_relative_entropy: f32,
}

#[derive(Debug, Serialize)]
pub struct FeaturesV2Response {
    pub feature_arr: ComplexMatrixJson,
    pub feature_vec: ComplexVecJson,
    pub feature_scalar: ComplexScalarJson,
}

#[derive(Debug, Serialize)]
pub struct ActiveInferenceV2Response {
    pub feature_arr: ComplexMatrixJson,
    pub feature_vec: ComplexVecJson,
    pub feature_scalar: ComplexScalarJson,
    pub total_relative_entropy: f32,
}

#[derive(Debug, Serialize)]
pub struct SampleV2Response {
    pub feature_array: ComplexMatrixJson,
    pub feature_vector: ComplexVecJson,
    pub feature_scalar: ComplexScalarJson,
    pub relative_entropy: f32,
}

/// Validate the nested matrix shape and cast each `[f64; 2]` element to
/// `(f32 re, f32 im)`. Rejects non-square, jagged rows, non-finite floats, and
/// dims outside `[2, max_dim]`.
pub fn parse_matrix_v2(
    m: &ComplexMatrixJson,
    max_dim: u32,
) -> Result<(Vec<f32>, Vec<f32>, usize), ServerError> {
    let n = m.len();
    if n < 2 {
        return Err(ServerError::BadRequest(
            "matrix dim must be >= 2 (N=0 is empty, N=1 leaves entropy / confidence undefined)"
                .into(),
        ));
    }
    if n as u32 > max_dim {
        return Err(ServerError::MatrixDimExceeded {
            actual: n as u32,
            limit: max_dim,
        });
    }
    let mut re = Vec::with_capacity(n * n);
    let mut im = Vec::with_capacity(n * n);
    for (i, row) in m.iter().enumerate() {
        if row.len() != n {
            return Err(ServerError::BadRequest(format!(
                "row {} length {} != matrix dim {}",
                i,
                row.len(),
                n
            )));
        }
        for (j, pair) in row.iter().enumerate() {
            let r = pair[0];
            let im_v = pair[1];
            if !r.is_finite() || !im_v.is_finite() {
                return Err(ServerError::BadRequest(format!(
                    "non-finite element at ({i}, {j})"
                )));
            }
            re.push(r as f32);
            im.push(im_v as f32);
        }
    }
    Ok((re, im, n))
}

/// Re-pack a flat row-major `Vec<f32>` real/imag pair as nested `[[[re, im],...],...]`.
pub fn encode_matrix_v2(re: &[f32], im: &[f32], n: usize) -> ComplexMatrixJson {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(n);
        for j in 0..n {
            let idx = i * n + j;
            row.push([re[idx] as f64, im[idx] as f64]);
        }
        out.push(row);
    }
    out
}

/// Re-pack a flat complex vector as nested `[[re, im], ...]`.
pub fn encode_vec_v2(arr: &[num_complex::Complex32]) -> ComplexVecJson {
    arr.iter().map(|c| [c.re as f64, c.im as f64]).collect()
}

/// Re-pack a flat complex matrix (slice of Complex32) as nested NxN `[[re, im],...]`.
pub fn encode_complex_matrix_v2(arr: &[num_complex::Complex32], n: usize) -> ComplexMatrixJson {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(n);
        for j in 0..n {
            let c = arr[i * n + j];
            row.push([c.re as f64, c.im as f64]);
        }
        out.push(row);
    }
    out
}
