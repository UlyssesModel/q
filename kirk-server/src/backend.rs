//! Server-side backend wrapper. Holds an `Arc<dyn ModelBackend>` and the
//! transport-wide configuration that does not vary per (model, env) — the
//! matrix-dim cap and a shutdown flag.

use std::sync::Arc;

use kirk_stub_realistic::variants::{ActiveInferenceOut, Features};
use kirk_stub_realistic::{KirkOutput, KirkSampleOutput};

use crate::backends::select_backend;
use crate::config::Config;
use crate::error::ServerError;
use crate::model::ModelBackend;

/// Re-exported `spawn_blocking` threshold (preserved for callers that import
/// it from this module).
pub use crate::backends::tiberius::SPAWN_BLOCKING_THRESHOLD;

/// Transport-facing wrapper. All three transports clone an `Arc<KirkBackend>`.
/// Internally the wrapper delegates compute to a `dyn ModelBackend`; the
/// matrix-dim cap and the shutdown flag live on the chosen backend.
pub struct KirkBackend {
    inner: Arc<dyn ModelBackend>,
    pub max_matrix_dim: u32,
}

impl KirkBackend {
    /// Default constructor used by `start_server*` shims. Builds a TiberiusBackend
    /// — preserves the pre-multimodel test fixture surface.
    pub fn new(
        temperature: f32,
        window_size: usize,
        max_matrix_dim: u32,
    ) -> anyhow::Result<Arc<Self>> {
        let inner =
            crate::backends::TiberiusBackend::new(temperature, window_size, max_matrix_dim)?;
        Ok(Arc::new(Self {
            inner: inner as Arc<dyn ModelBackend>,
            max_matrix_dim,
        }))
    }

    /// Build from a parsed `Config`. Inspects `--model` and `--env`.
    pub fn from_config(cfg: &Config) -> Result<Arc<Self>, ServerError> {
        let inner = select_backend(cfg)?;
        Ok(Arc::new(Self {
            inner,
            max_matrix_dim: cfg.max_matrix_dim,
        }))
    }

    /// Short, stable name for logs / metrics.
    pub fn name(&self) -> &'static str {
        self.inner.name()
    }

    pub fn signal_shutdown(&self) {
        self.inner.signal_shutdown();
    }

    pub fn is_shutting_down(&self) -> bool {
        self.inner.is_shutting_down()
    }

    /// Validate the matrix dimension N supplied by a request. Min dim is 2.
    pub fn check_dim(&self, n: u32) -> Result<(), ServerError> {
        self.inner.check_dim(n)
    }

    pub async fn forward(
        self: Arc<Self>,
        matrix_re: Vec<f32>,
        matrix_im: Vec<f32>,
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, ServerError> {
        self.inner
            .clone()
            .forward(matrix_re, matrix_im, n, timestamp_us)
            .await
    }

    pub async fn inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        self.inner.clone().inference_entropy(re, im, n).await
    }

    pub async fn inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        self.inner.clone().inference_features(re, im, n).await
    }

    pub async fn active_inference(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<ActiveInferenceOut, ServerError> {
        self.inner.clone().active_inference(re, im, n).await
    }

    pub async fn active_inference_entropy(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<f32, ServerError> {
        self.inner.clone().active_inference_entropy(re, im, n).await
    }

    pub async fn active_inference_features(
        self: Arc<Self>,
        re: Vec<f32>,
        im: Vec<f32>,
        n: usize,
    ) -> Result<Features, ServerError> {
        self.inner
            .clone()
            .active_inference_features(re, im, n)
            .await
    }

    pub async fn forward_sample(
        self: Arc<Self>,
        n: usize,
        seed: u64,
    ) -> Result<KirkSampleOutput, ServerError> {
        self.inner.clone().forward_sample(n, seed).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Env, Model};

    fn base_config() -> Config {
        Config {
            grpc_port: 0,
            rest_port: 0,
            tcp_port: 0,
            bind: "127.0.0.1".to_string(),
            workers: 0,
            temperature: 1.0,
            window_size: 256,
            max_matrix_dim: 64,
            max_connections: 8,
            max_in_flight_per_conn: 8,
            tcp_write_timeout_ms: 1000,
            log_level: "info".to_string(),
            healthcheck: false,
            model: Model::Tiberius,
            env: Env::Local,
        }
    }

    #[test]
    fn factory_tiberius_local() {
        let cfg = base_config();
        let b = KirkBackend::from_config(&cfg).expect("tiberius/local must build");
        assert_eq!(b.name(), "tiberius");
        assert!(b.check_dim(8).is_ok());
        assert!(b.check_dim(1).is_err());
        assert!(b.check_dim(65).is_err());
    }

    #[test]
    fn factory_kirk_local() {
        let mut cfg = base_config();
        cfg.model = Model::Kirk;
        let b = KirkBackend::from_config(&cfg).expect("kirk/local must build");
        assert_eq!(b.name(), "kirk");
    }

    #[test]
    fn factory_tiberius_prod_allowed() {
        let mut cfg = base_config();
        cfg.env = Env::Prod;
        let b = KirkBackend::from_config(&cfg).expect("tiberius/prod must build");
        assert_eq!(b.name(), "tiberius");
    }

    #[test]
    #[cfg(not(feature = "secret-kirk-edge"))]
    fn factory_kirk_prod_without_feature_rejected() {
        let mut cfg = base_config();
        cfg.env = Env::Prod;
        cfg.model = Model::Kirk;
        let result = KirkBackend::from_config(&cfg);
        let err = match result {
            Ok(_) => panic!("kirk/prod must reject without secret-kirk-edge feature"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("--env prod"),
            "error must mention --env prod, got: {msg}"
        );
        assert!(
            !msg.contains("secret-kirk-edge-v2.git"),
            "error must not leak the secret git URL, got: {msg}"
        );
        assert!(
            !msg.contains("ibis-allosaurus"),
            "error must not leak Tailnet host, got: {msg}"
        );
    }
}
