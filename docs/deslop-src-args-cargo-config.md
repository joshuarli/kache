# Deslop: src args, build_script inputs, cargo and config modules

Component 17 of 28, base `11cbdba8`, branch `deslop/k5`. Scope: the 15 paths in the
DSLP-21, DSLP-48 and DSLP-75 `Where` fields (19 files, 15,079 lines, 274 unit tests).
Host: macOS arm64, `nightly-2026-11-09`, load average 28 to 70 throughout, so every
duration is inflated. Cargo ran with `RUSTC_WRAPPER=` (the user config wraps rustc in an
installed kache; two clippy runs through it died with SIGKILL) and `CARGO_BUILD_JOBS=4`.
Test runs used the system `TMPDIR`, because a checkout-local one fails 11 tests (AGENTS.md).

## DSLP-21 Types

Command, run once for all four components of this lane (lib and bin targets, then the
lib test target):

```sh
cargo clippy -p kache --lib --bins --message-format=json -- --cap-lints=warn \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss -W clippy::unwrap_used \
  -W clippy::expect_used -W clippy::redundant_clone -W clippy::needless_pass_by_value \
  -W clippy::indexing_slicing -W clippy::as_conversions -W unreachable_pub -W unsafe_code
cargo clippy -p kache --lib --profile test --message-format=json -- <same, minus redundant_clone>
```

`redundant_clone` on the lib test target is killed with SIGKILL twice (exit 101,
`signal: 9`) while the same target passes without it; it was measured on lib and bins only.
`--cap-lints=warn` is needed because the workspace sets `warnings = "deny"`. Diagnostics
were bucketed by file and by whether the line is below the file's `mod tests`.

| Candidate | prod | test | Disposition |
| --- | --- | --- | --- |
| `unsafe_code` (forbid) | 1 | 1 | Rejected. `cargo_proxy.rs:45` is `setrlimit`, `signal`, `pthread_sigmask` and `raise`, with no std equivalent; the test use is `geteuid`. |
| `unwrap_used` | 4 | 682 | Adopt no setting. Three are `Mutex::lock().unwrap()` on the child handle in `compile.rs:343-354` (poison only after a reader panic, which the function already reports); one is fixed (below). |
| `expect_used` | 0 | 6 | Clean in production. |
| `wildcard_enum_match_arm` | 3 | 3 | Rejected: `Component` and `ArtifactKind` matches with a deliberate pass-through arm (`outside.rs:703,777`, `compiler_store.rs:13`). 3 false positives in 15k lines, over the 1 per 5k bar. |
| `match_wildcard_for_single_variants`, `cast_sign_loss`, `cast_possible_truncation` | 0 | 0, 0, 2 | Nothing to adopt. |
| `as_conversions` | 1 | 9 | Rejected: `blob_heal.rs:241` is `usize` to `u64`, lossless. |
| `indexing_slicing` | 19 | 33 | Rejected: every production index is behind a length check (`args.rs:386` guards `args[0]` and `args[1]`, `outside.rs:524` checks `parts.len()` first). |
| `needless_pass_by_value` | 1 | 0 | Rejected: `lexopt_error(error: lexopt::Error, ...)` consumes the error by convention at every call; no defect. |
| `unreachable_pub` | 42 | 0 | Rejected for the profile: visibility only, no defect. Filed as a finding. |

Escape hatches (`unsafe`, `#[allow(`, `.unwrap()`, `.expect(` over the 19 files):
677 at base, 676 now. The one `#[allow(dead_code)]` (`compile.rs:35`, `keepalive`) holds
temp files so their `Drop` does not run before the store put. Renaming it `_keepalive`
would remove the attribute but also touch `compiler/cc.rs` and `compiler/cc/tests.rs`,
which another lane owns, so it stays.

