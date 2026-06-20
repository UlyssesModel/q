# Secure Build

This page is the **only place in this repository** where the private upstream URL appears. It is intended for operators with Tailnet access who need to build the production Kirk variant. Do not copy the URL into any other file.

## Background

`kirk-server` supports a Cargo feature named `secret-kirk-edge`. When this feature is enabled at build time and the server is started with `--env prod --model kirk`, the server uses a production Kirk implementation sourced from a private crate hosted on the team's Tailnet.

Default builds — including all CI and Docker image builds — do NOT enable this feature. The feature is an empty stub (`secret-kirk-edge = []`) in the committed `Cargo.toml`. Building with `--features secret-kirk-edge` against the committed manifest alone will fail to compile because the private dep is not declared there. Operators must patch the dep in locally, as described below.

---

## Prerequisites

- Your build machine must be enrolled in the team Tailnet (access to `git-kavara.ibis-allosaurus.ts.net`).
- Rust toolchain (same version as the workspace `rust-version`).
- `cargo` must be able to reach the Tailnet host over HTTPS during the build.

Verify connectivity before starting:

```bash
curl -I https://git-kavara.ibis-allosaurus.ts.net/
# Expect: HTTP/2 200 (or a redirect; anything other than connection refused)
```

---

## Operator activation workflow

The committed `kirk-server/Cargo.toml` intentionally omits the git dep declaration for `secret-kirk-edge`. (See [Deviation note](#deviation-note) below for the reason.) To build with the feature, you must add the dep locally before running `cargo build`.

### Step 1: Edit `kirk-server/Cargo.toml` locally

Open `kirk-server/Cargo.toml` and add the following lines under the `[dependencies]` section:

```toml
[dependencies]
# ... existing deps ...
secret-kirk-edge = { git = "https://git-kavara.ibis-allosaurus.ts.net/kavara-ai/secret-kirk-edge-v2.git", optional = true }
```

Then update the `[features]` section to bind the feature to the optional dep:

```toml
[features]
default = []
secret-kirk-edge = ["dep:secret-kirk-edge"]
```

The committed manifest declares the feature as a placeholder with an empty deps list (`secret-kirk-edge = []`) per Deviation §1, so this edit is required for the feature flag to actually pull the optional dep at build time. Do not commit this change — see opsec rules below.

### Step 2: Build with the feature

```bash
cargo build --release -p kirk-server --features secret-kirk-edge
```

Cargo will fetch the private crate from the Tailnet URL during the build. The resulting binary in `target/release/kirk-server` has the feature compiled in.

### Step 3: Run with prod environment

```bash
./target/release/kirk-server --env prod --model kirk --bind 0.0.0.0
```

The server will log `backend selected: kirk` at startup and use the production variant.

---

## Verifying a default build is clean

Run the CI guard script before any release or after any `Cargo.toml` change:

```bash
bash scripts/check-default-build-clean.sh
```

Expected output:

```
default build is clean
```

The script checks two things:

1. `Cargo.lock` does not contain a package named `secret-kirk-edge`.
2. `cargo metadata --format-version 1` (default features) does not list the dep in the resolved graph.

If either check fails, the feature or URL has been accidentally committed. Fix the `Cargo.toml` before proceeding.

---

## Opsec rules

These rules are binding for all builds, CI steps, and Docker configurations:

1. **Never enable `--features secret-kirk-edge` in CI.** The GitHub Actions workflow (`.github/workflows/ci.yml`) must not include the feature flag in any `cargo build`, `cargo test`, or `cargo check` step. A CI assertion step runs `check-default-build-clean.sh` to verify this after every build.

2. **Never enable `--features secret-kirk-edge` in Docker images.** The `docker/Dockerfile` uses `cargo build --release -p kirk-server` with no `--features` argument. Do not add one. The Tailnet URL is not reachable from CI runners, so adding the flag would break Docker image builds in CI in addition to leaking the dep.

3. **Never commit `Cargo.toml` with the URL spliced in.** The local edit described in Step 1 above is a build-time-only change. Do not stage or commit `kirk-server/Cargo.toml` after adding the dep. Use `git checkout kirk-server/Cargo.toml` to revert after building.

4. **The runtime error is intentionally vague.** When a binary built without the feature is started with `--env prod --model kirk`, it exits with code 2 and prints:

   ```
   --env prod requires a build with the secure feature; see docs/SECURE_BUILD.md
   ```

   This message does not contain the URL, the crate name, or any Tailnet hostname. This is by design.

5. **Logs and metrics must not contain the URL or crate name.** The `tracing` log at startup emits only `backend selected: kirk` (or `tiberius`). `/metrics` output, `--version` strings, and error messages are all free of the private URL.

6. **Set `KIRK_ENV=prod` only in your orchestrator's prod profile.** If `KIRK_ENV=prod` is set in an environment where the secure binary is deployed by mistake, the server will use the production Kirk backend silently. Keep `KIRK_ENV` absent (or set to `local`) in dev and staging environments, even when the secure binary is deployed there.

---

## Deviation note

The original architecture spec (ADR-004, FR-013) called for declaring the optional git dep in `kirk-server/Cargo.toml` directly:

```toml
secret-kirk-edge = { git = "...", optional = true }
```

In practice, Cargo 1.93.0 resolves optional git dependencies eagerly during index resolution, which causes `cargo check` to fail on any machine without Tailnet access. To keep the default build universally green (CI, developer laptops, Docker), the dep declaration was removed from the committed manifest.

The security posture is actually stronger as a result: the URL does not appear in `Cargo.lock` at all by default, rather than appearing conditionally. The `scripts/check-default-build-clean.sh` script verifies this.

---

## Upgrading `secret-kirk-edge`

When the team releases a new version of the private crate:

1. Update the git URL or add a `?rev=<sha>` / `?tag=<tag>` specifier in your local `Cargo.toml` edit.
2. Run `cargo update -p secret-kirk-edge` to refresh the lock entry (only affects your local build environment).
3. Rebuild with `--features secret-kirk-edge`.
4. Test with `--env prod --model kirk` before deploying.
5. Do not commit any of these changes.
