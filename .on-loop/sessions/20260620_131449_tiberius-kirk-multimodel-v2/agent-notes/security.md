# Security Audit — multimodel + /v2 + secure build delta

## Methodology

Read-only delta audit. Anchor points:

1. `agent-notes/coding.md` (claimed state, deviations).
2. `plan.md` (binding user clarifications C-1 algorithm-name hiding, C-2 per-(model, env) impls).
3. Two greps mandated by the dispatch brief (algorithm structure names, Tailnet URL leakage).
4. Source review: `kirk-stub-kirk/src/{lib,kirk}.rs`, `kirk-server/src/{model,backend,main,config}.rs`,
   `kirk-server/src/backends/{factory,tiberius,kirk_local,kirk_prod,mod}.rs`,
   `kirk-server/src/rest/{schema_v2,routes_v2,routes}.rs`, `kirk-server/src/lib.rs`,
   `kirk-server/Cargo.toml`, `Cargo.toml`, `Cargo.lock`,
   `.github/workflows/ci.yml`, `docker/Dockerfile`, `scripts/check-default-build-clean.sh`,
   `docs/{SECURE_BUILD,MODELS}.md`.
5. Build gates: `cargo audit --ignore RUSTSEC-2024-0436`, `cargo check --workspace`,
   `cargo clippy --workspace --all-targets`, `bash scripts/check-default-build-clean.sh`,
   `cargo metadata --format-version 1` walk for the gated dep.

Scope is the delta only; prior baseline (PR #1) is treated as already remediated.
Out-of-scope items per dispatch brief: auth, TLS, Kafka.

## Rule 1 verification: kirk algorithm structure

Verification grep (from worktree root):

```
$ grep -rn "construct_hidden_activations\|construct_hamiltonian\|construct_density_matrix\
\|construct_observeable\|calculate_features\|update_weights\|init_rho_hat\|ndarray_linalg" \
  kirk-stub-kirk/ kirk-server/src/ docs/ proto/ \
  --include="*.rs" --include="*.toml" --include="*.md" --include="*.proto"
$ echo $?
1
```

**ZERO hits.** The eight private prototype helper names and `ndarray_linalg` import are
absent from `kirk-stub-kirk/`, `kirk-server/src/`, `docs/`, and `proto/`.

Supplementary grep for `calculate_entropy` (allowed in tiberius per dispatch brief):

```
$ grep -rn "calculate_entropy" --include="*.rs" --include="*.toml" --include="*.md"
$ echo $?
1
```

ZERO hits anywhere (including in `kirk-stub-realistic`, the Tiberius kernel — Tiberius's
entropy code lives in `entropy.rs` and uses different function names). No finding.

Observation (INFO, not a finding): the public bon-builder fields preserved from the
pasted skeleton — `rho_t`, `hamiltonian`, `obserable`, `hidden_bool_inter`,
`hidden_bool_intra` — DO appear at `kirk-stub-kirk/src/kirk.rs:65-77` and in the shape
test at line 200-209. The plan (lines 65-76) explicitly accepts this: the bon-builder
skeleton (FR-003, ADR-002) is preserved verbatim so the secret-kirk-edge crate can plug
in without changing construction call sites. The five public method bodies (lines
122-180) contain only row/col mean / norm_sqr math — no `Eigh`, no entropy formula,
no `construct_*` calls, no `calculate_*` calls. The grep needles the user picked are
clean. The bodies are deterministic-given-input and shape-correct only, per spec.

**Status: PASS for Rule 1 (binding).**

## Tailnet URL leakage

Verification grep (from worktree root):

```
$ grep -rn "ibis-allosaurus\|git-kavara\.\|secret-kirk-edge-v2\.git" \
  --include="*.rs" --include="*.toml" --include="*.md" \
  --include="*.yml" --include="*.yaml" --include="*.sh"
./kirk-server/src/backend.rs:208:            !msg.contains("secret-kirk-edge-v2.git"),
./kirk-server/src/backend.rs:212:            !msg.contains("ibis-allosaurus"),
```

Both hits are inside `#[cfg(test)] mod tests { ... factory_kirk_prod_without_feature_rejected ... }`
— a `#[cfg(not(feature = "secret-kirk-edge"))]` unit test asserting the runtime error
message does **NOT** contain those strings (negative assertions). The strings live
inside `!msg.contains(...)` calls and are not emitted by the binary; they exist purely
to fail the test if a future change leaked them.

Per the dispatch brief: "the URL appears anywhere except a security-test assertion that
confirms it doesn't leak at runtime, flag as a finding." Both hits are exactly the
permitted exception.

Also verified:
- `kirk-server/Cargo.toml` — no `git = "https://..."` URL (deviation per coding notes
  Deviations §1; dep declaration was removed to keep default `cargo check` green).
- `docker/Dockerfile`, `.github/workflows/ci.yml`, `docs/SECURE_BUILD.md`, `docs/MODELS.md`
  — zero URL/host/path mentions.
- `scripts/check-default-build-clean.sh` — uses literal `secret-kirk-edge` as the
  needle to grep for in Cargo.lock; this is the gated dep's crate NAME, not the URL,
  and the dispatch brief's needles are URL substrings (`ibis-allosaurus`, `git-kavara.`,
  `secret-kirk-edge-v2.git`). The crate name leakage is intentional — the script must
  name what it's protecting against.

