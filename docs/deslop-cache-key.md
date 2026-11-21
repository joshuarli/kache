# Deslop: src/cache_key.rs

Component 12 of the deslop campaign (DSLP-16, DSLP-43, DSLP-70). Scope:
`src/cache_key.rs`, 11,498 lines, of which 6,064 are production lines that the
coverage tool counts and the rest is `#[cfg(test)]` items and the `mod tests;`
declaration. The test module (`src/cache_key/tests.rs`) and the Apple install-name
reader are component 11, [recorded separately](deslop-cache-key-submodules.md).
Base `11cbdba8`, branch `deslop/k3`, macOS arm64, `nightly-2026-11-09`.

This file is the cache-key recipe: a wrong key silently serves a stale artifact.
The lane's stop condition applies without exception: no key byte changed. The
commits below add a lint guard and tests; `git diff 11cbdba8 -- src/cache_key.rs`
is empty.

Test runs used the system `TMPDIR` (a checkout-local one fails 4 tests in this
module tree, DSLP-88) and `RUSTC_WRAPPER=`.

## DSLP-16: Types

Commands and candidate list as in [deslop-support-crates.md](deslop-support-crates.md).
Counts (production = lib only; all = with tests):

| Candidate | Prod | All | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 0 | 0 | Adopted as a module attribute (below); tests set environment variables from `tests.rs`, which is why it is `cfg_attr(not(test))`. |
| `unwrap_used` | 2 | 40 | Rejected. The 2 production hits are `deser_json::to_string(&(...)).unwrap()` in `shared_prediction_args` (a tuple of four strings, inside an iterator closure) and `prediction_sample_identity` (strings, options and a bool); serializing plain data cannot fail. The 38 others are test assertions. |
| `expect_used` | 0 | 2 | Rejected: test messages. |
| `wildcard_enum_match_arm` | 2 | 2 | Rejected. `rlib_bundle` returns `None` for every spec that is not a bundled archive, and the second is an identity arm (`result => result`) in `scan_env_dep_use`. Both default to the conservative case and are pinned by `tests.rs`. |
| `needless_pass_by_value` | 3 | 3 | Rejected: `fold_labeled(base: String, ..)`, `prediction_identity_in_env(.., vars: Vec<..>)` and `sdk_identity_for_invocation(root: Option<String>, ..)` take ownership because their callers have owned values; borrowing would add clones on the other side. |
| `redundant_clone` | 2 | 2 | Rejected as typing candidates, filed as findings: `path.clone()` building a `TreeRoot` in `ancestor_top_digest` and `resolved.clone()` before an early return in `memo_input_is_unchanged`. |
| `indexing_slicing` | 13 | 16 | Rejected: byte cursors in the Rust-source scanner (`concat!`, raw strings, char literals) and the case-folding prefix match in `key_env_var_matches`, which deliberately slices bytes rather than `&str` so a lossy replacement character cannot split a boundary (its comment says so). No witness. |
| `as_conversions` | 4 | 4 | Rejected: `usize` and `u64` widenings in length prefixes. |
| `unreachable_pub` | 25 | 29 | Rejected: `pub` items of a private module; cosmetic. |
| `cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap`, `cast_lossless`, `match_wildcard_for_single_variants`, `fn_params_excessive_bools`, `struct_excessive_bools`, `if_not_else` | 0 | 0 | Nothing to adopt. |

Three bool-heavy signatures were read and left: `rlib_bundle(spec, emits_rlib,
packs_all) -> Option<bool>`, `same_tree_guard(tree, workspace_unit, covered)`
and `linker_value_has_unmodeled_file(value, apple_target)`. Each is documented
at its definition and table-tested in both directions (`tests.rs:3502-3510` for
`same_tree_guard`); replacing the flags with enums would rewrite key code to
change no behavior, which this lane does not do.

Escape hatches, `git grep` at base and now: `.unwrap()` 41, `.expect(` 1,
`#[allow(` 0, `unsafe` 0, unchanged.

Change: the module-level `forbid(unsafe_code)` is commit `d511f37e
build(cache-key)` (shared with component 11): `#[cfg_attr(not(test),
forbid(unsafe_code))] mod cache_key;`. Witness: an `unsafe { std::mem::zeroed() }`
function added to `cache_key.rs` failed `cargo check -p kache --lib` (exit 101,
`usage of an unsafe block`), restored with `cmp` exit 0.

