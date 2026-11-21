# Deslop: src daemon, events, fallback, engine

Component 19 of 28, base `11cbdba8`, branch `deslop/k5`. Scope: the 18 paths in the
DSLP-23, DSLP-50 and DSLP-77 `Where` fields (19 files, 15,964 lines, 283 unit tests; the
daemon modules mount as `daemon::lifecycle_client`, `daemon::execution`,
`daemon::lifecycle_control` and `daemon::capacity` through `#[path]`). Environment as in
`docs/deslop-src-args-cargo-config.md`: macOS arm64, `nightly-2026-11-09`, a loaded host,
`RUSTC_WRAPPER=`, system `TMPDIR` for tests. `cargo test -p kache --lib` as a whole: 3,135
passed, 30 ignored, 105s at base.

## DSLP-23 Types

Lint run as in the component 17 document; counts are by file, split at the file's
`mod tests`. The `events.rs` figures include `#[cfg(test)]` items declared above its test
module (`EventTailer`, see DSLP-77).

| Candidate | prod | test | Disposition |
| --- | --- | --- | --- |
| `unsafe_code` (forbid) | 14 | 18 | Rejected. Every site is FFI with no std equivalent: `poll`, `kill`, `getrlimit`, `geteuid`, `waitpid`, `fcntl`, `from_raw_fd` on an inherited descriptor, `dlsym` plus `transmute` of the private `sandbox_check_by_audit_token` (`fallback/macos.rs:26-31`), the Mach `proc_pidinfo` and `mach_timebase_info`, and the `link_section` Info.plist (`frontend.rs:5`). |
| `unwrap_used` | 5 | 1,097 | Not adoptable as a lint. Two fixed, two kept (below); the fifth, `events.rs:735`, sits in a `#[cfg(test)]` item. |
| `expect_used` | 3 | 51 | Kept: `daemon.rs:323` (a PID checked non-zero one call up), `daemon.rs:1617` (thread spawn at daemon start), `events.rs:353` (the helper pushed the window it pops). |
| `wildcard_enum_match_arm` | 2 | 2 | Rejected: the fallthrough over the external `kunobi_daemon` replacement `Outcome` (`daemon_lifecycle.rs:41`) and a logged default arm (`daemon.rs:1582`). |
| `cast_possible_truncation` | 12 | 4 | Rejected: ten are `Duration::as_millis() as u64` (overflow needs 584 million years); the others are `size_of::<T>() as c_int` and a clamped nanosecond count. |
| `cast_sign_loss` | 2 | 4 | Rejected: `file.size.max(0) as u64`, clamped on the same line. |
| `as_conversions` | 31 | 16 | Rejected: dominated by libc `pid_t` and `usize` to `u64` conversions. |
| `indexing_slicing` | 10 | 84 | Rejected: guarded indexes into fixed arrays and just-measured lengths. |
| `needless_pass_by_value` | 5 | 0 | Rejected: every flagged `Arc`, `Receiver` and `String` is moved into a thread or channel afterwards. |
| `redundant_clone` | 1 | 0 | True positive, not fixed: `daemon.rs:1709` clones a `DaemonCoordFile` (one `PathBuf`) that is never used again; the drop guard is a separate `DaemonCoordGuard`. Not a typing hatch or defect, so left; filed as F4. |
| `unreachable_pub` | 62 | 0 | Rejected: visibility only, nothing is exported. Filed. |

Escape hatches (`unsafe`, `#[allow(`, `.unwrap()`, `.expect(`, 19 files): 1,204 at base,
1,202 now. The two `#[allow]` attributes stay: `heartbeat.rs:384`
`#[allow(deprecated)]` for libc's Mach bindings (the comment names the alternative crate),
and `daemon/tests.rs:1350` `#[allow(dead_code)]` on a test-only enum whose variants are
never built because only its `Deserialize` shape matters.

