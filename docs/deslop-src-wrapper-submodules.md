# Deslop: src/wrapper submodules (23/28)

Scope (DSLP-27, DSLP-54, DSLP-81): `src/wrapper/` only: `cc_diagnostics.rs`, `hit.rs`,
`rustc_hit.rs`, their test files and the shared `src/wrapper/tests.rs` (11,555 lines, of which 326
are production). `src/wrapper.rs` is the next component. Base `11cbdba8`, macOS arm64,
nightly-2026-11-09, `RUSTC_WRAPPER=` and `CARGO_BUILD_JOBS=4`. The busy host killed some lib-test
and strict-lint compiles; each result below is from a run that finished.

## DSLP-27 Types

Commands: `cargo clippy -p kache --lib --bins -- -D warnings` (exit 0) and the strict candidate
run from `docs/deslop-src-native-archive.md` (`cargo clippy -p kache --lib --message-format=json --
--cap-lints warn -W <candidates>`), production code only.

Production escape hatches in `src/wrapper/`: 4 before (`.unwrap()` 1, `.expect(` 2,
`#[allow(clippy::too_many_arguments)]` 1), 2 after. Test files hold 1,230 `unwrap`/`expect`; they
are test code and were not counted against the profile.

| Candidate | Before | After | Disposition |
| --- | --- | --- | --- |
| `unwrap_used` | 1 | 0 | Adopted: fixed. `cc_diagnostics::restore` ensured two ranges existed and then unwrapped one; a `let (Some(literal), Some(_)) = .. else { bail! }` fails with the same message (`2edc7570`). |
| `too_many_arguments` allow | 1 | 0 | Adopted: `rustc_hit.rs:25` repeated `too_many_arguments = "allow"` from `[lints.clippy]` in `Cargo.toml`; clippy `-D warnings` stays clean without it. |
| `expect_used` | 2 | 2 | Rejected: `deser_json::to_string` of a `String` or of a record of strings and offsets cannot fail; a fallback would hide a bug (`cc_diagnostics.rs:111`, `:158`). |
| `cast_possible_truncation`, `as_conversions` | 2, 2 | 2, 2 | Rejected: `elapsed().as_millis() as u64` (`hit.rs:28`, `rustc_hit.rs:49`); 584 million years of milliseconds. The idiom repeats 42 times in `wrapper.rs` (next component). |
| `indexing_slicing` | 1 | 1 | Rejected: `json_fragment` slices the quotes off a serialized string. |
| `unreachable_pub`, `unsafe_code`, `wildcard_enum_match_arm`, `redundant_clone` | 0 | 0 | Clean. |

Witness for the changed contract: `malformed_ranges_and_missing_roots_are_rejected_without_panics`
(`cc_diagnostics/tests.rs:148`). `cargo test -p kache --lib -- wrapper::cc_diagnostics
wrapper::tests::hit wrapper::tests::rustc_hit` passes (15 passed, 1 ignored fixture).

## DSLP-54 Tests

Tiers: unit tests beside each module (`cc_diagnostics/tests.rs` 11, `tests/hit.rs` 1,
`tests/rustc_hit.rs` 4 plus a subprocess fixture) and the 254 tests of `wrapper/tests.rs`, which
mainly exercise `wrapper.rs`; integration tests drive the real wrapper:
`tests/integration_test.rs`, `tests/cc_release_regression_test.rs`, `tests/cc_passthrough_args_test.rs`.
`wrapper/tests.rs` is one 10,613-line file; reorganizing it belongs to the next component.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Cached C/C++ diagnostics keep source and message text literal and re-root only locations | `cc_diagnostics/tests.rs:8`, `:33`, `:77`, `:99`, `:120`, `:185` | Asserted; round trip against exact strings. |
| Malformed ranges and missing roots fail, never panic | `cc_diagnostics/tests.rs:148`, `:203` | Asserted; fault below. |
| Stored diagnostics carry the format they were printed with | `cc_diagnostics/tests.rs:219`, `:133` | Asserted. |
| A hit event keeps phase timings, sizes and the object path | `wrapper/tests/hit.rs:5` | Asserted field by field. |
| A rustc hit is reported only after every artifact restores; predictions go to the key store | `wrapper/tests/rustc_hit.rs:182`, `:228`, `:298` | Asserted in a subprocess so the real streams are observed. |
| C and C++ restore, relink and partial publication | `wrapper/tests.rs:3387`, `:3832`, `:4210`; `tests/cc_release_regression_test.rs:50` | Asserted. |

Test hygiene fix (`5cff227b`): `completion_child` was a bare `#[ignore]` with a no-op
`let _ = mode.as_str();`. It now says why it is ignored and the no-op is gone; the parent test
still runs it (`rustc_hit.rs:190`).

Injected fault (critical decision, diagnostic range validation): in `src/wrapper/cc_diagnostics.rs`
`ensure!(location.end > location.start, ..)` changed to `>=`. `cargo test -p kache --lib --
wrapper::cc_diagnostics` exits 101: `wrapper::cc_diagnostics::tests::malformed_ranges_and_missing_roots_are_rejected_without_panics`
FAILED (10 passed, 1 failed). Restored from a pre-edit copy; `cmp` exit 0.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed, so there is no
coverage or survivor list for this scope. No new test was needed: no `none` boundary, and the one
changed contract already had its witness.

## DSLP-81 Dead code

Tools: rustc `dead_code` under `warnings = "deny"` (`cargo clippy -p kache --lib --bins -- -D
warnings` exit 0; `cargo test -p kache --lib --no-run` exit 0). Needs environment:
`cargo-machete`, `cargo-udeps`.

Workspace-wide caller search (`.work/tmp/pubscan.sh`, `rg -w NAME src tests crates ...` outside the
defining file): the eight `pub(super)` items in the three modules are all used outside their file
(`store_formatted`, `stored_format`, `restore` from `src/wrapper.rs:1759`, `:2447`, `:2466`;
`HitCompletion` at `wrapper.rs:1513`, `:1618`; `RustcHitContext` at `wrapper.rs:3369`).
`cc_diagnostics::store` is `#[cfg(test)]` and used by `wrapper/tests.rs:2613`. No `allow(dead_code)`.
No candidate is dead; no code removed; no `Needs owner` walk-back.