## DSLP-43: Tests

Tiers and cost: the same in-process tier as component 11 (`cargo test -p kache
--lib -- cache_key::`, 472 tests at base, about 47 to 56s, serialized by
`key_test_lock`), plus the integration consumers `tests/cache_key_underkeying_test.rs`,
`proc_macro_env_key_test.rs`, `input_prediction_test.rs` and
`macos_native_install_name_test.rs`, which run in the final `make test`.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Keys relocate across checkouts, run-time paths do not | `key_matrix_direct_remap_path_prefix_is_keyed_portably_and_in_order`, `key_matrix_out_dir_include_pattern_stays_stable_across_workspaces`, `key_matrix_out_dir_dual_pattern_diverges_across_workspaces`, `key_matrix_manifest_dir_runtime_env_path_changes_key_across_workspaces` | Guarded. |
| Every codegen-affecting argument is keyed | `test_cache_key_changes_with_features`, `sysroot_changes_key`, `target_spec_contents_change_key`, `nodefaultlib_exclusion_values_remain_in_the_cache_key`, `response_file_flags_share_inline_key_and_track_contents`, `rust_keys_reject_the_legacy_explicit_emit_recipe`, `residual_args_are_order_independent` | Guarded. |
| Linker input files are keyed (the old DSLP-89 gap) | `linker_file_option_spellings_change_key` (11 spellings), `linker_script_content_changes_key`, `unknown_linker_file_changes_key`, `tests/cache_key_underkeying_test.rs` `sectcreate_file_change_misses` | Guarded. DSLP-89 appears already fixed at this base by `8407bacf fix(cache_key): hash linker input files (#85)`; the "known gap" line in `AGENTS.md` is stale (finding below). |
| Unmodelable linker arguments refuse caching | none; the tests only passed accepted spellings | Gap, closed: `linker_arguments_that_cannot_be_modeled_fail_closed` (`b6857aa8`). |
| Workspace-guard rows keep a guard only while it covers the closure | `a_workspace_row_keeps_its_guard_only_while_the_guard_covers_the_closure`, `this_checkouts_rows_keep_the_guard_while_it_covers_the_closure` | Guarded; fault below. |
| Clippy driver identity, config and `CLIPPY_ARGS` key a clippy chain only | `clippy_identity_follows_version_config_and_lint_arguments` tested the helper; the fold inside `compute_key_into` had no test | Gap, closed: `clippy_lint_arguments_key_a_clippy_chain_and_no_other_compile` (`c6a064f7`). |
| A configured client is a different recipe, and keys do not name where the roots are | `path_normalizer.rs` tests of the normalizer only | Gap, closed: `configured_base_dirs_key_the_set_and_not_where_the_roots_are` (`f26f4077`). |
| Toolchain identity follows an update behind a shim | `rustc_version_follows_a_toolchain_update_behind_its_shim`, `toolchain_selector_fingerprint_tracks_every_selection_source` | Guarded. |
| Linux host CRT/libc identity | the Linux CRT probes and their memo, 144 uncovered lines on this host | `Needs environment`: Linux; CI's Linux cells run them. |
| Daemon-backed input hash prefetch | `FileHasher::prefetch`, 32 uncovered lines | `Needs environment`: needs a running daemon; covered by integration tests that were not instrumented. |

Coverage (toolchain `llvm-cov` with `-C instrument-coverage`, `cargo test -p kache
--lib`, lines inside `#[cfg(test)]` items removed; recipe in
[deslop-support-crates.md](deslop-support-crates.md)): production lines 5,559 of
6,064 covered (91.7%), 505 uncovered. By function, the largest groups:

