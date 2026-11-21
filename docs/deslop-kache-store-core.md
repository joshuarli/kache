# Deslop: crates/kache-store core

Types, Tests and Dead code for DSLP-9, DSLP-36 and DSLP-63 at base `11cbdba8`
(branch `deslop/k2`), macOS arm64, nightly-2026-11-09, load average 25 to 50.
Scope: the core files named in DSLP-9 (`atomic`, `blob_drift`,
`blob_validation`, `cc_memo`, `config`, `eviction`, `eviction_evidence`
(+ `tests.rs`), `file_hash`, `filesystem`, `index_compaction`, `lib`, `link`,
`markers`, `opcounts`, `sharing`, `store/compile_time_backfill`,
`store/tests/cost_backfill_tests.rs`, `examples/bench-cc-memos.rs`). No on-disk
format is touched: the only edits are visibility of three crate-internal
constants and two test clones.

All commands: `export TMPDIR=$PWD/.work/tmp CARGO_BUILD_JOBS=4`, run from the
worktree root.

## DSLP-9 Types

Escape hatches (`rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('`) are
dominated by tests (`link.rs` 486, `file_hash.rs` 200, `cc_memo.rs` 195,
`cost_backfill_tests.rs` 109). Production code in scope has 5 `unsafe` blocks
in `link.rs` and 2 in `file_hash.rs` (`cfg(linux)`, not compiled on this host),
plus 2 `.expect`; `store.rs` is in the remaining-modules doc. No `#[allow]` in scope; the two crate-level clippy allows
are in `Cargo.toml`.

Candidate measurement:

```sh
cargo clippy --no-deps -p kache-store --lib --features test-support --message-format=json -- \
  --cap-lints warn -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss -W clippy::unwrap_used \
  -W clippy::expect_used -W clippy::redundant_clone -W clippy::needless_pass_by_value \
  -W unsafe_code -W unreachable_pub          # exit 0; same with --all-targets
```

`--no-deps` is required: without it the `-W` flags reach `kache-core` and
`kache-fs`, which fail under the workspace's `--deny=warnings`.

| Candidate | Core hits (lib / all targets) | Decision |
| --- | --- | --- |
| `unreachable_pub` | 3 / 3 | **Adopted** (`[lints.rust] unreachable_pub = "warn"`; `make lint` denies warnings). All three were `pub const` in the private `cc_memo` module: `UNPROVEN_SIZE`, `CC_MEMO_RULE`, `CC_MEMO_VARIANTS`. Nothing outside the crate can name them (`rg -w` over the workspace: 0 outside `crates/kache-store`). Now `pub(crate)`; 0 hits after. |
| `redundant_clone` | 0 / 2 | Adopted as a fix (clones removed in `cc_memo.rs:844` and `file_hash.rs:1095` tests); the lint itself is enabled in the remaining-modules commit, once `store.rs` and `store/tests.rs` are also clean. |
| `cast_sign_loss`, `cast_possible_truncation` | 6 / 8 | Rejected. Every `i64 as u64` is behind `.max(0)` (`blob_drift.rs:61`, `eviction.rs:214`, `eviction_evidence.rs:102,105`) or an explicit `clamp` (`compile_time_backfill.rs:47`); `eviction.rs:37` widens through `u128` and is the documented overflow guard. All false positives. |
| `expect_used` (prod) | 2 | Rejected. `atomic.rs:71` is the loop invariant of `retry_transient` (pinned by `retry_transient_gives_up_after_exactly_its_budget`, `atomic.rs:401`); `cc_memo.rs:510` reads `current` right after the row loop opens it when absent or different. Removing either needs a restructure with no type gain. |
| `unwrap_used` | 0 / 1128 | Rejected for tests; production has none in scope. |
| `unsafe_code = forbid` | 5 prod blocks (+3 in tests; 2 more `cfg(linux)`) | Rejected: `link.rs` `open(O_NOFOLLOW)`/`fstat`/`futimens(UTIME_NOW)` (`touch_mtime_write_clock`) and `file_hash.rs` `clock_gettime(CLOCK_REALTIME_COARSE)` have no std equivalent (`File::set_times` cannot express `UTIME_NOW`); each is the documented ordering mechanism, not an optimization. |
| `needless_pass_by_value` | 1 / 1 | Rejected (`eviction_evidence.rs:63`, private `observe(row: Observation)`; a small plain row, one false positive). |
| `wildcard_enum_match_arm` | 0 / 3 | Rejected: all three are in test code. |
| `match_wildcard_for_single_variants` | 0 | n/a here (one hit in `workspace.rs`, remaining-modules doc). |

False-positive density for the adopted set is 0; for the rejected cast/expect
set it is 8 production hits over the scope's roughly 12,970 lines, above the
1-per-5k bar.

Witness: `unreachable_pub` is itself the checker; reverting any of the three
constants to `pub` makes `cargo clippy ... -D warnings` fail with
`unreachable_pub`. Escape hatches before and after: unchanged (no hatch added).

## DSLP-36 Tests

Run: `cargo test -p kache-store --features test-support` exit 0, 165 s wall
cold under load 46 (compile dominated); unit target 624 passed, 2 ignored (two
subprocess fixtures in `store/tests.rs`), 19.5 s
of test time. Integration targets: `byte_counters` 5, `staged_publication` 1,
`thread_store_counters` 1. Tiers: inline unit tests (core), crate integration
tests, and the main crate's CLI and wrapper tests above them.

