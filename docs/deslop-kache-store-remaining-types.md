# kache-store remaining modules: Types audit

DSLP-11 (7/28). Base/latest fetched origin/main:
`6657ca0e7b707390c7497ea775f49fd0547c0b6c`. Independent deslop/dslp-11 worktree,
not stacked on an unmerged deslop branch. Followed deslop Types and typing-audit.

## Scope and discovery

| Path under crates/kache-store | Baseline lines | Escape-matching lines before -> after |
| --- | ---: | ---: |
| src/store.rs | 6708 | 18 -> 18 |
| src/test_support.rs | 85 | 6 -> 6 |
| src/workspace.rs | 279 | 9 -> 9 |
| tests/byte_counters.rs | 116 | 0 -> 0 |
| tests/staged_publication.rs | 80 | 16 -> 16 |
| tests/thread_store_counters.rs | 38 | 1 -> 1 |
| Total | 7306 | 50 -> 50 |

Matching-line recipe: `\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\(`.
It includes comments/inactive platform branches, not numeric casts. One cast is
removed separately; the textual counter does not change. src/store/tests.rs and
child modules from DSLP-9/10 are consumers, excluded from this denominator.
test_support is unit-test-only via lib.rs; store.rs also contains three optional
test-support methods. workspace.rs includes inline unit tests; tests/ contains
three separate integration-test processes.

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-store --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn -F unsafe_code \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value
```

Distinct primary spans, deduplicated across targets and filtered to the six files;
multiple calls on a line count separately. Discovery's local lint cap measures all
candidates without altering the normal build's policy. Both measurements completed:
CLIPPY_MEASURE_EXIT=0 and CLIPPY_AFTER_EXIT=0.

| Strict-profile candidate | Before -> after | Disposition |
| --- | ---: | --- |
| unsafe_code = forbid | 1 -> 1 | Reject: getuid in the unit-test root/permission helper; no production unsafe finding or removal witness. |
| wildcard_enum_match_arm | 3 -> 3 | Reject: retry whitelist defaults false for other SQL errors; two test workspaces panic on unexpected operations. |
| match_wildcard_for_single_variants | 1 -> 1 | Reject: publish rejects any non-Done outcome; explicit Prepared spelling removes no trust and proves no currently false contract. |
| cast_possible_truncation | 9 -> 9 | Reject broad adoption: count-to-usize portability assumptions and elapsed milliseconds. No reproduced supported-platform overflow. |
| cast_sign_loss | 22 -> 21 | Adopt only on positive_or_none: checked persisted-value conversion, zero local false positives. Retain other count/size/reporting assumptions rather than change their error or fallback policy. |
| unwrap_used | 37 -> 37 | Reject: three ordinary blob-parent assertions, three optional fixture assertions, 31 inline/helper/integration test assertions. |
| expect_used | 4 -> 4 | Reject: one SQLite retry invariant, two blob-parent invariants, one optional test row insert. |
| redundant_clone | 1 -> 1 | Reject: open_with_options config clone is ownership/allocation cleanup, not a type-contract or escape-hatch witness. |
| needless_pass_by_value | 0 -> 0 | Reject: no witness. |

## Selected contract

Class: **remove trust, not a runtime bug fix**. keys_for_crates reads SQLite i64
compile_time_ms and size columns, then passes both to positive_or_none to populate
the public CrateHistoryEntry Option<u64> fields. Zero and negatives mean unknown,
not a zero-cost/zero-size measurement. The old positive guard made its cast safe;
replace that cast with u64::try_from(value).ok() and retain the positive filter.

All i64 values retain exactly their prior result: negatives fail conversion and
return None; zero converts but is filtered to None; positives fit u64 and retain
their magnitude. NULL/wrong SQL types still fail row.get before this helper.
No query, public type, error propagation or supported state changes. The local
cast_sign_loss deny rejects reintroducing the cast with zero false positives,
meeting the at-most-one-per-5k adoption bar without forcing broad cleanup.

### Checker witness

First add the function-local deny while leaving the old body, then run:

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-store --lib --all-features --no-deps
```

```text
error: casting `i64` to `u64` may lose the sign of the value
    --> crates/kache-store/src/store.rs:2305:27
2305 |     (value > 0).then_some(value as u64)
note: the lint level is defined here
    --> crates/kache-store/src/store.rs:2303:8
2303 | #[deny(clippy::cast_sign_loss)]
TYPE_WITNESS_EXIT=101
```

This is a Clippy witness, not a rustdoc compile_fail claim. The persistent deny
protects this conversion; the normal make check lint run does not cap it.

### Runtime negative control and existing boundaries

