//! KirkProdBackend — production variant gated behind the `secret-kirk-edge`
//! cargo feature.
//!
//! This module is `#[cfg(feature = "secret-kirk-edge")]` so the dependency, its
//! source, and all references to it are absent from a default build. Default
//! cargo metadata, `Cargo.lock`, and the compiled binary therefore contain no
//! mention of the optional crate.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use kirk_stub_realistic::variants::{ActiveInferenceOut, Features};
use kirk_stub_realistic::{KirkOutput, KirkSampleOutput};
use parking_lot::Mutex;

use super::tiberius::check_dim_impl;
use crate::error::ServerError;
use crate::model::ModelBackend;

pub struct KirkProdBackend {
    handle: Mutex<secret_kirk_edge::Kirk>,
    max_matrix_dim: u32,
    shutdown: AtomicBool,
}

impl KirkProdBackend {
    pub fn new(visible_nodes: usize, max_matrix_dim: u32) -> anyhow::Result<Arc<Self>> {
        let handle = secret_kirk_edge::Kirk::builder()
            .visible_nodes(visible_nodes)
            .build();
        Ok(Arc::new(Self {
            handle: Mutex::new(handle),
            max_matrix_dim,
            shutdown: AtomicBool::new(false),
        }))
    }

    fn map_compute_err<E: std::fmt::Display>(e: E) -> ServerError {
        // Never propagate the inner error type or its messages verbatim — keep
        // operator-facing diagnostics opaque.
        ServerError::Internal(format!("backend compute error: {e}"))
    }
}

#[async_trait]
impl ModelBackend for KirkProdBackend {
    fn name(&self) -> &'static str {
        // Stable, opaque label — does not advertise the inner crate.
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
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            if me.is_shutting_down() {
                return Err(ServerError::Shutdown);
            }
            let mut guard = me.handle.lock();
            guard
                .forward(&matrix_re, &matrix_im, n, timestamp_us)
                .map_err(Self::map_compute_err)
        })
        .await
        .map_err(|e| ServerError::Internal(format!("join: {e}")))?
    }

    async fn inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            if me.is_shutting_down() {
                return Err(ServerError::Shutdown);
            }
            let mut guard = me.handle.lock();
            guard
                .inference_entropy(&re, &im, n)
                .map_err(Self::map_compute_err)
        })
        .await
        .map_err(|e| ServerError::Internal(format!("join: {e}")))?
    }

    async fn inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            if me.is_shutting_down() {
                return Err(ServerError::Shutdown);
            }
            let mut guard = me.handle.lock();
            guard
                .inference_features(&re, &im, n)
                .map_err(Self::map_compute_err)
        })
        .await
        .map_err(|e| ServerError::Internal(format!("join: {e}")))?
    }

    async fn active_inference(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<ActiveInferenceOut, ServerError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            if me.is_shutting_down() {
                return Err(ServerError::Shutdown);
            }
            let mut guard = me.handle.lock();
            guard
                .active_inference(&re, &im, n)
                .map_err(Self::map_compute_err)
        })
        .await
        .map_err(|e| ServerError::Internal(format!("join: {e}")))?
    }

    async fn active_inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            if me.is_shutting_down() {
                return Err(ServerError::Shutdown);
            }
            let mut guard = me.handle.lock();
            guard
                .active_inference_entropy(&re, &im, n)
                .map_err(Self::map_compute_err)
        })
        .await
        .map_err(|e| ServerError::Internal(format!("join: {e}")))?
    }

    async fn active_inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            if me.is_shutting_down() {
                return Err(ServerError::Shutdown);
            }
            let mut guard = me.handle.lock();
            guard
                .active_inference_features(&re, &im, n)
                .map_err(Self::map_compute_err)
        })
        .await
        .map_err(|e| ServerError::Internal(format!("join: {e}")))?
    }

    async fn forward_sample(
        self: Arc<Self>,
        n: usize,
        seed: u64,
    ) -> Result<KirkSampleOutput, ServerError> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || {
            if me.is_shutting_down() {
                return Err(ServerError::Shutdown);
            }
            let mut guard = me.handle.lock();
            guard.forward_sample(n, seed).map_err(Self::map_compute_err)
        })
        .await
        .map_err(|e| ServerError::Internal(format!("join: {e}")))?
    }
}
