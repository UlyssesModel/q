//! Public surface of the `kirk-stub-kirk` crate.
//!
//! Exposes a single struct `Kirk` constructed via `bon::Builder`. Five public
//! methods accept an `ArrayView2<Complex64>` sample and return shape-correct,
//! finite, deterministic-given-input outputs. The crate carries no LAPACK
//! backend and contains no algorithmic detail beyond linear-algebra plumbing.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod kirk;

pub use kirk::Kirk;
