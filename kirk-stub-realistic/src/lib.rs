//! Rust port of `kirk-stub-realistic` (v0.2.3).
//!
//! Pipeline: hermitianize -> eigendecomposition (real-symmetric 2N block trick on
//! a complex Hermitian matrix) -> Boltzmann softmax -> density matrix
//! reconstruction -> observables -> Shannon entropy.
//!
//! See the architect spec for the full functional / non-functional requirements.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod density_matrix;
pub mod eigensolver;
pub mod entropy;
pub mod kirk;
pub mod observables;
pub mod output;
pub mod reconstruct;
pub mod rng;
pub mod sample;
pub mod variants;

pub use kirk::KirkRealistic;
pub use output::{KirkError, KirkOutput, KirkSampleOutput};
pub use sample::forward_sample;
pub use variants::{
    active_inference, active_inference_entropy, active_inference_features, inference_entropy,
    inference_features,
};

/// Library schema version, mirrors Python `KirkOutput.schema_version`.
pub const SCHEMA_VERSION: &str = "v1.0";
