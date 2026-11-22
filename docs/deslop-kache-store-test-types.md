# kache-store test Types audit

DSLP-10 (6/28), based on latest origin/main at
`888de12c25642e5d22d98ddf4c3addfc9c5162b2`, independent of unmerged branches.
Scope: crates/kache-store/src/store/tests.rs, 14,735 lines and 366 #[test]
declarations before editing. The file is included by store.rs under cfg(test).
All scoped code is test code, not a production library or wire representation.
The child cost_backfill_tests module belongs to DSLP-9, not this denominator.

## Strict-profile measurement

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-store --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn -F unsafe_code \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value
```

Collect all diagnostics with the local lint cap, deduplicate primary spans across
targets, and retain only this file. Dependencies and sibling modules are excluded.
Both measurements completed: CLIPPY_MEASURE_EXIT=0 and CLIPPY_AFTER_EXIT=0.
Toolchain: rustc 1.102.0-nightly (5b7b37aef 2026-11-08), Cargo 1.102.0-nightly
(c07ac2c4d 2026-11-06), Clippy 0.1.101 (5b7b37aefc 2026-11-08).
Platform: Linux x86_64 gnu, all Cargo targets/features including test-support.
CPU cap: four of eight logical CPUs. No dependency/toolchain installation needed.

| Candidate | Before -> after | Disposition |
| --- | ---: | --- |
| unsafe_code = forbid | 5 -> 5 | Reject broad adoption: four edition-2024 environment mutations/restorations and one Unix effective-uid query. |
| wildcard_enum_match_arm | 1 -> 1 | Reject: file-hash lookup fallback panics for anything except the expected hit. |
| match_wildcard_for_single_variants | 0 -> 0 | Reject: no witness. |
| cast_possible_truncation | 10 -> 9 | One checked refcount conversion; retain bounded fixture sizes/times and sample/limit constants. No broad setting. |
| cast_sign_loss | 3 -> 2 | Adopt only on test_concurrent_puts_sharing_blob_are_consistent: one cast deleted, zero local false positives. The other two casts generate bytes from 0..3. |
| unwrap_used | 2469 -> 2469 | Reject: test setup and assertions, including unwrap_err. Production-only applicability has no scoped code here. |
| expect_used | 37 -> 37 | Reject: descriptive success, lock, timeout, fixture and error assertions, including expect_err; not fabricated supported states. |
| redundant_clone | 14 -> 14 | Reject: fixture copy/style suggestions, no false type contract or escape hatch removed. |
| needless_pass_by_value | 0 -> 0 | Reject: no witness. |

## Selected contract and witness

Class: **trust removal in a test assertion, not a demonstrated production bug**.
Eight independent store clients publish identical content under separate keys.
After every worker joins, the test reads the blob's SQLite INTEGER refcount as
i64 and compares it to the usize client count. Replace `refcount as usize` with
`usize::try_from(refcount)` compared directly to `Ok(N)`. No unwrap, saturation,
default, truncation or sign reinterpretation is added. Negative/out-of-range
values cannot equal Ok(N); representable counts compare by their exact magnitude.
The fixture N=8 and all valid store outcomes retain their existing behavior.

The following assert_blob_refs_match_mappings already compares i64 values without
the cast. Consequently this audit does not claim a demonstrated whole-test false
pass, even though the old cast could discard high bits on 32-bit platforms.
No 32-bit runtime reproduction is claimed. The selected assertion now proves its
conversion instead of trusting it, and the function-local deny rejects regression
without imposing unrelated changes on deliberate fixture casts elsewhere.

First add the local deny with the old assertion unchanged:

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-store --tests --all-features --no-deps
```

```text
error: casting `i64` to `usize` may lose the sign of the value
    --> crates/kache-store/src/store/tests.rs:9371:16
9371 |     assert_eq!(refcount as usize, N, "refcount must equal the entry count");
note: the lint level is defined here
    --> crates/kache-store/src/store/tests.rs:9312:8
9312 | #[deny(clippy::cast_sign_loss)]
TYPE_WITNESS_EXIT=101
```

Then replace the cast. This is a Clippy rejected-program witness, not a rustdoc
compile_fail claim. The normal lint gate keeps the deny active without a cap.
One cast removed, zero local false positives meets the at-most-one-per-5k bar.
No new parser or production behavior needs additional standard-library tests.

## Retained assumptions and producer/consumer review

- lock_poll_interval returns a Duration capped to 100ms; its as_millis cast is
  bounded. Backfill LIMIT, VERIFY_SAMPLE_RATE, fixture metadata lengths, 0..3
  byte tags, and fixed finite idle-hour fixtures explain the other cast warnings.
  None is a demonstrated runtime range failure; do not churn fixtures to impose
  a broader policy. The old refcount cast is the persisted-value boundary.
- EnvVarGuard snapshots an OsString and restores it on Drop, preserving missing
  versus present values. All six call sites take process_state_test_lock first.
  This serializes participating tests, not arbitrary foreign environment reads;
  process-wide environment safety is still trusted, not proved by this mutex.
  Subprocess isolation would be a separate test architecture change.
- The Unix-only geteuid call avoids claiming EACCES coverage when root bypasses
  mode 000. Keep that platform/privilege gate rather than changing test meaning.
- Reviewed helper and candidate paths across schema migration, publication and
  deferred durability, repair/quarantine, hash seeding, eviction equivalence,
  concurrent refcounts, lock lifetime, restore verification and read-only probes.
  Fallible calls deliberately assert successes/errors; absent entries, malformed
  metadata, zero-sized files, contention and partial/pending states stay covered.
- Recent store-test history includes active C/C++ memo work (6781df66, a2495b8b).
  Do not modify those tests, producer modules or forbidden campaign paths.

The selected consumer is the test harness using the candidate's local Cargo path:

```text
Compiling kache-store v2.0.0-rc.2 (/home/ubuntu/repos/kache/.work/wt/dslp-10/crates/kache-store)
test store::tests::test_concurrent_puts_sharing_blob_are_consistent ... ok
```

Public/runtime contracts: unaffected. No production source, store schema, cache
key, CLI/exit code, protocol, syscall pattern or public API changes. Performance
workload: unaffected because only a post-join test assertion changes; no workload
gate is warranted. No new finding needs an ID. Close only DSLP-10, retain Next: 103
and existing owner findings. The unavailable todo CLI uses the established direct
backlog-edit equivalent. Main pushes remain held; no PR is requested.

Escape-hatch matching lines (`\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\(`):
**2473 -> 2473**, all test code. This textual counter includes comments but not
numeric casts; the separately measured cast removal does not lower it. No new
allowance or shared lint/build/CI configuration is added.

## Verification

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache-store --all-features
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

```text
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 4.01s
STORE_TEST_EXIT=0
```

Cross-platform execution, 32-bit execution and full oracle/project/performance
campaigns were not run locally. Existing PR CI covers its configured platforms;
this branch was pushed without opening a PR.

Required gate output:

```text
test-resource guard: all checks passed
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.73s
MAKE_CHECK_EXIT=0
```
