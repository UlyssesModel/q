//! KirkLocalBackend — the local Kirk stub used for `(model = kirk, env = local)`.
//!
//! Wraps the `kirk_stub_kirk::Kirk` handle with a `parking_lot::Mutex` and uses
//! the same `spawn_blocking` threshold as Tiberius. Inputs arrive as f32; we
//! lift to Complex64 at the boundary, run the stub, and project back to f32 for
//! the trait return.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use kirk_stub_kirk::Kirk;
use kirk_stub_realistic::variants::{ActiveInferenceOut, Features};
use kirk_stub_realistic::{KirkOutput, KirkSampleOutput};
use ndarray::{Array1, Array2};
use num_complex::{Complex32, Complex64};
use parking_lot::Mutex;
use rand::{Rng, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;

use super::tiberius::{check_dim_impl, SPAWN_BLOCKING_THRESHOLD};
use crate::error::ServerError;
use crate::model::ModelBackend;

pub struct KirkLocalBackend {
    pub kirk: Mutex<Kirk>,
    pub max_matrix_dim: u32,
    shutdown: AtomicBool,
}

impl KirkLocalBackend {
    pub fn new(visible_nodes: usize, max_matrix_dim: u32) -> anyhow::Result<Arc<Self>> {
        let kirk = Kirk::builder().visible_nodes(visible_nodes).build();
        Ok(Arc::new(Self {
            kirk: Mutex::new(kirk),
            max_matrix_dim,
            shutdown: AtomicBool::new(false),
        }))
    }

    fn assemble_sample(re: &[f32], im: &[f32], n: usize) -> Result<Array2<Complex64>, ServerError> {
        if re.len() != n * n || im.len() != n * n {
            return Err(ServerError::BadRequest(format!(
                "matrix length {}/{} != expected {}",
                re.len(),
                im.len(),
                n * n
            )));
        }
        let mut a = Array2::<Complex64>::zeros((n, n));
        for i in 0..n {
            for j in 0..n {
                let idx = i * n + j;
                a[[i, j]] = Complex64::new(re[idx] as f64, im[idx] as f64);
            }
        }
        Ok(a)
    }

    fn array2_to_split(arr: &Array2<Complex64>) -> (Vec<f32>, Vec<f32>) {
        let mut re = Vec::with_capacity(arr.len());
        let mut im = Vec::with_capacity(arr.len());
        for c in arr.iter() {
            re.push(c.re as f32);
            im.push(c.im as f32);
        }
        (re, im)
    }

    fn array_to_complex32(arr: &Array2<Complex64>) -> Vec<Complex32> {
        arr.iter()
            .map(|c| Complex32::new(c.re as f32, c.im as f32))
            .collect()
    }

    fn vec_to_complex32(v: &Array1<Complex64>) -> Vec<Complex32> {
        v.iter()
            .map(|c| Complex32::new(c.re as f32, c.im as f32))
            .collect()
    }

    fn run_forward(
        &self,
        re: &[f32],
        im: &[f32],
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        let t0 = std::time::Instant::now();
        let sample = Self::assemble_sample(re, im, n)?;
        let mut guard = self.kirk.lock();
        let (arr, _vec, _scalar, ent) = guard.active_inference(sample.view());
        drop(guard);
        let (matrix_re, matrix_im) = Self::array2_to_split(&arr);
        let entropy = ent as f32;
        Ok(KirkOutput {
            entropy_re: entropy,
            entropy_im: 0.0,
            entropy,
            entropy_zscore: 0.0,
            regime: 1,
            confidence: 0.0,
            processing_time_us: t0.elapsed().as_micros() as u64,
            timestamp_us,
            matrix_re,
            matrix_im,
        })
    }

    fn run_entropy(
        &self,
        re: &[f32],
        im: &[f32],
        n: usize,
        active: bool,
    ) -> Result<f32, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        let sample = Self::assemble_sample(re, im, n)?;
        let mut guard = self.kirk.lock();
        let e = if active {
            guard.active_inference_entropy(sample.view())
        } else {
            guard.inference_entropy(sample.view())
        };
        Ok(e as f32)
    }

    fn run_features(
        &self,
        re: &[f32],
        im: &[f32],
        n: usize,
        active: bool,
    ) -> Result<Features, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        let sample = Self::assemble_sample(re, im, n)?;
        let mut guard = self.kirk.lock();
        let (arr, vec, scalar) = if active {
            guard.active_inference_features(sample.view())
        } else {
            guard.inference_features(sample.view())
        };
        Ok(Features {
            feature_arr: Self::array_to_complex32(&arr),
            feature_vec: Self::vec_to_complex32(&vec),
            feature_scalar: Complex32::new(scalar.re as f32, scalar.im as f32),
            n,
        })
    }

    fn run_active_inference(
        &self,
        re: &[f32],
        im: &[f32],
        n: usize,
    ) -> Result<ActiveInferenceOut, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        let sample = Self::assemble_sample(re, im, n)?;
        let mut guard = self.kirk.lock();
        let (arr, vec, scalar, ent) = guard.active_inference(sample.view());
        Ok(ActiveInferenceOut {
            features: Features {
                feature_arr: Self::array_to_complex32(&arr),
                feature_vec: Self::vec_to_complex32(&vec),
                feature_scalar: Complex32::new(scalar.re as f32, scalar.im as f32),
                n,
            },
            total_relative_entropy: ent as f32,
        })
    }

    fn run_forward_sample(&self, n: usize, seed: u64) -> Result<KirkSampleOutput, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        // Stub: build a deterministic seeded sample, then run inference_features.
        let mut rng = Xoshiro256StarStar::seed_from_u64(seed);
        let mut sample = Array2::<Complex64>::zeros((n, n));
        for el in sample.iter_mut() {
            let r: f64 = rng.gen_range(-1.0..=1.0);
            let im: f64 = rng.gen_range(-1.0..=1.0);
            *el = Complex64::new(r, im);
        }
        let mut guard = self.kirk.lock();
        let (arr, vec, scalar) = guard.inference_features(sample.view());
        let ent = guard.inference_entropy(sample.view());
        Ok(KirkSampleOutput {
            feature_array: Self::array_to_complex32(&arr),
            feature_vector: Self::vec_to_complex32(&vec),
            feature_scalar: Complex32::new(scalar.re as f32, scalar.im as f32),
            relative_entropy: ent as f32,
            matrix_dim: n,
        })
    }
}

