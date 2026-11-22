# Deslop: crates/kache-e2e bench, cc oracle, daemon

Types, Tests and Dead code passes (DSLP-6, DSLP-33, DSLP-60). Scope: `assertions.rs`,
`bench_host.rs`, `bench_otlp.rs`, `bench_profile.rs`, `bench_runner.rs` and
`bench_runner/tests.rs`, `cc_bench/*`, `cc_bench_cli.rs`, `cc_oracle/*`, `cc_oracle_cli.rs`,
`daemon.rs`, `disk_usage.rs` and the six `src/bin/*.rs` (about 18,800 lines). The other half of the
crate is in [deslop-e2e-fixture.md](deslop-e2e-fixture.md). Base `11cbdba8`, branch `deslop/k1`,
macOS arm64, nightly-2026-11-09, `TMPDIR=$PWD/.work/tmp CARGO_BUILD_JOBS=4`, host load 30 to 60.
`kache-e2e` is `publish = false`; its only consumers are its own six binaries.

## DSLP-6 Types

Escape hatches in scope (`rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('` on the file list):
production `unsafe` at `cc_bench/mod.rs:364` (`getrusage`) and `cc_bench/activity.rs:148`
(`host_statistics`, with `#[allow(deprecated)]` at `:142`), each with a `SAFETY:` comment and a
platform syscall that has no safe std form; seven `#[allow(clippy::too_many_arguments)]` on
`bench_runner.rs` functions, each with a stated reason; 13 `unwrap()` and 3 `expect()` in
production code, the rest in tests. No count changed in this pass (new tests add test-only
`unwrap`).

Candidates, measured with `cargo clippy --no-deps -p kache-e2e --lib --bins -- --cap-lints warn -W <lint>`
(unique lint and line, production; `--all-targets` adds the tests). Counts are for this scope:

| Candidate | Production | With tests | Disposition |
| --- | --- | --- | --- |
| `unwrap_used` | 13 | 461 | Rejected: 3.4 per 5k lines exceeds the 1-per-5k bar. Every hit is an invariant (`file_name()` of a path the caller built, `try_into()` on a slice just bounded, `tree.parent()`); none is reachable from input |
| `expect_used` | 3 | 34 | Rejected, same reasons |
| `indexing_slicing` | 49 | 228 | Rejected: ELF and `argv[1..]` reads after length checks |
| `as_conversions` | 49 | 56 | Rejected: `u128 as i64` nanoseconds, `f64 as u64` rounded ms, byte-order reads |
| `cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap` | 8, 6, 1 | 10, 6, 1 | Rejected, each inspected: `exec.rs:58,62` (`deadline - now` is non-negative after the `now > deadline` return), `cc_bench_cli.rs:109,111` (`--pairs` bounded 5..=10 afterwards, `--jobs` is a count), `cc_bench/mod.rs:369-371` (`rusage` fields), `bench_otlp.rs:151` (rounded non-negative MiB), `gcov.rs:174` (checked add) |
| `wildcard_enum_match_arm` | 5 | 5 | Rejected: two on `lexopt::Arg` (external enum), three `_` arms in `exec.rs:560,626` and `judge.rs:212` meaning "every other arm" or "every other reuse mode" |
| `map_err_ignore` | 3 | 3 | Rejected: `lexopt` errors replaced by a flag-specific message |
| `needless_pass_by_value`, `redundant_clone`, `struct_excessive_bools` | 8, 7, 1 | 9, 11, 1 | Rejected: owned by design or noise (`redundant_clone` at `bench_runner.rs:671,673,690`, `cc_bench/mod.rs:271,277`, `exec.rs:487,494`) |
| `unnecessary_wraps` | 2 | 2 | Rejected: `bench_runner.rs:1304`, `disk_usage.rs:124` keep a uniform signature |
| `unreachable_pub` | 16 | 20 | Rejected as a lint (nominal `pub` inside private modules `bench_otlp`, `bench_host`, `cc_oracle::{gcov,report}`); the items are used (see DSLP-60) |
| `unsafe_code` | 2 | 2 | Rejected: syscalls above |
| `missing_docs`, `unused_results`, `missing_const_for_fn`, `let_underscore_must_use` | 283, 61, 23, 15 | | Rejected: noise |

