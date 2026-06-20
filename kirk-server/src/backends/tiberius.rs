//! TiberiusBackend — the existing Boltzmann/Shannon Hermitian-eigh kernel.
//!
//! Used for `(model = tiberius, env = local | prod)`. Identical numerical
//! behavior to the previous `KirkBackend` — only the trait wrapper is new.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use kirk_stub_realistic::variants::{
    active_inference, active_inference_entropy, active_inference_features, inference_entropy,
    inference_features, ActiveInferenceOut, Features,
};
use kirk_stub_realistic::{forward_sample, KirkOutput, KirkRealistic, KirkSampleOutput};
use parking_lot::Mutex;
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;

use crate::error::ServerError;
use crate::model::ModelBackend;

/// `spawn_blocking` threshold per ADR-007.
pub const SPAWN_BLOCKING_THRESHOLD: usize = 128;

pub struct TiberiusBackend {
    pub kirk: Mutex<KirkRealistic>,
    pub max_matrix_dim: u32,
    shutdown: AtomicBool,
}

impl TiberiusBackend {
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

    /// Synchronous `forward`. Holds the kirk mutex for the duration of the call.
    fn forward_sync(
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
}

#[async_trait]
impl ModelBackend for TiberiusBackend {
    fn name(&self) -> &'static str {
        "tiberius"
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
                me.forward_sync(&matrix_re, &matrix_im, n, timestamp_us)
            })
            .await
            .map_err(|e| ServerError::Internal(format!("join: {e}")))?
        } else {
            self.forward_sync(&matrix_re, &matrix_im, n, timestamp_us)
        }
    }

    async fn inference_entropy(
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

    async fn inference_features(
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

    async fn active_inference(
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

    async fn active_inference_entropy(
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

    async fn active_inference_features(
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

    async fn forward_sample(
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

pub(crate) fn check_dim_impl(max_matrix_dim: u32, n: u32) -> Result<(), ServerError> {
    if n > max_matrix_dim {
        Err(ServerError::MatrixDimExceeded {
            actual: n,
            limit: max_matrix_dim,
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
