# Deslop: crates/kache-config

Types, Tests and Dead code passes (DSLP-3, DSLP-4, DSLP-5) over `crates/kache-config`
(5,564 lines: `config.rs` 2,469, `config/tests.rs` 2,631, `invocation.rs` 384, `policy.rs`
39, `testing.rs` 31, `lib.rs` 10). Base `11cbdba8`, branch `deslop/k1`, macOS arm64,
nightly-2026-11-09, run with `TMPDIR=$PWD/.work/tmp CARGO_BUILD_JOBS=4` on a host at load
40 to 60. Scratch scripts live in `.work/scripts/` (not committed).

## DSLP-3 Types

Escape hatches, counted with `rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('`:

| File | Count | Where |
| --- | --- | --- |
| `testing.rs` | 2 | two `unsafe` blocks around `std::env::set_var`/`remove_var` (edition 2024) |
| `config.rs` | 11 | all inside `#[cfg(test)]` modules (from line 2311) |
| `invocation.rs` | 19 | all inside `#[cfg(test)]` (from line 202) |
| `config/tests.rs` | 233 | tests |

Production code has no `unwrap`, `expect`, `allow`, `cast`-truncation or indexing. The only
`unsafe` is the test-support environment guard, which has no safe equivalent. Counts are
unchanged by this pass.

Candidates, measured with `.work/scripts/strict.sh kache-config prod|all --features test-support`
(`cargo clippy --no-deps -p kache-config ... -- -W <lint>`; unique lint and line):

| Candidate | Production | With tests | Disposition |
| --- | --- | --- | --- |
| `clippy::unwrap_used` | 0 | 262 | **Adopted**, tests exempt through `clippy.toml` |
| `clippy::expect_used` | 0 | 3 | **Adopted**, same |
| `cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap` | 0 | 0 | Nothing to adopt |
| `wildcard_enum_match_arm`, `match_wildcard_for_single_variants` | 0 | 0 | Nothing to adopt |
| `indexing_slicing`, `panic` | 0 | 6, 6 | Tests only; rejected |
| `as_conversions` | 4 | 4 | Rejected: `usize as u64` and a `repr` discriminant `as u8` in the provenance hash (`config.rs:1758-1768`); infallible |
| `unsafe_code` | 2 | 21 | Rejected: env mutation in `testing.rs` has no safe form |
| `rc_buffer` | 1 | 1 | Rejected: `Arc<Vec<_>>` in `InvocationEnv` is mutated by `Arc::make_mut` plus `push` |
| `struct_excessive_bools` | 2 | 2 | Rejected: `Config` and `EnvOverrides` mirror the documented settings |
| `needless_pass_by_value` | 2 | 2 | Rejected: owned parameters stored or consumed once |
| `missing_const_for_fn`, `unused_results`, `missing_docs` | 3, 19, 80 | | Rejected: noise |
| `redundant_clone` | 0 | 3 | Rejected: tests only (`tests.rs:355,850,1285`) |

Adopted profile: `crates/kache-config/Cargo.toml` gains `[lints.clippy] unwrap_used = "deny"`,
`expect_used = "deny"`; `crates/kache-config/clippy.toml` sets `allow-unwrap-in-tests` and
`allow-expect-in-tests`. False positives: 0 in 2,933 production lines. Negative control: an
injected `value.clone().unwrap()` in `policy.rs` made
`cargo clippy -p kache-config --all-targets --features test-support -- -D warnings` exit 101
with `used unwrap() on an Option value`; restored, `cmp` exit 0. Commit `2d7ed09e`.

Typing observations, not changed:

- `Result<FileConfig>` is passed to seven helpers that only call `.as_ref().ok()`
  (`ignore_env_enabled`, `explain_miss_enabled_for`, `trust_daemon_execution_for`,
  `load_volume_stores`, `ProjectRules::from_file_config`, `load_resolved_for`). The error is
  reported separately through `unusable_config_file`. `Option<&FileConfig>` states that
  contract; filed as a P4 finding because the change touches the public `Config::load_*`
  path and has no defect to witness.
- Doc comments are misplaced: the paragraph "A short name for this job's pull request
  prefix" sits on `index_db_path`, and "Whether the opt-in too-new-input guard is active"
  sits on `ignore_env_enabled`.

