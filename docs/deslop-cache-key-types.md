# Cache-key types audit

## Scope and result

DSLP-16 covers `src/cache_key.rs`, including its inline test modules: 11,498
lines at base `3fda624933be14ea8e6ab47134f3ea294cd57e35`.
`src/cache_key/tests.rs` and the cache-key submodules are related consumers and
test coverage, not additional files in this ticket's diagnostic counts.

No candidate meets the adoption bar. No source, lint configuration, cache-key
bytes, prediction schema, store format, CLI contract, or wrapper protocol changes.
No new finding warrants a ticket. `Next: 103` stays unchanged.

## Measurement

The host uses rustc `1.102.0-nightly (5b7b37aef 2026-11-08)` and
Clippy `0.1.101 (5b7b37aefc 2026-11-08)`. No tools were installed.

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn \
  -F unsafe_code \
  -W clippy::wildcard_enum_match_arm \
  -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation \
  -W clippy::cast_sign_loss \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value
```

The measurement exits 0. Counts below deduplicate primary spans in
`src/cache_key.rs` across targets. Warnings remain; this is not a claim that the
strict profile is clean. Inline test items are classified as tests even when
their `#[cfg(test)]` appears on the enclosing module or implementation.

| Candidate | Production | Tests | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 0 | 0 | No scoped violation or defect witness. |
| `wildcard_enum_match_arm` | 2 | 0 | Retain rejecting and value-preserving branches. |
| `match_wildcard_for_single_variants` | 0 | 0 | No scoped finding. |
| `cast_possible_truncation` | 0 | 0 | No scoped finding on this host. |
| `cast_sign_loss` | 0 | 0 | No scoped finding on this host. |
| `unwrap_used` | 2 | 38 | Retain fixed JSON shapes and test assertions. |
| `expect_used` | 0 | 2 | Retain test fixture assertions. |
| `redundant_clone` | 2 | 0 | Ownership cleanup without a defect witness. |
| `needless_pass_by_value` | 2 | 0 | Ownership-style suggestions without a defect witness. |

## Candidate dispositions and retained assumptions

- `shared_prediction_args` serializes a four-string tuple.
  `prediction_sample_identity` serializes a tuple containing strings, options,
  string collections, and a boolean. Both unwrap `deser_json::to_string`.
  The call is fallible in general, so the audit also checks the resolved
  `deser-json` and `deser-core` 0.10.2 implementations. These shapes emit JSON
  strings, nulls, booleans, and sequences. They have no map keys, float values,
  extension values, custom serializers, or recursive user-defined containers.
  String escaping and these primitive serializers do not return errors.
  No ordinary input reaches a serialization error with these concrete shapes.
  Replacing the unwraps with empty strings would fabricate prediction identities.
  Threading `Result` through callers without a reachable failure witness does
  not qualify. A serializer upgrade or shape change requires another review.
- `rlib_bundle` returns `Some` only for a bundled archive in a unit emitting
  an rlib. Its wildcard returns `None` for the remaining library forms and
  archive conditions. No incorrect archive classification was demonstrated.
- The non-UTF-8 source scan removes include-locator proof while retaining
  `SourceEnvDepUse` runtime vetoes. Its final wildcard returns the existing
  value rather than inventing a success state. No incorrect variant handling
  was demonstrated.
- The clones in `ancestor_top_digest` and `FileHasher::memo_input_is_unchanged`
  can be removed after their final uses. Neither bypasses a type check or hides
  an invalid state. Neither has a correctness or measured performance witness.
- `fold_labeled` takes ownership of the base key. `prediction_identity_in_env`
  accepts an owned environment snapshot. Borrowing suggestions change ownership
  style, not the represented contract. Both remain unchanged.
- The two test expects belong to `TreeGuard::stamping` and a rejected-toolchain
  fixture. Test unwraps assert setup, key computation, SQLite results, and
  filesystem operations. Replacing these with defaults would weaken the tests.
- `fold_field` writes `value.len()` as a little-endian `u64`. The conversion
  cannot narrow on supported 32-bit and 64-bit pointer widths. A hypothetical
  wider-pointer target was not tested and is not grounds for changing key bytes.