Fixed: `parse_optional_dir` peeked at the next argument and then unwrapped `raw.next()`.
`RawArgs::next_if` states the same decision once (`f51986b5`). Guard:
`src/main.rs:2264 init_shims_and_why_miss_without_a_crate_parse` covers `--shims DIR`,
`--shims --flag` and a bare `--shims`; it passes before and after.

## DSLP-48 Tests

Tools: `cargo-llvm-cov` and `cargo-mutants` are not installed on this host (DSLP-87), so
no coverage or mutation run exists. Needs environment: coverage, mutation survivors, the
scoped mutation lane. Disposition below is by reading each contract against its tests and
by hand-injected faults.

Tiers and costs (loaded host). Unit, `cargo test -p kache --lib` filtered by module, 8
threads: `args::` 84 tests 0.02s, `blob_heal::` 18 in 0.15s, `cargo_layout::` 11, `cargo_env::`
9, `config_cli::` 5 in 0.24s, `compile::` 44 in 0.4 to 1.3s (10s under 44 threads, see
below), the `build_script` filter (208 tests, includes `build_script_inputs`) 9.6s. The
whole lib target: 3,135 passed, 30 ignored, 105s. Integration, one binary per file against
a built `kache`: `cli_commands_test` 118 tests 122s, `build_script_declared_inputs_test`
4 tests 181s, `build_script_inputs_test` 5 in 31s, `config_commands_test` 6 in 16s,
`cargo_proxy_test` 6 in 12s, `build_script_launcher_test` 2 in 11s, `metadata_streaming_test`
3 in 3.8s (431s wall including compilation). Most behavior sits in fast unit tests; the
three build-script tests are the slow end-to-end tier and prove replay against a second
checkout.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Lint-level and `--cap-lints` flags enter the key and not the residual | `src/args.rs:1442 outcome_lint_gates_capture_flag_and_value` | Asserted; fault below |
| Diagnostic presentation flags reach the key | `src/args.rs:1262 key_diagnostic_flags_preserve_forwarded_arguments`, `tests/metadata_streaming_test.rs:123` | Asserted |
| Response-file content changes the key | `src/args.rs:1835`, `tests/cache_key_underkeying_test.rs:650` | Asserted |
| Target dir and workspace root from `--out-dir`, both Cargo layouts | `src/args.rs:2565 test_target_dir_from_out_dir`, `:2823 verified_workspace_root_rejects_external_target_parents`, `src/cargo_layout.rs:280` | Asserted |
| rustc pipe failure kills and reaps the child | `src/compile.rs:998 either_pipe_read_failure_kills_and_reaps_the_child` | Asserted; flake fixed; fault below |
| Metadata forwarded once before EOF | `src/compile.rs:1064`, `tests/metadata_streaming_test.rs:50` | Asserted |
| CLI grammar, exit 2 on argument errors | `tests/cli_commands_test.rs:293 argument_errors_exit_two`, `src/cli_args.rs:2098`, `src/main.rs:2264` | Asserted |
| Build script outputs replay under a second checkout | `tests/build_script_inputs_test.rs:221-241`, `tests/build_script_declared_inputs_test.rs:256` | Asserted end to end |
| Named files outside `OUT_DIR` and rooted links stay inside the target | `src/build_script/outside.rs:1069,1152`, `src/build_script/rooted_links.rs:233,297,424` | Asserted |
| Blob-index repair needs a quiet machine and backs off six hours | `src/blob_heal.rs:349,424` | Asserted |
| Config edits keep comments and file mode | `src/config_cli.rs:303,389`, `tests/config_commands_test.rs:37` | Asserted |
| `cargo` proxy passes arguments, signals and the wrapper | `tests/cargo_proxy_test.rs:35` and `:58` | Asserted |
| Cargo `[env]` keys never overwrite | `src/cargo_env.rs:145,163` | Asserted |

Survivors cannot be enumerated without `cargo-mutants`; none is claimed.

Fault A, `OUTCOME_AFFECTING_VALUE_FLAGS` loses `"--cap-lints"` (a cap-lints flag would fall
into the residual and stop changing the key):