## DSLP-4 Tests

Tiers: all 116 tests are crate-local unit tests (`config/tests.rs`, `invocation.rs`,
`policy.rs`, `config.rs::invocation_tests`); no integration tier exists in the crate. The
workspace tier is the root crate's use of `kache_config::config::*`. Run time is 0.42 s for
112 tests on an idle host; 8.8 s for 116 tests at load 40, dominated by scheduling, not the four new
single-tempdir tests.

Contracts and guards (file `crates/kache-config/src/`):

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Precedence: env beats chosen file beats host file beats default | `config/tests.rs:628` the_environment_beats_both_files, `:607` the_chosen_file_beats_the_host_key_by_key, `:1478` key salt, `:1551` cc flags | asserted; M2 caught |
| `ignore_env` locks the file in; `KACHE_DISABLED` is never gated | `:428`, `:792` a_host_ignore_env_cannot_silence_the_environment, `:2687` | asserted; M2, M15 caught |
| Env var table complete and consistent with the schema | `:940`, `:986` | asserted |
| Host layer: merge, atomic `volumes`, excluded keys, bad file ignored | `:684`, `:741`, `:769` | asserted; M7, M16 caught |
| Provenance fingerprint tracks bytes, absence, symlink retarget, host file | `:467`, `:513`, `:830`, `:1046` | asserted |
| Volume routing: longest prefix, dedupe, invalid skipped, shard budget | `:1132`, `:1102`, `:1167`, `:1253` | asserted; M1 caught |
| Disk-share budget floor, cap, rounding | `:316` | asserted; M3 caught |
| Trust-domain label and isolation | `:104`, `:131` | asserted; M4 caught |
| Bypass rules (crate exact, argv substring, env value, blank never matches) | `:1982`, `:2118` | asserted; M5 was a survivor, now caught |
| Project rule lists trimmed, blanks dropped | `:2650` (new) | M12 survivor, now caught |
| `key_env_vars` upper-cased, sorted, deduplicated; interior `*` kept | `:2671` (new), `:1678` | M13 survivor, now caught |
| `base_dirs` absolute, normalized, no `..`, not a root | `:247`, `:261` | asserted; M11 caught |
| Socket override absolute and not a directory | `:1922` | asserted; M6 caught |
| Invocation snapshot: last duplicate wins, `set` replaces, child gets only its env, cwd rooted | `invocation.rs:230,334,386` | M8, M9 survivors, now caught; M14 caught |
| `update_file_config_at` drops comments and unknown keys; byte-identical only when before equals after | `:2435`, `:2526` | previously false passes (see below), rewritten; M17 caught |
| CI detection | `policy.rs:18` | asserted |
| Unusable file named with line | `:2420`, `:2552` | asserted |
| `default_cache_executables` | `:95` | weak: compares against `cfg!` instead of `true`; `scripts/check-docs.sh` pins the literal body. Left |

False passes fixed (commit `c5ae6a43`): `update_file_config_keeps_comments_and_unknown_keys`
asserted neither comments nor unknown keys (and the function drops them, as its doc says);
`update_file_config_leaves_an_unchanged_file_byte_for_byte` never compared bytes and passed a
changed `after`. `test_shellexpand` only asserted the result lacked a `~/` prefix; it now pins
`home.join("foo")`.

Hand-injected faults (`cargo-mutants` is not installed; 17 mutants applied by
`.work/scripts/mut.sh`, which restores the file and checks `cmp` after each run;
command `cargo test -p kache-config --features test-support --lib -q`):

