# Documentation Agent Notes

## Summary

Expanded three stub documentation files to full production-quality references. No source code was modified. All field names were verified against the actual Rust source before being written into documentation.

## Decisions

- **v1 vs v2 section placement**: added before the v1 Endpoint Reference (retitled from "Endpoint Reference" to "v1 Endpoint Reference") so the explanation of why each version exists appears before the reader sees either set of endpoints.
- **Concrete size numbers**: used the numbers from the architect spec (1024×1024 f32 = 4 MiB raw; v1 ~11 MiB; v2 ~25-30 MiB) verbatim; also derived the v2 N=300 practical ceiling from the spec (64 MiB / ~30 bytes per `[re, im]` pair).
- **v2 field name choice**: the spec and the architect notes both contain two different names for the v2 forward-sample response fields. Verified against the actual struct in `schema_v2.rs`: `feature_array`, `feature_vector`, `feature_scalar`, `relative_entropy` (full words, matching `SampleV2Response`). The inference/features/active-inference responses use the abbreviated forms `feature_arr`, `feature_vec`, `feature_scalar` (matching `FeaturesV2Response` and `ActiveInferenceV2Response`).
- **Kirk algorithm internals**: MODELS.md deliberately says nothing about the algorithm's internal stages, pipeline steps, or mathematical structure beyond "shape-correct stub." Only the tiberius algorithm's published mathematical basis (Gibbs softmax, Shannon entropy, eigenspectrum) is described.
- **SECURE_BUILD.md URL placement**: the Tailnet URL `https://git-kavara.ibis-allosaurus.ts.net/kavara-ai/secret-kirk-edge-v2.git` appears only in SECURE_BUILD.md, in the Step 1 Cargo.toml snippet and nowhere else in the repo.
- **Decision matrix nuance**: the `(tiberius, prod, feature off)` row needed care. The `main.rs` guard actually rejects only `(prod, kirk)` without the feature; `(prod, tiberius)` is allowed. The matrix reflects the actual code in `factory.rs` and `main.rs`.

## Files Modified

- `docs/REST.md` — rewrote intro to mention both versions; added "API Versions: v1 vs v2" section with comparison table, size numbers, and when-to-use guidance; retitled "Endpoint Reference" to "v1 Endpoint Reference"; added full "v2 Endpoint Reference" section covering all 7 v2 routes; updated Configuration Cheat-Sheet to include `--model` and `--env`; updated Versioning section; added v2 practical ceiling to Limits.
- `docs/MODELS.md` — replaced placeholder stub with full coverage of Tiberius (numerical basis, stateful behavior, response field semantics) and Kirk (stub characteristics, what the builder fields are for, production variant pointer); decision matrix; numerical differences table; configuration examples.
- `docs/SECURE_BUILD.md` — replaced placeholder stub with operator-facing build workflow (Tailnet check, local Cargo.toml edit, build command, run command); clean-build verification; five binding opsec rules; deviation note explaining why the dep declaration was removed from the committed manifest; upgrade path.

## Drift Caught

The architect spec's `SampleV2Request` struct in the REST /v2 API design section describes a `SampleV2Request` with only a `matrix` field, used for all five inference/active-inference endpoints. The actual `routes_v2.rs` confirms this: `inference_entropy_v2`, `inference_features_v2`, `active_inference_v2`, `active_inference_entropy_v2`, and `active_inference_features_v2` all deserialize `Json<SampleV2Request>` with a single `matrix: ComplexMatrixJson` field. The doc was written to match the source.

No field name mismatches were found between the architect spec and the implemented code for the response types. The spec listed `feature_arr`/`feature_vec` for features responses and `feature_array`/`feature_vector` for the sample response; the source `schema_v2.rs` confirms both spellings exactly.

## Issues Found

- [INFO] The `(tiberius, prod, feature off)` combination: the `main.rs` guard (line 20-28) only triggers on `(prod, kirk)` without the feature. `(tiberius, prod)` is allowed at the main.rs level but the factory's `(Prod, Tiberius)` arm in `factory.rs` returns `TiberiusBackend` (allowed). The MODELS.md decision matrix initially drafted from the spec text said `(tiberius, prod, feature off)` is rejected — this is wrong. Fixed in the final MODELS.md to correctly reflect that `(tiberius, prod)` is allowed and uses the local TiberiusBackend.

## Recommendations for Next Agent

- The `kirk-server/README.md` still needs the `--model` and `--env` rows added to its flags table, and the v2 endpoint table referenced in the architect spec's documentation impact section. This was not in scope for this session.
- `CHANGELOG.md` entry under "Unreleased" for v2 routes, multi-model selector, and env guard is also in the architect's documentation impact list and was not written here.
- The `CLAUDE.md` (project-level) still references only v1 REST routes in its "New REST endpoint" guidance. Updating it to mention `routes_v2.rs` / `schema_v2.rs` would help future contributors.
