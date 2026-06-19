//! Parity tests against pre-computed reference fixtures. The test agent will
//! populate `tests/fixtures/seed42_N{8,16,32}.json` from the Python reference;
//! `handcalc_N2.json` is a hand-derived smoke check that ships with this commit.

use kirk_stub_realistic::KirkRealistic;
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct Expected {
    entropy: f32,
    entropy_re: f32,
    entropy_im: f32,
    confidence: f32,
    rho_re: Vec<f32>,
    rho_im: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    n: usize,
    temperature: f32,
    matrix_re: Vec<f32>,
    matrix_im: Vec<f32>,
    timestamp_us: i64,
    expected: Expected,
}

fn rel_close(actual: f32, expected: f32, tol: f32) -> bool {
    let denom = expected.abs().max(1.0);
    (actual - expected).abs() / denom <= tol
}

fn frobenius_relative(a_re: &[f32], a_im: &[f32], b_re: &[f32], b_im: &[f32]) -> f32 {
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for ((ar, ai), (br, bi)) in a_re.iter().zip(a_im).zip(b_re.iter().zip(b_im)) {
        let dr = (*ar - *br) as f64;
        let di = (*ai - *bi) as f64;
        num += dr * dr + di * di;
        den += (*br as f64).powi(2) + (*bi as f64).powi(2);
    }
    if den == 0.0 {
        num.sqrt() as f32
    } else {
        (num.sqrt() / den.sqrt()) as f32
    }
}

fn load_fixture<P: AsRef<Path>>(path: P) -> Fixture {
    let text = fs::read_to_string(path.as_ref()).unwrap_or_else(|e| {
        panic!("missing fixture {}: {e}", path.as_ref().display());
    });
    serde_json::from_str(&text).expect("fixture parse")
}

fn assert_fixture(path: &str) {
    let f = load_fixture(path);
    let mut kirk = KirkRealistic::new(f.temperature, 256).unwrap();
    let out = kirk
        .forward(&f.matrix_re, &f.matrix_im, f.n, f.timestamp_us)
        .expect("forward succeeds");

    assert!(
        rel_close(out.entropy, f.expected.entropy, 1e-4),
        "entropy mismatch: got {} expected {}",
        out.entropy,
        f.expected.entropy
    );
    assert!(
        rel_close(out.entropy_re, f.expected.entropy_re, 1e-4),
        "entropy_re mismatch: got {} expected {}",
        out.entropy_re,
        f.expected.entropy_re
    );
    assert!(
        rel_close(out.entropy_im, f.expected.entropy_im, 1e-4),
        "entropy_im mismatch: got {} expected {}",
        out.entropy_im,
        f.expected.entropy_im
    );
    assert!(
        (out.confidence - f.expected.confidence).abs() <= 1e-4,
        "confidence mismatch: got {} expected {}",
        out.confidence,
        f.expected.confidence
    );
    let frel = frobenius_relative(
        &out.matrix_re,
        &out.matrix_im,
        &f.expected.rho_re,
        &f.expected.rho_im,
    );
    assert!(frel <= 1e-3, "rho Frobenius rel error {frel} > 1e-3");
}

#[test]
fn parity_handcalc_n2() {
    assert_fixture("tests/fixtures/handcalc_N2.json");
}

// Placeholder hooks for Python-generated fixtures (test agent populates).
#[test]
fn parity_seed42_n8() {
    assert_fixture("tests/fixtures/seed42_N8.json");
}

#[test]
fn parity_seed42_n16() {
    assert_fixture("tests/fixtures/seed42_N16.json");
}

#[test]
fn parity_seed42_n32() {
    assert_fixture("tests/fixtures/seed42_N32.json");
}
