//! Shape-correct random sample output (mirrors Python `forward_sample`).
//!
//! Spec signature: `forward_sample(n, rng) -> KirkSampleOutput`.

use crate::output::KirkSampleOutput;
use num_complex::Complex32;
use rand::Rng;
use rand_distr::{Distribution, Exp, StandardNormal};

/// Draws a `(N,N)` complex feature array, `(2N,)` vector, scalar, and
/// non-negative `relative_entropy`. Pure shape contract; values are random.
pub fn forward_sample<R: Rng + ?Sized>(n: usize, rng: &mut R) -> KirkSampleOutput {
    let n = n.max(1);
    let feature_array: Vec<Complex32> = (0..n * n)
        .map(|_| {
            let re: f64 = StandardNormal.sample(rng);
            let im: f64 = StandardNormal.sample(rng);
            Complex32::new(re as f32, im as f32)
        })
        .collect();
    let feature_vector: Vec<Complex32> = (0..2 * n)
        .map(|_| {
            let re: f64 = StandardNormal.sample(rng);
            let im: f64 = StandardNormal.sample(rng);
            Complex32::new(re as f32, im as f32)
        })
        .collect();
    let s_re: f64 = StandardNormal.sample(rng);
    let s_im: f64 = StandardNormal.sample(rng);
    let exp = Exp::new(1.0).expect("lambda=1 is positive");
    let rel: f64 = exp.sample(rng);
    KirkSampleOutput {
        feature_array,
        feature_vector,
        feature_scalar: Complex32::new(s_re as f32, s_im as f32),
        relative_entropy: rel as f32,
        matrix_dim: n,
    }
}
