//! Stages 3-4: Boltzmann softmax + density-matrix reconstruction.

use crate::eigensolver::Eigh;
use crate::output::KirkError;
use num_complex::Complex32;

/// Numerically-stable softmax of `-eigenvalues / temperature`.
pub fn boltzmann_weights(eigenvalues: &[f32], temperature: f32) -> Result<Vec<f32>, KirkError> {
    if temperature <= 0.0 {
        return Err(KirkError::BadTemperature(temperature));
    }
    let n = eigenvalues.len();
    if n == 0 {
        return Err(KirkError::Empty);
    }
    // x_i = -lam_i / T; subtract max for stability before exp.
    let mut x: Vec<f32> = eigenvalues.iter().map(|&l| -l / temperature).collect();
    let mut m = f32::NEG_INFINITY;
    for &v in &x {
        if v > m {
            m = v;
        }
    }
    for v in x.iter_mut() {
        *v = (*v - m).exp();
    }
    let s: f32 = x.iter().sum();
    if !s.is_finite() || s <= 0.0 {
        return Err(KirkError::EigenFailure);
    }
    for v in x.iter_mut() {
        *v /= s;
    }
    Ok(x)
}

/// Density-matrix reconstruction:
///
/// `rho = V * diag(p) * V^†`
///
/// Returns `(rho (row-major flat complex), p)`.
pub fn density_matrix(
    eigh: &Eigh,
    temperature: f32,
) -> Result<(Vec<Complex32>, Vec<f32>), KirkError> {
    let n = eigh.dim;
    let p = boltzmann_weights(&eigh.lam, temperature)?;

    // M[r, k] = V[r, k] * p[k]
    let mut m_data: Vec<Complex32> = vec![Complex32::new(0.0, 0.0); n * n];
    for k in 0..n {
        let pk = p[k];
        for r in 0..n {
            m_data[r + k * n] = eigh.v_at(r, k) * pk;
        }
    }

    // rho[i, j] = sum_k M[i, k] * conj(V[j, k]), output row-major.
    let mut rho = vec![Complex32::new(0.0, 0.0); n * n];
    for i in 0..n {
        for j in 0..n {
            let mut acc = Complex32::new(0.0, 0.0);
            for k in 0..n {
                acc += m_data[i + k * n] * eigh.v_at(j, k).conj();
            }
            rho[i * n + j] = acc;
        }
    }
    Ok((rho, p))
}