**Status: PASS for the Tailnet URL constraint (binding).**

## Feature flag isolation

| Check | Location | Result |
| ----- | -------- | ------ |
| Feature declared | `kirk-server/Cargo.toml:27` | `secret-kirk-edge = []` (empty array) |
| Optional git dep present | `kirk-server/Cargo.toml` `[dependencies]` block | NO (removed; coding deviation §1) |
| Default build resolves the dep | `cargo metadata --format-version 1` (default profile) | NO — only `bon`, `ndarray`, `ndarray-rand`, `getset` appear; `secret-kirk-edge` and `ndarray-linalg` absent |
| Cargo.lock contains the dep | `grep '^name = "secret-kirk-edge"' Cargo.lock` | NO (exit 1) |
| `kirk_prod.rs` cfg-gated | `kirk-server/src/backends/kirk_prod.rs` (whole module) | YES (declared via `#[cfg(feature = "secret-kirk-edge")] pub mod kirk_prod;` in `backends/mod.rs:8-9`) |
| `factory::prod_kirk` cfg-arms | `kirk-server/src/backends/factory.rs:47-60` | YES (split between feature-on and feature-off impls) |
| Dockerfile enables the feature | `docker/Dockerfile:22` | NO (`cargo build --release -p kirk-server` only; explicit comment at lines 20-21) |
| CI workflow enables the feature | `.github/workflows/ci.yml` | NO (only standard `cargo build/check/test`; no `--features` flag anywhere) |
| CI guard script | `scripts/check-default-build-clean.sh` | YES — checks `Cargo.lock` for `^name = "secret-kirk-edge"` AND walks `cargo metadata` packages. Ran locally: prints "default build is clean", exit 0 |
| Guard wired into CI | `.github/workflows/ci.yml:56-59` | YES (step "Assert default build is clean") |

Caveat (NOT a security finding, but a correctness concern surfaced for the next agent):
the coding agent's Deviation §1 removed the optional git dep declaration from
`kirk-server/Cargo.toml`. The feature is therefore an empty `[]` — activating it with
`--features secret-kirk-edge` will NOT pull `secret-kirk-edge` (no `dep:` binding), and
`kirk_prod.rs` (which `use`s `secret_kirk_edge::Kirk`) will fail to compile. This is a
documented build-time deviation, not a runtime security weakness — by design, default
builds (CI + Docker) cannot accidentally enable the secret path. The operator workflow
for activation lives in `docs/SECURE_BUILD.md` and currently says "see the team's
internal runbook." Forwarded to the doc agent.

**Status: PASS for isolation — the default build cannot resolve or compile the secret
dep. The deviation strengthens, rather than weakens, the isolation property.**

## Runtime `--env prod` refusal

Error message string verified in two locations (must match exactly):

- `kirk-server/src/main.rs:24`:
  `"--env prod requires a build with the secure feature; see docs/SECURE_BUILD.md"`
- `kirk-server/src/backends/factory.rs:58`:
  `"--env prod requires a build with the secure feature; see docs/SECURE_BUILD.md"`

Neither string contains:

- the crate name `secret-kirk-edge` (only the docs path is mentioned),
- the Tailnet host `git-kavara.ibis-allosaurus.ts.net`,
- the git URL,
- any algorithm names (`hamiltonian`, `density`, `eigh`, etc.).

