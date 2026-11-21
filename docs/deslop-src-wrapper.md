# Deslop: src/wrapper.rs (24/28)

Scope (DSLP-28, DSLP-55, DSLP-82): `src/wrapper.rs`, 8,214 lines, all production code; its tests
live in `src/wrapper/tests.rs` (the previous component's file list). Base `11cbdba8`, macOS arm64,
nightly-2026-11-09, `RUSTC_WRAPPER=` and `CARGO_BUILD_JOBS=2` to `4`. The busy host (load 50) killed
the `kache` lib-test and lint compiles repeatedly (SIGKILL); results below are from runs that
finished. The wrapper protocol (`crates/kache-protocol`, envelope and event formats) is untouched:
every change below is local to function bodies and attributes.

## DSLP-28 Types

Commands: `cargo clippy -p kache --lib --bins -- -D warnings` (exit 0 after each change) and the
strict candidate run from `docs/deslop-src-native-archive.md`, production code only.

Production escape hatches in `wrapper.rs`: 12 before (6 `#[allow(clippy::too_many_arguments)]`
and the 6 `unwrap`/`expect` of `#[cfg(test)]` hooks at `:479` to `:497`, which are test seams),
6 after. No production `unwrap`, `expect` or `unsafe`.

| Candidate | Before | After | Disposition |
| --- | --- | --- | --- |
| `#[allow(clippy::too_many_arguments)]` | 6 | 0 | Adopted: `Cargo.toml` `[lints.clippy]` already allows it for the package and `rustc` is invoked with `--allow=clippy::too_many_arguments`; the six attributes (`:6033`, `:6243`, `:6522`, `:7093`, `:7122`, `:7297` before the edit) were redundant. |
| `redundant_clone` | 11 | 1 | Adopted: 10 removed in `8d74aaa6`: eight `rebuilt.clone()` in skipped-event branches, `options.clone()` after its last use, `guard.clone()` for the prediction record. One rejected: `tree_guard.clone()` in `compile_before_key` (`:5547`) is a false positive; removing it fails with E0382 because `tree_guard` is passed on again after the `stop_on_hit` block. |
| `wildcard_enum_match_arm` | 3 | 2 | One fixed (`939d391e`): `reclaim_after_declined_handoff` lists `Acquired` and `Committed` instead of binding every other claim. Two rejected: the final `_ => None` of `cc_cache_entry_rejection_reason` (`:2013`) follows `_ if` guards, and the `restore_cc_from_cache` (`:2485`) target `_ =>` arm is documented as "anything else"; exhaustive lists would restate `ArtifactKind` and `CompileMode` for no decision. |
| `needless_pass_by_value` | 1 | 1 | Rejected: `adaptive_compile_before_key` (`:5406`) clones `tree_guard` inside a closure it calls more than once; borrowing would only move the clone. |
| `cast_possible_truncation`, `as_conversions` | 42, 45 | 42, 45 | Rejected: 42 of them are `elapsed().as_millis() as u64` and `Duration` conversions of values under 584 million years; the rest are display and counter widths. Replacing them needs one shared helper across `wrapper.rs`, `cli.rs`, `daemon.rs`, `build_script.rs` and others (outside this lane), recorded as a finding. |
| `unwrap_used`, `expect_used`, `unsafe_code`, `indexing_slicing`, `cast_sign_loss`, `unreachable_pub`, `match_wildcard_for_single_variants` | 0 | 0 | Clean. |

Witnesses: `reclaim_after_a_declined_hand_off_takes_a_free_key_at_once` (`wrapper/tests.rs:5962`,
`Acquired`) and `reclaim_after_a_declined_hand_off_gives_up_on_a_long_holder` (`:5942`,
`Contended`). `cargo test -p kache --lib -- wrapper::` passes: 267 passed, 2 ignored fixtures.

## DSLP-55 Tests

Tiers: `wrapper/tests.rs` (254 unit tests; `wrapper::` runs 267 with the submodules, 9 s) and
integration binaries that invoke the real wrapper: `tests/integration_test.rs`
(`test_wrapper_hello_world` `:593`, `test_rust_restored_outputs_allow_build_without_wrapper` `:165`),
`tests/stale_artifact_test.rs`, `tests/cache_key_underkeying_test.rs`,
`tests/cc_release_regression_test.rs`, `tests/cc_passthrough_args_test.rs`.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Do not store an entry when inputs changed during the compile | `wrapper/tests.rs:2229`, `:2254` | Asserted; fault below. |
| One build owner per key; a declined hand-off reclaims at once or gives up on a long holder | `wrapper/tests.rs:5942`, `:5962`, `:8464` | Asserted. |
| C and C++ restore: only requested artifacts, partial publication, race winner kept | `wrapper/tests.rs:3387`, `:3832`, `:4210` | Asserted. |
| Rust restore finds the real executable, rewrites output argv exactly | `wrapper/tests.rs:5041`, `:7780` | Asserted. |
| Unaudited bundled archive members and activation of new keys are rejected | `wrapper/tests.rs:6655`, `:7352` | Asserted. |
| Events record real outputs and phase accumulators; reused workers carry no state over | `wrapper/tests.rs:6314`, `:9383`, `:9692` | Asserted. |
| Only an adaptive seed takes a scheduler slot | `wrapper/tests.rs:10523` | Asserted. |
| Diagnostics replay once, empty streams stay silent | `wrapper/tests.rs:2512` | Asserted. |

Injected fault (critical decision, `should_skip_cache_store_for_input_race`): in
`src/wrapper.rs` `(modified_input_guard && key_inputs_changed)` changed to `||`.
`cargo test -p kache --lib -- wrapper::tests::input_race` exits 101:
`wrapper::tests::input_race_store_suppression_truth_table` FAILED. Restored from a pre-edit copy;
`cmp` exit 0.

Audit note: that truth table restates the production expression instead of listing the eight
outcomes. It catches operator faults but not a wrong specification. Replacing it with explicit
rows is a test-audit finding, not done here.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed, so there is no
coverage figure and no survivor list for the 8,214 lines; the mutation lane for `wrapper.rs` is
unqualified (`AGENTS.md` mentions `make mutants-diff` for changed lines only). No test was added:
no `none` boundary was found in the tiers above.

## DSLP-82 Dead code

Tools: rustc `dead_code` under `warnings = "deny"` (`cargo clippy -p kache --lib --bins -- -D
warnings` exit 0; `cargo test -p kache --lib --no-run` exit 0, so test code is clean). Needs
environment: `cargo-machete`, `cargo-udeps`; Linux-only code (none gated by `target_os` in this
file).

Workspace-wide caller search (`.work/tmp/pubscan.sh`: `rg -w NAME src tests crates launcher
scenarios scripts` outside `wrapper.rs`): of 46 `pub` items, 2 have no outside reference
(`warn_nonlocal_cache_fs_once`, called at `wrapper.rs:2615`; `complete_extra_inputs_dep_info`,
called at `:2726` and `:4033`); both are live. `#[cfg(test)]` hooks (13 attributes) are used by
`wrapper/tests.rs`. `#[cfg(not(unix))]` branches at `:2367` and `:4330` stay: Windows is documented
in `docs/how-it-works/architecture.mdx`.

No dead code found; no code removed; no `Needs owner` walk-back.
