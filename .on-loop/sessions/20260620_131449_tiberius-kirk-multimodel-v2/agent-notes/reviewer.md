# Final Code Review — multimodel + /v2 + secure-build

## Methodology

Read-only delta review of the worktree at
`/Users/charmalloc/dev/kavara/q/.claude/worktrees/tiberius-kirk-multimodel-v2`.
Re-ran every binding gate from the dispatch brief: Rule 1 / Rule 2 greps,
`cargo fmt`, `cargo clippy` (`RUSTFLAGS=-D warnings`), `cargo test --lib
--bins`, `cargo audit --ignore RUSTSEC-2024-0436`, and
`bash scripts/check-default-build-clean.sh`. Then cross-checked the architect
spec, coding/security/build/documentation agent notes, and every file the
documentation agent updated against the actual implementation
(`backends/factory.rs`, `main.rs`, `rest/routes_v2.rs`, `rest/schema_v2.rs`,
`Cargo.toml`). Two doc-vs-code drifts were found in `docs/MODELS.md` and
`docs/SECURE_BUILD.md` that could mislead a user (see Findings) — both are
documentation-only and do not affect the binary.

Out of scope per dispatch brief: auth, TLS, Kafka. Not reviewed.

## Binding user rules

- **Rule 1 (no secret algorithm names exposed): PASS.** Grep across
  `kirk-stub-kirk/`, `kirk-server/src/`, `docs/`, and `proto/` for
  `construct_hidden_activations|construct_hamiltonian|construct_density_matrix|construct_observeable|calculate_features|update_weights|init_rho_hat|ndarray_linalg`
  returns zero hits. (`calculate_entropy` also returns zero hits; the
  five public methods on `kirk_stub_kirk::Kirk` contain only row/column
  reductions and `norm_sqr` sums — no algorithm vocabulary.)

- **Rule 2 (Tailnet URL contained): PASS.** Grep for
  `ibis-allosaurus|git-kavara\.|secret-kirk-edge-v2\.git` across `*.rs`,
  `*.toml`, `*.md`, `*.yml`, `*.yaml`, `*.sh` finds the URL/host only in:
  - `docs/SECURE_BUILD.md` (lines 15, 22, 39) — the allowed location, and
  - `kirk-server/src/backend.rs` lines 208, 212 — negative test assertions
    inside `#[cfg(not(feature = "secret-kirk-edge"))]` that confirm the
    runtime error does NOT leak the URL.
  Neither location emits the URL from a built binary; both are sanctioned
  by the dispatch brief.

## Summary

| Status | Count |
| ------ | ----- |
| REQUEST_CHANGES | 2 |
| NIT | 3 |

## Findings

### REQUEST_CHANGES

#### RC-1. `docs/MODELS.md` decision matrix contradicts itself and the code on `(tiberius, prod, feature off)`

- **Location**: `docs/MODELS.md` lines 105 and 113.
- **What it says**:
  - Row 105: `| tiberius | prod | off | Rejected at startup | --env prod requires the secure feature even for tiberius — update: see below. |`
  - Row 107: `| tiberius | prod | on | TiberiusBackend | Tiberius has no prod variant; uses local kernel regardless. |`
  - Note 113 (first sentence): "Note on `(tiberius, prod, feature off)`: this combination is rejected at the main.rs level before any listener starts."
  - Note 113 (second sentence): "The check applies to `--env prod` combined with `--model kirk` only. `(tiberius, prod)` is allowed and runs the local Tiberius backend regardless of the feature state."
- **What the code does** (`kirk-server/src/main.rs:20-28`, `backends/factory.rs:28-44`, plus the test `factory_tiberius_prod_allowed` at `backend.rs:183-189` which already asserts this):
  ```rust
  if matches!(cfg.env, Env::Prod) && matches!(cfg.model, Model::Kirk) {
      #[cfg(not(feature = "secret-kirk-edge"))]
      { eprintln!(...); std::process::exit(2); }
  }
  ```
  The reject guard fires ONLY for `(prod, kirk, feature off)`. `(prod, tiberius)` always succeeds and runs `TiberiusBackend`, regardless of the feature.
