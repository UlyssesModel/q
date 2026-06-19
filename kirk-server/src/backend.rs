//! Shared backend: one `Arc<KirkBackend>` driven by all three transports.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use kirk_stub_realistic::variants::{
    active_inference, active_inference_entropy, active_inference_features, inference_entropy,
    inference_features, ActiveInferenceOut, Features,
};
use kirk_stub_realistic::{forward_sample, KirkOutput, KirkRealistic, KirkSampleOutput};
use parking_lot::Mutex;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;

use crate::error::ServerError;

/// `spawn_blocking` threshold per ADR-007.
pub const SPAWN_BLOCKING_THRESHOLD: usize = 128;

pub struct KirkBackend {
    pub kirk: Mutex<KirkRealistic>,
    pub max_matrix_dim: u32,
    shutdown: AtomicBool,
}

impl KirkBackend {
    pub fn new(
        temperature: f32,
        window_size: usize,
        max_matrix_dim: u32,
    ) -> anyhow::Result<Arc<Self>> {
        let kirk = KirkRealistic::new(temperature, window_size)?;
        Ok(Arc::new(Self {
            kirk: Mutex::new(kirk),
            max_matrix_dim,
            shutdown: AtomicBool::new(false),
        }))
    }

    pub fn signal_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Validate the matrix dimension N supplied by a request.
    ///
    /// - Rejects `n == 0` (degenerate) and `n == 1` (the entropy/confidence
    ///   formula and the rolling-window z-score are mathematically degenerate
    ///   at N=1 — see SEC-008). Minimum allowed dim is therefore 2.
    /// - Rejects `n > max_matrix_dim` (the configured operator-side ceiling,
    ///   itself clamped to `[1, 4096]` by clap).
    /// - Also rejects values above the per-process arithmetic ceiling — see
    ///   `MAX_ALLOWED_MATRIX_DIM`.
    pub fn check_dim(&self, n: u32) -> Result<(), ServerError> {
        if n > self.max_matrix_dim {
            Err(ServerError::MatrixDimExceeded {
                actual: n,
                limit: self.max_matrix_dim,
            })
        } else if n < 2 {
            Err(ServerError::BadRequest(
                "matrix dim must be >= 2 (N=0 is empty, N=1 leaves entropy / confidence undefined)"
                    .into(),
            ))
        } else {
            Ok(())
        }
    }

    /// Synchronous `forward`. Holds the kirk mutex for the duration of the call.
    pub fn forward_sync(
        &self,
        matrix_re: &[f32],
        matrix_im: &[f32],
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        let mut guard = self.kirk.lock();
        guard
            .forward(matrix_re, matrix_im, n, timestamp_us)
            .map_err(ServerError::from)
    }

    /// Async `forward` that hops to `spawn_blocking` when N is large.
    pub async fn forward(
        self: Arc<Self>,
        matrix_re: Vec<f32>,
        matrix_im: Vec<f32>,
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, ServerError> {
        if n >= SPAWN_BLOCKING_THRESHOLD {
            let me = self.clone();
            tokio::task::spawn_blocking(move || {
                me.forward_sync(&matrix_re, &matrix_im, n, timestamp_us)
            })
            .await
            .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.forward_sync(&matrix_re, &matrix_im, n, timestamp_us)
        }
    }

    pub async fn inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        if n >= SPAWN_BLOCKING_THRESHOLD {
            tokio::task::spawn_blocking(move || inference_entropy(&re, &im, n))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
                .map_err(ServerError::from)
        } else {
            inference_entropy(&re, &im, n).map_err(ServerError::from)
        }
    }

    pub async fn inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        if n >= SPAWN_BLOCKING_THRESHOLD {
            tokio::task::spawn_blocking(move || inference_features(&re, &im, n))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
                .map_err(ServerError::from)
        } else {
            inference_features(&re, &im, n).map_err(ServerError::from)
        }
    }

    pub async fn active_inference(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<ActiveInferenceOut, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        if n >= SPAWN_BLOCKING_THRESHOLD {
            tokio::task::spawn_blocking(move || active_inference(&re, &im, n))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
                .map_err(ServerError::from)
        } else {
            active_inference(&re, &im, n).map_err(ServerError::from)
        }
    }

    pub async fn active_inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        if n >= SPAWN_BLOCKING_THRESHOLD {
            tokio::task::spawn_blocking(move || active_inference_entropy(&re, &im, n))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
                .map_err(ServerError::from)
        } else {
            active_inference_entropy(&re, &im, n).map_err(ServerError::from)
        }
    }

    pub async fn active_inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        if n >= SPAWN_BLOCKING_THRESHOLD {
            tokio::task::spawn_blocking(move || active_inference_features(&re, &im, n))
                .await
                .map_err(|e| ServerError::Internal(format!("join: {e}")))?
                .map_err(ServerError::from)
        } else {
            active_inference_features(&re, &im, n).map_err(ServerError::from)
        }
    }

    pub async fn forward_sample(
        self: Arc<Self>,
        n: usize,
        seed: u64,
    ) -> Result<KirkSampleOutput, ServerError> {
        if self.is_shutting_down() {
            return Err(ServerError::Shutdown);
        }
        if n >= SPAWN_BLOCKING_THRESHOLD {
            tokio::task::spawn_blocking(move || {
                let mut rng = Xoshiro256StarStar::seed_from_u64(seed);
                forward_sample(n, &mut rng)
            })
            .await
            .map_err(|e| ServerError::Internal(format!("join: {e}")))
        } else {
            let mut rng = Xoshiro256StarStar::seed_from_u64(seed);
            Ok(forward_sample(n, &mut rng))
        }
    }
}
