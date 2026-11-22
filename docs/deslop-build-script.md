# Deslop: src/build_script.rs

Component 10 of the deslop campaign (DSLP-14, DSLP-41, DSLP-68). Scope:
`src/build_script.rs`, 6,814 lines: about 2,425 of production code, then a
4,389-line `mod tests`. Its submodules under `src/build_script/` were measured
where a tool reports whole directories but are not edited here. Base `11cbdba8`,
branch `deslop/k3`, macOS arm64, `nightly-2026-11-09`.

Contracts touched by this file: the build-script run key (`action_key`, the
`${KACHE_*}` placeholder rewriting, `fold_dependency_inputs`), the recorded run
manifest and restore safety (`checked_relative`, `link_stays_inside`,
`restore_symlink`), and the launcher install. A change to a key, a manifest field
or the restore rules is a stop condition. None changed.

`RUSTC_WRAPPER=` was cleared for every command. Test runs of the root crate used
the system `TMPDIR`: a checkout-local `TMPDIR` fails 4 `cache_key` tests
(DSLP-88), which this file does not own.

## DSLP-14: Types

Commands:

```sh
cargo clippy --workspace --all-targets -- -D warnings            # baseline, exit 0
RUSTFLAGS="--cap-lints warn" CARGO_TARGET_DIR=.work/tmp/target-lint \
  cargo clippy --workspace [--all-targets] --message-format=json -- <15 candidate lints>
```

The candidate list and the bucketing are the ones in
[deslop-support-crates.md](deslop-support-crates.md). Counts for this file
(production = lib only; all = with tests):

| Candidate | Prod | All | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 0 | 42 | Adopted as a module attribute, see below. The 42 are tests setting environment variables and one `libc::geteuid` in `outside.rs` tests. |
| `unwrap_used` | 0 | 660 | Rejected: nothing to reject in production; every hit is a test assertion. |
| `expect_used` | 0 | 12 | Rejected for the same reason. |
| `wildcard_enum_match_arm` | 1 | 1 | Rejected: `link_stays_inside` returns `false` for every `Component` other than `ParentDir`, `Normal` and `CurDir`; failing closed is the point and `a_link_stays_inside_only_by_leading_climbs_within_its_depth` pins it. |
| `match_wildcard_for_single_variants`, `needless_pass_by_value`, `cast_sign_loss`, `cast_possible_wrap`, `cast_lossless`, `fn_params_excessive_bools`, `struct_excessive_bools`, `if_not_else` | 0 | 0 | Nothing to adopt. |
| `cast_possible_truncation` | 8 | 8 | Rejected: eight `elapsed().as_millis() as u64` in event timing, bounded by process lifetime. |
| `as_conversions` | 11 | 12 | Rejected: the same eight casts plus three `usize` to `u64` widenings (the two length prefixes in `fold` and one size comparison). |
| `indexing_slicing` | 21 | 57 | Rejected. The 21 are byte scanners (`replace_whole_paths`, `is_unmapped_absolute_path_token`, `end_of_url`) with computed bounds; none was shown to panic and `whole_paths_and_placeholder_paths`, `unmapped_absolute_path_detection` and `a_url_is_not_a_path` exercise them. Unaudited slices are not a witness. |
| `redundant_clone` | 1 | 7 | Rejected as a typing candidate; one is real (`options.clone()` at `run_cached`, `options` is not used afterwards) and is filed as a finding because dropping it is not a type, defect or dead-code change. |
| `unreachable_pub` | 7 | 7 | Rejected: `pub` items of a private module (`enabled`, `is_shim_invocation`, `run_shim`, ...) that other crate modules call; cosmetic. |

Escape hatches in the file (`git grep` at base and now): `#[allow(` 4 to 3,
`unsafe` words in tests unchanged, `.unwrap()` 660 and `.expect(` 13 unchanged
(all inside `mod tests`).

Changes, one commit each:

| Commit | Finding | Witness |
| --- | --- | --- |
| `fad14947` refactor(build-script) | `#[allow(clippy::too_many_arguments)]` on `Run::log` suppressed nothing: the root `Cargo.toml` sets `clippy::too_many_arguments = "allow"` for the whole crate. | `cargo clippy -p kache --lib --all-targets -- -D warnings` exit 0 without it. |
| `58a60c0f` build(build-script) | `#[cfg_attr(not(test), forbid(unsafe_code))]` on `mod build_script;` in `src/main.rs`; covers the file and `src/build_script/*`, whose production code has no `unsafe`. | Adding `fn injected() -> u8 { unsafe { std::mem::zeroed() } }` to `build_script.rs` failed `cargo check -p kache --lib` (exit 101, `usage of an unsafe block`); restored, `cmp` exit 0. |

## DSLP-41: Tests

