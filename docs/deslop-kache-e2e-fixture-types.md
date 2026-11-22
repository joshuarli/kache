# kache-e2e types audit: fixture and scenario runners

DSLP-7, component 3 of 28. Base: `88b9024298da2a4ae4ce8ffe67354b4673210aa8`.
This branch starts from main independently of unmerged DSLP-6. The owner
authorized DSLP-7 while DSLP-6's PR was held. Only DSLP-7 is closed here.

The 12 ticket files contain 5,586 Rust lines: 3,974 production and 1,612 test
lines. Scope: fixture.rs, fixture_runner.rs, lib.rs, phase.rs, report.rs,
result.rs, runner.rs, scenario.rs, scenario_runner.rs and source.rs under
crates/kache-e2e/src, plus tests/skip_summary.rs and tests/strict_gate.rs.
No source edit or lint adoption is justified by the measured findings.

## Strict-profile measurements

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-e2e --all-targets --no-deps --message-format=json -- \
  --cap-lints warn \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value

taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-e2e --all-targets --no-deps --message-format=json -- \
  --cap-lints warn -F unsafe_code
```

Both discovery commands exited 0. `--cap-lints warn` allows complete discovery
despite the manifest's `warnings = "deny"`; it is not a repository setting.
The required gate runs the existing warning-deny policy without that cap.
The crate declares no features. All Cargo targets were compiled on Linux x86_64.

Counts are distinct primary diagnostic spans filtered to the 12 ticket files,
deduplicated across library and library-test builds. Separate calls on one line
count separately. Inline cfg(test) modules and the two integration-test files
are tests. No optional test-support module exists in this scope.

| Candidate | Production | Tests | Decision |
| --- | ---: | ---: | --- |
| `unsafe_code = forbid` | 2 | 3 | Reject: process-environment mutation requires unsafe in edition 2024. Command-local environment propagation is not a drop-in replacement for the existing process-wide behavior. |
| `wildcard_enum_match_arm` | 0 | 0 | Reject: no defect or escape-hatch witness. |
| `match_wildcard_for_single_variants` | 0 | 1 | Reject: a test's fallback panics unless the parsed config has the expected clone variant. No supported variant is silently accepted. |
| `cast_possible_truncation` | 0 | 0 | Reject: no witness. |
| `cast_sign_loss` | 0 | 0 | Reject: no witness. |
| `unwrap_used` | 0 | 128 | Reject: production is already clean; tests assert fixture setup, exact results and expected errors. Includes unwrap_err, unlike the text counter. |
| `expect_used` | 0 | 18 | Reject: production is already clean; test assertions and setup are intentional. |
| `redundant_clone` | 2 | 0 | Reject for Types: removable copies, not invalid types or escape hatches. |
| `needless_pass_by_value` | 1 | 0 | Reject: ownership-style suggestion for a public entrypoint, not a false contract or trust removal. |

All before/after candidate counts are identical because no Rust file changes.
Zero-hit settings do not meet the adoption bar. No setting both removes trust
or rejects a demonstrated defect and requires at most one false positive per
5,000 lines.

The nonzero style findings are:

- fixture_runner.rs:24: `run(FixtureRunConfig)` borrows fields without consuming
  them. Its public owned-argument signature accepts valid callers and does not
  assert a false invariant. Changing the signature is not justified here.
- fixture_runner.rs:129: `missing.clone()` constructs MissingTools after the
  missing-tool log was printed. A move is possible but removes no type escape.
- report.rs:189: `deser_value::from_value(&raw.clone())` clones before a borrowed
  conversion. Removing the copy is not a Types witness. The typed report and raw
  JSON return values have deliberate, separate contracts.
- scenario.rs:827: the test expects ScenarioSourceConfig::Clone. Its wildcard
  panics for the other variant rather than hiding an unhandled successful state.

The unsafe probe also finds cc_bench/mod.rs:364 outside this ticket. That Unix
FFI block belongs to DSLP-6 and is excluded from these counts.

## Escape hatches

The text counter uses `rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('
with the 12 ticket files as arguments. Counts are matching lines, not calls.
The after counts are unchanged: the final diff contains no source or test edits.

| Scope | Before | After |
| --- | ---: | ---: |
| Production | 4 | 4 |
| Tests | 139 | 139 |
| Total | 143 | 143 |

| File under crates/kache-e2e | Production | Tests |
| --- | ---: | ---: |
| src/fixture.rs | 0 | 5 |
| src/fixture_runner.rs | 1 | 0 |
| src/lib.rs | 0 | 0 |
| src/phase.rs | 0 | 0 |
| src/report.rs | 0 | 5 |
| src/result.rs | 0 | 0 |
| src/runner.rs | 1 | 36 |
| src/scenario.rs | 0 | 18 |
| src/scenario_runner.rs | 1 | 4 |
| src/source.rs | 1 | 60 |
| tests/skip_summary.rs | 0 | 4 |
| tests/strict_gate.rs | 0 | 7 |

The four production matches remain for these reasons:

- fixture_runner.rs:89 sets KACHE_DISABLED before the sequential negative-control
  fixture loop. scenario_runner.rs:171 sets an absent KACHE_HOST_CONFIG to an empty
  value at CLI startup. Descendant commands inherit those values. Replacing the
  mutations requires an explicit environment input propagated through command
  producers, not deleting unsafe while keeping the same calls.
- runner.rs:779 allows too_many_arguments on run_phase. Deleting that annotation
  changes no type truth and adds no checker evidence about lifecycle inputs.
- source.rs:356 allows permissions_set_readonly_false only on the non-Unix
  branch. Unix uses owner-write mode bits. make_writable skips symlinks first.
  No portable replacement preserving these branches was established.

The three test unsafe blocks remove, set and restore KACHE_HOST_CONFIG in
scenario_runner's existing environment test. They are not newly introduced.
This audit does not establish safety for multithreaded embedding or arbitrary
concurrent process-environment access. No concurrency reproducer or sanitizer
run was performed. The CLI startup comments and environment-test assumptions
are retained, not promoted to a stronger safety guarantee.

## Boundary and consumer review

| Boundary | Producer and consumers reviewed | Disposition |
| --- | --- | --- |
| Fixture TOML | Fixture::load and discover, token expansion, assertion/verify records, runner::run_fixture | Keep optional verify/diff/modify, defaults and unknown-field rejection. Existing schema tests load shipped fixtures and reject unknown fields/OS values. |
| Scenario selection | Selector::parse, effective_tags, metadata discovery, ScenarioSourceConfig and ScenarioSource, scenario_runner dispatch | Keep open tag strings and substring name selectors. Source kind and lifecycle phases already use enums. |
| CLI | parse_cli, has_bench_options, main, kache-scenario binary | Preserve help versus error results, exit codes, clone-only flag checks, source dispatch and environment inheritance. |
| Report JSON | fetch_since_with_root, parse_report_json, KacheReport/Event/ReportSummary, runner and cc_oracle::exec consumer | Schema version 3 is validated. Fields used by assertions are typed. Raw JSON deliberately forwards additional fields. Event result strings remain open for future event kinds. |
| Results JSON | RunResults, FixtureResult, SkipReason, PhaseResult, fixture_status, fixture runner | Preserve field names, optional skip reasons and open status/phase strings. Closing public string fields into enums would change the accepted API. |
| Phase execution | Phase helpers, run_fixture/run_phase, run_step/run_verify, modified-phase verify selection | Preserve clean/noop distinctions, report/event deltas, missing verification versus empty checks, child exit statuses and failure records. |
| Source lifecycle | RelocatedDir, clone/worktree helpers, snapshot_dir, remove_tree, recursive copying | Keep TempDir ownership, commit/ref handling, fallible filesystem operations, symlink handling and platform permission branches. |

Repository callers of fixture_runner::run lead from scenario_runner::main,
which is called by src/bin/kache-scenario.rs. Report parsing is also consumed by
cc_oracle::exec; that consumer was read, not edited. Integration tests invoke
Cargo's CARGO_BIN_EXE_kache-scenario path and assert skip JSON and exit codes.
The candidate is resolved through this worktree's workspace path dependencies.

Contracts are unaffected because this diff changes only documentation and the
backlog. No runtime, public API, TOML, CLI, cache-key input, store format or
wrapper protocol changes. No fixed contract needs a negative compiler witness.
No performance workload is needed for a documentary change.

No new demonstrated defect or API-retirement request is filed. Next remains
103, preserving main's DSLP-102 finding from DSLP-5. DSLP-7 is removed by the
existing ticket-closure convention. No forbidden path is edited.

## Verification environment

Tools: rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`, Cargo
`1.102.0-nightly (c07ac2c4d 2026-11-06)`, Clippy
`0.1.101 (5b7b37aefc 2026-11-08)`. Four of eight logical CPUs were used.
Python 3.15.2 deduplicated compiler JSON diagnostics and separated source/test
matching-line counts. No dependency, toolchain, CI lane or formatter changed.

Required gate:

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 make check
```

The gate covers the existing lint policy, client-crate graph, resource guard,
workspace library tests and integration tests. macOS/Windows execution and full
compiler-oracle/project/performance campaigns are not claimed.

Gate output:

```text
test-resource guard: all checks passed
test result: ok. 379 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 4.71s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
MAKE_CHECK_EXIT=0
```
