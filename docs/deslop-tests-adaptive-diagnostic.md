# Deslop: tests `adaptive_incremental` through `diagnostic_width` (component 25 of 28)

Tickets DSLP-29 (types), DSLP-56 (tests), DSLP-83 (dead code). Base `11cbdba8`, branch `deslop/k7`, macOS arm64,
`rustc`/`clippy` `nightly-2026-11-09` (`rust-toolchain.toml`). Scope: the 24 `tests/*.rs` files from
`adaptive_incremental_test.rs` to `input_prediction_test.rs`, `tests/common/mod.rs`, and `tests/fixtures/*` (17,255
Rust lines, 227 fixture lines, 285 `#[test]` functions, one `#[ignore]` subprocess fixture). Every one of these is a
tier-2 integration binary: it spawns the real `kache` (and real `cargo`, `rustc`, `cc`) into an isolated cache
directory.

## DSLP-29: Types

Candidate run, one invocation, deduplicated over lib, bin and test targets by lint, file, line and column. The first
two attempts died to `SIGTERM` from outside (other lanes on the host); the third finished.

```sh
CARGO_BUILD_RUSTC_WRAPPER= cargo clippy -p kache --all-targets --message-format=json -- --cap-lints warn \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::redundant_clone -W clippy::needless_pass_by_value -W unreachable_pub -W unsafe_code \
  -W dead_code -W unused_crate_dependencies          # attempt 3: exit 0
```

| Candidate | Hits in scope | Disposition |
| --- | ---: | --- |
| `unsafe_code` (forbid) | 16 | Reject. All are libc calls with no safe std equivalent and no new dependency allowed: `openpty`, `fcntl`, `from_raw_fd`, `setsid`/`ioctl` in `pre_exec`, `kill`, `umask`, `geteuid`, and `set_var`/`remove_var` (edition 2024) under `ENV_LOCK`. Every block carries a `SAFETY` note except the two `kill(pid, 0)` probes in `engine_bootstrap_test.rs`. |
| `clippy::unwrap_used` | 1,380 | Reject. Test code: a panic is the failure report. |
| `clippy::expect_used` | 97 | Reject, same reason. |
| `unreachable_pub` | 20 | Reject. All in `tests/common/mod.rs`, whose `pub fn` items are the module's interface to 37 test binaries; narrowing to `pub(crate)` changes nothing observable. |
| `clippy::needless_pass_by_value` | 2 | Reject. `input_prediction_test.rs:935,1307` take a fixture value by move on purpose. |
| `clippy::redundant_clone` | 1 | Reject. `cached_units_regression_test.rs:537` is a conditional-branch clone in a fixture, not a defect. |
| `clippy::cast_possible_truncation` | 1 | Reject. `cli_commands_test.rs:1649`, `meta.len() as usize` on a test file. |
| `wildcard_enum_match_arm`, `match_wildcard_for_single_variants`, `cast_sign_loss` | 0 | No witness. |
| `dead_code`, `unused_crate_dependencies` | 0 | Clean (the module-wide allowance hid dead helpers, see DSLP-83). |

Escape-hatch count (`rg -o '#!?\[allow\(|\bunsafe\b|\.unwrap\(\)|\.expect\('`, token matches, scope files only):
`allow` attributes 38 to 1; `unsafe` 19 to 19 (one `geteuid` moved into `tests/common`); `unwrap` 1,493 and `expect`
106 unchanged.

Adopted fixes, each behavior-preserving and checked by `cargo clippy -p kache --tests -- -D warnings` (exit 0):

- `371a7ab7`: the dead-code allowance for the shared module was 21 function-level attributes plus 22
  `#[allow(dead_code)] mod common;` sites (16 here, 6 in the next component's files). One `#![allow(dead_code)]` in
  `tests/common/mod.rs` replaces all 43. Witness: with both present, `clippy::duplicated_attributes` fails the lint in
  `msvc_passthrough_test` and `macos_*`, so the inner attribute is the one that governs.
- `371a7ab7`: `common::running_as_root()` carries the one `geteuid` block; `input_prediction_test.rs` uses it, and the
  three call sites in the next component's files follow there.
- `extra_inputs_warm_target_test.rs`: `#[allow(clippy::too_many_arguments)]` was redundant, because the root
  `Cargo.toml` already allows that lint for the package.

## DSLP-56: Tests

Tiers (test-pyramid audit), from one `cargo test --no-fail-fast --workspace --lib --tests` on a host at load 25 to 45:
4,876 passed, 0 failed, 38 ignored, 1,130 s wall. Tier 1 is the in-source unit suite (3,135 tests in the `kache` lib,
71 s of test time). Tier 2 is `tests/`. The scope's slowest binaries: `cargo_fix_test` 206 s (one test, a real
`cargo fix` and `cargo clippy --fix` over a cached tree), `adaptive_incremental_test` 157 s, `cli_commands_test` 123 s
(118 run on macOS; the 4 Linux-only tests of its 122 are compiled out), `cc_passthrough_args_test` 83 s,
`build_script_declared_inputs_test` 48 s, `cached_units_regression_test` 46 s, `cache_key_underkeying_test` 33 s,
`input_prediction_test` 28 s; the other 16 binaries each take under 30 s. No test was added or removed here, so the
20% and 30 s budget holds trivially. (Two unit tests, `execution_handoff_streams_two_compilers_...` and
`abandoned_startup_reaps_the_child_...`, failed in an earlier fail-fast run at load 45 and passed in this one: timing
flakes outside this scope, filed below.)

