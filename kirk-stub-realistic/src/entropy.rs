//! Stage 6: Shannon-reducible von Neumann entropy.

use crate::density_matrix::boltzmann_weights;
use crate::eigensolver::{diagonalize, Hermitian};
use crate::output::KirkError;

const EPSILON: f32 = 1e-12;

/// Shannon entropy `H(p) = -sum_i p_i * ln(p_i + eps)`.
pub fn shannon_entropy(probabilities: &[f32]) -> f32 {
    let mut acc = 0.0f64;
    for &p in probabilities {
        let inner = (p as f64) + EPSILON as f64;
        acc += (p as f64) * inner.ln();
    }
    -(acc as f32)
}

/// Compute entropy of the Boltzmann distribution derived from a Hermitian
/// Hamiltonian. Mirrors Python `entropy_from_hamiltonian`.
pub fn entropy_from_hamiltonian(h: &Hermitian, temperature: f32) -> Result<f32, KirkError> {
    let eigh = diagonalize(h)?;
    let p = boltzmann_weights(&eigh.lam, temperature)?;
    Ok(shannon_entropy(&p))
}
