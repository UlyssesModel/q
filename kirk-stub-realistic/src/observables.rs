//! Stage 5: observable expectation values (kept for parity, not surfaced by the
//! main `KirkRealistic.forward` path).

use num_complex::Complex32;

pub fn expected_energy(eigenvalues: &[f32], probabilities: &[f32]) -> f32 {
    eigenvalues
        .iter()
        .zip(probabilities)
        .map(|(&l, &p)| l * p)
        .sum()
}

/// `O_z = diag(+1, -1, +1, ...)`; expectation = sum_i rho[i,i].re * O_z[i].
pub fn expected_magnetization(rho: &[Complex32], n: usize) -> f32 {
    let mut acc = 0.0f32;
    for i in 0..n {
        let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
        acc += rho[i * n + i].re * sign;
    }
    acc
}

/// `O_n = diag(0, 1, 2, ..., N-1)`.
pub fn expected_occupancy(rho: &[Complex32], n: usize) -> f32 {
    let mut acc = 0.0f32;
    for i in 0..n {
        acc += rho[i * n + i].re * (i as f32);
    }
    acc
}

pub fn all_observables(
    eigenvalues: &[f32],
    probabilities: &[f32],
    rho: &[Complex32],
    n: usize,
) -> [f32; 3] {
    [
        expected_energy(eigenvalues, probabilities),
        expected_magnetization(rho, n),
        expected_occupancy(rho, n),
    ]
}
