//! Per-(model, env) concrete backend implementations.
//!
//! Each backend stands alone: its own state, its own conversions, its own
//! concurrency model. The factory in [`factory`] picks one at startup.

pub mod factory;
pub mod kirk_local;
#[cfg(feature = "secret-kirk-edge")]
pub mod kirk_prod;
pub mod tiberius;

pub use factory::select_backend;
pub use kirk_local::KirkLocalBackend;
#[cfg(feature = "secret-kirk-edge")]
pub use kirk_prod::KirkProdBackend;
pub use tiberius::TiberiusBackend;
