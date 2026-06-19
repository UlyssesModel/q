# Documentation Agent Notes

## Summary

Audited all four prior agent notes (architect, coding, testing, security) and all existing READMEs in the worktree. Expanded each README to release quality and created the missing files (`CLAUDE.md`, `proto/README.md`, `docs/ARCHITECTURE.md`, `CHANGELOG.md`).

## Files Created / Edited

| File | Action | Notes |
|------|--------|-------|
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/README.md` | Rewritten | Added overview paragraph, Mermaid component diagram, prerequisites (protoc, Bun), full repo layout tree, usage (host + Docker), transport wire-format reference table, numerical parity section with tolerances, security posture section, development commands, per-component pointers. |
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/kirk-stub-realistic/README.md` | Rewritten | Added overview, six-stage pipeline table, full public API surface with type signatures, KirkOutput field table, five variant signatures, forward_sample, parity tolerance table, caveats section (2N block trick, no LAPACK, real-only on wire), test listing, fixture inventory. |
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/kirk-server/README.md` | Rewritten | Added full CLI flag table (matched to config.rs post-security), REST endpoint table with request/response JSON shapes, gRPC RPC table, complete TCP byte-level specification (verbatim from architect spec, verified against implementation), TCP connection lifecycle semantics (out-of-order completion documented), operational notes (graceful shutdown, healthcheck flag, tracing, Prometheus metrics), security posture section with all security finding dispositions. |
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/bench-ts/README.md` | Rewritten | Added Bun installation command, full `run` flag table with defaults, `compare` example output, output JSON schema (from src/results.ts + src/summary.ts), pipelining model table per transport, Docker Compose end-to-end example, MatrixPool and Float64Array notes. |
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/proto/README.md` | Created | Short (20 lines): single source of truth, consumers (tonic-build in build.rs, proto-loader in bench-ts), editing instructions. |
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/CLAUDE.md` | Created | Under 100 lines: codebase overview, key commands, where to put new transports/kernel changes/proto changes, coding conventions, Python reference read-only note, pre-commit checklist. |
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/docs/ARCHITECTURE.md` | Created | Full Mermaid diagrams (system, data-flow sequence per transport, concurrency model, kernel pipeline flowchart), all 7 ADRs from architect spec. |
| `/Users/charmalloc/dev/kavara/q/.claude/worktrees/kirk-stub-rs-multiproto/CHANGELOG.md` | Created | `## 0.1.0 - 2026-06-19 — Initial Release` with one bullet set per component. |

## Decisions

- **ARCHITECTURE.md in docs/**: placed in `docs/` rather than inline in the top-level README to avoid an 800-line README. The top README links to it and to per-crate READMEs.
- **TCP wire format verbatim in kirk-server/README.md**: copied the full byte layout from architect.md rather than summarizing; a client implementer should not need to read two documents.
- **Response payload sizes in TCP spec**: added explicit byte-count formulas (`4 + 8N²`, etc.) that are derivable from the spec but not spelled out there. These are validated against the codec implementation.
- **Security finding table in kirk-server/README.md**: summarized as dispositions (resolved / accepted / deferred) so an operator reading the README gets the security posture without reading the full security.md.
- **`make bench-tcp` as the Quick Start third command**: matches the actual Makefile target rather than the docker compose variable form, which is more typing.

## Drift Detected and Reconciled

| Item | Drift | Resolution |
|------|-------|------------|
| `--healthcheck` flag | Not present in original kirk-server/README.md; added by security remediation (SEC-009) | Added to CLI flag table and Operational Notes section |
| `docker-compose.yml` healthcheck | Original spec used `wget`; security agent switched to `--healthcheck` CLI flag | docs/ARCHITECTURE.md and bench-ts README both reference the correct `service_healthy` condition |
| `--max-matrix-dim` value_parser range | spec says "configurable up to 4096"; original README said `(clamped to [1, 4096] per spec S-001)` which was accurate | Verified against config.rs: `value_parser = clap::value_parser!(u32).range(1..=4096)`. Range is 1..=4096, documented correctly. |
| `--max-connections` / `--max-in-flight-per-conn` / `--tcp-write-timeout-ms` | Not in original kirk-server/README.md; added by security remediation | Added to CLI flag table with correct defaults and ranges from config.rs |
| TCP connection ordering | architect spec said "processes in order arrived" but implementation processes in parallel per-connection | Documented actual behavior (out-of-order completion, req_id correlation) in kirk-server/README.md under "TCP Connection Lifecycle" and TCP semantics. Matches SEC-012 disposition. |
| `bench-ts` `--op` flag allowed values | cli.ts usage string shows only `forward`; spec says `forward, inference_entropy, inference_features, active_inference, forward_sample` | Documented spec values in bench-ts/README.md. The runner dispatches on `op` but the cli.ts usage string is abbreviated. |

## Issues Found

- [INFO] No drift between config.rs and the new flag table; all flags verified one-to-one against the source.
- [INFO] The `bench-ts` `--op` flag accepts all five operations at runtime (the runner passes the op string through to the transport client), but the cli.ts usage string only mentions `forward`. This is not a documentation gap that we introduce — we document the full spec-specified set.
- [INFO] `FORWARD_SAMPLE` request payload comment in architect.md says `[u32 N][u64 seed]` (12 bytes, no per-N matrix data). The codec implementation matches this. Documented correctly.

## Recommendations for Next Agent

- NFR-002 throughput ordering (TCP >= gRPC >= REST at N=32, 100 users) has not been measured because Bun is not installed on the development host. Once Bun is available, run `make bench-all` and `make compare` to validate.
- The parity fixtures (`tests/fixtures/`) are pre-generated; if the kernel is modified, re-run `/tmp/gen_fixtures.py` against the Python reference to regenerate them.
- The `docs/ARCHITECTURE.md` Mermaid diagrams should be verified in GitHub's Mermaid renderer when the PR is opened.
