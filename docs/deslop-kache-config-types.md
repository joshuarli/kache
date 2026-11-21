# Deslop: kache-config types

DSLP-3, component 1 of 28. Base: `da350481a58373829051df9b6eff35400e0ba9a9`.
Scope: `crates/kache-config`, 5,564 Rust lines, including unit tests and the
optional test-support API. Platform: Linux x86_64 (Ubuntu), default features
and all features (`file-write`, `test-support`). No source changes were needed.

## Strict-profile measurement

```sh
cargo clippy -p kache-config --all-targets --all-features --no-deps -- \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value -W unsafe_code
cargo clippy -p kache-config --lib --no-default-features --no-deps -- -F unsafe_code
```

The first command also ran without `--all-features`, using `-F unsafe_code`:
it rejected 21 unsafe blocks in the test target, as intended. A first attempt
without `--no-deps` stopped in kache-fs; dependency diagnostics are excluded.
The all-features warning-only measurement completed successfully. The final
production-only forbid probe completed successfully but provides no adoption
witness. No candidate was adopted: zero-hit settings do not meet the required
real-defect/escape-hatch-removal bar.

Counts below are distinct source findings, not repeated lib/lib-test diagnostics.
Test support is shown separately because its public helpers are also compiled
into a normal library when explicitly enabled.

| Candidate | Production | Test support | Unit tests (all features) | Disposition |
| --- | ---: | ---: | ---: | --- |
| `unsafe_code = forbid` | 0 | 2 | 19 | Reject: edition-2024 environment mutation/restoration requires unsafe; no safe std replacement preserving this test API. |
| `wildcard_enum_match_arm` | 0 | 0 | 0 | Reject: no defect or removed escape hatch. |
| `match_wildcard_for_single_variants` | 0 | 0 | 0 | Reject: no defect or removed escape hatch. |
| `cast_possible_truncation` | 0 | 0 | 0 | Reject: no defect or removed escape hatch. |
| `cast_sign_loss` | 0 | 0 | 0 | Reject: no defect or removed escape hatch. |
| `unwrap_used` | 0 | 0 | 262 | Reject: production is already clean; test assertions/setup intentionally fail on errors. Includes `unwrap_err`, unlike the text counter. Default-feature test count: 241. |
| `expect_used` | 0 | 0 | 3 | Reject: production is already clean; test setup and explicit error assertions. |
| `redundant_clone` | 0 | 0 | 3 | Reject for Types: unnecessary test copies, not a type defect or escape hatch. |
| `needless_pass_by_value` | 2 | 0 | 0 | Reject: ownership-style suggestions, not incorrect types; two false positives exceed one per 5k lines. |

The two pass-by-value suggestions are `Config::load_resolved_for`'s private
`Result<FileConfig>` parameter (`config.rs:529`) and the private
`resolve_config_path_from`'s `Option<PathBuf>` (`config.rs:1883`). Both accept
owned temporaries and borrow them internally; neither permits an invalid state,
introduces a clone at the call boundary, or suppresses a check. The three test
copy suggestions are `config/tests.rs:355` (`String::to_string`), `:850`
(`provenance.path.clone`) and `:1285` (`main.cache_dir.clone`). These are style
cleanup, not demonstrated runtime defects. They do not justify a strict setting
under the specified bar.

## Escape hatches and boundary audit

Command: `rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('
crates/kache-config`. Counts are matching lines, not occurrences, across all
source text regardless of enabled features.

| Scope | Before | After |
| --- | ---: | ---: |
| Production excluding test support and cfg(test) modules | 0 | 0 |
| Optional `src/testing.rs` helpers | 2 | 2 |
| Tests in `config/tests.rs`, `config.rs`, `invocation.rs` | 263 | 263 |
| Total | 265 | 265 |

Per-file counts: `config/tests.rs` 233, `config.rs` 11, `invocation.rs` 19,
`testing.rs` 2; all others zero. Categories: 240 unwrap lines, 3 expect lines,
22 unsafe-word lines, no allow attributes. One unsafe-word match is an assertion
message (`"unsafe labels fail open"`); there are 21 actual unsafe blocks.
Test assertions intentionally retain unwrap/expect, and environment test helpers
retain unsafe mutation/restoration. Removing the public test-support contract
would be a walk-back and was not attempted.

The audit traced raw `FileConfig` optional values through file/host layering,
environment precedence, `Config::load_resolved_for`, volume routing and the
conversion to `kache_core::store_config::Config`; it also read `InvocationEnv`
construction, validation, native-string access, resolution and child commands,
and CI policy. Missing versus empty remains meaningful for environment lists,
unset config fields, cwd/home and the optional cache location. The arbitrary
`workspace` value is deliberately preserved by the editor, not falsely modeled
as a validated compiler schema. `ConfigFileState` distinguishes absent, present
and unreadable snapshots; its small enum-to-byte casts are not truncations.
No demonstrated incorrect typing claim or removable production escape hatch
was found. No fixed contracts, compile-fail witnesses, runtime changes or new
out-of-scope findings result from this pass.

Contracts are **unaffected**: this diff is documentary and preserves every
public API, TOML representation, CLI behavior, cache-key input, store format and
wrapper protocol. The direct manifest consumers are the root kache package
(`file-write`, plus `test-support` for tests) and kache-client (default features),
both using workspace path dependencies. No performance gate is needed for a
documentation-only change. macOS-specific branches were read but not executed.

## Verification environment

- rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`;
  cargo `1.102.0-nightly (c07ac2c4d 2026-11-06)`;
  clippy `0.1.101 (5b7b37aefc 2026-11-08)`.
- GNU Make 4.3; Git 2.56.1.
- `todo` and `cpu-guard` are not installed in the loaded environment. Ticket
  closure follows the preceding deslop commits by removing the open entry.
  The full gate uses `taskset -c 0-3` as the available hard four-core cap on
  this eight-core host, with `CARGO_BUILD_JOBS=4`.
- Required gate: `env -u TMPDIR -u KACHE_CACHE_DIR make check` (wrapped in that
  CPU cap). It includes `make lint`, client-only dependency checking, kache-ipc
  tests, the resource-guard self-check and workspace library/integration tests,
  including kache-config, kache-client and root kache consumer tests.

The gate exited 0. Its output includes:

```text
test-resource guard: all checks passed
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.72s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
MAKE_CHECK_EXIT=0
```