Audit for false passes: a body scan for tests without `assert*`/`panic!`/
`unwrap_err`/`is_err` finds three in core. `fsync_file_flushes_a_writable_blob`
and `fsync_dir_succeeds_on_a_real_directory` assert through `.expect`, and
`accepts_matching_files_including_empty_metadata_outputs` through `.unwrap()`
over both the empty and non-empty case; none is vacuous.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Eviction trigger fires only above the cap; target is exact without overflow | `eviction.rs:326`, `:335` | Asserted; fault below |
| Selection policies (size-pressure, value-density, older-than, duplicate) rank and fail closed | `eviction.rs:359` to `:569` | Asserted |
| Blob size/type validation | `blob_validation.rs:22`, `:32`, `:42`, `:49` | Asserted |
| Refcount drift classes and clamping | `blob_drift.rs:96`, `:112`, `:141`, `:169` | Asserted |
| Atomic replace, durable directory sync, temp cleanup | `atomic.rs:459`, `:468`, `:494`, `:547` | Asserted |
| Transient rename retry budget and classification | `atomic.rs:348` to `:444` | Asserted |
| Compaction needs size and fraction, space guard, contention only is busy | `index_compaction.rs:215` to `:268` | Asserted |
| File-hash memo: stamp settling, rule column, prune window, schema upgrades | `file_hash.rs:837` to `:1693` | Asserted |
| C/C++ memo: unsettled stamps never recorded, older-release rows never served, bounded storage, rollback | `cc_memo.rs:738` to `:1516` | Asserted |
| Restore strategies, clone/hardlink races, depinfo rebase, write-clock touch refuses symlink and FIFO | `link.rs:2070` to `:4552` (97 tests; touch at `:2892`, `:2930`, `:2959`) | Asserted |
| Marker freshness, symlink refusal, session dedup across processes | `markers.rs:300` to `:528` | Asserted |
| Sharing probes, retainers, executable bit | `sharing.rs:94` to `:280`, `filesystem.rs:82` to `:185` | Asserted |
| Eviction evidence cohorts and horizons | `eviction_evidence/tests.rs:32` to `:269` | Asserted |
| Thread/invocation byte counters | `opcounts.rs:467` to `:503` | Asserted |
| Cost backfill budget and retry delay | `compile_time_backfill.rs:224`, `:231`, `cost_backfill_tests.rs` (14) | Asserted |

No boundary is `none`, so no test was added.

Injected fault (critical decision: when GC fires). In
`crates/kache-store/src/eviction.rs:49` change `physical_size > max_size` to
`>=`; `cargo test -p kache-store --lib eviction_trigger_is_strictly_above`
exits 101 with `eviction::tests::eviction_trigger_is_strictly_above_the_configured_cap`
panicking at `eviction.rs:337`. Restored from a saved copy; `cmp` exit 0 and
`git diff --stat -- crates/kache-store/src/eviction.rs` empty.

Needs environment: coverage (`cargo-llvm-cov` not installed) and mutation
(`cargo-mutants` 27.1.0 not installed); no scoped survivor list exists for this
component. `.cargo/mutants.toml` already carries the lane's exclusions. Test
cost: this change adds no tests, so warm `make test` is unchanged.

## DSLP-63 Dead code

Tools: rustc `dead_code` and `unused_crate_dependencies` (the repo's
`--force-warn=unused_crate_dependencies`) are clean under
`cargo clippy --no-deps -p kache-store --all-targets --features test-support -- -D warnings`,
exit 0. `unreachable_pub` found the three constants above and nothing else.
Needs environment: `cargo-machete`, `cargo-udeps`, `cargo-bloat` not installed;
every manifest dependency is imported by `rg` (`anyhow`, `rusqlite`, `deser`,
`deser-json`, `blake3`, `libc`, `log`, `tempfile`, `rustc-hash`, and the three
`kache-*` crates; dev: `filetime`, `deser-value`).

Public-item scan (`.work/tmp/pubscan.sh`: for each `pub` item in the core
files, `rg -w` outside `crates/kache-store`, in other crate files, and in its
own file). 198 items; 55 have no reference outside the crate, 20 of those none
outside their own file. Of the 20, 17 are used by production code in the same
file (constants such as `FILE_HASH_RETENTION_SECS`, the `BlobRetainer` family,
`IndexPageStats::fits_in`, `marker_file_is_fresh`). The remaining three have no
production caller anywhere in the workspace:

| Item | Where | Evidence | Disposition |
| --- | --- | --- | --- |
| `rebase_depinfo_content` | `link.rs:1387` | Callers: 7 tests (`link.rs:3030` to `:3089`) only; docs and other crates: 0 | Consumer question (public API of a published crate); not removed |
| `set_shared_hardlink_restores` | `link.rs:104` | Callers: tests (`:1875`, `:4053`, `:4061`) only; `[cache] shared_hardlink_restores` is applied through the invocation scope | Consumer question; not removed |
| `rewrite_depinfo` | `link.rs:1825` | `#[cfg(test)]`; used by 2 tests | Test helper, no action |

No code removed. No platform-specific code was proven dead (`cfg(linux)`
`stamp_clock_ns` branch and macOS `getxattr` are reachable on their targets;
only the macOS target was built here).
