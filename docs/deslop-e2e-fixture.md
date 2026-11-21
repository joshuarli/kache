# Deslop: crates/kache-e2e fixture and scenario runners

Types, Tests and Dead code passes (DSLP-7, DSLP-34, DSLP-61). Scope: `fixture.rs`,
`fixture_runner.rs`, `lib.rs`, `phase.rs`, `report.rs`, `result.rs`, `runner.rs`, `scenario.rs`,
`scenario_runner.rs`, `source.rs`, `tests/skip_summary.rs`, `tests/strict_gate.rs` (about 5,600 lines
before this pass). The bench, oracle and daemon half is in
[deslop-e2e-bench.md](deslop-e2e-bench.md). Same base, host and environment as that document.
Public contracts here: `kache-scenario` flags and exit codes 0, 1, 2, the `results.json` shape
(`FixtureResult.status` is `"pass"`, `"fail"` or `"skip"`, `skip_reason.kind`), and the
`scenario.toml` and fixture schemas that `Fixture`, `BenchProfile` and `ScenarioMetadata` load.

## DSLP-7 Types

Escape hatches (production only; the rest are in `#[cfg(test)]` modules): `unsafe` at
`fixture_runner.rs:89` (`set_var("KACHE_DISABLED")`) and `scenario_runner.rs:171`
(`set_var("KACHE_HOST_CONFIG")`), each with a `SAFETY:` comment that the call precedes any thread;
`#[allow(clippy::too_many_arguments)]` at `runner.rs:779` with a stated reason;
`#[allow(clippy::permissions_set_readonly_false)]` at `source.rs:356`. Production `unwrap()` and
`expect()`: 0. Test code: three `unsafe` env blocks in `scenario_runner.rs`, 124 `unwrap` and 17
`expect`.

Candidates (`cargo clippy --no-deps -p kache-e2e ... -- --cap-lints warn -W <lint>`, this scope only):

| Candidate | Production | With tests | Disposition |
| --- | --- | --- | --- |
| `unwrap_used` | 0 | 124 | Not adoptable alone: the lint is crate-wide and the other half has 13 production hits (DSLP-6). Adopt together once those are converted (finding below) |
| `expect_used` | 0 | 17 | Same |
| `indexing_slicing` | 1 | 12 | Rejected: `runner.rs:891` slices after the `prev_event_count <= len` check on the line above; tests index parsed JSON |
| `as_conversions` | 2 | 2 | Rejected: `report.rs:133` and `runner.rs:1085` are `u64 as f64` for a percentage and rounding |
| `let_underscore_must_use` | 7 | 9 | Rejected as a lint; the sites are best-effort cleanup (`git worktree` commands, `create_dir_all`). One may hide a real failure: `source.rs:45` discards the result of copying a pinned `.cargo` config into the work tree (finding below) |
| `map_err_ignore` | 3 | 3 | Rejected: `lexopt` error replaced by a flag message (`scenario_runner.rs:101,134,138`) |
| `struct_excessive_bools` | 2 | 2 | Rejected: `Fixture` and the CLI `Args` mirror documented flags |
| `unsafe_code` | 2 | 5 | Rejected: process-wide env mutation has no safe form |
| `redundant_clone`, `needless_pass_by_value` | 2, 1 | | Rejected: noise |
| `missing_docs`, `unused_results`, `missing_const_for_fn` | 154, 16, 16 | | Rejected: noise |

No setting adopted; no escape hatch removed or added. Typing findings (not changed; each needs a
`results.json` compatibility decision):

- `FixtureResult.status`, `PhaseResult.status` are `String` for a closed set (`pass`, `fail`,
  `skip`), compared as literals at about 36 sites (`runner.rs` 21, `fixture_runner.rs` 6,
  `result.rs` 5, `tests/strict_gate.rs` 3, `fixture.rs` 1). `PhaseResult.phase` is a `String` that
  `Phase::name()` already produces. A `#[non_exhaustive]` enum with the same wire strings would make
  a misspelled status a compile error (P3).
- `lib.rs::portable_path` is an identity function (`p.to_path_buf()`), a Windows remnant, called at
  9 sites outside its own tests (P4).

## DSLP-34 Tests

Tiers: unit tests in each module (68 before, 74 after for this scope), plus two process-level
tests that spawn `kache-scenario` (`tests/strict_gate.rs`, `tests/skip_summary.rs`). Nothing here
runs a real fixture build: `runner::run_fixture` and `run_phase` need a kache binary and a
toolchain, so the phase loop is exercised only by `make e2e` in CI (Scenario tier).