Fixed (`refactor(daemon)`, one commit): `Step::Prepare` stored the replacement executable
and unwrapped it for the metadata check; `Option::insert` returns the stored value.
`Step::Start` unwrapped an executable the step order guarantees; it now fails the step
with `replacement executable was not prepared` instead of panicking. The step order makes
the message unreachable, so there is no new witness; guard: the 19
`daemon::lifecycle_client` tests (for example `src/daemon_lifecycle.rs:707
replacement_requests_drain_before_waiting_for_cache_ownership`), which pass before and
after, and `tests/cli_commands_test.rs init_waits_for_a_new_service_and_rejects_failed_startup`.
Kept: `std::fs::create_dir_all(socket_path.parent().unwrap())` at `daemon.rs:1460,1539`;
`config.socket_path()` always joins a runtime directory and a file name.

## DSLP-50 Tests

`cargo-llvm-cov` and `cargo-mutants` are not installed (DSLP-87). Needs environment:
coverage figures, the mutation survivor list, the scoped mutation lane. Disposition below
comes from reading each contract against its tests and from a hand-injected fault.

Tiers and costs (loaded host, 8 threads): unit `daemon::tests::` 93 tests 1.8s,
`daemon::execution::` 21 tests 1.6s (plus 16 `#[ignore]` fixture bodies that their parent
tests re-execute as subprocesses; one more ignored fixture in `daemon::tests`),
`daemon::lifecycle_client::` 19 in 0.07s, `daemon::lifecycle_control::` 9 in 0.2s,
`daemon::capacity::` 7, `daemon_publish::` 26 in 1.5s, `events::` 67 in 7.5s,
`heartbeat::` 8 in 2.0s, `disk_recovery::` 5, `demand::` 3, `fallback::` 1. Integration
(33.5s wall including the build): `engine_bootstrap_test` 7 tests 11.9s,
`event_log_contention_test` 2 tests (1 ignored) 4.8s, `eviction_evidence_test` 1 test 1.1s.
The Linux decision module compiles under `cfg(any(test, target_os = "linux"))`, so its six
`/proc`-fixture tests run on this macOS host.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Delegation needs both peers' trust and a kernel-verified identity | `src/daemon_execution.rs:689 trusting_the_pipeline_requires_both_peers` (Linux), `:1952 execution_rejects_sandboxed_client_using_kernel_identity` (macOS), `src/fallback/sandbox.rs:10` | Asserted on both platforms; fault below |
| A forged peer PID is declined before a worker exists | `:1893 fixture_execution_declines_a_forged_peer_pid_before_preparing_a_worker` | Asserted |
| A prepared compiler never runs without commit | `:1675` | Asserted |
| Replacement drains before waiting for ownership; a readiness proof is required | `src/daemon_lifecycle.rs:707`, `:836`, `:585` | Asserted |
| A control record cannot name another PID or cache; no silent downgrade to the legacy endpoint | `src/daemon_control.rs:545`, `:638` | Asserted |
| Admission respects the descriptor budget | `src/daemon_capacity.rs:125`, `:175` | Asserted |
| GC takes each store's lock; automatic GC without an age fails closed | `src/daemon/tests.rs:2155`, `:1620` | Asserted |
| Drain does not wait for silent handlers past the deadline | `src/daemon/tests.rs:573` | Asserted |
| Event log rotation delivers retained events exactly once | `src/events.rs:2836`, `:2761` (through the test-only tailer) | Asserted |
| A held event-log lock never blocks a compile or `--version` | `tests/event_log_contention_test.rs:114,144` | Asserted |
| Disk-pressure recovery backs off and survives clock changes | `src/disk_recovery.rs:156` | Asserted |
| Heartbeat emits a structured line and does not wait on a stalled ticker | `src/heartbeat.rs:571`, `:627` | Asserted |
| A daemon and engine bootstrap and keep the platform process owner | `tests/engine_bootstrap_test.rs:176,259,370` | Asserted |

Survivors cannot be enumerated without `cargo-mutants`; none is claimed.

Fault, `trusts_pipeline` loosened from `client && daemon` to `client || daemon`:

