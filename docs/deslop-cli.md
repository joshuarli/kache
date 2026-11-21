# Deslop: src/cli.rs

Scope: `src/cli.rs` (7,174 lines), base `ab5351ba` (its parent `11cbdba8`),
macOS arm64, nightly-2026-11-09. Three typing escape hatches removed in three
commits (`685b8085`, `7a3749e9`, `8285450b`); no flag, output or exit code
changed. Host note and the `RUSTC_WRAPPER=` bypass are in
[deslop-cli-submodules.md](deslop-cli-submodules.md).

## DSLP-18 Types

Commands: the two `cargo clippy -p kache --lib [--tests] --message-format=json
-- -W warnings -W clippy::... -W unreachable_pub -W unsafe_code` runs listed in
the submodule doc, both exit 0. Counts for `src/cli.rs`, production (lib
target) before the commits, after them, and test-only code after:

| Candidate | Before | After | Test-only | Verdict |
| --- | --- | --- | --- | --- |
| `unwrap_used` | 2 | 0 | 49 | Adopted by fix: `last_miss.unwrap()` (why-miss) and `service_path.as_ref().unwrap()` (init) removed. The 49 are the `shim_install_tests` module; rejected there. |
| `expect_used` | 5 | 3 | 0 | Two removed: `provenance.as_ref().expect(..)` (doctor) and the `position(..).expect(..)` that recovered `last_miss`' index. The remaining 3 are `failures.lock().expect("scrub mutex")` (poisoned-lock invariant); rejected. |
| `wildcard_enum_match_arm` | 9 | 9 | 0 | Rejected: `_` over `CacheClean`, `HostConfigStatus`, `CargoWrapperPlan` and the cache-fs verdict, where the arm names "everything else" on purpose; no test or defect shows a variant absorbed. |
| `cast_possible_truncation` | 5 | 5 | 0 | Rejected: `Duration::as_millis()` (`u128`) into `u64` duration fields (about 585 million years) and one percentage computed in `u128`; no defect. |
| `cast_sign_loss` | 3 | 3 | 0 | Rejected: SQLite `i64` sizes and row ids clamped with `.max(0)` before the cast (2), and a counter that starts at 0 and only increments (1). |
| `needless_pass_by_value` | 3 | 3 | 0 | Rejected: `ReportFilter`, `PathBuf` and `RunOutcome` parameters are owned for the callers' convenience; the lint's fix changes signatures, not types. |
| `redundant_clone` | 1 | 1 | 0 | Rejected: `machine.clone()` at the last use of `machine` (`cli.rs:603`) is an allocation, not a contract; not an escape hatch, left alone. |
| `unreachable_pub` | 24 | 24 | 0 | Rejected: style, 24 per 7k lines. |
| `unsafe_code` | 0 | 0 | 1 | Test `geteuid` root guard only. |

Adopted profile: none as a standing lint (each candidate is above one false
positive per 5k lines or has no witness); the three real hatches were fixed by
types. Escape hatches
(`rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('`): 54 before, 50
after (unwrap 48 to 46, expect 5 to 3); not increased.

Fixed contracts and witnesses:

- why-miss with no miss event, and with a miss, now derives the event and its
  index from one `enumerate().rev().find(..)`. Witnesses:
  `why_miss_leads_with_a_store_failure_when_there_was_one` and
  `why_miss_prioritizes_same_key_lookup_rejection` (`src/cli/tests.rs:2194`,
  `:2220`), `why_miss_on_unknown_crate_succeeds`
  (`tests/cli_commands_test.rs:1126`), `why_miss_reports_key_mismatch_after_source_edit`
  (`:3111`).
- doctor holds `Option<(Config, ConfigFileProvenance)>`, so a loaded config
  without provenance cannot be expressed. Witness:
  `doctor_reports_reachable_daemon_version` (`tests/cli_commands_test.rs:1708`)
  reaches the daemon-config check that needed the provenance.
- init binds `installed_service_path`; `service_installed` is
  `installed_service_path.is_some()`, which equals the old
  `service_path.is_some_and(exists)`. Witness: none for the
  `Login service: configured (<path>)` line on this host; see Tests.

Proof: `cargo test -p kache --lib -- cli:: compiler::` exit 0 (746 passed, 2
ignored); `cargo test -p kache --test cli_commands_test -- doctor why_miss
init` exit 0 (57 passed, 199.7s); `cargo clippy -p kache --lib --tests -- -D
warnings` exit 0.

## DSLP-45 Tests

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed
(DSLP-87), so no coverage numbers and no survivor list. The audit below is by
hand.

Tiers: 195 unit tests in `src/cli/tests.rs` over pure rendering and decision
functions (cheap, in-process), 15 inline tests in `cli.rs`, and
`tests/cli_commands_test.rs` driving the real binary with a temporary `HOME`
(the exit-code and output boundary).

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| CLI flags and exit code 2 on argument errors | `tests/cli_commands_test.rs:293` `argument_errors_exit_two` | Asserted. |
| Only `doctor --fix` replaces a stale daemon | `src/cli/tests.rs:383` `only_fix_restarts_a_stale_daemon` | Asserted (full truth table). |
| Doctor optionality and issue classification | `cli/tests.rs:284` `doctor_check_optionality_truth_table`, `:1666` `test_is_doctor_issue`, `:2833` `doctor_groups_each_label_by_its_component` | Asserted. |
| why-miss cause precedence and JSON/text parity | `cli/tests.rs:2194`, `:2220`, `:2240`; `tests/cli_commands_test.rs:3733`, `:3749` | Asserted. |
| Cache clean decides from what it counted; previews never remove | `cli/tests.rs:3748`, `:3629`, `:3674` | Asserted. |
| init plan, backups and result facts | `cli/tests.rs:3996` `init_steps_follow_the_plans`, `:4024`, `:4043` | Asserted. |
| init prints `Login service: configured (<path>)` for an installed, current service | none; the existing service tests (`tests/cli_commands_test.rs:2299` onward) are Linux-only fake-systemd runs and assert the install, skip and update lines only | Gap, not provable on this macOS host; filed below. |

Injected fault, `should_restart_stale_daemon`: `fix_requested && daemon_is_stale`
changed to `||`. `cargo test -p kache --lib -- cli::tests::only_fix_restarts`
exit 101, `cli::tests::only_fix_restarts_a_stale_daemon` FAILED at
`src/cli/tests.rs:386` (`"plain doctor"`). Restored from a copy; `cmp` exit 0,
file back at the committed content.

## DSLP-72 Dead code

- rustc `dead_code` clean at `-D warnings` for `--lib` (production) and
  `--lib --tests`.
- Workspace-wide caller search (`rg -w NAME src crates tests`) for all 45
  visibility-qualified items in `cli.rs`. Least referenced:
  `record_session` (one call, `cli.rs:1132`), `DURABILITY_FLUSH_BATCH`
  (`src/daemon.rs:724`), `fetch_stats_snapshot` (`cli.rs:481`),
  `telemetry_write` (`src/main.rs:832`), `run_auto_gc_workers`
  (`cli.rs:3138`), `command_args` (`cli.rs:3528`, `:6129`). None unreferenced.
- The one `#[cfg(test)]` helper, `render_stats` (`cli.rs:684`), has consumers in
  `src/cli/tests.rs`, `src/store_view/tests.rs` and the CLI integration test;
  kept.
- `#[allow(`/`#[ignore]`: none in the file.
- Needs environment: `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat`.

Disposition: nothing removed.