| Mutant | Before | After the new tests |
| --- | --- | --- |
| M1 `max_by_key` to `min_by_key` in `match_volume_store` | caught, `match_volume_store_prefers_the_tightest_root` | |
| M2 invert `env_or_ignored_for` | caught | |
| M3 drop the disk-share floor | caught | |
| M4 `>` to `>=` on trust-domain length | caught | |
| M5 drop `!rule.is_empty()` on crate rules | **survived** (exit 0) | caught by `blank_bypass_entries_never_match`, exit 101 |
| M6 drop `is_absolute` for the socket path | caught | |
| M7 recurse into atomic host tables | caught | |
| M8 first duplicate wins in `var_os` | **survived** | caught by `a_later_duplicate_wins_and_set_replaces_every_earlier_entry` |
| M9 `set_os` keeps earlier entries | **survived** | caught, same test |
| M10 drop the absolute-cwd filter in `resolve` | equivalent: the final `resolved.is_absolute()` check enforces it. Filter is redundant, left | |
| M11 drop the absolute check on `base_dirs` | caught | |
| M12 keep blank rule entries | **survived** | caught by `project_rules_are_trimmed_and_drop_blank_entries` |
| M13 do not upper-case `key_env_vars` | **survived** | caught by `key_env_var_patterns_are_upper_cased_deduplicated_and_sorted` |
| M14 drop `env_clear` in `command` | caught | |
| M15 `KACHE_DISABLED` accepts only `1` | **survived** | caught by `kache_disabled_accepts_one_and_true_in_any_case` |
| M16 drop the `workspace` host exclusion | caught | |
| M17 `update_file_config_at` always writes | not run on the old test (its assertions never read bytes) | caught by `update_file_config_leaves_an_unchanged_file_byte_for_byte` |

Survivors: 6 found, 6 killed, 1 equivalent (M10). The first run's M10 was an invalid
mutant (build error) and is the equivalent case above by inspection, not by run.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` (DSLP-87), so line and branch
coverage and the scoped mutation lane (`make mutants-diff`) are unmeasured; the 17 mutants
above are a hand-picked sample of the critical decisions, not a sweep. `.cargo/mutants.toml`
still excludes `run_config_editor` and `run_monitor`, which no longer exist.

## DSLP-5 Dead code

Commands: `cargo clippy --no-deps -p kache-config --all-targets <features> -- -W dead_code
-W unused -W unreachable_pub` for no features, `file-write`, `test-support`: no warnings in
any combination (exit 0). `-W unused_crate_dependencies` reports nothing; by hand,
`kache-log`, `rustc-hash` and `tempfile` are dev-only and used in tests, and every normal
dependency is named in `config.rs`, `invocation.rs` or `testing.rs`. No private function is
dead.

Needs environment: `cargo-machete` and `cargo +nightly udeps` are not installed; the manual
dependency check above stands in.

Public items with no consumer outside the crate (workspace-wide `rg -w`, excluding `.work`
and `target`; the root crate reaches the crate through `pub(crate) use kache_config::config::*`
in `src/config.rs`, `src/invocation.rs:28`, `src/cache_key.rs:5619`; `kache-client` uses
`InvocationEnv` and `Config::load_for`). `kache-config` is publishable
(`crates/kache-config/Cargo.toml` has no `publish = false`), so these are consumer questions,
not cuts:

| Item | Callers outside the crate | Inside the crate |
| --- | --- | --- |
| `Config::update_file_config_at`, feature `file-write`, `Config::save_file_config_to` | none (the root crate enables `file-write` and never calls them) | tests only |
| `EnvOverrides`, `EnvOverrides::detect` | none | one test (`:1711`) |
| `Config::load_raw_file_config_from` | none (`#[cfg(test)]`) | tests only |
| `Config::user_bypass_reason` | none (callers use `ProjectRules::user_bypass_reason_for`) | `:2078` |
| `Config::cache_dir_for_path` | none | none (no reference at all) |
| `user_config_path`, `host_config_path_for`, `config_file_fingerprint`, `Config::volume_store_for` | none | tests only |
| `DEFAULT_DAEMON_IDLE_TIMEOUT_SECS`, `DEFAULT_GC_MAX_AGE_HOURS`, `DEFAULT_MIN_STORE_COMPILE_MS` | none | none (no reference at all) |
| `DEFAULT_RECOVERY_FLOOR` | none | `default_recovery_floor`, tests |
| `default_cache_executables` | none (read textually by `scripts/check-docs.sh`) | `:1315`, `:95` |
| `parse_size_checked`, `sanitize_trust_domain`, `describe_toml_error`, `warn_inert_path_only_env_vars`, `running_in_ci_with`, `expand_exclude_pattern*` | none | used internally |

No removal made: nothing here is proven dead private code, and the unused public set looks
like the remains of a config editor (the stale `.cargo/mutants.toml` comments name
`run_config_editor`). Walk-back: decide whether `file-write`, `EnvOverrides` and the
`FileConfig` writers are supported API; if not, delete them in one commit with their tests.