The message names "the secure feature" (intentional, allowed per dispatch brief) and
points the operator at `docs/SECURE_BUILD.md`.

Refusal scope:

- `main.rs:20-28` short-circuits with `process::exit(2)` ONLY when `--env prod` AND
  `--model kirk` AND `cfg!(not(feature = "secret-kirk-edge"))`. This is intentional:
  per the binding clarification C-2 in `plan.md`, `(prod, tiberius)` is permitted
  without the feature (Tiberius has no remote variant).
- `factory.rs:55-60` returns `ServerError::BadRequest(...)` for `(Prod, Kirk)` without
  the feature, so even if `main.rs` is bypassed (library-only path via
  `start_server_with`), the listener fails to come up. Belt-and-suspenders.

Asserting test verified:

- `kirk-server/src/backend.rs:182-215` —
  `factory_kirk_prod_without_feature_rejected`, gated `#[cfg(not(feature =
  "secret-kirk-edge"))]`. It calls `KirkBackend::from_config(&cfg)` with `env=Prod,
  model=Kirk`, asserts:
  1. `from_config` returns `Err`,
  2. `err.to_string()` contains `"--env prod"`,
  3. `err.to_string()` does NOT contain `"secret-kirk-edge-v2.git"`,
  4. `err.to_string()` does NOT contain `"ibis-allosaurus"`.

Per coding notes, this test passes (`67 tests pass with --test-threads=1`). I did not
re-run the test suite from scratch but the code matches the claimed behavior.

**Status: PASS for runtime refusal and message hygiene.**

## /v2 REST attack surface

### Body limit

`kirk-server/src/rest/routes.rs:78` applies
`.layer(DefaultBodyLimit::max(REST_BODY_LIMIT_BYTES))` AFTER both the v1 and v2 route
registrations. `REST_BODY_LIMIT_BYTES` is defined at line 17 as `64 * 1024 * 1024`
(64 MiB). The axum layer covers the entire router, so v2 routes share the same cap.

This addresses S-006 (do not lower the cap, do not add a v2-specific cap) and prevents
unbounded JSON parses.

### Pre-flatten shape / dim validation

`kirk-server/src/rest/schema_v2.rs::parse_matrix_v2` (lines 80-121) validates **before**
constructing the flat `(Vec<f32>, Vec<f32>, usize)`:

1. **Dim lower bound** (`n < 2` → 400) — line 85-90.
2. **Dim upper bound vs `max_matrix_dim`** (line 91-96) — uses
   `ServerError::MatrixDimExceeded { actual, limit }`.
3. **Row length == matrix dim** (jagged-row rejection) — line 100-106, executed per row
   BEFORE pushing any inner element. Note: serde already enforces inner `[f64; 2]`
   length == 2 at deserialization (the type alias is `Vec<Vec<[f64; 2]>>`, and a
   `[f64; 2]` cannot deserialize from a longer or shorter JSON array). A row with
   length ≠ N is caught here.
4. **Per-element finiteness** — line 111-115, checks both `r` and `im_v` for
   `is_finite()` (rejects NaN/±Inf). The cast to f32 happens AFTER the check, on
   line 116-117.
5. **`request_dim` precheck** in `routes_v2.rs:17-24` calls
   `state.backend.check_dim(n)` on the OUTER `req.matrix.len()` before the heavy
   per-element loop runs. So an attacker submitting `N > max_matrix_dim` is rejected
   in O(1) (after JSON parse).

What is NOT pre-checked:

- **Decoded byte size**. The 64 MiB body cap is the only bound on raw JSON. For a
  compact JSON encoding (≈30 bytes per `[re, im]` pair for normal-magnitude doubles),
  64 MiB ≈ 2.2M elements ≈ N ≈ 1480. With `--max-matrix-dim 1024`, the dim check
  catches that first. The interaction is documented in spec NFR-002 / S-003 (architect)
  and is acceptable per dispatch brief.
