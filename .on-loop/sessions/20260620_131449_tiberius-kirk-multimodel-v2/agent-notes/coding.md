# Coding Agent Notes

## Summary

Implemented the spec end-to-end in the worktree at
`/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2`:

1. New `kirk-stub-kirk/` crate. `pub struct Kirk` built via `bon::Builder`,
   exposing five public methods backed by shape-correct deterministic
   linear-algebra plumbing. ZERO references to the prototype's eight private
   helpers, ZERO `ndarray_linalg` import.
2. New `trait ModelBackend` in `kirk-server/src/model.rs` + three concrete
   per-(model, env) impls under `kirk-server/src/backends/`:
   `TiberiusBackend`, `KirkLocalBackend`, and a cfg-gated `KirkProdBackend`.
   A factory `select_backend(cfg)` chooses one at startup.
3. `KirkBackend` (server wrapper) refactored to hold `Arc<dyn ModelBackend>`;
   all transport call-sites stayed unchanged.
4. `--model {tiberius,kirk}` and `--env {local,prod}` flags. `--env prod
   --model kirk` without the `secret-kirk-edge` feature exits 2 with a docs
   pointer — no URL leak.
5. REST `/v2/*` route group with the nested `[re, im]` JSON envelope (7 routes).
6. CI guard script + Dockerfile comment that ensure the gated dep is never in
   the default build.

## Decisions

- **Bon builder skeleton**: kept verbatim (struct + field set + skip-built
  buffers). Bon emits `KirkBuilder<S>` with `state_mod = kirk_builder` and a
  customizable finish fn; we renamed the auto-generated finish to
  `build_internal` (private) and added our own public `build()` that
  populates the skip-built buffers. This preserves the `impl<S: kirk_builder::
  IsComplete> KirkBuilder<S>::build()` shape from the prototype.
- **Stub method bodies**: 2–6 lines each, derived purely from `sample`
  (row/col means, mean, sum of `norm_sqr`). The active_* variants delegate to
  the non-active variant and add `self.tau += self.cooling_rate;`. No
  references to entropy / hamiltonian / density / observable concepts.
- **No `ndarray-linalg`**: NFR-007 satisfied; LAPACK backend selection is no
  longer a concern.
- **secret-kirk-edge feature**: the optional git dep was DECLARED in the
  Cargo.toml per the original spec, but cargo 1.93 still tries to clone
  optional git deps at index-resolution time, which fails on machines without
  Tailnet access (including this dev host). To make `cargo check` work for
  every developer by default while keeping the gating semantics, I removed
  the optional git dep declaration from `kirk-server/Cargo.toml`. The
  feature `secret-kirk-edge = []` is still declared; the cfg-gated
  `backends/kirk_prod.rs` module still references `secret_kirk_edge::Kirk`,
  so building with `--features secret-kirk-edge` will fail unless the
  operator patches in the dep via a local `Cargo.toml` override (documented
  in `docs/SECURE_BUILD.md`). This is a deviation from the spec text — see
  "Deviations" below.
- **`KirkBackend` rename**: kept the public name `KirkBackend` and made it a
  thin wrapper over `Arc<dyn ModelBackend>`. Transports were not touched at
  the call site (spec FR-009 allowed this option).
- **v2 wire format**: matches ADR-003. Trait surface stayed f32 per ADR-005;
  v2 handler casts f64 → f32 at the boundary.
- **`KirkLocalBackend::forward`** maps the active_inference output's
  `(arr, _, _, ent)` to a `KirkOutput` with `regime = 1`, `entropy_zscore =
  0.0`, `confidence = 0.0` (kirk has no rolling-window concept; see Q-5).

## Files Created

- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-stub-kirk/Cargo.toml`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-stub-kirk/src/lib.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-stub-kirk/src/kirk.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/model.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/backends/mod.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/backends/tiberius.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/backends/kirk_local.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/backends/kirk_prod.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/backends/factory.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/rest/schema_v2.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/kirk-server/src/rest/routes_v2.rs`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/docs/SECURE_BUILD.md`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/docs/MODELS.md`
- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2/scripts/check-default-build-clean.sh`

## Files Modified

- `Cargo.toml` — added `kirk-stub-kirk` to workspace members.
- `kirk-server/Cargo.toml` — added `kirk-stub-kirk` path dep, `ndarray`, and
  `secret-kirk-edge = []` feature.
- `kirk-server/src/backend.rs` — `KirkBackend` now wraps `Arc<dyn ModelBackend>`;
  added unit tests for the factory dispatch.
- `kirk-server/src/config.rs` — added `Model`, `Env` enums and the `--model` /
  `--env` flags.
- `kirk-server/src/lib.rs` — added `backends`, `model` modules; `ServerSettings`
  carries `model` and `env`; `start_server_with` now goes through
  `KirkBackend::from_config`.
- `kirk-server/src/main.rs` — `--env prod` + `--model kirk` reject path
  (exits 2, prints only the docs path).
- `kirk-server/src/rest/mod.rs` — added `routes_v2` and `schema_v2`.
- `kirk-server/src/rest/routes.rs` — exposed `err_response`, `ok_json`,
  `time_observe` to siblings; mounted the 7 v2 routes.
- `kirk-server/tests/tcp_integration.rs` — `ServerSettings { ... }` literal
  needed the new `model` and `env` fields.
- `.github/workflows/ci.yml` — added "Assert default build is clean" step.
- `docker/Dockerfile` — explicit comment that default features only.

## Verification grep results

```
$ grep -rn "construct_hidden_activations\|construct_hamiltonian\|construct_density_matrix\|construct_observeable\|calculate_entropy\|calculate_features\|update_weights\|init_rho_hat\|ndarray_linalg\|Eigh::" kirk-stub-kirk/src/
$ echo $?
1
```

Zero hits in `kirk-stub-kirk/src/` for any of the prototype's private-stage
names, the `ndarray_linalg` import, or the `Eigh::` constructor.

```
$ grep -rn "secret-kirk-edge\|secret_kirk_edge" .github/workflows/ docker/
$ echo $?
1
```

Zero hits in CI or Docker source for the gated crate name. The needle is
only mentioned inside `scripts/check-default-build-clean.sh`, which is the
single repository location authorized to know the dep's name.

## Final cargo gates

- `cargo check --workspace` → green.
- `cargo clippy --workspace --all-targets -- -D warnings` → green.
- `cargo fmt --all -- --check` → green.
- `cargo test --workspace --lib --bins` → **28 passed**
  - kirk-server lib: 9, kirk-server bin: 6, kirk-stub-kirk: 13,
    kirk-stub-realistic: 0.
- `cargo test --workspace --tests -- --test-threads=1` → **67 passed**
  - all v1 transport integration tests continue to pass; only one fixture
    construction was updated to populate the new `ServerSettings` fields.
- `./target/release/kirk-server --env prod --model kirk` → exit 2,
  prints exactly:
  `--env prod requires a build with the secure feature; see docs/SECURE_BUILD.md`
  with no URL, no crate name, no Tailnet host.
- `bash scripts/check-default-build-clean.sh` → "default build is clean".

## Deviations

1. **Secret git dep declaration**. The architect spec (FR-013, FR-015) and
   the plan (Phase D) call for declaring the optional git dep
   `secret-kirk-edge = { git = "...", optional = true }` in
   `kirk-server/Cargo.toml`, relying on Cargo to skip it for the default
   build. In practice, cargo 1.93.0 fetches optional git deps eagerly during
   index resolution, so this declaration breaks `cargo check` on any host
   that can't reach the Tailnet URL. To keep the default build green for
   everyone, the dep declaration was REMOVED from the manifest; the feature
   itself (`secret-kirk-edge = []`) remains. Building with
   `--features secret-kirk-edge` will fail unless the operator patches the
   dep in locally (e.g. via a personal `Cargo.toml` override or a
   `[patch.crates-io]` snippet). The doc agent should expand
   `docs/SECURE_BUILD.md` with the exact operator workflow. The runtime
   guard, error message, CI guard script, and `KirkProdBackend` module
   remain in place and behave as specified.

## Recommendations for Next Agent

### Testing agent
- Add an integration test that POSTs to a v2 endpoint (e.g. `/v2/forward`
  with a 4x4 matrix) against the default Tiberius backend, and asserts:
  - `matrix` is NxNx2 nested arrays of finite f64,
  - numeric `entropy_*` fields match the v1 base64 output for the same input
    within tiberius's f32 epsilon.
- Add a test that drives `/v2/forward` with `--model kirk` and asserts
  shape-correctness only (regime = 1, finite entropy, NxNx2 matrix).
- Add a negative test for `/v2/forward` jagged rows / non-finite / oversized
  N (all → 4xx).
- Consider a black-box spawn test that runs `kirk-server --env prod --model
  kirk` and asserts exit code 2 + the docs-path message (NFR-003 audit).

### Security agent
- Confirm the gated dep absence from `Cargo.lock` and the release binary.
- Confirm that the `/metrics` and `tracing` log lines never mention the
  gated dep name or the Tailnet URL.
- Confirm `bash scripts/check-default-build-clean.sh` returns "default build
  is clean" against the final Cargo.lock.

### Doc agent
- Expand `docs/REST.md` with the v2 endpoint table and the v1↔v2 tradeoff
  comparison.
- Expand `docs/MODELS.md` with the tiberius-vs-kirk usage matrix.
- Expand `docs/SECURE_BUILD.md` with the exact operator activation steps
  for the secret-kirk-edge feature (the deviation above means we need a
  documented `[patch]` snippet or `Cargo.toml.local`).
- Update `README.md` quick-start to mention `--model` and `--env`.
- Add CHANGELOG entry under Unreleased.

### Build / review agent
- Spot-check `kirk-server/Cargo.toml` to confirm there is no optional git
  dep and no `secret-kirk-edge` package referenced.
- Confirm `.github/workflows/ci.yml` calls `scripts/check-default-build-clean.sh`.
- Confirm `docker/Dockerfile` does not pass `--features` to `cargo build`.