Contracts and guards (`crates/kache-e2e/`):

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Exit 1 when every selected fixture skipped | `tests/skip_summary.rs:4` | asserted; H1 caught |
| `--deny-missing-tools`: exit 2 for a missing tool, OS skips allowed, default lenient | `tests/strict_gate.rs` (3 tests) | asserted |
| `results.json` skip shape (`status`, `skip_reason.kind`, tools, `current_os`) | `tests/strict_gate.rs` | asserted |
| Selector grammar: tag, `name:` substring, source-kind alias, all-must-match, never empty | `selectors_match_all_requested_tags`, `selectors_support_name_filter_and_source_alias`, new `selectors_are_trimmed_and_never_empty`, `the_source_kind_alias_selects_only_that_kind` | G1 to G3 caught; G11 survivor now caught; G12b caught |
| Default tier and source tags | `fixture_gets_gate_and_source_tags_by_default`, `clone_gets_nightly_and_source_tags_by_default`, `explicit_tier_replaces_default_tier` | G2 caught |
| Scenario metadata discovery | `discovers_scenario_metadata_from_shared_root` | asserted |
| Gate tables bind per phase | `scenario.rs` `pull_phase_gates_bind`, `warm_same_tree_phase_gates_bind`, `scenario_checks_reject_misspelled_fields` | asserted |
| Fixture and bench schemas load, reject unknown fields | `fixture.rs:515,542,573`, `bench_profile.rs` (other doc) | asserted |
| Token expansion in fixture commands | `fixture.rs:587..618` | asserted |
| Phase flags: which phases clean first and verify | `phase.rs` (2 tests, covered only `Pull` and `WarmSameTree`) | G9 survivor; new `only_the_noop_phases_skip_the_clean_...` covers all eight |
| Fixture status: pass only when every phase passed | `result.rs` had no test | G6 survivor; new `a_fixture_passes_only_when_every_phase_passed` |
| Report delta: hits, dups, misses, hit rate, no NaN | `report.rs:232` covered counts only | G5 survivor; new `the_delta_hit_rate_is_hits_over_all_crates...` |
| Report schema version gate | `report.rs:258..335` | asserted |
| Per-phase measurements and warning thresholds | none | **was `none`**, new `measurement_warnings_fire_strictly...`, `phase_measurements_report_counts...`; R1 to R3 |
| Speedup against cold, floor warning, zero-second phases | none | **was `none`**, new `speedup_is_cold_wall_over_phase_wall...`; R4, R7 |
| `[modify]` edit replaces all occurrences and refuses a no-op | none | **was `none`**, new `a_modification_replaces_every_occurrence...`; R5, R6 |
| Reference build with the cache disabled, relocate-modified verify, compiler shims | `runner.rs:1766..1875` | asserted |
| Key-trace parsing and divergence report | `runner.rs:1503..1591` | asserted |
| Artifact resolution and restored dep-info check | `runner.rs:1623..1685` | asserted |
| Source materialisation, worktrees, relocation, symlinks | `source.rs:399..640` | asserted |
| Host layer off for scenario children, benchmark option parsing | `scenario_runner.rs:329,366,394` | asserted |
| OS and libc tokens, skip counting | `fixture_runner.rs:272,294,302` | asserted |
| Negative-control verdict (`VACUOUS`, `falsifiable`, exit 1) | none: decision lives inside `fixture_runner::run` next to `process::exit` | gap, finding below |

Hand-injected faults (`cargo-mutants` is not installed), same harness and command as the bench
document (`cargo test -p kache-e2e --lib -q`, `H1` with `--test skip_summary --test strict_gate`),
file restored and `cmp`-checked after each:

| Mutant | Before | After |
| --- | --- | --- |
| G1 `name:` as equality | caught (`bench_profile` discovery tests) | |
| G2 default tier inverted | caught (`scenario.rs` tag tests) | |
| G3 selectors `all` to `any` | caught | |
| G4 delta dups not subtracted | caught (`delta_since_keeps_hits_dups_and_misses_separate`) | |
| G5 delta rate at one crate | **survived** (exit 0) | caught, exit 101 |
| G6 `fixture_status` `all` to `any` | **survived** | caught |
| G7, G8 `min` and `max` comparison bounds | caught (`assertions.rs`) | |
| G9 relocate no-op cleans first | **survived** | caught |
| G10 `runs_verify` always true | caught | |
| G11 blank selector accepted | **survived** | caught |
| G12 alias selector (first form did not build under `warnings = "deny"`; second form `G12b`) | | caught |
| R1, R2 warning bounds `>`/`<` to `>=`/`<=` | no test | caught |
| R3 rate rounded to two places | no test | caught |
| R4 speedup inverted | no test | caught |
| R5 no-op edit allowed | no test | caught |
| R6 replace first occurrence only | no test | caught |
| R7 zero cold wall accepted | no test | caught |
| H1 all-skipped check disabled | caught (`a_selection_whose_fixtures_all_skip_fails`, exit 101) | |

