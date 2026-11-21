# Deslop: src native_archive, out_dir_alias, path_normalizer, prestage (21/28)

Scope (DSLP-25, DSLP-52, DSLP-79): `src/native_archive.rs`, `notice.rs`, `opcounts.rs`, `otel.rs`,
`out_dir_alias.rs`, `path_normalizer.rs`, `phase_trace.rs`, `platform.rs`, `prestage.rs`
(14,122 lines). Base `11cbdba8`, macOS arm64, nightly-2026-11-09. `RUSTC_WRAPPER=` (the host
`~/.cargo/config.toml` sets `kache`) and `CARGO_BUILD_JOBS=4`. The busy host killed the
`kache` lib-test compile with SIGKILL or SIGTERM on about half of the attempts; each result
below comes from a rerun that finished.

## DSLP-25 Types

Commands (production code only; `--cap-lints warn` keeps the workspace `warnings = "deny"` from
turning candidates into errors, `--no-deps` is implied by `-p kache --lib`):

```sh
cargo clippy -p kache --lib --bins -- -D warnings                        # exit 0
cargo clippy -p kache --lib --message-format=json -- --cap-lints warn \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss -W clippy::unwrap_used \
  -W clippy::expect_used -W clippy::redundant_clone -W clippy::needless_pass_by_value \
  -W clippy::indexing_slicing -W clippy::as_conversions -W unreachable_pub -W unsafe_code
```

Escape hatches (`unsafe`, `allow(`, `.unwrap()`, `.expect(`) in non-test code: 25 before, 24 after
(`path_normalizer.rs` 3 to 2). Test code holds 641 more, all in `#[cfg(test)]`. No `allow(`
and no unwrap in production code in scope. First-party `unsafe` is libc FFI: `platform.rs` 17
sites (10 `SAFETY` comments, several cover adjacent calls), `otel.rs` 1 (`getloadavg`),
`out_dir_alias.rs` 1 (`geteuid`), `path_normalizer.rs` 1; std has no safe equivalent and no
dependency may be added.

| Candidate | Count | Disposition |
| --- | --- | --- |
| `redundant_clone` | 1 | Adopted: fixed (`push_rule_with_variants_target_and_source` moves its prefix). |
| `needless_pass_by_value` | 1 | Adopted: fixed (`push_configured_base_dir_rule` borrows `&str`). |
| `expect_used` | 2 | One fixed: `leak_window` used `.chain(once(len)).find().expect(..)` and now `.find().unwrap_or(len)`. One kept: the dep-info sentinel lookup for the fixed list of source sentinels, guarded by `every_source_sentinel_has_a_distinct_depinfo_sentinel` (`path_normalizer.rs:2756`). |
| `unwrap_used`, `wildcard_enum_match_arm`, `match_wildcard_for_single_variants`, `cast_possible_truncation` | 0 | Nothing to adopt; clean already. |
| `cast_sign_loss` | 2 | Rejected: `rlim_t` from a checked `i32` (`platform.rs:265`) and a clamped `timestamp().max(0)` (`otel.rs:293`). |
| `as_conversions` | 29 | Rejected: widening and `libc` type spellings; `TryFrom` would add error paths nobody handles. |
| `indexing_slicing` | 35 | Rejected: every `native_archive.rs` slice follows a length check on a fixed 60-byte ar header or a located NUL; the file's malformed-archive tests (`member_names_refuse_what_they_cannot_read`, `native_archive.rs:2078`) exercise the failure arms. 1 false positive per 400 lines, over the 1-per-5k bar. |
| `unreachable_pub` | 48 | Rejected: every module is private (`mod x;` in `src/main.rs`), so rustc's `dead_code` already treats these as crate-private; the lint renames `pub` to `pub(crate)` and finds nothing. |
| `unsafe_code` | 20 | Rejected as a lint (FFI above); each site has a `SAFETY` comment. |

After the two commits the same run reports `expect_used` 1 and no `redundant_clone` or
`needless_pass_by_value` in scope; other counts are unchanged. Fix: `bf1508f6`
(`refactor(path_normalizer)`); `cargo test -p kache --lib -- path_normalizer` passes (74 tests),
including `leak_window_widens_to_char_boundaries` and the end-of-string cases at
`path_normalizer.rs:1738` and `:1741`.

## DSLP-52 Tests

Tiers: unit tests in each file's `#[cfg(test)]` module (313 run, 2 ignored, under 1 s for the
whole scope); integration tests drive the same code through the `kache` binary:
`tests/out_dir_alias_test.rs` (shared `OUT_DIR` across checkouts), `tests/input_prediction_test.rs`
and `crates/kache-protocol/src/envelopes.rs` (prestage), `tests/cached_units_regression_test.rs`
(`static=` archives). `make test` runs both.