| Contract | Guarding tests (file:line) | Disposition |
| --- | --- | --- |
| CLI flags and exit codes: usage errors exit 2, JSON mode rejects mutation, closed stdout ends quietly | `cli_commands_test.rs:292` `argument_errors_exit_two`, `:270` `json_rejects_mutating_commands`, `:334` `a_closed_stdout_ends_quietly`, `config_commands_test.rs:132` `every_config_subcommand_emits_one_json_document` | Guarded; fault control below |
| Cache key under-keying: a changed input must miss | `cache_key_underkeying_test.rs:395` (semantic oracle against bare `rustc`), `:650` response files, `:807` sysroot, `:1166-1174` ld64 linker input files | Guarded, including the `-C link-arg` file gap AGENTS.md still lists as open (DSLP-89): the three ld64 tests exist and pass on macOS; AGENTS.md is stale |
| Build-script declared inputs and checkout relocation | `build_script_declared_inputs_test.rs:256,333`, `build_script_inputs_test.rs:221`, `cached_units_regression_test.rs:562`, `build_script_launcher_test.rs:93,120` | Guarded |
| Concurrent writers share one store | `cached_units_regression_test.rs:330` `concurrent_checkouts_compile_each_cache_key_once`, `event_log_contention_test.rs:144` | Guarded |
| Adaptive incremental lanes | `adaptive_incremental_test.rs:267,368`, `adaptive_incremental_workspace_test.rs:194`, `incremental_passthrough_test.rs:8` | Guarded |
| Input predictions across checkouts and targets | `input_prediction_test.rs:293` and 36 more | Guarded |
| Wrapper and daemon protocol: engine bootstrap, client kill stops its compiler, fail closed on non-UTF-8 argv | `engine_bootstrap_test.rs:175,369`, `cargo_proxy_test.rs:134`, `host_config_test.rs:92` | Guarded |
| Direct execution when the toolchain is untrusted | `codegen_backend_test.rs:14`, `cc_family_probe_test.rs:35`, `cc_passthrough_args_test.rs:49` | Guarded |
| Store format and integrity | Owned by `crates/kache-store/src/store/tests.rs`; here only `eviction_evidence_test.rs:22` and `custom_harness_test.rs:140` read restored output | Out of scope, guarded elsewhere |

False passes: 28 `"skipping ..."` early returns across `tests/` (as root, no `cc` on `PATH`, a compiler that rejects a
probe flag) end the test green without asserting; seven are in this scope: `cc_release_regression_test.rs:20`,
`cli_commands_test.rs:3303,3345,3414,3461`, `cached_units_regression_test.rs:1064` and `input_prediction_test.rs:2010`.
The hosts that matter run all of these tools, so nothing is red-washed today, but a runner image that loses `cc` would
pass silently. Filed below.

Hand-injected fault (critical decision: the exit status of a rejected command line). `src/main.rs`, human-mode usage
error branch: `std::process::exit(2)` changed to `exit(1)`.

```sh
cargo test --test cli_commands_test argument_errors_exit_two     # exit 101
  test argument_errors_exit_two ... FAILED   (tests/cli_commands_test.rs:305, left: Some(1), right: Some(2))
cp .work/tmp/main.rs.orig src/main.rs; cmp .work/tmp/main.rs.orig src/main.rs   # cmp exit 0
git diff --exit-code src/main.rs                                                 # exit 0
```

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed on this host (`which` exits 1 for both), so
there is no line or branch coverage and no mutation score for this scope. `.cargo/mutants.toml` and `make
mutants-diff` exist for the lane once the tool is installed (`cargo install --locked cargo-mutants --version 27.1.0`).
No test was added, so no new boundary needs its own injected fault.

## DSLP-83: Dead code

Tools: rustc `dead_code`, `unused_crate_dependencies` and `unreachable_pub` through the clippy run above (no
`cargo-machete`, `cargo-udeps` or `cargo-bloat` on this host: Needs environment). The allowance made rustc blind inside
`tests/common`, so it was lifted for one run to measure every item against every binary:

```sh
sed -i '' '/^#!\[allow(dead_code)\]$/d' tests/common/mod.rs
cargo clippy -p kache --tests --message-format=json -- --cap-lints warn     # exit 0, 31 binaries report
```

Items dead in all 31 binaries that include the module: `profile_has_fingerprints`, `fingerprint_files` and
`fingerprint_dirs`. Workspace-wide caller search for each name (`rg -w NAME`, whole checkout including `.github`,
`scripts`, `scenarios`, `crates`, `docs`): only the definitions inside `tests/common/mod.rs`. They are test-only
private helpers, not an API, so `84c66644` removes them (50 lines); the 20 other helpers are each used by at least one
binary (for example `settle_writes` by 13 files, `kache_binary` by 37). The allowance was restored before that commit.

Fixtures: each of the five mock-compiler scripts and `proc-attr-unavailable.c` has a consumer (`src/compiler/cc/tests.rs`,
`src/probe/mod.rs`, `scripts/test-unavailable-proc-labels.sh`, `cc_release_regression_test.rs`); `native_archive_fuzz` is
read by `src/native_archive.rs`. None removed. Public-API questions for consumers: none, since `tests/` exports nothing.

## Findings filed from this component

- P3: silent skips in `tests/` (see false passes). `tests/common` could offer a `skip_unless(tool)` that panics when
  `KACHE_TEST_DENY_SKIPS` is set, which CI would export.
- P3: `daemon::execution::tests::execution_handoff_streams_two_compilers_without_serializing_or_mixing_clients`
  (`src/daemon_execution.rs:653,749`) and `daemon::lifecycle_client::tests::abandoned_startup_reaps_the_child_without_terminating_it`
  (`src/daemon_lifecycle.rs:612`) fail under host load around 45 and pass at 25. Cause is a wall-clock readiness wait.
- P4: AGENTS.md says the `-C link-arg` file gap (DSLP-89) is open, but `cache_key_underkeying_test.rs:1166-1174` guard
  it and pass.
