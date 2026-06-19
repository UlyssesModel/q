//! Phase-A sanity tests: shape, symmetry, trace, confidence bounds, variants
//! non-NaN at N in {2,4,8,16,32}. Numerical parity with Python lives in
//! `parity.rs` (loads fixtures).

use kirk_stub_realistic::density_matrix::density_matrix;
use kirk_stub_realistic::eigensolver::{diagonalize, Hermitian};
use kirk_stub_realistic::entropy::shannon_entropy;
use kirk_stub_realistic::rng::seeded;
use kirk_stub_realistic::{
    active_inference, active_inference_entropy, active_inference_features, forward_sample,
    inference_entropy, inference_features, KirkRealistic,
};
use rand::Rng;

fn random_matrix(n: usize, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let mut rng = seeded(seed);
    let mut re = vec![0.0f32; n * n];
    let mut im = vec![0.0f32; n * n];
    for i in 0..n * n {
        // Uniform in [-1, 1) — not normal, but adequate for shape/NaN tests.
        re[i] = (rng.gen::<f32>() - 0.5) * 2.0;
        im[i] = (rng.gen::<f32>() - 0.5) * 2.0;
    }
    (re, im)
}

#[test]
fn hermitianize_is_hermitian() {
    let n = 8;
    let (re, im) = random_matrix(n, 42);
    let h = Hermitian::from_parts(&re, &im, n).unwrap();
    // H should equal its conjugate transpose: H[i,j] = conj(H[j,i]).
    for i in 0..n {
        for j in 0..n {
            let a = h.at(i, j);
            let b = h.at(j, i).conj();
            assert!((a.re - b.re).abs() < 1e-5, "Re mismatch at ({i},{j})");
            assert!((a.im - b.im).abs() < 1e-5, "Im mismatch at ({i},{j})");
        }
    }
    // Diagonal must be real.
    for i in 0..n {
        assert!(h.at(i, i).im.abs() < 1e-5);
    }
}

#[test]
fn eigenvalues_sum_equals_trace() {
    let n = 8;
    let (re, im) = random_matrix(n, 7);
    let h = Hermitian::from_parts(&re, &im, n).unwrap();
    let mut trace = 0.0f32;
    for i in 0..n {
        trace += h.re[i * n + i];
    }
    let eigh = diagonalize(&h).unwrap();
    let sum: f32 = eigh.lam.iter().sum();
    assert!((sum - trace).abs() < 1e-3, "sum={sum} trace={trace}");
}

#[test]
fn rho_trace_is_one() {
    for &n in &[2usize, 4, 8, 16, 32] {
        let (re, im) = random_matrix(n, 11);
        let h = Hermitian::from_parts(&re, &im, n).unwrap();
        let eigh = diagonalize(&h).unwrap();
        let (rho, p) = density_matrix(&eigh, 1.0).unwrap();
        let mut tr = num_complex::Complex32::new(0.0, 0.0);
        for i in 0..n {
            tr += rho[i * n + i];
        }
        assert!((tr.re - 1.0).abs() < 1e-3, "n={n} Re(tr)={}", tr.re);
        assert!(tr.im.abs() < 1e-3, "n={n} Im(tr)={}", tr.im);
        let s: f32 = p.iter().sum();
        assert!((s - 1.0).abs() < 1e-4, "p sum drift: {s}");
    }
}

#[test]
fn confidence_in_unit_interval() {
    let mut kirk = KirkRealistic::new(1.0, 256).unwrap();
    for &n in &[2usize, 4, 8, 16, 32] {
        let (re, im) = random_matrix(n, 99);
        let out = kirk.forward(&re, &im, n, 0).unwrap();
        assert!(
            out.confidence >= 0.0 && out.confidence <= 1.0,
            "conf={} for n={n}",
            out.confidence
        );
        assert!(out.entropy.is_finite(), "entropy NaN at n={n}");
    }
}

#[test]
fn shannon_entropy_nonnegative_for_normalized() {
    let p = vec![0.25f32; 4];
    let h = shannon_entropy(&p);
    assert!(h > 0.0);
    let p_delta = vec![1.0f32, 0.0, 0.0, 0.0];
    let h_delta = shannon_entropy(&p_delta);
    assert!(
        h_delta.abs() < 1e-3,
        "delta entropy should be ~0; got {h_delta}"
    );
}

#[test]
fn variants_non_nan_across_sizes() {
    for &n in &[2usize, 4, 8, 16, 32] {
        let (re, im) = random_matrix(n, 17);
        let e = inference_entropy(&re, &im, n).unwrap();
        assert!(e.is_finite(), "inference_entropy NaN at n={n}");
        let f = inference_features(&re, &im, n).unwrap();
        assert!(f.feature_scalar.re.is_finite() && f.feature_scalar.im.is_finite());
        assert_eq!(f.feature_arr.len(), n * n);
        assert_eq!(f.feature_vec.len(), 2 * n);
        let ai = active_inference(&re, &im, n).unwrap();
        assert!(ai.total_relative_entropy.is_finite());
        assert_eq!(ai.features.feature_arr.len(), n * n);
        let aie = active_inference_entropy(&re, &im, n).unwrap();
        assert!(aie.is_finite());
        let aif = active_inference_features(&re, &im, n).unwrap();
        assert_eq!(aif.feature_arr.len(), n * n);
    }
}

#[test]
fn forward_sample_shapes() {
    let mut rng = seeded(42);
    for &n in &[2usize, 4, 8, 16, 32] {
        let out = forward_sample(n, &mut rng);
        assert_eq!(out.feature_array.len(), n * n);
        assert_eq!(out.feature_vector.len(), 2 * n);
        assert_eq!(out.matrix_dim, n);
        assert!(out.relative_entropy >= 0.0);
    }
}