```sh
cp src/args.rs .work/tmp/args.rs.orig        # edit, then
cargo test -p kache --lib -- args::          # exit 101
#  args::tests::outcome_lint_gates_capture_flag_and_value FAILED
#  left: []  right: ["--cap-lints", "allow"]   (83 passed, 1 failed)
cp .work/tmp/args.rs.orig src/args.rs; cmp src/args.rs .work/tmp/args.rs.orig   # exit 0
```

Fault B, the stderr-failure kill removed from `capture_rustc_output_with`:
`either_pipe_read_failure_kills_and_reaps_the_child` fails (exit 101, 10.04s, the child
slept its full ten seconds and exited with no signal); restored, `cmp` exit 0, 1 passed.

Defect fixed (`00bd6dfc`): the same test asserted `started.elapsed() < 5s`. At 44 test
threads on this host it failed 1 run in 6 at that line with an elapsed time near 9s while
the child had in fact been killed. It now asserts the child died by `SIGKILL`, which a
child sleeping out its own `sleep 10` cannot satisfy, with no clock. Eight reruns at 44
threads passed once load fell (the failure could not be re-provoked after the fix, so the
before-fix witness is the 1 in 6 above plus fault B). No test was added, so test cost is
unchanged.

## DSLP-75 Dead code

Tools run: rustc `dead_code` and `unused` at the repository's `warnings = "deny"`
(zero `dead_code` or `unused_*` diagnostics in the 19 files across lib, bins and the test
target); `unreachable_pub`; a scripted search of every `#[cfg(test)]` non-module item; a
manual dependency pass (each of the 32 `[dependencies]` names searched under `src/`; all
have at least three references, `getrandom` one real call at `daemon_execution.rs:288`).
`src/lib.rs` is outside this component. Needs environment: `cargo-machete`, `cargo-udeps`,
`cargo-bloat` (not installed); a Linux dead-code pass (`cargo check --target
aarch64-unknown-linux-musl` stops in the `blake3` and `zstd-sys` build scripts, which need a
musl sysroot and `aarch64-linux-musl-gcc`). No item in this scope is gated on `target_os`
(the one match in `compile.rs:792` is a comment), so no Linux-only code goes unchecked; the
`cfg(not(unix))` arms in `rooted_links.rs:185` and `cargo_proxy.rs:61` are unchecked on this host.

| Finding | Entry points and callers checked | Disposition |
| --- | --- | --- |
| 42 `unreachable_pub` items | Two have no reference outside their file: `unit_id_from_extra_filename` (`args.rs:80`) and `CompilerArtifactPolicy` (`compiler_store.rs:6`); searched `src/`, `tests/`, `crates/` | Visibility only; the crate exports `main` and `VERSION`. Finding F1 |
| `#[allow(dead_code)] keepalive` | `compile.rs:36`; read by `compiler/cc/tests.rs:6313` | Kept: drop guard |
| Seven `#[cfg(test)]` helpers in production position (`RustcArgs::parse`, `with_digest`, `declares_inputs`, `compile.rs:57,372`, `config.rs:3`, the name table) | Each has test callers | Kept: used |
| `config.rs` re-exports `config_path_lock` at two paths | `crate::config::config_path_lock` (probe, extra_inputs, wrapper) and `crate::config::tests::config_path_lock` (`compiler/mod.rs:2881`) | Not deleted: the second caller is another lane's file. Finding F2 |
| Glob re-exports (`cache_fs.rs`, `config.rs`) | rustc cannot report unused names through `pub use x::*` | Not provable here |

No code removed; tools are clean.

## New findings

- P4 `unreachable_pub` over src: 42 items in this component alone could be `pub(crate)`; adopt the lint once across `src/`.
- P4 `src/config.rs`: one test lock helper exported as `config::config_path_lock` and `config::tests::config_path_lock`; pick one (touches `src/compiler/mod.rs`).
