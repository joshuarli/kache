# Deslop: src/cli submodules

Scope: `src/cli/doctor_daemon.rs`, `doctor_probe.rs`, `doctor_storage.rs`,
`miss_diagnosis.rs` and `src/cli/tests.rs` (6,129 lines), at base `11cbdba8`,
macOS arm64, nightly-2026-11-09. Result: no code change. Every candidate was
a false positive or an unremovable FFI call, and the existing tests killed the
injected fault.

Host note: with the installed `kache` as `RUSTC_WRAPPER`, clippy runs on this
loaded host were killed (SIGKILL, then SIGTERM) and three attempts failed.
Every command below sets `RUSTC_WRAPPER=` (bypasses the wrapper; `rustc` and
`clippy-driver` run directly), `CARGO_BUILD_JOBS=4` and, for lint runs,
`TMPDIR=$PWD/.work/tmp`. Test runs use the system `TMPDIR` as `AGENTS.md`
requires.

## DSLP-17 Types

Commands (production target, then test target; `-W warnings` stops the crate's
`warnings = "deny"` from turning the candidates into build errors):

```sh
A="-W warnings -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
 -W clippy::cast_possible_truncation -W clippy::cast_sign_loss -W clippy::unwrap_used \
 -W clippy::expect_used -W clippy::redundant_clone -W clippy::needless_pass_by_value \
 -W unreachable_pub -W unsafe_code"
cargo clippy -p kache --lib --message-format=json -- $A
cargo clippy -p kache --lib --tests --message-format=json -- $A
```

Both exit 0. Counts for this scope (production / test-only code):

| Candidate | Count | Verdict |
| --- | --- | --- |
| `unsafe_code` | 1 / 1 | Rejected. `libc::getrlimit` (`doctor_storage.rs:170`, has a SAFETY comment) has no std equivalent and the crate has no direct `rustix`; the test site is a `geteuid` root guard. |
| `unwrap_used` | 0 / 504 | Rejected. All in test code (`tests.rs` 468, `doctor_storage` tests 29, others 7); a test failure is the intended report. |
| `expect_used` | 0 / 30 | Rejected, same reason (`tests.rs`). |
| `redundant_clone` | 0 / 6 | Rejected: copies in test fixtures, no defect. |
| `cast_possible_truncation`, `cast_sign_loss`, `needless_pass_by_value` | 0 / 10 | Rejected: test fixtures only. |
| `unreachable_pub` | 1 / 0 | Rejected: `pub` fields on a `pub(super)` struct (`miss_diagnosis.rs:18`); style only. |
| `wildcard_enum_match_arm`, `match_wildcard_for_single_variants` | 0 / 0 | Nothing to adopt. |

Adopted profile: none beyond the existing `warnings = "deny"`; no candidate
rejects a defect or deletes a hatch in this scope. Escape hatches
(`rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('`): production 1
(`unsafe`), tests 521; unchanged.

## DSLP-44 Tests

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed on
this host (DSLP-87), so there is no coverage figure and no mutation survivor
list. Hand-checked instead:

Tiers: colocated unit tests with real temp directories and a real `/bin/sh`
child (`doctor_probe`), 195 pure-render unit tests in `tests.rs`, and the
binary-level `tests/cli_commands_test.rs`. Cost of the whole `cli::` and
`compiler::` lib subset: `cargo test -p kache --lib -- cli:: compiler::` exit
0, 746 passed, 2 ignored, 47.66s run time.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Storage access probe never creates, truncates or writes SQLite files; removes its own probe | `doctor_storage.rs:327` `access_probe_preserves_sqlite_files_and_removes_its_probe`, `:344` `missing_sqlite_files_are_not_created_by_the_access_probe`, `:366` permission-denied per sidecar | Asserted. |
| Free-space floor and inode exhaustion are independent; unavailable stats fail visibly; 1 GiB reserve survives `auto_recover_min_free_bytes = 0`; the recovery floor applies only to the cache volume | `doctor_storage.rs:206`, `:263`, `:296` | Asserted (floor boundary 1023/1024 pinned). |
| Daemon config mismatch and process identity (other user, missing limits, old daemon) | `doctor_daemon.rs:77`, `:122` | Asserted. |
| `doctor --probe` stores and restores in an isolated cache, stops its daemon, reports a missing compiler | `tests/cli_commands_test.rs:4182`, `:4230`, `:4272`; deadline and descendant kill: `doctor_probe.rs:187` | Asserted. |
| Miss cause precedence and the schema-15 cutover | `miss_diagnosis.rs:112` (schemas 14, 15, 16; store error over lookup rejection over repeat), `:138`, `:149` | Asserted. |
| CLI flags and exit codes | `tests/cli_commands_test.rs:293` `argument_errors_exit_two` | Owned by DSLP-18. |

Injected fault, `doctor_storage.rs:disk_check`: `let low = usage.free < floor;`
changed to `usage.free <= floor`.
`cargo test -p kache --lib -- cli::doctor_storage` exit 101, 8 passed, 1
failed: `cli::doctor_storage::tests::space_and_inode_pressure_are_independent_and_missing_data_is_explicit`
(`assertion failed: healthy.pass`, `doctor_storage.rs:223`). Restored from a
copy; `cmp` exit 0, `git status` clean for the file.

No new tests: every contract above has a guard, none is `none`.

## DSLP-71 Dead code

- rustc `dead_code`: clean at `-D warnings` for `cargo clippy -p kache --lib`
  (production, so a helper used only by tests would be reported) and for
  `--lib --tests`.
- `unreachable_pub`: 1 finding, style (above).
- Workspace-wide caller search (`rg -w NAME src crates tests scripts build.rs`)
  of every visibility-qualified item in the four modules (8 items): the least
  used is `miss_diagnosis::env_hints` with 3 references (its definition's
  callers in `cli.rs` and its test); none is unreferenced.
- `#[allow(`, `#[cfg(test)]`-only helper and `#[ignore]` sweep of the scope:
  none.
- Needs environment: `cargo-machete`, `cargo +nightly udeps` and `cargo-bloat`
  are not installed; dependency-level findings for these modules are
  unmeasured.

Disposition: nothing removed; no walk-backs.
