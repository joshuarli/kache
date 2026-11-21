# Deslop: src probe, report, savings, scheduler (22/28)

Scope (DSLP-26, DSLP-53, DSLP-80): `src/probe/`, `src/probe_memo.rs`, `src/report/`,
`src/report.rs`, `src/savings/`, `src/savings.rs`, `src/scheduler/` (13,397 lines; `src/scheduler.rs`
is not listed by the tickets and was not edited, though its tests exercise `scheduler/`). Base
`11cbdba8`, macOS arm64, nightly-2026-11-09. `RUSTC_WRAPPER=` and `CARGO_BUILD_JOBS=4`; the busy
host killed the `kache` lib-test compile (SIGKILL/SIGTERM) on about half of the attempts and each
result below is from a run that finished.

## DSLP-26 Types

Commands (production code, `--cap-lints warn` so the workspace `warnings = "deny"` does not turn
candidates into errors):

```sh
cargo clippy -p kache --lib --bins -- -D warnings                  # exit 0
cargo clippy -p kache --lib --message-format=json -- --cap-lints warn <candidate -W flags>
```

Escape hatches in non-test code (`unsafe`, `allow(`, `.unwrap()`, `.expect(`): 4 production
`.unwrap()` before (`probe/resolve.rs` 2, `report.rs` 2), 0 after. What remains is FFI `unsafe` in
`scheduler/permit_wait.rs` (kqueue, futex, mmap; SAFETY comments on each site) and
`scheduler/pressure.rs` (`sysctlbyname`), plus the one `allow(dead_code)` below.

| Candidate | Before | After | Disposition |
| --- | --- | --- | --- |
| `unwrap_used` | 4 | 0 | Adopted: fixed. `tokenize` and `tokenize_gnu` (`probe/resolve.rs`) peeked then unwrapped `next()`; they use `next_if` now (`e4009858`). `load_gc_summary_all` (`report.rs`) reparsed already-validated `last_run` strings and unwrapped; `load_gc_summary` now returns the parsed `UtcTime` and the reduce compares instants (`f8670c81`, `d56ff6e8`). |
| `wildcard_enum_match_arm` | 3 | 1 | Two fixed: `ran_compiler_uncached` and `bypass_route` list every `EventResult` so a new variant forces a decision. One rejected: `scrub_json` matches `deser_value::Kind`, a foreign enum. |
| `needless_pass_by_value` | 1 | 1 | Rejected: `savings::add_locked` takes the lock `File` so it is released when the call returns. |
| `cast_possible_truncation` / `cast_sign_loss` | 3 / 3 | 3 / 3 | Rejected: millisecond and fd casts on bounded values (`report.rs:2366`, `scheduler/permit_wait.rs:139`, `:171`). |
| `as_conversions` | 103 | 103 | Rejected: 97 in `report.rs` are counter-to-`f64` and width conversions for display. |
| `indexing_slicing` | 14 | 14 | Rejected: fixed-width splits after length checks. |
| `unreachable_pub` | 66 | 66 | Rejected: modules are private, so rustc `dead_code` already sees these as crate-private; the lint finds nothing. |
| `unsafe_code` | 6 | 6 | Rejected as a lint: FFI with no std equivalent; no dependency may be added. |
| `redundant_clone`, `expect_used`, `match_wildcard_for_single_variants` | 0 | 0 | Clean. |

The adopted settings were not turned into workspace lints: the remaining findings are above the
1-per-5k-lines false-positive bar. Witnesses: `tokenize_handles_escaped_quote_and_backslash`
(`probe/resolve.rs:500`), `tokenize_gnu_handles_mixed_quoted_and_unquoted` (`:953`),
`load_gc_summary_all_sums_unique_stores_and_selects_latest_instant` (`report/tests.rs:76`, checks
the later store wins and the order does not matter), `test_bypass_route_reflects_result_and_fallback`
(`report/tests.rs:2301`). `cargo test -p kache --lib -- report::` passes (108 tests).

## DSLP-53 Tests