No setting adopted for this scope; no typing escape hatch removed. Typing findings, filed rather
than changed because each touches serialized contracts or needs a design pass:

- `cc_oracle::judge::Verdict` and `Arm` matches use `_` for "every other arm"; a new `Arm` would be
  silently handled as a kache-like arm in `exec.rs:626`. Needs an exhaustive match (P4).
- The harness binaries share stringly flags with `lexopt` and return `Result<_, String>` from the
  parsers (`cc_bench_cli::parse`, `cc_oracle_cli::parse`); a typed error would not change behavior (P4).

## DSLP-33 Tests

Tiers: unit tests in every module (371 lib tests before this pass, 6.0 s warm; 383 after), no
integration tier for this half (the two `tests/*.rs` files drive `kache-scenario` and belong to the
fixture doc). Six binaries have no tests of their own; `kache-cc-bench` and `kache-cc-oracle` are
thin over `cc_bench_cli::main` and `cc_oracle_cli::main`, `kache-cc`/`kache-cxx` over
`run_kache_compiler_shim` (no test; needs a sibling `kache` binary), `e2e-fallback` is a 59-line
forwarder (no test).

Contracts and guards (`crates/kache-e2e/src/`):

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Oracle verdict ladder: harness error, nondeterminism, wrong output, stale hit, missed reuse | `cc_oracle/judge.rs:386,397,404,426,463,471,477,487` | asserted; E9 to E12 caught |
| Oracle gate exit, cohort selection, controls | `cc_oracle/mod.rs:346,393,412,444`, `judge.rs:623` | asserted |
| Oracle case schema validation | `cc_oracle/case.rs:395..547` | asserted |
| Raw compiler execution, shims, fault injection, response files | `cc_oracle/exec.rs:881..1166` | asserted |
| ccache pin per lane | `cc_oracle/pin.rs` (2 tests, includes the committed `lock.toml`) | asserted |
| `kache-cc-oracle` flags | `cc_oracle_cli.rs:149,158,172` | asserted |
| `kache-cc-bench` flags, pair budget 5..=10, usage exit 2, help exit 0 | `cc_bench_cli.rs` new `defaults_take_five...`, `help_and_every_malformed...`, `the_pair_budget...`, `usage_errors_exit_two...` | **was `none`**, now asserted; E1 |
| `kache-cc-bench --acceptance` exit codes 0, 1, 3 | `cc_bench_cli.rs` new `acceptance_exits_zero_only_when...` | **was `none`**, now asserted; E2 |
| Locked estimator: geomean, t-interval, verdict rule | `cc_bench/stats.rs:99,109,118` | boundary was unasserted (E3, E4); new `the_verdict_boundaries...` |
| Locked seed stream | `stats.rs:129` only checked self-consistency | new `the_locked_stream_is_splitmix64` pins the vectors; E13 |
| t-table covers 2..=10 pairs | | new `the_t_table_covers_exactly_two_to_ten_pairs`; E14 |
| Acceptance verdict and eligibility | `cc_bench/mod.rs:1126..1201` | asserted |
| Workload set and edit targets | `cc_bench/workload.rs` had no tests | new `the_comparison_set...`, `generated_corpora_contain...`, `an_edit_appends...`, `outputs_are_the_sorted...`; E15, E16, E18 |
| Host activity eligibility | `cc_bench/activity.rs:210..279` | asserted |
| OTLP units and attribute allowlist | `bench_otlp.rs:668..1196` | asserted; E8 caught |
| Bench profile schemas and shipped profiles | `bench_profile.rs:528..` | asserted |
| Host facts | `bench_host.rs:469..873` | asserted |
| Disk accounting | `disk_usage.rs:149..269` | asserted |
| Daemon drain | `daemon.rs:149,182,209,222` | asserted |
| Phase assertions (`min`, `max`, noop, diff) | `assertions.rs:412..728` | asserted; G7, G8 below |
| Bench runner | `bench_runner/tests.rs` (108 tests) | not sampled; see Needs environment |

