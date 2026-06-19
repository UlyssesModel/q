# kirk-stub-realistic (Rust)

Drop-in Rust port of the Python `kirk_stub_realistic` package (v0.2.3). Pure Rust, no LAPACK, no C dependencies, `#![forbid(unsafe_code)]`.

## Overview

The crate provides a six-stage Hamiltonian-to-entropy compute pipeline, stateful rolling-window regime detection, five stateless Joel-API variant functions, and a shape-correct random sample generator. It is the compute kernel consumed by `kirk-server` behind all three transports.

## Quick Start

```rust
use kirk_stub_realistic::{KirkRealistic, KirkError};

fn main() -> Result<(), KirkError> {
    let mut kirk = KirkRealistic::new(1.0, 256)?;

    // 4x4 identity Hamiltonian (real part only; imag all zeros)
    let n: usize = 4;
    let matrix_re: Vec<f32> = vec![
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    let matrix_im: Vec<f32> = vec![0.0; n * n];
    let ts = 0_i64;

    let out = kirk.forward(&matrix_re, &matrix_im, n, ts)?;
    println!("entropy = {:.4}", out.entropy);
    println!("regime  = {}", out.regime);
    println!("confidence = {:.4}", out.confidence);
    Ok(())
}
```

## Six-Stage Pipeline

| Stage | Function | Description |
|-------|----------|-------------|
| 1 | `hermitianize` | Symmetrize: `H = (H_raw + H_raw†) / 2` |
| 2 | `diagonalize` | Eigendecomposition via the real symmetric 2N block trick (see Caveats) |
| 3 | `boltzmann_weights` | Numerically stable softmax of `-λ / T` |
| 4 | `density_matrix` | `ρ = V * diag(p) * V†` |
| 5 | Observables | Expected energy, magnetization, occupancy (parity bookkeeping, not surfaced by `forward`) |
| 6 | `shannon_entropy` | `-Σ p_i ln(p_i + 1e-12)` |

`KirkRealistic::forward` adds a rolling-window z-score over the last `window_size` entropies, a regime classifier (0 = low, 1 = medium, 2 = high), and `confidence = clip(1 - H / ln(N), 0, 1)`.

## Public API Surface

### `KirkRealistic`

```rust
pub struct KirkRealistic { /* rolling-window state */ }

impl KirkRealistic {
    /// Create a new stateful processor.
    /// `temperature` must be > 0; `window_size` must be >= 1.
    pub fn new(temperature: f32, window_size: usize) -> Result<Self, KirkError>;

    /// Run the six-stage pipeline on a flattened row-major complex Hermitian matrix.
    /// `N` must be >= 2 (the server rejects N < 2 at the boundary; N=1 is degenerate).
    pub fn forward(
        &mut self,
        matrix_re: &[f32],
        matrix_im: &[f32],
        n: usize,
        timestamp_us: i64,
    ) -> Result<KirkOutput, KirkError>;
}
```

### `KirkOutput`

All 10 fields from the Python pydantic model:

| Field | Type | Description |
|-------|------|-------------|
| `entropy_re` | `f32` | Shannon entropy of Re(ρ) eigenvalues |
| `entropy_im` | `f32` | Shannon entropy of Im(ρ) eigenvalues |
| `entropy` | `f32` | Shannon entropy of ρ eigenvalues (main output) |
| `entropy_zscore` | `f32` | Z-score over the rolling window |
| `regime` | `u32` | 0 / 1 / 2 (low / medium / high entropy) |
| `confidence` | `f32` | `clip(1 - H / ln(N), 0, 1)` |
| `processing_time_us` | `u64` | Wall-clock time for `forward` in microseconds |
| `timestamp_us` | `i64` | Echo of the caller-supplied timestamp |
| `matrix_re` | `Vec<f32>` | Flattened row-major Re(ρ), length N*N |
| `matrix_im` | `Vec<f32>` | Flattened row-major Im(ρ), length N*N |

Accessor: `out.matrix_dim()` returns `N`.

### `KirkSampleOutput`

Returned by `forward_sample`:

| Field | Type | Description |
|-------|------|-------------|
| `feature_array` | `Array2<Complex32>` | Shape (N, N) |
| `feature_vector` | `Array1<Complex32>` | Shape (2N,) |
| `feature_scalar` | `Complex32` | Scalar |
| `relative_entropy` | `f32` | >= 0 |