No lint is adopted solely because it currently has zero findings. No negative
compiler witness or mutation test is claimed: there is no adopted guard or
source change to exercise.

## Escape-hatch inventory

The text counter matches lines containing `unsafe`, `#[allow(`, `#![allow(`,
`.unwrap()`, or `.expect(`. It counts matching lines rather than method calls.
The test inventory includes inline test modules and `TreeGuard`'s test-only
implementation. `expect_err` is not an escape-hatch match.

| Scope | Before | After |
| --- | ---: | ---: |
| Production | 2 | 2 |
| Tests | 40 | 40 |
| Total | 42 | 42 |

Both production matches are the JSON serialization unwraps above. The source
contains no scoped unsafe block or allow attribute. Text counts differ from
Clippy's primary-span counts when nested calls share a diagnostic.

## Producer-to-consumer boundaries

| Boundary | Checked contract and disposition |
| --- | --- |
| `RustcArgs`, `KeyEnv`, `FileHasher`, and `PathNormalizer` to `compute_key_into` | Keying uses invocation snapshots, source identities, target and toolchain identities, environment inputs, dependency hashes, and native inputs. Missing or unreadable required inputs remain errors rather than empty hashes. |
| `compute_cache_key_with_outputs` to `src/compiler/rustc.rs::cache_key_in` | The function initializes `KeyOutputs` before calling `compute_key_into`, then returns `(Result<String>, KeyOutputs)`. The compiler maps only the successful key through `finish_key`. Partial outputs survive errors. |
| Compiler keying to `src/wrapper/rust_flow.rs` | The wrapper consumes the key result and outputs separately. Prediction identities, dep-info, tree guards, and observed inputs remain available across key failures and compilation. No Result or Option is collapsed into a fabricated key. |
| `resolve_key_inputs` to dependency discovery | Supplied dep-info, validated predictions, deferred discovery, and `dep_info_pre_pass_for` remain distinct. Prediction verification uses the authoritative pre-pass result after disagreement. `Result<Option<DepInfo>>` keeps errors separate from absent dep-info. |
| Prediction identities to `FileHasher` persistence | Local and portable prediction records retain their schema checks and fallible JSON decoding. Missing, disabled, ineligible, stale, and malformed records remain rejection or absence states, not invented dependency closures. |
| `TreeRoot` and `TreeGuard` to prediction reuse | Tree digests and post-key checks retain traversal bounds, exclusions, memo fingerprints, and write-time guards. Guard failure cannot be represented by a successful empty digest. |
| `SourceEnvDepUse` to source scanning and memo lookup | The enum preserves unused, locator, concatenation, and runtime uses. `from_memo_code` rejects unknown codes. String parsing and memo decoding do not cast arbitrary integers into enum values. |
| `fold_native_link_inputs` and native resolvers to key fields | Static and dynamic libraries, Apple SDK identity, install names, and linker placement remain hashed or rejected by existing branches. Unsupported inputs do not gain fallback identities. Related Apple parsing remains outside this ticket's source scope. |
| Key outputs to store and prediction writers | `src/wrapper/rust_flow.rs` retains the guards, optional identities, and prediction eligibility checks before publishing records. No persisted field, cache-key version, or wire shape changes. |

These are code-trace observations, not an exhaustive proof against under-keying.
Existing recorded limitations, including the linker-input gap in `AGENTS.md`,
are not reclassified as new typing findings.

## Verification and limits

The focused command uses the same CPU and environment prefix as the gate:

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache --all-features --lib cache_key::
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

The focused suite reports 475 passed, 0 failed, and 2 ignored. It covers normal
keys, absent inputs, rejected inputs, prediction sampling, tree guards, source
environment scanning, native libraries, and parser fixtures.

The required gate completes with `MAKE_CHECK_EXIT=0`. Linux x86_64 is the local
execution platform. macOS, Windows, 32-bit targets, real Apple SDK tools,
performance workloads, and full campaigns were not run. No user-facing surface
changes, so no visual check applies.