The existing test_positive_or_none_treats_non_positive_as_unknown covers 0, -1,
1, 4200 and i64::MAX. Temporarily replace the helper with just
u64::try_from(value).ok(), deliberately dropping the positive filter. Run:

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache-store --all-features --lib \
  test_positive_or_none_treats_non_positive_as_unknown
```

```text
assertion `left == right` failed: 0 is unknown, not Some(0)
  left: Some(0)
 right: None
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 621 filtered out; finished in 0.00s
INJECTED_FAULT_EXIT=101
```

Restore the filter before positive checks. No injected fault remains. Existing
keys_for_crates tests cover empty input, absent names, populated rows and a SQL
type error. The candidate runs those plus the helper range test successfully;
there is no new parser/predicate contract requiring new tests of std TryFrom.

## Retained assumptions and boundary review

- try_open_index_db has six attempts. Success returns a connection; each failure
  stores last_error before a retry or break, so the tail expect has an error.
  Keep transient retry, quarantine/recovery and error classification behavior.
- Blob parent unwrap/expect sites retain the generated content-address path
  invariant. blob_path_in_store_dir preserves its defensive short/Unicode-hash
  slicing fallback. This audit does not prove all arbitrary public hash/path
  inputs have parents or re-audit remote extraction's hash-validation boundary.
  Do not change staging, import, fsync or legacy migration semantics to adopt a
  blanket unwrap/expect ban. Optional test-support failures stay fail-fast.
- Raw aggregate/size/hit casts assume index values fit the public unsigned
  representation. Counts are nonnegative; several reporting/reclaim paths clamp
  signed inputs. Raw size/hit casts still trust index integrity; accepting negative
  corruption differently would be a behavior decision, not a mechanical lint fix.
  Count-to-usize overflow on 32-bit and extreme elapsed time were not reproduced.
  No supported-producer defect is established by merely inserting arbitrary SQL.
- Workspace Scope restores invocation ownership with a thread-affine guard.
  Prepared owns its executor/id, marks published only on Done, and discards on
  Drop after errors/unpublished stages. Preserve rejection of wrong Outcome,
  propagation of operation errors and no fallback into daemon writes when client
  access is denied. Tests cover scope nesting, prepare/publish/replace/discard,
  denied clients and NotFound versus PermissionDenied.
- Test-support lock poisoning recovery and best-effort cwd restoration avoid
  cascading failures; the uid probe preserves root-aware permission skips.
  Counter integration binaries isolate process-global totals; thread snapshots
  exclude other threads; staged publication accounts only committed bytes and
  preserves old output after failed publish/replace. Retain all these contracts.
- Recent history includes active store restoration/C++ work (18339376, a2495b8b).
  No C/C++ keying, validation, restore or forbidden campaign path is edited.

## Consumers, contracts and verification

The helper has only two call sites, both in keys_for_crates. Repository-wide Rust
search finds that API called only by store tests, not a production workspace
consumer. Retain the public API for external callers rather than infer dead code.
External consumers are not checked; all i64 results remain behavior-identical.
Workspace tests compile the local Cargo path, not a released registry package.
No CLI, cache key, store format/schema, wrapper protocol or public signature changes.
Declared performance workloads are unaffected: no repository production call path
reaches this helper, and there is no I/O/allocation/query change. No performance
campaign or absolute performance claim is warranted.

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache-store --all-features
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

```text
test store::tests::keys_for_crates_errors_on_non_text_cache_key_row ... ok
test store::tests::test_positive_or_none_treats_non_positive_as_unknown ... ok
test store::tests::test_store_keys_for_crates_empty ... ok
test store::tests::test_store_keys_for_crates_nonexistent ... ok
test store::tests::test_store_keys_for_crates_with_entries ... ok
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.76s
STORE_TEST_EXIT=0
```

rustc 1.102.0-nightly (5b7b37aef 2026-11-08), Cargo 1.102.0-nightly
(c07ac2c4d 2026-11-06), Clippy 0.1.101 (5b7b37aefc 2026-11-08). Linux x86_64
gnu, four of eight CPUs, all features including test-support. No maintenance
installation, shared config, dependency or lint suppression changes. Unavailable
todo CLI uses the established direct backlog-edit equivalent. Close only DSLP-11;
no new demonstrated defect warrants an ID, so preserve Next: 103 and owner tickets.
Cross-platform execution, 32-bit execution and full campaigns were not run locally.
No PR and no main push; main merge hold remains in effect.

Required gate output:

```text
test-resource guard: all checks passed
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.67s
MAKE_CHECK_EXIT=0
```