| Function | Uncovered lines | Disposition |
| --- | ---: | --- |
| `compute_key_into` | 63 | 14 are the clippy fold and the base-dir fold, both now tested; 5 are the unreadable-target-spec warning (the key then simply lacks a `target_spec:` field, and rustc itself cannot compile against that file); the rest are error contexts. |
| `record_crt_placement_memo`, `crt_placement_memo_for`, `probe_files_with_hash`, `print_file_name_for`, `probe_linux_crt_objects_memoized_for`, `place_linux_crt_objects`, `driver_search_dirs_for`, `driver_library_search_dirs_for` | 144 | Linux driver probes; `Needs environment`. |
| `prefetch`, `cc_preprocess_fingerprints`, `hash_inner`, `header_hash`, `cc_preprocess_memo_recorded` | 78 | cc preprocess memo and daemon prefetch; exercised by the cc and integration suites, not by this unit tier. |
| `unix_link_arg_inputs` | 12 | The separated `-L` and `-l` forms and the three refusals; now tested. |
| `fold_native_link_inputs`, `fold_native_link_runtime_identity_for`, `from_words`, `query`, `xcrun` | 40 | Native-link and SDK probes that need a linker or `xcrun` result; partly `Needs environment`. |

Injected fault in a critical decision: `same_tree_guard`'s `if workspace_unit
&& !covered` changed to `||`. `cargo test -p kache --lib -- cache_key::` exit
101: `a_workspace_row_keeps_its_guard_only_while_the_guard_covers_the_closure` and
`this_checkouts_rows_keep_the_guard_while_it_covers_the_closure` FAILED (470
passed, 2 failed, system `TMPDIR`); restored, `cmp` exit 0. Faults for the new
tests, each against the one test, each restored with `cmp` exit 0:

| Fault in `cache_key.rs` | Failing test |
| --- | --- |
| `fold_field(.., b"clippy.v1:", ..)` replaced by `let _ = &identity;` | `clippy_lint_arguments_key_a_clippy_chain_and_no_other_compile` |
| `if args.is_clippy_chain()` inverted | the same test |
| the `-L=`/`--library-path` refusal replaced by `let _ = value;` | `linker_arguments_that_cannot_be_modeled_fail_closed` |
| the empty-library-name refusal disabled | the same test |
| `if configured_base_dirs != 0` made unreachable | `configured_base_dirs_key_the_set_and_not_where_the_roots_are` |

The three new tests add about 0.5s of test time under the serial lock.

Needs environment: `cargo-mutants` and `cargo-llvm-cov` are not installed
(DSLP-87), so there is no mutant survivor list and the scoped lane is
unqualified; the uncovered-line table and the injected faults are the substitute.
Branch coverage was not enabled. A mutation run of this file is large (a 11k-line
module under a 47s serial suite), and any survivor list has to be sharded before it
is a lane job.

## DSLP-70: Dead code

| Tool | Result |
| --- | --- |
| rustc `dead_code` through `cargo clippy --workspace --all-targets -- -D warnings` | Clean at base and after. Dozens of `#[cfg(test)]` items exist (for example `dep_info_from_emitted`, a test-only shim over `dep_info_from_emitted_for`); each is compiled only for tests and is used. |
| Whole-word search of every `pub` item in the file across `.rs`, `.md`, `.toml`, `.sh`, `.py`, `.yml` | No item with a single reference. Twelve have exactly one caller outside their definition: `cc_preprocess_memo_recorded` and `cc_probe_resolution` (`src/compiler/cc.rs`), `key_env_guard_in` (`src/wrapper.rs`), `dep_info_from_emitted` (`tests.rs`, the `cfg(test)` shim) and eight used inside the file. |
| `cargo` `unused_dependencies` lint over `cargo check --workspace --all-targets` | No warnings, negative control fired (see the support-crates record). |
| `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat` | `Needs environment`: not installed. |

Removed: nothing. No walk-backs, no allowlist.

## Findings for the parent

- P3, `AGENTS.md` Verification section: the "Known gap: linker input files named by `-C link-arg` (DSLP-89)" note is stale. `linker_file_option`, `unix_link_arg_inputs` and `linker_file_option_spellings_change_key` hash those files at this base. Re-run the DSLP-89 reproduction in `docs/deslop-diagnose.md` and close the ticket or correct the note.
- P4, `src/cache_key.rs`: `path.clone()` in `workspace_tree_digest` and `resolved.clone()` in the cc input lookup are redundant (`clippy::redundant_clone`).
- P3, `src/cache_key.rs`, Linux driver probes (`probe_linux_crt_objects_memoized_for`, `place_linux_crt_objects`, `print_file_name_for`) have 144 uncovered lines on a macOS host; confirm in a Linux cell's coverage run rather than assuming CI covers them.
