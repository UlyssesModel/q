# Documentation Agent Notes

## Summary

Created `/docs/REST.md` — 602 lines, 8 sections. Covers all nine REST endpoints, matrix encoding, error envelope, limits, configuration, and versioning.

## Endpoints Covered

| Endpoint | Description |
|----------|-------------|
| `GET /healthz` | Liveness check; 200 when ready, 503 during shutdown |
| `GET /metrics` | Prometheus text exposition (counters + latency histograms) |
| `POST /v1/forward` | Full stateful pipeline: eigh + density matrix + rolling-window z-score |
| `POST /v1/inference/entropy` | Stateless entropy scalar on a caller-supplied sample |
| `POST /v1/inference/features` | Stateless feature array / vector / scalar extraction |
| `POST /v1/active-inference` | Stateless combined features + entropy in one call |
| `POST /v1/active-inference/entropy` | Active-inference variant of entropy-only |
| `POST /v1/active-inference/features` | Active-inference variant of features-only |
| `POST /v1/forward-sample` | Random sample generation + inference (seed-controlled) |

## Drift Found and Resolved

### 1. Error envelope shape (HIGH)
- `kirk-server/README.md` documents the error response as `{"error": "...", "message": "...", "detail": {}}`.
- Actual `schema.rs` struct `ErrorResponse` has only two fields: `error: String` and `message: String`. No `detail` field exists.
- Resolution: documented the actual two-field shape. The README is wrong; REST.md is correct.

### 2. FeaturesResponse field names in existing README (HIGH)
- `kirk-server/README.md` shows `feature_arr_dim`, `feature_scalar_re`, `feature_scalar_im` as top-level fields.
- Actual `schema.rs` uses `matrix_dim` (not `feature_arr_dim`) and nests the scalar as `feature_scalar: {re, im}` (not flat top-level fields).
- Resolution: documented the actual struct field names.

### 3. SampleResponse vs FeaturesResponse naming inconsistency (MEDIUM)
- `SampleResponse` (used by `/v1/forward-sample`) uses `feature_array_re`, `feature_array_im`, `feature_vector_re`, `feature_vector_im` (full words).
- `FeaturesResponse` and `ActiveInferenceResponse` (used by all other feature endpoints) use `feature_arr_re`, `feature_arr_im`, `feature_vec_re`, `feature_vec_im` (abbreviated).
- This inconsistency is in the source; documented it explicitly with a note warning callers to match field names exactly.

### 4. Default bind address (LOW)
- The top-level `README.md` recommends `--bind 127.0.0.1` as if it were the default.
- `config.rs` shows `default_value = "0.0.0.0"`.
- Resolution: documented `0.0.0.0` as the default with a note to use `127.0.0.1` on non-Docker hosts.

### 5. No drift: all nine routes confirmed
- Every route in the documented endpoint list exists in `routes.rs::build_router`. No extra or missing routes.

### 6. No drift: error codes confirmed
- All six `code()` return values (`bad_request`, `payload_too_large`, `matrix_dim_exceeded`, `compute_error`, `internal`, `shutdown_in_progress`) confirmed against `error.rs`.

### 7. No drift: all CLI flags confirmed
- All flags and env vars documented in the configuration cheat-sheet confirmed against `config.rs`.

## Files Modified

- `/Users/charmalloc/dev/kavara/q/.claude/worktrees/rest-user-docs/docs/REST.md` — created (602 lines, 8 sections)

## Issues Found

- [HIGH] `kirk-server/README.md` error envelope example includes a `"detail": {}` field that does not exist in the actual `ErrorResponse` struct. Any integrator reading that README will expect a field that is never sent.
- [MEDIUM] `SampleResponse` field naming (`feature_array_*`, `feature_vector_*`) differs from `FeaturesResponse` (`feature_arr_*`, `feature_vec_*`). This is a source-level inconsistency that could cause client bugs if callers reuse the same deserialization struct for both responses.
- [LOW] `kirk-server/README.md` features response example shows flat `feature_scalar_re` / `feature_scalar_im` instead of the actual nested `feature_scalar: {re, im}` object.

## Recommendations for Next Agent

- Consider fixing `kirk-server/README.md` to match the actual error envelope (remove `"detail": {}`) and actual features response field names.
- Consider normalizing `SampleResponse` field names to match `FeaturesResponse` (`arr`/`vec` abbreviations) in a future breaking version.
- The Quick Start in `REST.md` uses the hand-computed N=2 fixture from `kirk-stub-realistic/tests/fixtures/handcalc_N2.json`. If that fixture is updated, the encoded base64 values in the Quick Start section should be regenerated.
