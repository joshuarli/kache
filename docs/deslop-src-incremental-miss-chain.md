# Deslop: src incremental_policy through miss_chain

Component 20 of 28, base `11cbdba8`, branch `deslop/k5`. Scope: the 13 paths in the
DSLP-24, DSLP-51 and DSLP-78 `Where` fields (18 files, 14,021 lines, 263 unit tests;
`src/main.rs` is the crate's lib root, `src/lib.rs` was empty). Environment as in
`docs/deslop-src-args-cargo-config.md`: macOS arm64, `nightly-2026-11-09`, load average 28
to 55, `RUSTC_WRAPPER=`, system `TMPDIR` for tests. During this component the host killed
`rustc` for the crate several times (`SIGTERM`, `SIGKILL`); each build was rerun unchanged.

## DSLP-24 Types

Lint run as in the component 17 document.

| Candidate | prod | test | Disposition |
| --- | --- | --- | --- |
| `unsafe_code` (forbid) | 18 | 10 | Rejected. All FFI with no std equivalent: `wait4`, `waitid`, `poll`, the macOS Mach task and `proc_pidinfo` calls in `unix_broker/macos.rs`, `getuid`, the `link_section` Info.plist, `Notifier::from_env` (takes ownership of an inherited descriptor, `main.rs:436`), and `remove_var` (unsafe in edition 2024, `main.rs:495`). |
| `unwrap_used` | 0 | 662 | Clean in production. |
| `expect_used` | 2 | 25 | Kept: `main.rs:312` parses a constant filter string; `maintenance.rs:135` is a thread spawn at daemon start. |
| `wildcard_enum_match_arm` | 7 | 1 | Rejected: `init_shell.rs:84,92,103` default to the POSIX quoting arm; `unix_broker.rs:234,259,367` and `io.rs:66` deliberately map any unexpected protocol response to an error, which fails closed. |
| `cast_possible_truncation` | 3 | 2 | Rejected: a `size_of` to `i32`, a `u128` product divided back to `u64` in `machine.rs:289`, and `as_millis() as u64`. |
| `as_conversions` | 16 | 6 | Rejected: `pid_t` and length conversions. |
| `indexing_slicing` | 33 | 71 | Rejected: every `args[0]` and `args[1..]` in `main.rs` follows an `is_empty` or `len` guard (`:1120`, `:264`, `:449`), and `miss_chain.rs` indexes positions it was handed and bounded (`before.min(events.len())`). |
| `needless_pass_by_value` | 4 | 0 | Rejected: `Command`, `Operation` and `Config` are consumed by the callee. |
| `redundant_clone` | 2 | 0 | True positives, not fixed: `incremental_policy.rs:386` (`busy.clone()` after `busy`'s last use) and `main.rs:1205` (`daemon.clone()`). Not typing hatches or defects; filed as F2. |
| `cast_sign_loss`, `match_wildcard_for_single_variants` | 0 | 0 | Nothing to adopt. |
| `unreachable_pub` | 44 | 0 | Rejected: visibility only, filed as F1. |

Escape hatches (`unsafe`, `#[allow(`, `.unwrap()`, `.expect(`, 18 files): 705 at base and
now; there is no production hatch removable by a local change. The strict profile fixed
nothing here.

## DSLP-51 Tests

`cargo-llvm-cov` and `cargo-mutants` are not installed (DSLP-87). Needs environment:
coverage, mutation survivors, the scoped lane. Dispositions are by reading and by one
injected fault.

Tiers and costs (loaded host, 8 threads): unit `incremental_policy::` 62 tests (1 ignored)
1.9s, `index_compact::` 19 in 3.5s, `init_shell::` 8, `invocation::` 33 in 1.1s,
`key_env::` 6, `link_probe::` 31, `machine::` 10, `maintenance::` 6 in 1.4s,
`miss_chain::` 38. Integration, one binary per file: `adaptive_incremental_test` 9 tests
99.9s, `adaptive_incremental_workspace_test` 5 in 24.1s, `incremental_passthrough_test` 2 in
17.4s (153.6s wall including the build). Two full `cargo test -p kache --lib` runs on the
loaded host failed before the fix below (3,132 and 3,133 passed, 2 and 1 failed; only the
second run's name was recorded) against 3,135 passed at base on a quieter host.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| An isolated incremental seed needs a qualifying key pair (same stable groups, moved sources or externs) | `src/incremental_policy.rs:1477 deferred_seed_requires_a_qualifying_exact_key`, `:2207 seed_requires_dynamic_only_change` | Asserted; fault below |
| An active lease needs recent use, a ready state and a free slot | `src/incremental_policy.rs:2260-2269` (`active_lease_allowed`, boundary at `ACTIVE_IDLE_SECS`) | Asserted both sides of the boundary |
| A compile error keeps the state; a crash or an incremental-files error resets it | `tests/adaptive_incremental_test.rs:369,596,658` | Asserted end to end |
| A save while policy state compiles is not stored | `tests/adaptive_incremental_workspace_test.rs:411,445,476` | Asserted |
| Cargo's incremental arguments pass through unchanged and a cached compile never uses Cargo's directory | `tests/incremental_passthrough_test.rs:9,88` | Asserted |
| Argument errors exit 2, others 1 | `src/machine.rs:539 argument_errors_exit_two_and_others_one`, `tests/cli_commands_test.rs:293` | Asserted |
| The why-miss walk follows renamed dependencies, separates same-named units, and terminates on cycles and fan-out | `src/miss_chain.rs:1017,1077,1316,1336` | Asserted |
| Index compaction waives request age but not permits or size | `src/index_compact.rs:448,397,667` | Asserted |
| `init` keeps user content and permissions and backs up only changes | `src/init_shell.rs:354,456` | Asserted |
| Maintenance stays busy until every owner finishes | `src/maintenance.rs:205` | Asserted |
| A pinned compiler is retried and a slow workspace reply is cancellable | `src/invocation/unix_broker.rs:431,673`, `tests/engine_bootstrap_test.rs:370` | Asserted |
| The link probe names mounts, errno and hardening in its report | `src/link_probe.rs:450,523,607,642` | Asserted (Linux-gated branches are not compiled on this host) |

Survivors cannot be enumerated without `cargo-mutants`; none is claimed.

Fault, the `previous.stable == current.stable` conjunct removed from `qualifying_pair`
(a seed would start from a pair whose stable key groups differ):

```sh
cp src/incremental_policy.rs .work/tmp/incremental_policy.rs.orig   # edit, then
target/debug/build/kache/<hash>/out/kache-<hash> incremental_policy:: --test-threads 8
#  exit 101; deferred_seed_requires_a_qualifying_exact_key FAILED (src/incremental_policy.rs:1502)
#  and seed_requires_dynamic_only_change FAILED (:2217): 60 passed, 2 failed, 1 ignored
cp .work/tmp/incremental_policy.rs.orig src/incremental_policy.rs
cmp src/incremental_policy.rs .work/tmp/incremental_policy.rs.orig   # exit 0
```

Defect fixed (`test(invocation)`): in a full lib run at load average about 50,
`invocation::process::tests::polling_a_running_child_does_not_record_a_memory_sample`
panicked with `child never became ready` at `src/invocation/process.rs:603`: `wait_for_file`
and the exit poll allowed 3 seconds for a shell to start and exit. Both are hang guards, so
they now allow 30 seconds. The test passes at 44 threads three times; it passed in
isolation before, so the before-fix witness is the recorded panic, not a deterministic
reproduction. Other 2 second bounds in the file (`:489,565,639,815,845`) assert promptness
of cancellation and were left; they are the next candidates if the host flakes again (F3).
No test was added, so test cost is unchanged.

## DSLP-78 Dead code

Tools: rustc `dead_code` and `unused` at the repository's `warnings = "deny"` (none in
these files on lib, bins or the test target); `unreachable_pub`; `cargo metadata` target
listing before and after a removal; `rg` searches across the repository for each named
item. Needs environment: `cargo-machete`, `cargo-udeps`, `cargo-bloat` (not installed), and
a Linux build (`cargo check --target aarch64-unknown-linux-musl` stops in the `blake3` and
`zstd-sys` build scripts without a musl sysroot). Linux-gated code in this scope
(`link_probe.rs:196-413`, `invocation/workspace.rs:81`, `invocation/unix_broker.rs:393-533`)
is not compiled here and is covered by Linux CI only.

Removed: `src/lib.rs` (`138a2868`), a zero-byte tracked file left by `3a5f0f40`. `Cargo.toml`
sets `[lib] path = "src/main.rs"`, so Cargo never reads it. `cargo metadata --no-deps` lists
the same lib (`src/main.rs`), bins (`src/frontend.rs`, `src/engine.rs`) and build script with
and without the file (`cmp` exit 0); `cargo check -p kache --lib --bins` passes; no
Dockerfile, workflow, manifest or script names `src/lib.rs` (the other matches are fixture
crates' own `src/lib.rs` strings). Platforms and features covered: macOS arm64, default
features.

| Finding | Callers checked | Disposition |
| --- | --- | --- |
| 44 `unreachable_pub` items, 10 with no reference outside their file (`plan_block`, `LifecycleSnapshot`, `DrainOutcome`, `RequestGuard`, `LinkLayoutReport`, `ClonedCoverage`, `emit_outcome`, `cloned_coverage`, `Hop`, `is_resolved`) | `rg -w` over `src/`, `tests/`, `crates/` | Visibility only. Filed as F1 |
| Eleven `#[cfg(test)]` items above `mod tests` (`init_shell.rs:219 apply`, `invocation/execution.rs:7 try_execute`, `process.rs:74 borrow`, `unix_broker.rs:332 wait`, `unix_broker/macos.rs:39 stale_for_test`, `key_env.rs:52 capture`, `main.rs:917,1025`, `maintenance.rs:40 idle`, ...) | Each has test callers | Kept |
| `src/invocation.rs`, `src/lifecycle.rs`, `src/machine.rs` modules | Every module is mounted and referenced from `main.rs` | Live |

## New findings

- P4 `src/lifecycle.rs`, `src/machine.rs`, `src/miss_chain.rs`, `src/link_probe.rs`, `src/init_shell.rs`: 44 `unreachable_pub` items in this component; adopt the lint across `src/` after narrowing.
- P4 `src/incremental_policy.rs:386`, `src/main.rs:1205`: two `redundant_clone`s on values not used afterwards.
- P3 `src/invocation/process.rs`: the remaining 2 second promptness bounds (`:489,565,639,815,845`) are sized for an idle host; assert the outcome (signal, reaped child) and keep the clock as a hang guard only, as done for `compile.rs`.
