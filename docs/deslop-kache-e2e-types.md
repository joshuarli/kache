# kache-e2e types audit: bench, cc oracle and daemon

DSLP-6, component 2 of 28. Base: `a66965c0ec73f1834237dc331bb1f15eaa30ac21`.
The original audit branch started directly from origin/main, independently of
DSLP-4 and DSLP-5. Integration rebased it onto `88b90242`, which includes both.
Scope is the 27 files in DSLP-6 at that base, not all of kache-e2e. The other
fixture/scenario modules belong to DSLP-7. Current scope is 19,232 lines before
the change: 11,753 production lines and 7,479 test lines.

Platform: Linux x86_64, all Cargo targets. The crate declares no features.
macOS-specific paths were reviewed but not compiled or executed.

## Strict-profile measurement

Both before and after measurements used:

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-e2e --all-targets --no-deps --message-format=json -- \
  --cap-lints warn \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value
```

The initial probe without `--cap-lints warn` failed because the manifest denies
warnings. The capped discovery runs completed with exit 0 and enumerate all
targets. The cap is only a command-line measurement setting. The required
`make check` uses the unchanged warning-deny policy and the new module guard,
without the cap.

Counts are unique primary diagnostic spans, deduplicated across library and
library-test compilations and filtered to the ticket paths. Tests include
bench_runner/tests.rs and inline cfg(test) modules. Multiple arguments or calls
on one line count separately here. Diagnostics outside these paths are excluded.

| Candidate | Production before → after | Tests before → after | Decision |
| --- | ---: | ---: | --- |
| `unsafe_code = forbid` | 1 → 1 compiled; 2 → 2 lexical blocks | 0 → 0 scoped lexical blocks | Reject: getrusage requires Unix FFI; macOS host counters also use FFI. No safe equivalent established. |
| `wildcard_enum_match_arm` | 5 → 5 | 0 → 0 | Reject: shared arm handling, guarded reuse fallthrough, and lexopt's unexpected-argument errors. No demonstrated missing variant defect. |
| `match_wildcard_for_single_variants` | 0 → 0 | 0 → 0 | Reject: no scoped finding or witness. |
| `cast_possible_truncation` | 8 → 8 | 2 → 2 | Reject: rounded metric conversions, practical duration bounds and 64-bit targets; shuffled index is bounded by slice length. No demonstrated reachable truncation defect. |
| `cast_sign_loss` | 8 → 8 | 0 → 0 | Reject: nonnegative resource/time measurements and guarded clock differences. Malformed arbitrary inputs are not evidence of a reachable tool-output defect. |
| `unwrap_used` | 14 → 14 | 466 → 466 | Reject broad adoption: existing producer/length invariants below. Tests intentionally assert fixture success. |
| `expect_used` | 3 → 2 | 32 → 32 | Adopt only in production bench_runner: its single expect becomes a required borrowed argument, leaving zero local hits and zero suppressions. The two other production expects follow piped-stdin setup. |
| `redundant_clone` | 7 → 7 | 4 → 4 | Reject for Types: removable copies, not escape hatches or false type contracts. Leave active campaign files alone. |
| `needless_pass_by_value` | 10 → 10 | 1 → 1 | Reject: ownership-style suggestions, not false contracts. Ten production false positives exceed one per 5k lines. |

Forbid probe:

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo rustc -p kache-e2e --lib --message-format=json -- -F unsafe_code
```

Expected exit 101: one scoped unsafe block in cc_bench/mod.rs and two blocks in
out-of-scope fixture_runner/scenario_runner. The macOS-only block in
cc_bench/activity.rs was not compiled on Linux. No forbid setting was adopted.

## Trust removed and compiler witness

`run_bench` previously called the private `run_pull_bench` only when
`BenchProfile::is_pull()` returned true. That predicate is exactly
`self.ref_next.is_some()`. The helper then repeated that fact with
`expect("run_pull_bench requires ref_next")`.

The caller now binds `Some(ref_next)` directly and supplies `ref_next: &str` to
the helper. The helper cannot accept an absent revision. The same immutable
profile supplies the same bytes, with no extra allocation or filesystem work.
The public `is_pull` method remains unchanged. Existing backend and warm-tree
rejections stay in the same branch, before the same helper calls.

The production-only module setting is:

```rust
#![cfg_attr(not(test), deny(clippy::expect_used))]
```

