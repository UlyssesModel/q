//! Server-wide error type with mappings to each transport's error surface.

use kirk_stub_realistic::KirkError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("payload too large: {actual} bytes (limit {limit})")]
    PayloadTooLarge { actual: usize, limit: usize },
    #[error("matrix dim {actual} exceeds max {limit}")]
    MatrixDimExceeded { actual: u32, limit: u32 },
    #[error("compute error: {0}")]
    Compute(#[from] KirkError),
    #[error("server is shutting down")]
    Shutdown,
    #[error("internal: {0}")]
    Internal(String),
}

impl ServerError {
    pub fn http_status(&self) -> u16 {
        match self {
            ServerError::BadRequest(_) => 400,
            ServerError::PayloadTooLarge { .. } => 413,
            ServerError::MatrixDimExceeded { .. } => 413,
            ServerError::Compute(_) => 422,
            ServerError::Shutdown => 503,
            ServerError::Internal(_) => 500,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ServerError::BadRequest(_) => "bad_request",
            ServerError::PayloadTooLarge { .. } => "payload_too_large",
            ServerError::MatrixDimExceeded { .. } => "matrix_dim_exceeded",
            ServerError::Compute(_) => "compute_error",
            ServerError::Shutdown => "shutdown_in_progress",
            ServerError::Internal(_) => "internal",
        }
    }
}
