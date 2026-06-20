# Plan — Tiberius + Kirk multi-model selector, --env local|prod, REST /v2

Authoritative source: `agent-notes/architect.md`.

## User clarifications (binding)

**C-1. Secret internal steps stay secret.** The pasted Kirk prototype names internal stages (`construct_hidden_activations`, `construct_hamiltonian`, `construct_density_matrix`, `construct_observeable`, `calculate_entropy`, `calculate_features`, `update_weights`, `init_rho_hat`). These are **proprietary algorithm structure**. In the public `kirk-stub-kirk` crate:
- DO NOT define any of these private helper functions.
- DO NOT reference them in comments, doc-strings, error messages, log messages, or step-numbered comments inside the five public methods.
- DO NOT include `use ndarray_linalg::{Eig, Eigh, Scalar, Trace}` (the stub doesn't need eigh; importing these advertises the real algorithm shape).
- The five public methods' bodies are short, shape-correct, deterministic-given-input expressions and nothing more.
- `init_rho_hat` is the only exception — it's called from the builder. Replace it with a one-liner inside the builder body (not a separate fn).

**C-2. Highest-order functions implement their own way per (model, env).** No shared abstraction beyond the `trait ModelBackend`. Each (model, env) combination is its own concrete impl:
- `TiberiusBackend` — `--env local` and `--env prod` both use the local Tiberius kernel (Tiberius has no remote variant).
- `KirkLocalBackend` — `--env local` uses the local `kirk-stub-kirk` stub.
- `KirkProdBackend` — `--env prod` calls into the `secret-kirk-edge` crate. cfg-gated on feature `secret-kirk-edge`. Compiles only when that feature is on.
- Selection happens once at startup in `main.rs`. Each backend keeps its own state, errors, conversions, and concurrency model. No inheritance, no shared base impl.

## Phases

### Phase A — Workspace + new `kirk-stub-kirk` crate
1. Add `kirk-stub-kirk/` to workspace members.
2. `Cargo.toml`: `ndarray`, `bon`, `getset`, `num-complex`, `num-traits`, `serde`, `ndarray-rand`. NO `ndarray-linalg`.
3. `src/lib.rs`: `pub use kirk::Kirk;`.
4. `src/kirk.rs`: bon::Builder skeleton verbatim from the pasted code (struct fields, attribute order, custom `KirkBuilder<S>::build`, `Default`, `pub fn new()`). FIVE public methods only. Each body: 2-5 lines of shape-correct math. ZERO private helpers. ZERO references to the secret algorithm.
5. Inline unit tests: shape + finite outputs for N ∈ {2, 4, 8}.

### Phase B — `ModelBackend` trait + per-(model,env) impls
1. `kirk-server/src/model.rs`: define `pub trait ModelBackend: Send + Sync` with the public method set the transports already call (`forward`, `inference_entropy`, `inference_features`, `active_inference`, `active_inference_entropy`, `active_inference_features`, `forward_sample`, `check_dim`).
2. `kirk-server/src/backends/tiberius.rs`: wraps `KirkRealistic`. Implements the trait. Same behavior for local and prod (Tiberius has no remote).
3. `kirk-server/src/backends/kirk_local.rs`: wraps `kirk_stub_kirk::Kirk`. Boundary: f32 → f64 → Complex64 in; Complex64 → f32 out. Implements the trait.
4. `kirk-server/src/backends/kirk_prod.rs`: `#[cfg(feature = "secret-kirk-edge")]`. Calls into `secret_kirk_edge::...` for the same surface. **No URL, no crate name, no algorithm hints in any log / error / panic message.**
5. `kirk-server/src/backend.rs`: keep `Arc<dyn ModelBackend>`. Drop the hardcoded `KirkRealistic` field. `KirkBackend::new(model, env, ...) -> Result<Arc<Self>, ServerError>` does the dispatch.

### Phase C — CLI flags
1. `--model {tiberius, kirk}` env `KIRK_MODEL`, default `tiberius`.
2. `--env {local, prod}` env `KIRK_ENV`, default `local`.
3. Runtime guard: if `--env prod` and `cfg!(not(feature = "secret-kirk-edge"))`, exit with `ServerError::Config("--env prod requires a build with the secure feature; see docs/SECURE_BUILD.md")`. No URL printed.
4. `--env prod` + `--model tiberius` → allowed (uses TiberiusBackend; same as local).

### Phase D — Cargo feature wiring
1. `kirk-server/Cargo.toml`:
   ```toml
   [dependencies]
   secret-kirk-edge = { git = "https://git-kavara.ibis-allosaurus.ts.net/kavara-ai/secret-kirk-edge-v2.git", optional = true }
   [features]
   default = []
   secret-kirk-edge = ["dep:secret-kirk-edge"]
   ```
2. `docker/Dockerfile` and `.github/workflows/ci.yml` MUST NOT set `--features secret-kirk-edge`. Add a CI assertion: `! cargo metadata --format-version 1 | grep -q '"secret-kirk-edge"'` after the standard `cargo build`. (Failsafe so an accidental flag doesn't leak the dep into the image.)

### Phase E — REST `/v2/` routes
1. `kirk-server/src/rest/schema_v2.rs`: serde structs for the nested `[[[re, im], …], …]` shape. `MatrixJson` type alias = `Vec<Vec<[f64; 2]>>`. Validation: square + each pair length 2 + matrix-dim cap. Convert to flat `(Vec<f32>, Vec<f32>, usize)` for backends.
2. `kirk-server/src/rest/routes_v2.rs`: 7 routes (`/v2/forward`, `/v2/inference/{entropy,features}`, `/v2/active-inference{,/entropy,/features}`, `/v2/forward-sample`).
3. Reuse `ServerError` / `ErrorResponse`. Same 64 MiB body cap; document practical max-N ≈ 300.
4. Mount alongside v1 in `rest/routes.rs::build_router`.

### Phase F — Docs
1. `docs/REST.md` — add a "v1 vs v2" comparison table + when to use each.
2. `docs/MODELS.md` (new) — Tiberius vs Kirk; when to use each; numerical caveats.
3. `docs/SECURE_BUILD.md` (new) — how to build with `--features secret-kirk-edge`. No URLs in the public doc — only an env var the user fills in locally (`SECRET_KIRK_EDGE_GIT=...`). Cargo.toml uses `{ git = "$SECRET_KIRK_EDGE_GIT", ... }`? Actually Cargo doesn't expand env vars in deps. Two options: (a) commit the URL but mark it private (Tailnet, anyway), or (b) `[patch.crates-io]` snippet documented in SECURE_BUILD.md that the user copies in locally.
   - Pick: commit the URL (it's already Tailnet-private; not a credential).
   - SECURE_BUILD.md documents: requires Tailnet access, never enable in CI/Docker, opsec notes.

### Phase G — Tests
1. `kirk-stub-kirk` unit tests: shape, finite, deterministic given seed.
2. `kirk-server` integration tests: each backend dispatcher path; v2 ↔ v1 numerical agreement; `--env prod` rejected without feature.

## Acceptance
- `cargo build --workspace` clean (default features).
- `cargo build --workspace --features secret-kirk-edge` fails on this host (Tailnet not reachable) — expected, document it.
- `cargo test --workspace --lib --bins` 100% pass.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- v2 endpoint produces same KirkOutput as v1 for the same matrix (up to f64↔f32 rounding) when --model tiberius.
- `grep -rn "construct_\|calculate_\|update_weights\|init_rho_hat\|Eigh\|hidden_bool_inter\|hidden_bool_intra\|rho_t\|hamiltonian\|obserable" kirk-stub-kirk/src/ kirk-server/src/` returns ONLY the bon::Builder struct field definitions for `rho_t`, `hamiltonian`, `obserable`, `hidden_bool_inter`, `hidden_bool_intra` (these are public fields the builder owns — they're allocated zero-valued and never read by the stub). Everything algorithm-shaped is gone.