- **Why this blocks**: the matrix row 105 is the canonical reference an operator will read. It says `(tiberius, prod, off)` is rejected when in fact it is allowed. The contradictory note at 113 is self-inconsistent ("is rejected" / "is allowed"). A user provisioning a non-secure binary for tiberius prod use will believe the binary will refuse to start, when in fact it will silently come up. That is exactly the misleading-doc class the brief calls out.
- **Recommendation** (read-only, do not apply): change row 105 to:
  `| tiberius | prod | off | TiberiusBackend | Allowed; --env prod only gates Kirk. Tiberius has no remote variant. |`
  and remove the first sentence of the note at line 113, or rephrase to clarify only `(kirk, prod, off)` is rejected.

#### RC-2. `docs/SECURE_BUILD.md` Step 1 claims the `dep:secret-kirk-edge` feature binding "is already present in the committed manifest" — it is not.

- **Location**: `docs/SECURE_BUILD.md` lines 42-50.
- **What it says**:
  ```toml
  [features]
  default = []
  secret-kirk-edge = ["dep:secret-kirk-edge"]
  ```
  "This binding (`"dep:secret-kirk-edge"`) is already present in the committed manifest."
- **What the manifest actually contains** (`kirk-server/Cargo.toml:27`):
  ```toml
  secret-kirk-edge = []
  ```
  The feature is empty. There is no `dep:` binding because the optional git
  dep declaration was intentionally removed (per coding-agent Deviation §1).
- **Why this blocks**: an operator who follows Step 1 literally — adding only the dep line under `[dependencies]` without editing the `[features]` line — will end up with the git dep declared but the feature having no effect on resolution. Running `cargo build --features secret-kirk-edge` will then either fail to compile `kirk_prod.rs` (because the dep is not enabled by the feature) or, worse, succeed in pulling the dep into the lock without the feature actually gating anything. The doc is asking the operator to trust the manifest, which is wrong.
- **Recommendation** (read-only, do not apply): add an explicit "edit the feature line too" instruction:
  ```toml
  [features]
  default = []
  secret-kirk-edge = ["dep:secret-kirk-edge"]   # ← change `[]` to this
  ```
  and remove the "is already present" sentence.

### NITs

#### N-1. No v2 REST integration test in `kirk-server/tests/`

- **Where**: `kirk-server/tests/rest_integration.rs` exercises only v1; there is no `rest_v2.rs` or `/v2/` coverage in any integration file. Coding-agent recommendations already flagged this for the testing agent.
- **Impact**: v2 handlers are exercised by clippy + lib unit tests on `parse_matrix_v2` only. The integration assertion that v2/forward returns the same numeric result as v1/forward for the same matrix (NFR-005 (c) in the architect spec) is not enforced.
- **Why NIT**: the spec explicitly lists this in §"Testing strategy" but does not gate the spec on it; the coding agent flagged it; the security audit does not require it.

#### N-2. `Kirk::inference_entropy` docstring asserts "non-negative" — relies on the input being well-formed

- **Where**: `kirk-stub-kirk/src/kirk.rs:153-157`.
- **Detail**: `c.norm_sqr()` is mathematically non-negative for any finite f64, but if the caller passes a `Complex64` with NaN component, `norm_sqr` returns NaN, which is neither non-negative nor finite. Test `inference_entropy_finite_and_nonnegative` only uses a sanitized input.
- **Why NIT**: REST v2 rejects NaN at the boundary, and tests cover the happy path. Worth a single-line clarification "for finite inputs, non-negative" or a defensive `.max(0.0)`, but not a release blocker.

#### N-3. Makefile `ci` target does not call `check-secure-isolation`

