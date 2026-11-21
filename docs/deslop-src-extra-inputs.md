# Deslop: src/extra_inputs.rs

Component 18 of 28, base `11cbdba8`, branch `deslop/k5`. Scope: `src/extra_inputs.rs`
(4,789 lines at base, 4,699 now; 2,303 production and 2,396 test lines; 87 unit tests at
base, 86 now). Environment as in the component 17 document
(`docs/deslop-src-args-cargo-config.md`): macOS arm64, `nightly-2026-11-09`, a heavily
loaded host, `RUSTC_WRAPPER=` and `CARGO_BUILD_JOBS=3..4`, system `TMPDIR` for test runs.
During the Dead code edit the host killed `rustc` for this crate twice (`SIGTERM`, `SIGKILL`
on the lib test target); the third identical build passed.

## DSLP-22 Types

The lint run is the one in the component 17 document (one clippy pass over the crate,
bucketed per file; the lib test pass omits `redundant_clone`, which `SIGKILL`s it).

| Candidate | prod | test | Disposition |
| --- | --- | --- | --- |
| `unsafe_code` | 0 | 2 | Test only: `std::env::set_var` in the `EnvVarGuard` fixture. Nothing to forbid in production. |
| `unwrap_used` | 0 | 254 | Clean in production. |
| `expect_used` | 9 | 59 | Not adoptable: 8 of the 9 are invariants the type does not carry (below). One fixed. |
| `wildcard_enum_match_arm` | 2 | 0 | Rejected: `Component` matches in `parent_traversal_follows_glob` and `lexical_normalize` with a deliberate pass-through arm. |
| `as_conversions` | 1 | 0 | Rejected: `propagate_to_dependents as u8` (`bool` to `u8`, lossless). |
| `indexing_slicing` | 4 | 9 | Rejected: indices come from the same collection a few lines earlier (`workspace_config_body.extra_inputs[*index]`, `dependency_components[..common_len]` after `take_while(..).count()`). |
| `needless_pass_by_value` | 1 | 0 | Rejected: `resolve_declared_inputs(crate_dir: PathBuf, ...)`; a signature change with no defect behind it. |
| `redundant_clone`, `cast_possible_truncation`, `cast_sign_loss`, `match_wildcard_for_single_variants`, `unreachable_pub` | 0 | 0 | Clean. Everything in the file is `pub(crate)` or private. |

Escape hatches (`unsafe`, `#[allow(`, `.unwrap()`, `.expect(` anywhere in the file): 311 at
base, 310 now; 12 to 11 above `mod tests`.

Fixed: `load_workspace_packages` tested `manifest.lib.is_some()` and then called
`manifest.lib.as_ref().expect("checked above")`. It binds the option once with `if let`
(`9fe16deb`). Guard: the workspace package tests, for example
`src/extra_inputs.rs:3139 workspace_snapshot_is_relocation_stable_and_external_config_is_ignored`
and `tests/workspace_extra_inputs_test.rs:268`; all pass before and after.

Not fixed, filed as F1: `ExtraInputsSnapshot::digest` is `Option<String>` because an
explicit empty declaration has none, but a workspace rule is validated non-empty and
`resolve_declared_inputs(..).expect("validated workspace rule has at least one input")`,
`snapshot.digest.clone().expect("workspace rule relabel retains a digest")` and
`relabel_workspace_snapshot`'s `.expect("non-empty workspace rule has a digest")` re-assert
that fact three times. The same shape holds for `provider.package.crate_name` (two
`expect`s, "checked above") and for `combine_snapshots`, which keeps one declaration path in
`config_path` chosen as the first element of an `FxHashSet` and the rest, sorted, in
`additional_config_paths`; every reader takes the union (`merge_snapshot_into_dep_info`,
the test at `:3072`), so the split carries nothing. A typed split would be a
redesign of a 2,300-line cache-key module and was not attempted.

## DSLP-49 Tests

`cargo-llvm-cov` and `cargo-mutants` are not installed (DSLP-87), so there is no coverage
figure and no survivor list. Needs environment: coverage, mutation, the scoped lane.

Tiers and costs (loaded host): 85 unit tests, `extra_inputs::` filter, 8 threads, 1.24s;
integration `tests/extra_inputs_warm_target_test.rs` 5 tests 53.7s (real warm Cargo target,
rebuild after an input edit) and `tests/workspace_extra_inputs_test.rs` 1 test 18.2s; the
two binaries together 85.7s wall including the build. The key contract sits in the fast
unit tier; the two integration files prove that Cargo itself sees the invalidation.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| No `kache.toml`: key untouched | `src/extra_inputs.rs:2460 no_colocated_file_is_noop` | Asserted |
| A declared file's content change re-keys | `:2489 declared_input_change_rekeys`, `tests/extra_inputs_warm_target_test.rs:428` | Asserted |
| Pattern strings fold even with zero matches | `:2506 zero_match_still_folds_pattern_set` | Asserted |
| Path is bound to content, so swapped files re-key | `:4472 content_swap_between_matched_files_rekeys` | Asserted; fault below |
| Out-of-crate patterns fold, not reject | `:2556 out_of_crate_patterns_are_folded_not_rejected` | Asserted |
| Relocation is stable | `:2618 relocation_is_stable`, `:3139` | Asserted |
| Invalid or unsafe declarations fail closed | `:2415 strict_snapshot_rejects_invalid_glob_syntax`, `:2426 pattern_safety_helpers_cover_both_sides_of_each_boundary`, `:3227 workspace_rules_reject_glob_selectors_unresolved_packages_and_escaping_inputs` | Asserted |
| Strict and legacy error policies are complementary | `:2330` | Asserted |
| Provider digest reaches direct consumers' keys | `:2918`, `tests/workspace_extra_inputs_test.rs:268` | Asserted |
| Producer dep-info provenance is required | `:3399 artifact_provenance_rejects_incomplete_and_missing_producer_dep_info` | Asserted |
| Concurrent clients resolve in their own directory | `:2736` | Asserted |
| Dep-info rewrite escapes and fails closed | `:4273`, `:4315`, `:4414` | Asserted |
| A warm Cargo target notices an activation mid-build | `tests/extra_inputs_warm_target_test.rs:855` | Asserted |