Tiers and cost (system `TMPDIR`, warm): `cargo test -p kache --lib -- build_script::`
runs 153 tests (93 in this file, 60 in submodules) in 1.4s after a 15 to 110s
rebuild. Integration consumers that drive the shim through Cargo:
`tests/build_script_declared_inputs_test.rs`, `build_script_inputs_test.rs`,
`build_script_launcher_test.rs` (2 tests, 183s on this loaded host, they build
the launcher and run Cargo) and `out_dir_alias_test.rs`.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Restore never writes outside `OUT_DIR` | `src/build_script.rs` tests `a_link_stays_inside_only_by_leading_climbs_within_its_depth`, `out_dir_symlinks_that_leave_it_are_not_recorded_or_restored`, `a_link_under_a_restored_link_is_refused`, `a_launcher_without_kache_never_writes_through_a_linked_out_dir`, `rejected_restore_cleans_outputs_and_preserves_its_error_for_fallback` | Guarded both directions. |
| A run replays only for matching inputs | `a_links_dependency_is_keyed_by_its_text_with_the_roots_mapped`, `non_utf8_dependency_values_are_distinct_in_the_action_key`, `captured_dependency_paths_are_keyed_by_content_and_presence`, `declarations_follow_cargo_rerun_rules`, `script_execution_captures_tools_before_the_script_can_edit_them` | Guarded. |
| Paths relocate across checkouts, unmapped paths bind the run | `a_base_dir_keys_the_same_across_checkouts_only_in_remapping_flags`, `remap_flag_base_dirs_rewrites_only_path_remapping_sources`, `unmapped_absolute_paths_in_stdout_bind_run_to_manifest_dir`, `output_roots_round_trip_through_placeholders` | Guarded. |
| Manifest versions and legacy rows | `a_manifest_version_outside_the_known_range_is_refused`, `a_version_one_manifest_restores_without_outside_fields_or_a_target_directory`, `legacy_rerun_predictions_are_rejected_before_reuse` | Guarded. |
| Size caps and unrecorded runs | `an_out_dir_past_the_byte_cap_is_not_recorded`, `symlinks_count_toward_the_out_dir_entry_cap`, `an_unrecorded_run_is_a_refusal_that_says_why` | Guarded. |
| Launcher install and pin | `launcher_install_moves_a_shared_binary_without_writing_to_it`, `pinned_kache_*` (4), `the_launcher_passes_every_variable_name_on`, `tests/build_script_launcher_test.rs` | Guarded. |
| Second run after the script wrote into its own inputs (`run_cached`, the `InputsChanged` branch with `reruns_settle`) | `reruns_settle` and `mark_unsettled` alone are unit-tested (`src/build_script.rs:4008-4016`); the branch is not reached by any lib test | Gap, see below. |

Coverage (toolchain `llvm-profdata` and `llvm-cov` with `-C instrument-coverage`,
`cargo test -p kache --lib`, lines inside `#[cfg(test)]` items excluded; recipe
in [deslop-support-crates.md](deslop-support-crates.md)): production lines
1,337 of 1,441 covered (92.8%). The 104 uncovered lines sit in `run_cached`
(28: the disabled bail, `continue_after_restore_failure`, and lines 585 to 621,
the replay-then-settle-rerun path), `run_shim` (17), `log_unrecorded` (15),
`restore_outputs` (9, the empty-file restore loop) and small platform branches.
`tests/build_script_*.rs` were not instrumented, so some of these are likely
covered there; that was not measured.

Injected fault in a critical decision: `climbed <= depth` changed to
`climbed <= depth + 1` in `link_stays_inside` (a symlink may climb one level
above `OUT_DIR`). `cargo test -p kache --lib -- build_script::` exit 101:
`a_link_stays_inside_only_by_leading_climbs_within_its_depth` and
`out_dir_symlinks_that_leave_it_are_not_recorded_or_restored` FAILED, 151 passed;
restored, `cmp` exit 0. (That run used a checkout-local `TMPDIR`; the baseline
for this filter had no other failure.)

Needs environment: `cargo-mutants` and `cargo-llvm-cov` are not installed
(DSLP-87), so there is no mutant survivor list and the scoped lane is
unqualified; the uncovered-line list and the injected fault are the substitute.
Branch coverage was not enabled. No new test was added for the settle branch:
`run_cached` loads `Config`, opens the real store and spawns the preserved script,
so a faithful test is an integration test. It is filed below.

## DSLP-68: Dead code

| Tool | Result |
| --- | --- |
| rustc `dead_code` through `cargo clippy --workspace --all-targets -- -D warnings` | Clean at base and after. The only `allow(dead_code)` are three `cfg_attr(not(unix), allow(dead_code))` on the non-Unix shim stubs (lines 77, 79, 240), needed by the Windows build. |
| Whole-word search of every `pub` item in the file across `.rs`, `.md`, `.toml`, `.sh`, `.py`, `.yml` | Every name has at least one reference outside its definition; the smallest are `is_shim_invocation` (one caller in `src/main.rs`) and `enabled`. |
| `cargo` `unused_dependencies` lint over `cargo check --workspace --all-targets` | No warnings, negative control fired (see the support-crates record). |
| `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat` | `Needs environment`: not installed. |

Removed: nothing. No walk-backs, no allowlist.

## Findings for the parent

- P3, `src/build_script.rs:598-621`: the second-run settle path of `run_cached` has no lib test; recommend an integration test where a script rewrites a file it declared, asserting one rerun and an `unsettled:` marker on the second failure.
- P4, `src/build_script.rs:536`: `options.clone()` before `Store::open_with_options` is redundant (`clippy::redundant_clone`); drop the clone.