- The `Vec::with_capacity(n * n)` allocation at line 97-98 uses an attacker-controlled
  `n` AFTER the dim check, so `n ≤ max_matrix_dim ≤ 4096`. Worst-case allocation per
  request: `4096 * 4096 * 4 bytes * 2` = 128 MiB per vec, 256 MiB combined. With the
  default `--max-matrix-dim 1024`, it drops to 8 MiB per vec, 16 MiB total — bounded.
  Operators who set `--max-matrix-dim 4096` should be aware. This is the SAME
  amplification factor as v1's `decode_f32_matrix` and matches PR #1's accepted risk
  envelope.

### f64 → f32 conversion (NaN/Inf safety)

`parse_matrix_v2` rejects NaN/Inf f64 *before* casting (lines 111-117). A finite f64
that exceeds f32's representable range becomes f32 `±Inf` after `as f32` — but the
upstream kernel's own validation handles non-finite Complex32 (per PR #1 baseline,
which already accepted this risk for v1). The tiberius kernel
(`kirk_stub_realistic::KirkRealistic::forward`) maps overflow to a `KirkError` it
already produces today; no new vector is introduced. The kirk backend's stub bodies
are pure row/col reductions and `norm_sqr` sums; non-finite input produces non-finite
output but does not panic or read out of bounds (ndarray operations are safe on
non-finite floats).

### Other route surface

- `forward_sample_v2` (routes_v2.rs:187-213): trusts only `matrix_dim: u32` and
  `seed: u64` from the client. `check_dim(req.matrix_dim)` is called BEFORE the
  expensive `forward_sample` call. No JSON-matrix attack vector.
- All seven v2 handlers route through the existing `KirkBackend` → `ModelBackend`
  surface, so the existing in-flight semaphore + shutdown flag both apply.

### Per-connection caps

Per the dispatch brief: the REST routes run through axum on top of hyper, so the TCP
listener's per-connection in-flight semaphore (`max_in_flight_per_conn`) does NOT
apply to REST. The relevant cap for v2 is the 64 MiB body limit + per-request CPU cost.
This matches v1's exposure model — no new resource-exhaustion vector is opened by v2.

**Status: PASS — validation order is correct, body cap is shared with v1, no new
attack surface beyond what S-003/S-006 (architect spec) already accepted.**

## Dispatch / concurrency

- `select_backend(cfg)` is invoked exactly once: at `kirk-server/src/lib.rs:148-149`,
  inside `start_server_with`. The returned `Arc<dyn ModelBackend>` is stored in
  `KirkBackend.inner` and never re-selected at request time. Verified with `grep -rn
  'select_backend\|from_config\|KirkBackend::new'` — no per-request callers.

- `Arc<dyn ModelBackend>` is shared across all three transports (gRPC handler, REST
  handlers, TCP handler) via `Arc::clone`. The trait is `pub trait ModelBackend: Send +
  Sync`. All concrete impls (`TiberiusBackend`, `KirkLocalBackend`, `KirkProdBackend`)
  auto-derive `Send + Sync` through their fields (`parking_lot::Mutex<T>` where T is
  the model handle, plus `AtomicBool`). No `unsafe impl` exists in the source —
  verified with `grep -rn "unsafe impl\|unsafe " --include="*.rs"`, zero hits.
  `#![forbid(unsafe_code)]` is on `lib.rs:5`, `main.rs:4`, and `kirk-stub-kirk/lib.rs:8`.

- Lock-across-await analysis:
  - `tiberius.rs`: `self.kirk.lock()` only inside `forward_sync` (sync method) and
    inside `spawn_blocking` closures. The async dispatcher methods either call
    `forward_sync` directly (no `.await` between lock and unlock) or hand the work to
    `spawn_blocking` (which runs sync on a separate thread; the `.await` on the
    `JoinHandle` is OUTSIDE the locked region). Safe.
  - `kirk_local.rs`: same pattern — `run_*` methods are sync, the trait impls either
    call them directly or via `spawn_blocking`. Inside the sync helpers (e.g. line
    94), the guard is dropped (`drop(guard);` at line 96 of `run_forward`) before
    encoding the output. Safe.
  - `kirk_prod.rs`: `me.handle.lock()` is inside the `spawn_blocking` closure (lines
    77, 97, 117, 137, 157, 177, 196); the outer `.await` operates on the join handle.
    Safe.

- Mutex choice is `parking_lot::Mutex` (sync-only), as required by `CLAUDE.md`. No
  `tokio::sync::Mutex` is used for the model handles.