It removes one existing trust assertion and has zero false positives in its
adoption scope. It does not weaken test assertions or impose unrelated rules on
DSLP-7 or the active C/C++ campaign modules.

Negative compiler witness: temporarily replace the new call argument `ref_next`
with `profile.ref_next.as_deref()`, then run
`cargo check -p kache-e2e --lib`. The real call site failed as intended:

```text
error[E0308]: mismatched types
321 |             profile.ref_next.as_deref(),
    |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `&str`, found `Option<&str>`
TYPE_WITNESS_EXIT=101
```

The checker output was checked for both expected and actual types. The witness
edit was restored immediately. The optional revision cannot cross the helper
boundary without being narrowed. This is a private signature change, not a new
runtime parser or public contract change. No new runtime test is needed.

## Escape hatches and retained invariants

The text counter uses the exact requested expression on the 27 ticket files:
`rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('`.
Counts are matching lines, not occurrences. cfg(test) modules and tests.rs are
separated from production. cfg_attr allow attributes do not match that expression.

| Scope | Before | After |
| --- | ---: | ---: |
| Production | 26 | 25 |
| Tests | 480 | 480 |
| Total | 506 | 505 |

Production before: 13 unwrap lines, 3 expect lines, 2 unsafe lines and 8 allow
lines. After: 13, 2, 2 and 8. One gcov line contains two unwrap calls.
The surviving production hatches have these producer checks:

- bench_profile's two stdin expects follow `Stdio::piped()` and successful spawn.
- cc_oracle/gcov checks a 12-byte notes header before taking fixed four-byte
  arrays. Relocation records come from `as_chunks::<24>()`; all field slices
  and the first u64 fit those exact records.
- cc_bench's compiler paths come from resolved toolchains; its tree/store paths
  are constructed beneath the workload root. Unit scenarios come from workloads
  that supply a single-unit command. Validation covers every Arm before timing.
- cc_bench fills `pair = Some(pair)` before admitting timed replays. It finds
  the raw and same-arm validation rows from the complete Arm::ALL set. Target
  unwraps follow an is_some filter. These are private producer invariants, not
  demonstrated panics on supported inputs.
- Seven bench_runner `too_many_arguments` allows and the macOS deprecated Mach
  binding allow remain. Deleting those annotations alone changes no type truth.
- Test unwraps/expects encode fixture setup and exact assertions. Removing them
  would not improve the production type contract.

No new defect or owner-decision ticket is justified. Next remains 103 because
this branch allocates no new ticket number. DSLP-6 is removed per closure
convention. No public API, CLI argument/exit code, benchmark/report schema,
cache-key input, store format or wrapper protocol changes.

## Active campaign and consumers

Recent main history includes `041c4ea5` (time-macro raw capability gating),
`86415a7f` (portable capability-probe fixtures), `c0ef2a06` (unsupported flags),
and `8d0cc4d0` (process lifetimes in activity sampling). No cc_oracle or cc_bench
file is edited. No edit touches src/compiler/cc*, src/probe/* or campaigns/cpp/*.

The caller is the kache-scenario benchmark path. Existing bench_profile tests
cover present and absent ref_next. Bench-runner tests cover pull result schema,
mbx pull phases, backend behavior, fake build tools and benchmark reports.
Cargo resolves the candidate through this worktree's workspace path.
The required workspace gate also compiles the six harness binaries and exercises
root consumers. No actual benchmark performance run, full compiler-oracle
campaign or cross-platform execution is claimed. The change does not alter the
measured build work or add work to the benchmark hot path.

## Verification

Tools: rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`, Cargo
`1.102.0-nightly (c07ac2c4d 2026-11-06)`, Clippy
`0.1.101 (5b7b37aefc 2026-11-08)`. Four of eight logical CPUs were used.
Node was unavailable in the loaded environment; Python 3.15.2 supplied the
read-only diagnostic deduplication and source counters instead. No tool install,
dependency, toolchain, CI configuration or formatter change was needed.

Scoped test command:

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo test -p kache-e2e --lib bench_runner::tests
```

Required gate:

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 make check
```

Results:

```text
test result: ok. 108 passed; 0 failed; 0 ignored; 0 measured; 271 filtered out; finished in 0.22s
BENCH_RUNNER_TEST_EXIT=0
```

The uncapped lint gate, client dependency graph, resource guard and workspace
tests all completed. Gate tail:

```text
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.66s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
MAKE_CHECK_EXIT=0
```
