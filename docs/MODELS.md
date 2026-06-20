# Model Selector

`kirk-server` supports two model backends, selected at startup with `--model {tiberius,kirk}`. The wire format on every transport is unchanged; the selector only affects which compute implementation runs behind the scenes.

## Overview

| Flag value | Local backend | Prod backend |
|------------|---------------|--------------|
| `tiberius` (default) | `kirk-stub-realistic` Boltzmann/Shannon kernel | Same as local — Tiberius has no remote variant |
| `kirk` | `kirk-stub-kirk` shape-correct stub | Secure variant via the `secret-kirk-edge` Cargo feature (see [SECURE_BUILD.md](SECURE_BUILD.md)) |

With the defaults (`--model tiberius --env local`), behavior is byte-for-bit identical to the pre-multi-model release on every existing test fixture and integration test.

---

## Tiberius

**Flag**: `--model tiberius` (default)

**Crate**: `kirk-stub-realistic`

Tiberius is the production-grade Boltzmann/Shannon Hermitian kernel. It is the implementation that shipped before the multi-model selector was added, and it is the only model that has been benchmarked against the Python reference fixtures.

### What it computes

At each call the kernel:

1. Hermitianizes the input matrix `H` (forces self-adjointness).
2. Eigendecomposes the resulting Hermitian matrix to get the real eigenvalue spectrum.
3. Applies Gibbs / Boltzmann softmax weighting (controlled by `--temperature`, default 1.0) to turn the eigenvalues into a probability distribution.
4. Constructs a density matrix `rho` from the weighted eigenvectors.
5. Computes Shannon entropy on the distribution.
6. Updates a rolling window of entropy values and computes a z-score for regime labeling.

The math is grounded in Gibbs statistical mechanics and Shannon information theory applied to the eigenspectrum of a Hermitian operator.

### Numerical properties

- Operates internally in **f32 / Complex32**.
- Entropy and eigenvalues are f32; the density matrix `rho` is f32.
- **Stateful**: holds a rolling window of past entropy values (length `--window-size`, default 256). The `entropy_zscore` and `regime` fields are meaningful once the window has filled. Before the window is full, `entropy_zscore = 0.0` and `regime = 0`.
- Shared single state instance across all transports. Concurrent calls are serialized through `parking_lot::Mutex`.

### Response fields (tiberius-specific behavior)

| Field | Tiberius behavior |
|-------|-------------------|
| `entropy_re` | Shannon entropy from real-eigenvalue distribution |
| `entropy_im` | Shannon entropy from imaginary-eigenvalue distribution |
| `entropy` | Combined scalar entropy |
| `entropy_zscore` | Z-score from rolling window; 0.0 before window fills |
| `regime` | Rolling-window regime label (0 = below threshold, 1 = above) |
| `confidence` | `1 - entropy / ln(N)` (0 = maximum entropy, 1 = pure state) |

### When to use Tiberius

- **Default choice for all production workloads** where you want a mathematically meaningful entropy signal that fluctuates window-to-window in response to the input.
- Benchmarking and regression testing — the parity fixtures in `kirk-stub-realistic/tests/fixtures/` are defined against this backend.
- Any scenario where the rolling-window z-score (`entropy_zscore`) or regime label is needed.
- When numerical precision at f32 is sufficient.

---

## Kirk

**Flag**: `--model kirk`

**Crate**: `kirk-stub-kirk` (local stub) or the secure `secret-kirk-edge` crate (prod)

### Local stub (`--env local`)

The local Kirk backend is a **shape-correct, deterministic stub**. It exposes the same five-method surface as the real implementation and returns outputs with correct shapes and finite values, but the numerical values are derived from simple row/column reductions of the input rather than the full algorithm.

Key characteristics of the stub:

- Operates internally in **f64 / Complex64** (approximately 2x the memory of Tiberius for the same N).
- Output shapes are identical to what the real Kirk backend would return.
- Outputs are **deterministic given the same input** — useful for writing shape and regression tests.
- `entropy_zscore` is always `0.0`; `regime` is always `1`; `confidence` is always `0.0`. The kirk stub has no rolling window.
- The bon::Builder fields `rho_hat`, `hamiltonian`, `rho_t`, `obserable`, `hidden_bool_inter`, and `hidden_bool_intra` are allocated at construction and zeroed. They exist for builder-shape compatibility with the secure production variant and are not read or written by the stub's five public methods.
- A startup warning is logged when `--model kirk --env local` is active: `backend selected: kirk`. This prevents silent confusion between the stub and the production variant.