#[async_trait]
impl ModelBackend for KirkLocalBackend {
    fn name(&self) -> &'static str {
        "kirk"
    }

    fn check_dim(&self, n: u32) -> Result<(), ServerError> {
        check_dim_impl(self.max_matrix_dim, n)
    }

    fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    async fn forward(
        self: Arc<Self>,
        matrix_re: Vec<f32>,
        matrix_im: Vec<f32>,
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || {
                me.run_forward(&matrix_re, &matrix_im, n, timestamp_us)
            })
            .await
            .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.run_forward(&matrix_re, &matrix_im, n, timestamp_us)
        }
    }

    async fn inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || me.run_entropy(&re, &im, n, false))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.run_entropy(&re, &im, n, false)
        }
    }

    async fn inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || me.run_features(&re, &im, n, false))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.run_features(&re, &im, n, false)
        }
    }

    async fn active_inference(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<ActiveInferenceOut, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || me.run_active_inference(&re, &im, n))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.run_active_inference(&re, &im, n)
        }
    }

    async fn active_inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || me.run_entropy(&re, &im, n, true))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.run_entropy(&re, &im, n, true)
        }
    }

    async fn active_inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || me.run_features(&re, &im, n, true))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.run_features(&re, &im, n, true)
        }
    }

    async fn forward_sample(
        self: Arc<Self>,
        n: usize,
        seed: u64,
    ) -> Result<KirkSampleOutput, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || me.run_forward_sample(n, seed))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.run_forward_sample(n, seed)
        }
    }
}