- **Where**: `Makefile:55-64`.
- **Detail**: The CI workflow runs the guard, but `make ci` (the local equivalent) does not. Build-agent notes already flagged this for follow-up.
- **Why NIT**: CI enforces it; local `make ci` is for developer convenience.

## Verification

```
$ cargo fmt --all -- --check
(no output, exit 0)

$ RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.37s
(exit 0)

$ cargo test --workspace --lib --bins
✓ cargo test: 28 passed (4 suites)

$ cargo audit --ignore RUSTSEC-2024-0436
    Loaded 1134 security advisories
    Scanning Cargo.lock for vulnerabilities (271 crate dependencies)
(0 advisories, exit 0)

$ bash scripts/check-default-build-clean.sh
default build is clean

$ grep -rn "construct_hidden_activations|construct_hamiltonian|construct_density_matrix|\
            construct_observeable|calculate_features|update_weights|init_rho_hat|ndarray_linalg" \
            kirk-stub-kirk/ kirk-server/src/ docs/ proto/
(no output, exit 1)

$ grep -rn "ibis-allosaurus|git-kavara\.|secret-kirk-edge-v2\.git" \
            --include="*.rs" --include="*.toml" --include="*.md" \
            --include="*.yml" --include="*.yaml" --include="*.sh"
./kirk-server/src/backend.rs:208:            !msg.contains("secret-kirk-edge-v2.git"),
./kirk-server/src/backend.rs:212:            !msg.contains("ibis-allosaurus"),
./docs/SECURE_BUILD.md:15: ...
./docs/SECURE_BUILD.md:22: ...
./docs/SECURE_BUILD.md:39: ...
(only the sanctioned locations)
```

Integration tests (`cargo test --workspace --tests -- --test-threads=1`) were
not re-run by the reviewer — these are TIME_WAIT-sensitive and the coding
agent reports 67 passed; no source changed since.

## Production-readiness checklist

| Item | Status |
| ---- | ------ |
| All three transports use trait dispatcher | PASS (gRPC, REST v1, REST v2, TCP all go through `KirkBackend` → `Arc<dyn ModelBackend>`) |
| /v2 honors body limit + dim cap | PASS (`DefaultBodyLimit::max(64 MiB)` covers v2 routes at `routes.rs:78`; `request_dim` precheck before per-element scan in `routes_v2.rs:17-24`) |
| --env prod refusal message clean | PASS (no URL, no crate name, no host in `main.rs:24` or `factory.rs:58`; verified by negative-assertion unit test) |
| Default build does not pull secret crate | PASS (`secret-kirk-edge` absent from `Cargo.lock` and `cargo metadata`; guard script wired into CI in two jobs) |
| Docker default build still works | PASS (Dockerfile copies `kirk-stub-kirk`, `Cargo.lock`, no `--features` flag; build-agent fix verified) |
| Docs match code | **FAIL — RC-1, RC-2 above** |
| Tailnet URL only in SECURE_BUILD.md + test assertion | PASS |
| Algorithm structure not exposed | PASS (Rule 1 grep clean) |

## Recommendation

**REQUEST_CHANGES** — two doc-vs-code drifts in `docs/MODELS.md` (decision
matrix row + self-contradictory note) and `docs/SECURE_BUILD.md` (Step 1
"already present" claim) will mislead operators. Both are documentation-only
fixes; the implementation, security posture, build pipeline, and tests are
clean and ready to ship once the docs are corrected.

## Files Reviewed

