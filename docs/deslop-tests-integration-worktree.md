# Deslop: tests `integration` through `worktree_depinfo` (component 26 of 28)

Tickets DSLP-30 (types), DSLP-57 (tests), DSLP-84 (dead code). Base `11cbdba8`, branch `deslop/k7`, macOS arm64,
`nightly-2026-11-09`. Scope: the 15 `tests/*.rs` files from `integration_test.rs` to `worktree_depinfo_test.rs`
(7,844 lines, 86 `#[test]` functions). All are tier-2 binaries driving the real `kache`, `rustc`, `cargo` and `cc`
in isolated cache directories. Five files (`macos_*`) are macOS-only, so this host cannot lint or run their Linux
counterparts.

## DSLP-30: Types

Same single clippy run as component 25 (`docs/deslop-tests-adaptive-diagnostic.md`), deduplicated over targets.

| Candidate | Hits in scope | Disposition |
| --- | ---: | --- |
| `unsafe_code` (forbid) | 5 | Reject. `geteuid` (1, now through `common::running_as_root`), the `pre_exec` `umask` closure (`integration_test.rs:1961`), and fixture source text. libc has no safe std equivalent and no dependency may be added. |
| `clippy::unwrap_used` | 710 | Reject: test code, a panic is the failure report. |
| `clippy::expect_used` | 81 | Reject, same reason. |
| `clippy::cast_sign_loss` | 3 | Reject. `integration_test.rs:3319,3494`, `size_before as u64` on a store size that is never negative. |
| `clippy::cast_possible_truncation` | 1 | Reject. `integration_test.rs:1963`, `mask as libc::mode_t` on a literal octal mask. |
| `wildcard_enum_match_arm`, `match_wildcard_for_single_variants`, `needless_pass_by_value`, `redundant_clone`, `unreachable_pub` | 0 | No witness. |
| `dead_code`, `unused_crate_dependencies` | 0 | Clean. |

Escape-hatch count (token matches in scope files): `allow` attributes 7 to 0, `unsafe` 8 to 5, `unwrap` 739 and
`expect` 92 unchanged. `rg` counts text, including fixture source strings, so its 739 `.unwrap()` exceeds
clippy's 710 compiled call sites.

Adopted fixes (all behavior-preserving; `cargo clippy -p kache --tests -- -D warnings` exits 0 after them):

- The six `#[allow(dead_code)] mod common;` sites in `macos_adaptive_debug_info_test.rs`,
  `macos_debug_bundle_policy_test.rs`, `macos_local_network_metadata_test.rs`, `macos_native_install_name_test.rs`,
  `msvc_passthrough_test.rs` and `unknown_compiler_probe_test.rs` went with `371a7ab7`, the module-level allowance
  moved into `tests/common/mod.rs`. Leaving them fails clippy with `duplicated_attributes`.
- `cbae9e80`: `integration_test.rs:1742` and `out_dir_alias_test.rs:320,610` replaced three bare `unsafe { libc::geteuid() }`
  with `common::running_as_root()`.
- `d393fd9c`: `#[allow(clippy::permissions_set_readonly_false)]` in `stale_artifact_test.rs` was redundant, because the
  root `Cargo.toml` already allows the lint.

## DSLP-57: Tests

Tier and cost from the one full run (4,876 passed, 0 failed, 38 ignored, 1,130 s, load 25 to 45): `out_dir_alias_test`
45 s (two tests), `integration_test` 28 s (51 run), `stale_artifact_test` 21 s, and every other binary here under 8 s.
No test was added or removed, so the 20% and 30 s budget holds.