### Five Stateless Variants

```rust
pub fn inference_entropy(sample_re: &[f32], sample_im: &[f32], n: usize) -> Result<f32, KirkError>;
pub fn inference_features(sample_re: &[f32], sample_im: &[f32], n: usize)
    -> Result<(Array2<Complex32>, Array1<Complex32>, Complex32), KirkError>;
pub fn active_inference(sample_re: &[f32], sample_im: &[f32], n: usize)
    -> Result<(Array2<Complex32>, Array1<Complex32>, Complex32, f32), KirkError>;
pub fn active_inference_entropy(sample_re: &[f32], sample_im: &[f32], n: usize) -> Result<f32, KirkError>;
pub fn active_inference_features(sample_re: &[f32], sample_im: &[f32], n: usize)
    -> Result<(Array2<Complex32>, Array1<Complex32>, Complex32), KirkError>;
```

All default to `temperature = 1.0`. Real-only senders may pass `sample_im` as an all-zeros slice.

### `forward_sample`

```rust
pub fn forward_sample(n: usize, rng: &mut impl Rng) -> KirkSampleOutput;
```

Generates a shape-correct random complex64 output. Mirrors the Python `sample.forward_sample` surface. Use `kirk_stub_realistic::seeded_rng(seed)` for reproducible outputs.

## Numerical Parity (NFR-001)

For seeded inputs at N ∈ {8, 16, 32}, the Rust outputs match the Python `numpy` reference within:

| Metric | Tolerance |
|--------|-----------|
| `entropy`, `entropy_re`, `entropy_im` | `|rs - py| / max(1, |py|) <= 1e-4` |
| `confidence` | `|rs - py| <= 1e-4` absolute |
| `regime` | exact match |
| density matrix `rho` Frobenius | `||rho_rs - rho_py||_F / ||rho_py||_F <= 1e-3` |

Eigenvector sign is not compared (gauge ambiguity in `V D V†`).

## Caveats

**2N block trick.** Because `nalgebra::SymmetricEigen` operates on real symmetric matrices, complex Hermitian eigendecomposition is implemented via the standard real embedding:

```
[[Re(H), -Im(H)],
 [Im(H),  Re(H)]]
```

This `2N x 2N` real symmetric matrix has each eigenvalue appearing twice. The implementation sorts eigenvalues ascending and picks every other one (with a `1e-5` relative tolerance gate on paired duplicates). For highly degenerate spectra the gate may need tightening — see the coding notes for details.

**No LAPACK.** Everything is pure Rust via `nalgebra 0.33`. For large N (>= 128) the server offloads eigendecomposition to `spawn_blocking`; the crate itself has no built-in concurrency.

**Real-only on the wire.** The gRPC and REST transports encode the density matrix as two separate `f32` byte arrays (`data_re`, `data_im` / `matrix_re`, `matrix_im`). The TCP protocol does the same with raw little-endian floats.

## Tests

```bash
cargo test -p kirk-stub-realistic
```

`tests/basic.rs` (7 tests):
- hermitianize round-trip
- eigenvalue sum equals trace
- `rho.trace() ≈ 1.0`
- `confidence ∈ [0, 1]`
- Shannon entropy non-negative
- all five variants produce finite outputs for N ∈ {2, 4, 8, 16, 32}
- `forward_sample` produces finite non-negative `relative_entropy`

`tests/parity.rs` (4 tests):
- hand-computed N=2 (`H = diag(1, -1)`) — analytical entropy `≈ 0.3653`
- seed42 N=8, N=16, N=32 — all tolerances above met

### Fixture Files

```
tests/fixtures/handcalc_N2.json    — hand-computed reference
tests/fixtures/seed42_N8.json      — Python-generated
tests/fixtures/seed42_N16.json     — Python-generated
tests/fixtures/seed42_N32.json     — Python-generated
```

To regenerate Python-generated fixtures:

```bash
cd /Users/charmalloc/dev/kavara/kirk-stub-realistic
uv run --python 3.13 python /tmp/gen_fixtures.py
```

Do not modify the Python reference at `/Users/charmalloc/dev/kavara/kirk-stub-realistic/`.