- `kirk-stub-kirk/src/kirk.rs` — clean; matches spec
- `kirk-stub-kirk/src/lib.rs` — clean
- `kirk-server/src/main.rs` — `--env prod` guard correct
- `kirk-server/src/config.rs` — `--model` / `--env` clap enums correct
- `kirk-server/src/backend.rs` — `Arc<dyn ModelBackend>` wrapper + negative tests
- `kirk-server/src/model.rs` — trait surface clean (`async_trait`, `Send + Sync`, `Arc<Self>`)
- `kirk-server/src/backends/factory.rs` — dispatch matrix correct
- `kirk-server/src/backends/tiberius.rs` — lock+await analysis clean
- `kirk-server/src/backends/kirk_local.rs` — lock+await analysis clean
- `kirk-server/src/backends/kirk_prod.rs` — opaque error mapping; cfg-gated
- `kirk-server/src/rest/schema_v2.rs` — validation order correct (dim → row → finite → cast)
- `kirk-server/src/rest/routes_v2.rs` — body limit shared; precheck via `request_dim`
- `kirk-server/src/rest/routes.rs` — v2 mounted under same `DefaultBodyLimit`
- `kirk-server/src/grpc/service.rs` — calls `backend.clone().<method>(...).await` for all 7 ops
- `kirk-server/Cargo.toml` — feature stub `[]`, no git URL
- `docker/Dockerfile` — no `--features` flag, copies `kirk-stub-kirk` + `Cargo.lock`
- `.github/workflows/ci.yml` — two redundant feature-isolation gates
- `Makefile` — `check-secure-isolation`, `build-secure` targets present
- `scripts/check-default-build-clean.sh` — checks both `Cargo.lock` and `cargo metadata`
- `docs/REST.md` — accurate against `routes_v2.rs` / `schema_v2.rs`
- `docs/MODELS.md` — **drift RC-1**
- `docs/SECURE_BUILD.md` — **drift RC-2**

## Commendations

- The factory's `(env, model)` match arms are tidy, and the per-arm split
  between feature-on and feature-off `prod_kirk` is much cleaner than a
  sprinkled `#[cfg]` inside one function body.
- Negative-assertion unit test for the URL-leak surface
  (`factory_kirk_prod_without_feature_rejected`) is a model technique —
  small, fast, and gives the security agent a hard checkpoint.
- The coding agent's documented deviation §1 (removing the optional git dep
  declaration) actually strengthens isolation: the URL is now not even in
  `Cargo.lock`. The trade-off (operator must patch in the dep) is real but
  acceptable for a Tailnet-only crate.
- The build agent's catch of the missing `kirk-stub-kirk` + `Cargo.lock`
  copies in the Dockerfile prevented a CI-only Docker-build regression that
  would have been silent until the next image rebuild.
- v2 validation order — outer-dim check before any per-element allocation —
  prevents a NaN-flood DoS at the parse layer (S-003 envelope respected).
- Separating `check-secure-isolation` into its own parallel CI job in
  addition to the in-job step is genuine belt-and-suspenders.

## Recommendations for Next Agent

### If REQUEST_CHANGES triggers a remediation pass (preferred)

- **RC-1 fix** in `docs/MODELS.md`: change the `(tiberius, prod, off)` row
  to "TiberiusBackend / Allowed; --env prod only gates Kirk" and replace
  the contradictory note at line 113 with a single unambiguous sentence:
  "Only `(prod, kirk, feature off)` is rejected at startup. `(prod,
  tiberius)` is allowed and uses the local Tiberius kernel."
- **RC-2 fix** in `docs/SECURE_BUILD.md`: change Step 1 to instruct the
  operator to edit BOTH the `[dependencies]` AND the `[features]` line
  (`secret-kirk-edge = ["dep:secret-kirk-edge"]`), and drop the "already
  present" claim. Cross-reference the deviation note below it.

### Out of scope for the current loop but worth a follow-up issue

- Add `kirk-server/tests/rest_v2.rs` with the round-trip parity test
  (architect §"Testing strategy"). The handler is unit-test-clean but the
  end-to-end body-cap and validation envelope is not asserted from a real
  HTTP client.
- Add `check-secure-isolation` to the local `make ci` target (build-agent
  note already flagged this).
- Update `kirk-server/README.md` flags table and `CLAUDE.md` "New REST
  endpoint" guidance to reference `routes_v2.rs` (documentation-agent
  out-of-scope list).