The stub is not a substitute for the real Kirk algorithm. **Do not use the stub's numerical outputs for inference or analysis** — only its shape guarantees hold.

### Production variant (`--env prod`)

`--model kirk --env prod` activates the secure `secret-kirk-edge` crate. This requires a special build; see [SECURE_BUILD.md](SECURE_BUILD.md) for operator instructions.

The production variant has a different implementation behind a feature flag. This document does not describe its internal structure.

### When to use Kirk

- Testing that your client correctly handles the response shapes (feature arrays, feature vectors, scalars) without needing meaningful values.
- Integration testing of the `--model` flag dispatch code.
- Preparing for future use of the production variant: build your client against the stub's shapes now.
- `--env prod --model kirk` with the secure feature: the production use case for the real Kirk algorithm.

---

## Decision matrix

| `--model` | `--env` | `secret-kirk-edge` feature | What runs | Notes |
|-----------|---------|---------------------------|-----------|-------|
| `tiberius` | `local` | off (default) | `TiberiusBackend` | Default. Full rolling-window behavior. |
| `tiberius` | `prod` | off | `TiberiusBackend` | Allowed. Tiberius has no remote variant; runs the local kernel. |
| `tiberius` | `local` | on | `TiberiusBackend` | Feature being on has no effect for tiberius. |
| `tiberius` | `prod` | on | `TiberiusBackend` | Tiberius has no prod variant; uses local kernel regardless. |
| `kirk` | `local` | off (default) | `KirkLocalBackend` | Shape-correct stub. |
| `kirk` | `local` | on | `KirkLocalBackend` | Local stub is still used for `--env local` even when the feature is on. |
| `kirk` | `prod` | off | Rejected at startup (exit 2) | Error message points to [SECURE_BUILD.md](SECURE_BUILD.md). |
| `kirk` | `prod` | on | `KirkProdBackend` (secure crate) | Requires Tailnet access at build time. |

Note: the only combination that the startup guard rejects is `(--model kirk, --env prod)` without the `secret-kirk-edge` feature. Every other combination — including `(--model tiberius, --env prod)` regardless of feature state — is allowed and runs the local Tiberius kernel. See [SECURE_BUILD.md](SECURE_BUILD.md) for the exact guard logic.

---

## Numerical differences

| Property | Tiberius | Kirk (stub) |
|----------|----------|-------------|
| Internal float type | f32 / Complex32 | f64 / Complex64 |
| Memory for N=128 model state | ~4 MiB | ~8 MiB |
| Input precision | f32 (from trait boundary) | f32 received, lifted to f64 internally |
| Output precision (entropy scalar) | f32 | f64, returned as f32 via the trait |
| Entropy computation | Shannon entropy on eigenspectrum (Gibbs softmax) | Mean squared norm of input elements |
| Rolling window | Yes (`--window-size`, default 256) | No |
| `regime` | Derived from rolling-window z-score | Always 1 |
| `entropy_zscore` | Window-based | Always 0.0 |
| `confidence` | `1 - entropy / ln(N)` | Always 0.0 |

The v2 JSON envelope carries f64 values. For both backends, v2 input f64 values are truncated to f32 at the trait boundary before entering the compute path. Kirk's f64 advantage is therefore limited to its internal computation, not to end-to-end precision. See ADR-005 in `agent-notes/architect.md` for the rationale.

---

## Configuration

### CLI flags

```bash
# Default: tiberius, local
cargo run --release -p kirk-server -- --bind 127.0.0.1

# Kirk stub, local
cargo run --release -p kirk-server -- --bind 127.0.0.1 --model kirk

# Kirk production (requires secret-kirk-edge build; see SECURE_BUILD.md)
./kirk-server --model kirk --env prod
```

### Environment variables

| Variable | Values | Default |
|----------|--------|---------|
| `KIRK_MODEL` | `tiberius`, `kirk` | `tiberius` |
| `KIRK_ENV` | `local`, `prod` | `local` |

### Docker Compose example

```yaml
services:
  kirk-server:
    image: kirk-server:latest
    environment:
      KIRK_MODEL: kirk
      KIRK_ENV: local
    ports:
      - "8080:8080"
      - "50051:50051"
      - "9090:9090"
```

Do not set `KIRK_ENV=prod` in your Docker Compose file unless the image was built with the secure feature. Default Docker images are built without it; see [SECURE_BUILD.md](SECURE_BUILD.md).
