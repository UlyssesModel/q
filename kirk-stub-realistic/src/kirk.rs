//! Stateful KirkRealistic — port of Python `kirk.KirkRealistic`.

use std::collections::VecDeque;
use std::time::Instant;

use crate::density_matrix::density_matrix;
use crate::eigensolver::{diagonalize, Hermitian};
use crate::entropy::{entropy_from_hamiltonian, shannon_entropy};
use crate::output::{KirkError, KirkOutput};
use crate::reconstruct::rho_to_real_imag;

const MIN_WINDOW_FOR_ZSCORE: usize = 8;

pub struct KirkRealistic {
    pub temperature: f32,
    pub window_size: usize,
    entropy_window: VecDeque<f32>,
}

impl KirkRealistic {
    pub fn new(temperature: f32, window_size: usize) -> Result<Self, KirkError> {
        if temperature <= 0.0 {
            return Err(KirkError::BadTemperature(temperature));
        }
        if window_size < 1 {
            return Err(KirkError::BadWindow(window_size));
        }
        Ok(Self {
            temperature,
            window_size,
            entropy_window: VecDeque::with_capacity(window_size),
        })
    }

    /// Mirrors Python `KirkRealistic.forward`.
    ///
    /// N=1 caveat: the documented confidence formula `1 - H/ln(N)` is
    /// undefined at N=1, so this kernel falls back to `ln_n = 1` (yielding a
    /// constant `confidence = 1.0` since `H = 0` for any 1×1 spectrum). The
    /// `kirk-server` boundary explicitly rejects N=1 in `KirkBackend::check_dim`
    /// to avoid exposing this sentinel over the wire — see SEC-008.
    pub fn forward(
        &mut self,
        matrix_re: &[f32],
        matrix_im: &[f32],
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, KirkError> {
        let t0 = Instant::now();

        let h = Hermitian::from_parts(matrix_re, matrix_im, n)?;
        let eigh = diagonalize(&h)?;
        let (rho, p) = density_matrix(&eigh, self.temperature)?;

        let entropy = shannon_entropy(&p);

        // entropy_re / entropy_im — re-Hermitianize the (re, 0) and (0, im)
        // projections and compute their Boltzmann entropies.
        let zeros = vec![0.0f32; n * n];
        let h_re = Hermitian::from_parts(matrix_re, &zeros, n)?;
        let entropy_re = entropy_from_hamiltonian(&h_re, self.temperature)?;
        let h_im = Hermitian::from_parts(&zeros, matrix_im, n)?;
        let entropy_im = entropy_from_hamiltonian(&h_im, self.temperature)?;

        // Rolling-window z-score using the population stddev (ddof=0).
        let (z, regime) = if self.entropy_window.len() < MIN_WINDOW_FOR_ZSCORE {
            (0.0f32, 1u32)
        } else {
            let len = self.entropy_window.len() as f64;
            let sum: f64 = self.entropy_window.iter().map(|&v| v as f64).sum();
            let mu = sum / len;
            let var: f64 = self
                .entropy_window
                .iter()
                .map(|&v| {
                    let d = v as f64 - mu;
                    d * d
                })
                .sum::<f64>()
                / len;
            let sigma = var.sqrt();
            let z = if sigma < 1e-12 {
                0.0f64
            } else {
                (entropy as f64 - mu) / sigma
            };
            let z32 = z as f32;
            let regime = if z32 < -1.0 {
                0u32
            } else if z32 > 1.0 {
                2u32
            } else {
                1u32
            };
            (z32, regime)
        };

        if self.entropy_window.len() == self.window_size {
            self.entropy_window.pop_front();
        }
        self.entropy_window.push_back(entropy);

        let ln_n = if n > 1 { (n as f32).ln() } else { 1.0 };
        let confidence = (1.0 - entropy / ln_n).clamp(0.0, 1.0);

        let (rho_re, rho_im) = rho_to_real_imag(&rho);
        let dt_ns = t0.elapsed().as_nanos() as u64;
        let processing_time_us = dt_ns / 1000;

        Ok(KirkOutput {
            entropy_re,
            entropy_im,
            entropy,
            entropy_zscore: z,
            regime,
            confidence,
            processing_time_us,
            timestamp_us,
            matrix_re: rho_re,
            matrix_im: rho_im,
        })
    }

    /// Returns a reference to the rolling-window for tests / introspection.
    pub fn window(&self) -> &VecDeque<f32> {
        &self.entropy_window
    }
}
