//! Error and output dataclasses (parity with Python `output.py`).

use num_complex::Complex32;
use thiserror::Error;

/// Maximum bytes accepted on the wire / from a base64 decode (mirrors Python
/// `_MAX_DECODED_BYTES = 64 * 1024 * 1024`).
pub const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum KirkError {
    #[error("matrix_re and matrix_im must have equal shape; got ({rows_re}, {cols_re}) vs ({rows_im}, {cols_im})")]
    ShapeMismatch {
        rows_re: usize,
        cols_re: usize,
        rows_im: usize,
        cols_im: usize,
    },
    #[error("matrix must be square and 2-D with dim >= 1; got dim={dim}, len={len}")]
    NotSquare { dim: usize, len: usize },
    #[error("matrix is empty")]
    Empty,
    #[error("temperature must be > 0; got {0}")]
    BadTemperature(f32),
    #[error("window_size must be >= 1; got {0}")]
    BadWindow(usize),
    #[error("matrix dimension {requested} exceeds limit {limit}")]
    DimExceeded { requested: usize, limit: usize },
    #[error("eigendecomposition failed to converge")]
    EigenFailure,
    #[error("payload exceeds maximum decoded size {MAX_DECODED_BYTES} bytes (got {0})")]
    PayloadTooLarge(usize),
}

/// Pipeline output mirroring Python `KirkOutput`.
#[derive(Debug, Clone)]
pub struct KirkOutput {
    pub entropy_re: f32,
    pub entropy_im: f32,
    pub entropy: f32,
    pub entropy_zscore: f32,
    pub regime: u32,
    pub confidence: f32,
    pub processing_time_us: u64,
    pub timestamp_us: i64,
    /// Density-matrix real part, row-major flattened, length = `matrix_dim * matrix_dim`.
    pub matrix_re: Vec<f32>,
    /// Density-matrix imaginary part, row-major flattened, length = `matrix_dim * matrix_dim`.
    pub matrix_im: Vec<f32>,
}

impl KirkOutput {
    /// Returns N where the density matrix is `N x N`.
    #[inline]
    pub fn matrix_dim(&self) -> usize {
        (self.matrix_re.len() as f64).sqrt() as usize
    }
}

/// Joel-shape sample output (mirrors Python `KirkSampleOutput`).
#[derive(Debug, Clone)]
pub struct KirkSampleOutput {
    /// (N, N) row-major complex.
    pub feature_array: Vec<Complex32>,
    /// (2N,) complex vector — concatenation of N row-means and N col-means in the
    /// stateful variant; pure-random in the `forward_sample` sample-style helper.
    pub feature_vector: Vec<Complex32>,
    /// Scalar complex (mean of the array in the stateful variant; random in
    /// `forward_sample`).
    pub feature_scalar: Complex32,
    /// Non-negative real (Shannon entropy of Boltzmann distribution / random in
    /// `forward_sample`).
    pub relative_entropy: f32,
    /// `N` accessor.
    pub matrix_dim: usize,
}