Fault: the fold in `resolve_declared_inputs` loses the path,
`Ok(h) => readable.push(format!("{rel}={h}"))` becomes `format!("{h}")`:

```sh
cp src/extra_inputs.rs .work/tmp/extra_inputs.rs.orig   # edit, then
cargo test -p kache --lib -- extra_inputs::             # exit 101
#  extra_inputs::tests::content_swap_between_matched_files_rekeys FAILED
#  (85 passed, 1 failed; assertion at src/extra_inputs.rs:4575 at the time)
cp .work/tmp/extra_inputs.rs.orig src/extra_inputs.rs
cmp src/extra_inputs.rs .work/tmp/extra_inputs.rs.orig  # exit 0
```

No test was added or changed apart from the removal described under DSLP-76, so test cost
is unchanged.

## DSLP-76 Dead code

Tools: rustc `dead_code` and `unused` at the repository's `warnings = "deny"` (none in the
file on lib, bins or the test target); `unreachable_pub` (none: nothing is `pub`); an
outside-the-file caller search for every `pub(crate)` item (`rg -w` over `src/` and
`tests/`); a scripted listing of `#[cfg(test)]` items outside `mod tests`. Needs
environment: `cargo-machete` and `cargo-udeps` (not installed; the dependency set is
crate-wide, so the component 17 document carries the manual dependency pass),
`cargo-bloat`, and a Linux pass (no `target_os` gating in this file; the musl sysroot is
missing, see component 17).

Removed, as proven-dead private code (`2a43dc22`): `windows_pattern_has_ambiguous_root` and
`windows_path_uses_device_namespace`, two stubs that return `false` on every target, the
three `bail!` branches that called them (`resolve_snapshot` for the crate directory,
`resolve_declared_inputs` for each pattern), the test-only predicates
`windows_root_shape_is_ambiguous` and `windows_prefix_uses_device_namespace`, and their
assertions (the whole of `windows_only_path_rejections_stay_disabled_on_other_platforms`,
which asserted that the stubs return `false`, and two loops in
`pattern_safety_helpers_cover_both_sides_of_each_boundary`).

Evidence: `git show d09be683` ("drop Windows support") removed the Windows targets; the
shipped targets are the two musl targets and Apple silicon. A search for a Windows
`cfg` over `src/` and `crates/` finds one `cfg(windows)` in `crates/kache-store/src/link.rs`
and `cfg(not(unix))` fallbacks, none of which builds this file differently, and no code in
the file is gated on Windows. The branches' conditions were constant `false` on every
supported target, so no behavior changes. Consumer test run: `extra_inputs::` 85 passed;
`cargo check -p kache --lib --bins` clean. Platforms and features covered: macOS arm64,
default features (the file has no `cfg` on either).

| Item | Callers checked | Disposition |
| --- | --- | --- |
| `merge_into_dep_info` | 0 outside the file, 6 in tests; `#[cfg(test)]` | Kept: a test wrapper over `merge_into_dep_info_for` |
| `declared`, `resolve`, `crate_dir_from_source`, `normalize_pattern`, `resolve_artifact_provider` | `#[cfg(test)]` items; production uses `resolve_for_rustc`, `crate_dir_from_source_for`, `normalize_pattern_info`, `resolve_artifact_provider_for` | Kept: tests call them; filed as F2 |
| `apply_extra_inputs`, `merge_dep_info_content_for`, `resolve_for_rustc`, `merge_into_dep_info_for`, `parse_dep_info_dependencies` | 1 to 3 callers each in `wrapper.rs`, `compiler/cc.rs` | Live |

## New findings

- P3 `src/extra_inputs.rs`: `ExtraInputsSnapshot::digest` and `provider.package.crate_name` are `Option` while eight production `expect`s re-assert that they are `Some`; split the snapshot into a digest-bearing type and give `combine_snapshots` one declaration-path list instead of a primary plus an `FxHashSet`-chosen "first" path.
- P4 `src/extra_inputs.rs`: six `#[cfg(test)]` items outside `mod tests` (`declared`, `resolve`, `merge_into_dep_info`, `crate_dir_from_source`, `normalize_pattern`, `resolve_artifact_provider`) exist only so tests can call a shorter name than production; move them into `mod tests` or have tests call the production forms.
- P4 repository: `.gitignore` ignores `docs/**/*.md`, so each `docs/deslop-*.md` needs `git add -f`; add `!docs/deslop-*.md` once.
