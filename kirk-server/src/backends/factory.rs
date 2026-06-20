//! Per-(model, env) backend factory. Called once at startup; no dynamic
//! re-selection at runtime.

use std::sync::Arc;

use crate::config::{Config, Env, Model};
use crate::error::ServerError;
use crate::model::ModelBackend;

use super::kirk_local::KirkLocalBackend;
use super::tiberius::TiberiusBackend;

#[cfg(feature = "secret-kirk-edge")]
use super::kirk_prod::KirkProdBackend;

/// Build the concrete backend implied by `(cfg.model, cfg.env)` plus the
/// compile-time `secret-kirk-edge` feature flag.
///
/// Behavior matrix:
///
/// | env   | model    | feature on | result                                  |
/// |-------|----------|------------|-----------------------------------------|
/// | local | tiberius | any        | TiberiusBackend                         |
/// | prod  | tiberius | any        | TiberiusBackend (no remote variant)     |
/// | local | kirk     | any        | KirkLocalBackend                        |
/// | prod  | kirk     | on         | KirkProdBackend                         |
/// | prod  | kirk     | off        | Err(ServerError::BadRequest(...))       |
pub fn select_backend(cfg: &Config) -> Result<Arc<dyn ModelBackend>, ServerError> {
    match (cfg.env, cfg.model) {
        (Env::Local, Model::Tiberius) | (Env::Prod, Model::Tiberius) => {
            let b = TiberiusBackend::new(cfg.temperature, cfg.window_size, cfg.max_matrix_dim)
                .map_err(|e| ServerError::Internal(format!("backend init failed: {e}")))?;
            Ok(b as Arc<dyn ModelBackend>)
        }
        (Env::Local, Model::Kirk) => {
            // Pick a reasonable default node count derived from the dim cap; the
            // stub does not actually read this for compute, only for buffer sizes.
            let visible_nodes = pick_visible_nodes(cfg.max_matrix_dim);
            let b = KirkLocalBackend::new(visible_nodes, cfg.max_matrix_dim)
                .map_err(|e| ServerError::Internal(format!("backend init failed: {e}")))?;
            Ok(b as Arc<dyn ModelBackend>)
        }
        (Env::Prod, Model::Kirk) => prod_kirk(cfg),
    }
}

#[cfg(feature = "secret-kirk-edge")]
fn prod_kirk(cfg: &Config) -> Result<Arc<dyn ModelBackend>, ServerError> {
    let visible_nodes = pick_visible_nodes(cfg.max_matrix_dim);
    let b = KirkProdBackend::new(visible_nodes, cfg.max_matrix_dim)
        .map_err(|e| ServerError::Internal(format!("backend init failed: {e}")))?;
    Ok(b as Arc<dyn ModelBackend>)
}

#[cfg(not(feature = "secret-kirk-edge"))]
fn prod_kirk(_cfg: &Config) -> Result<Arc<dyn ModelBackend>, ServerError> {
    Err(ServerError::BadRequest(
        "--env prod requires a build with the secure feature; see docs/SECURE_BUILD.md".into(),
    ))
}

fn pick_visible_nodes(max_dim: u32) -> usize {
    // Modest default; the stub's buffers scale as `(2 * visible_nodes)^2`. The
    // configured dim cap is independent — runtime checks live in `check_dim`.
    let clamped = max_dim.clamp(2, 64);
    clamped as usize
}