Hand-injected faults (`cargo-mutants` is not installed). `.work/scripts/mut.sh` applies one
substitution, runs `cargo test -p kache-e2e --lib -q`, restores the file and checks `cmp`. Exit 101
means a test failed:

| Mutant | Result before | Result after |
| --- | --- | --- |
| E1 `5..=10` to `5..10` in `cc_bench_cli::parse` | no test existed | caught (`the_pair_budget_is_five_to_ten_inclusive`) |
| E2 exit 3 on `Fail` instead of `Ineligible` | no test existed | caught (`acceptance_exits_zero_only_when_every_workload_passes`) |
| E3 `high <= target` to `<` | **survived** (exit 0) | caught (`the_verdict_boundaries_...`, exit 101) |
| E4 `low > target` to `>=` | **survived** (exit 0) | caught, same test |
| E5 t-table index `n - 2` to `n - 1` | caught (`the_interval_is_on_log_ratios...`) | |
| E6 variance divisor `n - 1` to `n` | caught, same test | |
| E7 median parity | caught (`the_seeded_order_is_reproducible`) | |
| E8 MiB as 10^6 | caught (`cache_size_is_bytes_not_megabytes`) | |
| E9 to E12 judge: stale hit, missed reuse, nondeterminism, raw status | caught (`judge.rs` tests named above) | |
| E13 splitmix constant | not run on the old tests: they assert only determinism and that orders vary | caught (`the_locked_stream_is_splitmix64`) |
| E14 t-table shortened | not run on the old tests: none counts the table | caught (`the_t_table_covers_exactly_two_to_ten_pairs`) |
| E15 wrong edit target | no test existed | caught |
| E16 do not skip `CMakeScratch` | no test existed | caught |
| E17 drop `found.sort()` | | **survived**: APFS lists directories in name order, so the order is unobservable here; the new test would catch it on ext4 |
| E18 drop the tag from the edit declaration | | build error (unused variable), not a test result |

Survivors: 3 behavior survivors found (E3, E4, E17); E3 and E4 killed, E17 filesystem-dependent.
Warm lib tests 6.0 s before, with 12 new tests unchanged within noise.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` (DSLP-87): no coverage numbers, no
`make mutants-diff` lane qualification, and the 108 `bench_runner` tests and the 5,092-line runner
were not mutation-sampled.

## DSLP-60 Dead code

Commands: `cargo clippy --no-deps -p kache-e2e --all-targets -- -D warnings` (crate manifest sets
`warnings = "deny"`, so rustc `dead_code` and `unused_*` are already clean; exit 0 in `make lint`
shape). `.work/scripts/pubrefs.py crates/kache-e2e` scans all 303 `pub` items for references in
production code and in test code across the workspace (excluding `.work`, `target`): none in this
scope is unreferenced or test-only. Binary registrations checked: `Cargo.toml` `[[bin]]` entries,
`Makefile` `cc-oracle*` and `cc-bench` goals (`kache-cc-oracle`, `kache-cc-bench`), `scripts/cc-bench.sh`,
`$FALLBACK` expansion for `e2e-fallback`, and `kache-cc`/`kache-cxx` through fixtures.

Dependencies, by `rg` on `src` and `tests`: every `[dependencies]` entry is used in non-test code;
`kache-fs` (dev) is used only in tests (`runner.rs`, `cc_oracle/mod.rs`, `bench_runner/tests.rs`), as
declared. `log` and `kache-log` are dev-only and used in tests.

Needs environment: `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat`.

Findings: none removed. `lib.rs::portable_path` (in the fixture half) is an identity function and is
recorded there.