Tiers: in-file and sibling `tests.rs` unit tests (318 passed, 3 ignored for
`report:: probe:: savings:: scheduler:: probe_memo::`, 7 s) and integration binaries that drive
the same code through `kache`: `tests/cc_family_probe_test.rs`, `tests/unknown_compiler_probe_test.rs`
(probe), `tests/eviction_evidence_test.rs` (report), `tests/integration_test.rs`; the scheduler tests live in `src/scheduler.rs`.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Compiler probe runs once and is served from its on-disk cache, keyed by program and key args, schema-checked | `probe/mod.rs:835`, `probe/cache.rs:528`, `:573`, `:642`, `:694`, `:754` | Asserted. |
| Probe subprocess is bounded (stderr head, deadline) | `probe/mod.rs:1138`, `:1451` | Asserted. |
| `cc -###` tokens unquote and unescape exactly | `probe/resolve.rs:500`, `:953` | Asserted. |
| Probe memo key is length-prefixed; a read needs its digest | `probe_memo.rs:89`, `:138` | Asserted. |
| GC window includes the cutoff instant, sums stores, picks the latest | `report/tests.rs:16`, `:76` | Asserted. |
| JSON, text and trace report shapes | `report/tests.rs:1015`, `:1823`; `eviction_evidence/tests.rs:5`, `:75` | Asserted. |
| Savings ledger rotation folds what it drops once; corrupt or newer ledgers are not overwritten; updates are not lost | `savings/tests.rs:152`, `:275`, `:296`, `:346` | Asserted. |
| Scheduler admits by slot and wakes waiters; pressure takes the whole pool | `scheduler.rs:2132`, `:2167`; `scheduler/pressure.rs:230`, `:254` | Asserted. |

Injected fault (critical decision, memory pressure): `scheduler/pressure.rs` `macos_pressured`,
`level >= 2` changed to `level > 2`. `cargo test -p kache --lib -- scheduler::pressure` exits 101:
`scheduler::pressure::tests::reads_the_macos_level` FAILED (7 passed, 1 failed). Restored from a
pre-edit copy; `cmp` exit 0.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed, so there is no
coverage figure and no survivor list; the Linux paths (`pressure::under_pressure`,
`permit_wait::linux`) do not compile on this host. No test was added: no `none` boundary and no
survivor was measured.

## DSLP-80 Dead code

Tools: rustc `dead_code` under `warnings = "deny"` (`cargo clippy -p kache --lib --bins -- -D
warnings` exit 0; `cargo test -p kache --lib --no-run` exit 0, so test code is clean too). The
refactor in DSLP-26 briefly produced a test-only `load_gc_summary` wrapper; `dead_code` flagged
it, and it was folded back (`d56ff6e8`). Needs environment: `cargo-machete`, `cargo-udeps`; the
Linux-only code.

Workspace-wide caller search (`.work/tmp/pubscan.sh`, `rg -w NAME src tests crates launcher
scenarios scripts` outside the defining file): 24 of 129 `pub` items have no reference outside
their file. All are live in the file's non-test code or are `#[cfg(test)]` /
`#[cfg(any(test, ...))]` seams; none is dead.

| Finding | Entry points and consumers checked | Disposition |
| --- | --- | --- |
| `#[allow(dead_code)] pub(super) struct Armed;` (`scheduler/permit_wait.rs:32`) | The `cfg(not(target_os = "macos"))` stand-in for the macOS `Armed`; `arm()` returns `Option<Armed>` and `park()` takes it on every OS (`permit_wait.rs:37`, `:43`, `:64`) | Kept: justified suppression; the type is never built off macOS. |
| `pressure::ENABLED` and `set_enabled` are only written by tests (`pressure.rs:36`); production calls `InvocationPressureScope::enter(true)` (`src/main.rs:1233`); the doc comment names `[cache] scheduler_memory_pressure`, a knob removed in `c2eca807` | `rg 'scheduler_memory_pressure\|set_enabled\|InvocationPressureScope' src crates tests` | Not cut: the clean-up spans `main.rs` and `scheduler.rs`, outside this scope. Filed as a finding. |
| `psi_pressured`, `macos_pressured`, `PSI_SOME_AVG10` carry `cfg(any(test, target_os = ...))` | Used by `under_pressure` on their OS and by tests everywhere | Kept: intended seams. |
| 24 file-local `pub` items | table of `pubscan.sh` | Kept: live. |

No code removed in this step. No `Needs owner` walk-back.
