//! Wire-facing model backend trait.
//!
//! Concrete implementations live under `backends/`. Each `(model, env)` combination
//! is its own concrete impl; the trait is the narrowest possible surface that
//! every transport already calls.
//!
//! All methods receive owned `Vec<f32>` real/imag matrices in row-major order to
//! be compatible with the existing `Arc<Self>` + `tokio::task::spawn_blocking`
//! pattern used by the transports.

use std::sync::Arc;

use async_trait::async_trait;
use kirk_stub_realistic::variants::{ActiveInferenceOut, Features};
use kirk_stub_realistic::{KirkOutput, KirkSampleOutput};

use crate::error::ServerError;

/// Backend dispatcher used by all transports. The transports clone `Arc<Self>`
/// across `spawn_blocking` boundaries; each method is therefore taken on
/// `Arc<Self>` rather than `&self`.
#[async_trait]
pub trait ModelBackend: Send + Sync {
    /// Short, stable label for logs / metrics. MUST NOT advertise crate names,
    /// hosts, or algorithm internals beyond the model family.
    fn name(&self) -> &'static str;

    /// Reject `n == 0`, `n == 1`, and `n > max_matrix_dim`.
    fn check_dim(&self, n: u32) -> Result<(), ServerError>;

    /// Signal that the listener should stop accepting new work.
    fn signal_shutdown(&self);

    /// Are we draining?
    fn is_shutting_down(&self) -> bool;

    async fn forward(
        self: Arc<Self>,
        matrix_re: Vec<f32>,
        matrix_im: Vec<f32>,
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, ServerError>;

    async fn inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError>;

    async fn inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError>;

    async fn active_inference(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<ActiveInferenceOut, ServerError>;

    async fn active_inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError>;

    async fn active_inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError>;

    async fn forward_sample(
        self: Arc<Self>,
        n: usize,
        seed: u64,
    ) -> Result<KirkSampleOutput, ServerError>;
}
