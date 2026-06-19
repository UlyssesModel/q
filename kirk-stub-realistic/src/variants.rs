//! Five Joel-API variants (stateless) — port of Python `variants.py`.
//!
//! All variants take a complex sample `(sample_re, sample_im, n)`. The real-only
//! case is signalled by passing `sample_im` as zeros (which is the standard
//! Python `_split_sample` behavior for real input).

use crate::density_matrix::density_matrix;
use crate::eigensolver::{diagonalize, Hermitian};
use crate::entropy::shannon_entropy;
use crate::output::KirkError;
use num_complex::Complex32;

const DEFAULT_TEMPERATURE: f32 = 1.0;

struct PipelineOutputs {
    rho: Vec<Complex32>,
    p: Vec<f32>,
    n: usize,
}

fn run_pipeline(
    sample_re: &[f32],
    sample_im: &[f32],
    n: usize,
) -> Result<PipelineOutputs, KirkError> {
    let h = Hermitian::from_parts(sample_re, sample_im, n)?;
    let eigh = diagonalize(&h)?;
    let (rho, p) = density_matrix(&eigh, DEFAULT_TEMPERATURE)?;
    Ok(PipelineOutputs { rho, p, n })
}

/// Features tuple: `(feature_arr (N*N row-major), feature_vec (2N), feature_scalar)`.
pub struct Features {
    pub feature_arr: Vec<Complex32>,
    pub feature_vec: Vec<Complex32>,
    pub feature_scalar: Complex32,
    pub n: usize,
}

fn features_from_rho(rho: &[Complex32], n: usize) -> Features {
    let mut row_means = vec![Complex32::new(0.0, 0.0); n];
    let mut col_means = vec![Complex32::new(0.0, 0.0); n];
    let denom = Complex32::new(n as f32, 0.0);
    for i in 0..n {
        let mut row = Complex32::new(0.0, 0.0);
        let mut col = Complex32::new(0.0, 0.0);
        for j in 0..n {
            row += rho[i * n + j];
            col += rho[j * n + i];
        }
        row_means[i] = row / denom;
        col_means[i] = col / denom;
    }
    let mut feature_vec = Vec::with_capacity(2 * n);
    feature_vec.extend_from_slice(&row_means);
    feature_vec.extend_from_slice(&col_means);

    let mut total = Complex32::new(0.0, 0.0);
    for c in rho {
        total += *c;
    }
    let feature_scalar = total / Complex32::new((n * n) as f32, 0.0);

    Features {
        feature_arr: rho.to_vec(),
        feature_vec,
        feature_scalar,
        n,
    }
}

pub fn inference_entropy(sample_re: &[f32], sample_im: &[f32], n: usize) -> Result<f32, KirkError> {
    let out = run_pipeline(sample_re, sample_im, n)?;
    Ok(shannon_entropy(&out.p))
}

pub fn inference_features(
    sample_re: &[f32],
    sample_im: &[f32],
    n: usize,
) -> Result<Features, KirkError> {
    let out = run_pipeline(sample_re, sample_im, n)?;
    Ok(features_from_rho(&out.rho, out.n))
}

pub struct ActiveInferenceOut {
    pub features: Features,
    pub total_relative_entropy: f32,
}

pub fn active_inference(
    sample_re: &[f32],
    sample_im: &[f32],
    n: usize,
) -> Result<ActiveInferenceOut, KirkError> {
    let out = run_pipeline(sample_re, sample_im, n)?;
    let entropy = shannon_entropy(&out.p);
    Ok(ActiveInferenceOut {
        features: features_from_rho(&out.rho, out.n),
        total_relative_entropy: entropy,
    })
}

pub fn active_inference_entropy(
    sample_re: &[f32],
    sample_im: &[f32],
    n: usize,
) -> Result<f32, KirkError> {
    inference_entropy(sample_re, sample_im, n)
}

pub fn active_inference_features(
    sample_re: &[f32],
    sample_im: &[f32],
    n: usize,
) -> Result<Features, KirkError> {
    inference_features(sample_re, sample_im, n)
}