**Status: PASS — single startup-time dispatch, no `unsafe`, no lock-across-await
violations.**

## cargo audit

```
$ cargo audit --ignore RUSTSEC-2024-0436
    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 1134 security advisories (from ~/.cargo/advisory-db)
    Updating crates.io index
    Scanning Cargo.lock for vulnerabilities (271 crate dependencies)
$ echo $?
0
```

Zero new advisories. Diff vs. PR #1 baseline:

- The added deps `bon @ 3.9.3`, `getset @ 0.1.7`, `ndarray-rand @ 0.15.0` have no
  open advisories.
- `ndarray @ 0.16.1` was already in the workspace via `kirk-stub-realistic`; the
  version is unchanged.
- `RUSTSEC-2024-0436` (paste, unmaintained) — pre-existing, still ignored per CI yaml
  line 136-137; transitively pulled by tonic/nalgebra; no exploitable surface.

Walk of `cargo metadata` confirmed:

```
$ cargo metadata --format-version 1 | python3 -c '...filter on ndarray-linalg, secret-kirk-edge...'
(no hits)
```

NFR-007 (no LAPACK backend) is satisfied: `ndarray-linalg` is absent from the resolved
graph.

**Status: PASS.**

## Build gates

| Gate | Command | Result |
| ---- | ------- | ------ |
| Compile (workspace) | `cargo check --workspace` | clean (exit 0) |
| Lints | `cargo clippy --workspace --all-targets` | clean (exit 0) |
| Default-build-clean script | `bash scripts/check-default-build-clean.sh` | "default build is clean" (exit 0) |
| Secret dep absent | `grep '^name = "secret-kirk-edge"' Cargo.lock` | no hits (exit 1) |
| Advisory scan | `cargo audit --ignore RUSTSEC-2024-0436` | no advisories (exit 0) |

(Note: an earlier attempt at `cargo clippy --workspace --all-targets -- -D warnings`
returned an `rustc: multiple input filenames provided` error — this was an artifact of
shell-level argument forwarding for the `-D warnings` suffix, not a real clippy
finding. Re-running without `-- -D warnings` confirms clippy is clean. The coding
notes also claim the `-D warnings` form passes locally for them.)

## Summary

| Severity | Count |
| -------- | ----- |
| CRITICAL | 0 |
| HIGH     | 0 |
| MEDIUM   | 0 |
| LOW      | 0 |
| INFO     | 2 |

## Findings (numbered)

### [INFO 1] bon-builder field names leak data-shape vocabulary

- **Location**: `kirk-stub-kirk/src/kirk.rs:62-77` (public fields), 200-209 (shape test).
- **Description**: The struct fields `rho_t`, `hamiltonian`, `obserable`,
  `hidden_bool_inter`, `hidden_bool_intra` are public on `Kirk`. They are
  algorithm-shaped vocabulary even though the five public methods do not use them.
- **Impact**: A reader can infer that the production variant likely operates on a
  density matrix `ρ`, a Hamiltonian `H`, and an observable — i.e. some quantum / open-
  systems flavour of algorithm. The eight private STAGE names (`construct_hamiltonian`,
  `calculate_entropy`, …) — which the user explicitly enumerated as Rule 1 — are gone.
  The field names are a weaker, structural-only hint.
- **Why this is INFO, not a finding**: The plan (lines 65-76) and architect spec
  (FR-003, ADR-002) explicitly require preserving the bon-builder skeleton verbatim so
  the secret-kirk-edge crate can plug in without changing the construction call site.
  This is a documented, deliberate trade-off the user signed off on. The user's
  specific Rule 1 needles (`construct_*`, `calculate_*`, `update_weights`,
  `init_rho_hat`, `ndarray_linalg`) all return zero hits.
- **Remediation**: None required. If the project later decides to obscure the field
  names too, an alias like `pub buf_a: Array2<Complex64>` + a private internal name
  could be considered, but this would diverge from the secret crate's expected
  builder shape and break Phase D of the plan.
- **Reference**: CWE-200 (Information Exposure) — not exploitable.

### [INFO 2] `secret-kirk-edge` feature is empty (build-deviation note)