Run: `cargo test -p kache --lib -- out_dir_alias path_normalizer native_archive prestage platform
otel opcounts phase_trace notice` with the system `TMPDIR`: 313 passed, 0 failed. With a
checkout-local `TMPDIR` four `out_dir_alias::tests::fs::gather_*` tests fail because they require
a private root outside the checkout (known, `AGENTS.md`).

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Static archive digest changes with content, order, member timestamp, symbol table, long-name table | `native_archive.rs:1866`, `:1877`, `:1900`, `:1916`, `:1927`, `:1938` | Asserted; byte-level inputs, exact digest comparison. |
| Digest is stable across build paths (member names) and falls back on foreign archives | `native_archive.rs:1609`, `:1840`, `:1988`, `:2011`; `tests/cached_units_regression_test.rs:1576` | Asserted. |
| Alias decision: first failed rule wins, lib needs macro-only evidence | `out_dir_alias.rs:1238`, `:1254`, `:1273`; `tests/out_dir_alias_test.rs:318` | Asserted; fault injected below. |
| Alias root is private, gather reads unit facts | `out_dir_alias.rs:2471` to `:2587` | Asserted (system `TMPDIR`). |
| Machine paths never reach keys; sentinels are distinct and rewrite exactly | `path_normalizer.rs:1764`, `:1846`, `:2354`, `:2756`, `:2946`, `:3390` | Asserted. |
| Concurrent invocations keep their own normalizer, counters and trace lanes | `path_normalizer.rs:1551`; `opcounts.rs:450`; `phase_trace.rs:175` | Asserted. |
| Prestage copies are private, one publication per generation, prune keeps live activity | `prestage.rs:816`, `:856`, `:1085`, `:1207`, `:1498` | Asserted. |
| Descriptor limit raise restores and caps | `platform.rs:496`, `:516` | Asserted. |
| OTLP attributes are an allowlist; gauges stay gauges | `otel.rs:450`, `:754` | Asserted. |
| Notice reaches the terminal, else stderr, else the log | `notice.rs:77`, `:92`, `:102`, `:110` | Asserted. |

Injected fault (critical decision, `out_dir_alias::decide`): in `src/out_dir_alias.rs` change
`if tier == Tier::Lib && verdict != LibVerdict::MacrosOnly {` to `== `.
`cargo test -p kache --lib -- out_dir_alias` exits 101 with
`out_dir_alias::tests::decide_aliases_a_lib_only_on_macro_only_evidence` and
`out_dir_alias::tests::fs::gather_reads_evidence_denials_and_the_alias_state` FAILED (2 failed,
51 passed). Restored from a pre-edit copy; `cmp` exit 0.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed on this host, so
there is no line or branch coverage and no survivor list for the scope; the mutation lane for
these files is not qualified. No new tests were added: no `none` boundary and no survivor was
measured.

## DSLP-79 Dead code

Tools run: rustc `dead_code` under the workspace `warnings = "deny"`:
`cargo clippy -p kache --lib --bins -- -D warnings` exit 0 and
`cargo test -p kache --lib --no-run` exit 0 (test cfg also clean). `rg 'allow\(dead_code\)'` over the nine files is empty. Needs
environment: `cargo-machete`, `cargo-udeps` (not installed); Linux-only paths
(`target_os = "linux"`: 2 in `native_archive.rs`, 3 in `otel.rs`, 8 in `out_dir_alias.rs`,
8 in `path_normalizer.rs`, 5 in `platform.rs`, 3 in `prestage.rs`) are not compiled on this host.

Workspace-wide caller search (`.work/tmp/pubscan.sh`: for each `pub` item, `rg -w NAME src tests
crates launcher scenarios scripts` outside its own file): 36 items have no reference outside
their file. None is dead: rustc proves each is used in the file's non-test code (or is
`#[cfg(test)]`, for example `portable_static_archive_hash` and `stage_for_test`). They are
visibility-narrowing opportunities only and no cut is justified.

| Finding | Entry points and consumers checked | Disposition |
| --- | --- | --- |
| `#[cfg(test)]` helpers in production files (`native_archive.rs:139`, `:144`, `:149`; `prestage.rs:328`) | `rg 'stage_for_test\|portable_static_archive_hash' src tests crates`: used by `prestage.rs` tests and `src/wrapper/tests.rs` | Kept: test seams. |
| `#[cfg(not(unix))]` branches (`out_dir_alias.rs:528`, `:540`, `:560`, `:625`; `prestage.rs:312`; `otel.rs:384`) | `docs/how-it-works/architecture.mdx` documents Windows support; `AGENTS.md` names Windows `-D warnings` | Kept: live on a supported target I cannot build here. |
| 36 file-local `pub`/`pub(crate)` items | table above | Kept: live. |

No code removed. No `Needs owner` walk-back.