| Contract | Guarding tests (file:line) | Disposition |
| --- | --- | --- |
| Wrapper invocation and exit codes | `integration_test.rs:593` `test_wrapper_hello_world`, `:165` `test_rust_restored_outputs_allow_build_without_wrapper`, `:564` `test_disabled_passthrough`, `:1503` `test_cc_passthrough_preserves_non_utf8_streams_and_exit_status` | Guarded |
| CLI surface | `integration_test.rs:501` `test_cli_version_matches_package_version`, `:520` `test_cli_help`, `:530,547` list and purge on an empty cache, `owned_help_test.rs:17` `built_help_identifies_the_maintainer_without_changing_other_output` | Guarded; fault control below |
| Cache key: a changed input must miss, a relocated checkout must hit | `stale_artifact_test.rs:131-345` (crate root, module, new module, feature, rustflags, profile, env, coverage), `integration_test.rs:772,815,884,941,2042`, `worktree_depinfo_test.rs:141`, `workspace_extra_inputs_test.rs:268`, `proc_macro_env_key_test.rs:197`, `out_dir_alias_test.rs:318,609` | Guarded |
| Store integrity and recovery | `stale_artifact_test.rs:378` corrupted artifact, `:442` concurrent builds, `:631` unrestorable entry, `integration_test.rs:257` corrupt entry recompiles, `:3226,3404` gc bounds | Guarded |
| C/C++ output files: symlinks, read-only, hardlinks, `/dev/null`, writable after hit, missing parent | `integration_test.rs:1293-1944` | Guarded; `:1739` skips as root |
| Metadata and diagnostics replay | `metadata_streaming_test.rs:50,123,221`, `integration_test.rs:55` | Guarded |
| Unknown or hostile compilers: wrapper fork bomb, 8 KB then hang, descendant on stdout; MSVC via cc-rs | `unknown_compiler_probe_test.rs:24,55,88`, `msvc_passthrough_test.rs:9` | Guarded |
| Build-script launcher links against the selected toolchain | `launcher_personality_test.rs:14` | Guarded |
| macOS artifacts: install names, debug map, split-debuginfo bundles, local-network privacy keys | `macos_native_install_name_test.rs:163,168`, `macos_adaptive_debug_info_test.rs:237`, `macos_debug_bundle_policy_test.rs:82,87`, `macos_local_network_metadata_test.rs:9` | Guarded on macOS only; Linux has no equivalent contract |

False passes: 19 of the repository's 28 `"skipping ..."` early returns sit in `integration_test.rs` (no `cc`, a
driver that rejects a probe flag, a host where probe-keyed flags are not cacheable, running as root at `:1743`) and 2
in `out_dir_alias_test.rs:321,611` (as root). They end the test green. Filed below with the same recommendation as
component 25.

Hand-injected fault (critical decision: the maintainer footer in the help output that fleet tooling reads). `src/cli_args.rs`
line 1114, `Maintained by Joshua Li:` changed to `Maintained by Joshua Lee:`.

```sh
cargo test --test owned_help_test      # exit 101
  test built_help_identifies_the_maintainer_without_changing_other_output ... FAILED
  panicked at tests/owned_help_test.rs:23:9: assertion failed: help.trim_end().ends_with(FOOTER)
cp .work/tmp/cli_args.rs.orig src/cli_args.rs; cmp .work/tmp/cli_args.rs.orig src/cli_args.rs   # cmp exit 0
git diff --exit-code src/cli_args.rs                                                            # exit 0
```

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed (`which` exits 1 for both), so there is no
coverage figure and no mutation score; `make mutants-diff` is the lane for it. No test was added, so no new boundary
test needs its own fault.

## DSLP-84: Dead code

Tools: rustc `dead_code` and `unused_crate_dependencies` through the clippy run (clean, 0 diagnostics); `cargo-machete`,
`cargo-udeps` and `cargo-bloat` are not installed (Needs environment). With the per-site allows gone, rustc now checks
each of these 15 files against the shared module and its own items: no diagnostic. Every private fixture helper in
the scope (`tests/` has no other fixtures here) is called. No removals in this component; the only dead code the
shared module hid (three fingerprint helpers) was removed with component 25, whose files own `tests/common`.

Platforms and features covered: macOS arm64, default features, `--tests`. Not covered: Linux-only `cfg` branches
(`engine_bootstrap`-style `/proc` checks live in component 25; here `unknown_compiler_probe_test.rs:15-21` reads
`/proc/{pid}/status` on Linux only) and `cfg(windows)` (Windows support was dropped, `d09be683`).

## Findings filed from this component

- P3: silent `"skipping"` returns (28 across `tests/`, 21 in this scope); see component 25.
- P3: `unknown_compiler_probe_test.rs:21` and `engine_bootstrap_test.rs:417,422` each carry a bare `unsafe { libc::kill }`
  with no `SAFETY` note, and the two files each implement their own pid-liveness probe. One `common::pid_alive` would
  serve both.