Survivors: 4 behavior survivors found (G5, G6, G9, G11), 4 killed. Lib tests: 371 before the
pass, 389 after (12 added to the bench half, 9 here, 3 removed with the dead schema); wall time
is dominated by host load (6.0 s to 22.7 s for the same binary), so the budget claim is by
construction: the new tests are tempdir and parse only.

Needs environment: `cargo-llvm-cov`, `cargo-mutants` (DSLP-87). The phase loop in `run_phase`
(about 250 lines) and the negative-control branch are not reachable without a kache binary and
are covered only by `make e2e` in CI.

## DSLP-61 Dead code

Commands: `cargo clippy -p kache-e2e --all-targets -- -D warnings` (exit 0 before and after) and
`.work/scripts/pubrefs.py crates/kache-e2e`, which counts references to every `pub` item outside
its definition, split into production and test code across the workspace. Six items had no
production reference:

| Item | Production refs | Test refs | Disposition |
| --- | --- | --- | --- |
| `result::collect_meta` | 0 | 0 | Unreferenced (workspace-wide `rg -w` finds only the definition) |
| `Selectors::matches_scenario`, `Scenario::from_fixture`, `Scenario::from_bench_profile` | 0 | 0 | Unreferenced |
| `ScenarioConfig::phases_or_default`, `ScenarioConfig::into_scenario` | 0 | 1 each | Test only |

The first three lead to the same cluster, which was removed in `3ada8021`: `Scenario`,
`ScenarioSource`, `ScenarioCommands`, `ScenarioConfig`, `ScenarioCommandsConfig`,
`default_phases`, `fixture_phases` and `Selectors::matches_scenario` (about 270 lines) in
`scenario.rs`, plus three tests that deserialized `ScenarioConfig` and nothing else
(`parses_unified_fixture_scenario_config`, `parses_unified_clone_scenario_config`,
`scenario_source_config_parses_ref_next`; `ref_next` is still covered by
`bench_profile::source_ref_next_parses_and_marks_pull`). Evidence per removal:

- Entry points and registrations: `Cargo.toml` `[[bin]]` entries and `Makefile` goals build against
  the library; none names these types. `rg -w` over the workspace (`.rs`, `.toml`, `.yml`, `.md`,
  scripts, excluding `.work` and `target`) finds the names only in `scenario.rs`.
- Consumers: `kache-e2e` is `publish = false`; its only consumers are its six binaries.
  `ScenarioSourceConfig` is not removed: `discover_metadata` uses it. `ScenarioChecks` stays: fixtures
  and bench profiles use it.
- Platforms and features: crate has no features; all targets are built with
  `cargo clippy -p kache-e2e --all-targets -- -D warnings`, which passed after the cut.
- Consumer tests: `cargo test -p kache-e2e` after the cut: 389 lib tests and the two integration
  files pass (3 lib tests removed with the code).

Left as is: `result::collect_meta` (unreferenced, 11 lines; its doc says the binary builds the
metadata, and `fixture_runner::run` builds `RunResults` inline, so it is dead, but I did not remove
it in the same commit as the schema; finding below). Dependencies: every `[dependencies]` entry is
still used in non-test code.

Needs environment: `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat`.

## Findings filed by this component

- P3: convert the 13 production `unwrap()` and 3 `expect()` in `cc_bench/mod.rs`, `cc_oracle/gcov.rs`,
  `bench_profile.rs` and `bench_runner.rs`, then deny `unwrap_used` and `expect_used` crate-wide with
  `allow-unwrap-in-tests` (the pattern adopted in `crates/kache-config`).
- P3: type the `results.json` statuses (`pass`, `fail`, `skip`) and phase names.
- P3: extract the negative-control decision in `fixture_runner::run` (want-pass, passed, verdict,
  exit code) into a pure function; it is untested because it sits beside `process::exit`.
- P4: `result::collect_meta` and `lib.rs::portable_path` are vestigial.
- P4: `source.rs:45` and `:60` discard the result of copying a pinned `.cargo/config.toml` into a
  fixture work tree; a failed copy silently builds without the pin.