```sh
cp src/daemon_execution.rs .work/tmp/daemon_execution.rs.orig   # edit, then
target/debug/build/kache/<hash>/out/kache-<hash> daemon::execution::tests::execution_rejects_sandboxed_client_using_kernel_identity
#  exit 101; execution_rejects_sandboxed_client_using_kernel_identity FAILED at
#  src/daemon_execution.rs:653 and its subprocess fixture at :1999
#  (the wider `daemon::` run: 148 passed, 1 failed, 17 ignored)
cp .work/tmp/daemon_execution.rs.orig src/daemon_execution.rs
cmp src/daemon_execution.rs .work/tmp/daemon_execution.rs.orig   # exit 0
```

The truth-table test named above is `#[cfg(target_os = "linux")]` although the function is
platform-independent, so on macOS the fault is caught only by the sandbox test, which
needs `sandbox_check` and the macOS test fixture; filed as F2. No test was added or
changed, so test cost is unchanged.

## DSLP-77 Dead code

Tools: rustc `dead_code` and `unused` at the repository's `warnings = "deny"` (none in
these files on lib, bins or the test target); `unreachable_pub`; a scripted listing of
`#[cfg(test)]` items declared above `mod tests`; `rg -w` caller searches across `src/`,
`tests/` and `crates/`. Needs environment: `cargo-machete`, `cargo-udeps`, `cargo-bloat`
(not installed), and a Linux build (`cargo check --target aarch64-unknown-linux-musl` stops
in the `blake3` and `zstd-sys` build scripts for lack of a musl sysroot). The Linux-only
decision module is partly covered by its tests running here (above); the
`target_os = "linux"` branches of `daemon_execution.rs` (lines 35, 662 to 982) are
unverified for dead code on this host and are protected by Linux CI.

| Finding | Callers checked | Disposition |
| --- | --- | --- |
| `unreachable_pub`, 62 items, 10 with no reference outside their file (`handle_hash_files`, `GcRequestOutcome`, `event_archive_dir`, `read_transfers`, `compute_stats_onto`, `CompileMonitor`, and the four `EventTailer` and `EventRecord` names) | `rg -w` over `src/`, `tests/`, `crates/` | Visibility only, no dead code. Filed as F1 |
| `EventTailer`, `EventRecord`, `parse_event_line`, `poll*`, `from_start`, `clear_events`, `log_event` in `events.rs` | Test-only (`#[cfg(test)]`); the docs say "only the rotation tests read through this"; the TUI that consumed it was deleted in `3a5f0f40`; 15 uses in `events::tests` | Kept: it is the oracle for rotation exactly-once delivery (`:2836`) and the marker format. The nested `#[cfg(test)]` on methods of a `#[cfg(test)]` impl is redundant. Not removed because deleting it removes the only reader of the rotation marker; filed as F3 |
| `#[cfg(test)]` seams `daemon_control.rs:294`, `daemon_execution_linux.rs:8,34`, `daemon.rs:558,606,2077` | Each has test callers | Kept |
| Glob and `#[path]` modules | `daemon_lifecycle.rs` and friends mount through `#[path]`; all are referenced | Live |
| `heartbeat.rs:384` `#[allow(deprecated)]` | Needed for libc's Mach bindings | Kept |

No code removed; tools are clean.

## New findings

- P4 `src/daemon.rs`, `src/events.rs`, `src/heartbeat.rs`: 62 `unreachable_pub` items in this component; adopt the lint across `src/` after narrowing.
- P4 `src/daemon_execution.rs:688`: `trusting_the_pipeline_requires_both_peers` tests a pure `bool` function but is Linux-only; drop the `cfg` so every host runs the truth table.
- P3 `src/events.rs:607`: `EventTailer` is the only reader of the rotation marker and exists only under `cfg(test)`; decide whether the marker is a contract for external tailers (move the tailer and its ten tests to `tests/`, document the format) or remove both.
- P4 `src/daemon.rs:1709`: `let heartbeat_coord = coord.clone();` is redundant (`coord` is last used on the line above); move `coord` into the closure.