- **Location**: `kirk-server/Cargo.toml:27` — `secret-kirk-edge = []`.
- **Description**: Coding agent Deviation §1: the optional git-dep declaration
  (`secret-kirk-edge = { git = "...", optional = true }`) was REMOVED from the manifest
  to keep `cargo check` green on hosts without Tailnet access. The feature still
  exists as a cfg gate but compiling with `--features secret-kirk-edge` will fail to
  resolve `secret_kirk_edge::Kirk` in `kirk_prod.rs:22, 29` — operators must patch
  the dep in locally.
- **Security impact**: NONE — strictly strengthens the default-build isolation
  (the URL is now not even in the manifest). Functional deviation is for the doc agent
  to document, not a security concern.
- **Remediation**: Doc agent should expand `docs/SECURE_BUILD.md` with the exact
  `[patch.crates-io]` snippet or `Cargo.toml.local` workflow.
- **Reference**: Coding-agent notes Deviations §1.

## Compliance Notes

- **SOC2 (audit logging)**: `kirk-server/src/lib.rs:150` logs
  `tracing::info!(backend = backend.name(), "backend selected")` at startup. The label
  is the opaque short string `"tiberius"` or `"kirk"` — does not reveal the crate
  source or env. Acceptable.
- **GDPR / PII**: No PII or matrix-contents logging anywhere (verified per CLAUDE.md
  rule and PR #1 baseline).
- **NIST 800-53 SC-12 (cryptographic key management)**: Not applicable to this delta.

## Dependency Audit

- `bon @ 3.9.3` — no advisories.
- `getset @ 0.1.7` — no advisories.
- `ndarray @ 0.16.1` — pre-existing version, no advisories.
- `ndarray-rand @ 0.15.0` — no advisories.
- `secret-kirk-edge` — NOT in `Cargo.lock`, NOT in `cargo metadata`, NOT referenced in
  any committed file as a git URL.
- `ndarray-linalg` — NOT in `Cargo.lock`, NFR-007 satisfied.
- `paste` @ pre-existing transitive (RUSTSEC-2024-0436) — ignored, tracked.

## Decisions

- The two negative-assertion test strings (`secret-kirk-edge-v2.git`,
  `ibis-allosaurus`) inside `#[cfg(test)] mod tests` of `backend.rs` are treated as
  the user-permitted exception ("a security-test assertion that confirms it doesn't
  leak at runtime") per dispatch brief. They strengthen, not weaken, the URL-hygiene
  posture.
- The empty `secret-kirk-edge = []` feature stub plus the cfg-gated `kirk_prod.rs`
  module is treated as a deliberate isolation hardening, not a deviation finding for
  this audit.
- No CRITICAL or HIGH findings; the two INFO items are documentation / next-agent
  hand-offs, not gates.

## Recommendations for Next Agent

### Documentation agent
- Expand `docs/SECURE_BUILD.md` with the exact operator patch workflow (per coding
  Deviation §1). Until then, the secure build path is undefined for any operator
  not on the team. Include explicit guidance NOT to commit the URL.
- Expand `docs/MODELS.md` with the field-name disclosure note: the public
  `rho_t / hamiltonian / obserable / hidden_bool_*` fields exist on the stub for
  builder-shape parity and contain no algorithm logic.

### Build / review agent
- Add a CI step that runs the binary with `--env prod --model kirk` and asserts
  `exit 2` plus the exact error message text (the unit test
  `factory_kirk_prod_without_feature_rejected` covers the library path; an end-to-end
  exec test would cover the `main.rs:24` print path too).
- Confirm `/metrics` and `tracing::info!` log lines never substring-match
  `secret-kirk-edge`, `ibis-allosaurus`, or `git-kavara` at runtime (a quick `cargo
  run -- ... 2>&1 | grep -E '<needles>'` check, expected: no hits).
- The next time `cargo update` runs, re-execute `cargo audit --ignore RUSTSEC-2024-0436`
  and update CI's ignore list if a new advisory lands on `bon / getset / ndarray-rand`.

## Verdict

**PASS.** Zero CRITICAL, zero HIGH, zero MEDIUM, zero LOW findings. Both binding
user rules (Rule 1 algorithm-structure hiding; Rule 2 Tailnet-URL leakage) are
upheld with the documented test-assertion exception. The feature flag isolation,
runtime `--env prod` refusal, error-message hygiene, /v2 REST attack surface,
dispatch concurrency model, and dependency audit are all clean.

Recommendation: PROCEED to documentation phase.
