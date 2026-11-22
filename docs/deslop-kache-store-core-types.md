# kache-store core Types audit

DSLP-9 (component 5/28). Base `d4979af38e98fc8d7fbe965ce3ef81aaf44f25fd`.
Independent branch from origin/main, not from an unmerged deslop branch.
The owner explicitly authorized this slice while main pushes remain held.

## Scope and measurements

The 19 ticket files contain 12,970 lines before editing: 5,892 non-test lines
and 7,078 test lines. Non-test includes the benchmark example and feature-gated
helper declarations in the named files. Scope under crates/kache-store:

- examples/bench-cc-memos.rs;
- src/atomic.rs, blob_drift.rs, blob_validation.rs, cc_memo.rs, config.rs,
  eviction.rs, eviction_evidence.rs, eviction_evidence/tests.rs, file_hash.rs,
  filesystem.rs, index_compaction.rs, lib.rs, link.rs, markers.rs, opcounts.rs,
  sharing.rs;
- src/store/compile_time_backfill.rs and store/tests/cost_backfill_tests.rs.

Diagnostics are distinct primary spans, deduplicated across library/test targets,
then filtered to these paths. Separate calls on the same line count separately.
Inline test modules and the two separate test files count as tests. The rest of
store.rs and its test tree are consumers, not added to the measurement scope.

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-store --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value

taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-store --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn -F unsafe_code
```

Linux x86_64 gnu; all targets and feature test-support (including the required-
feature example). Discovery's local cap allows collecting every candidate; it
does not weaken repository settings. The first command completed before and
after; the unsafe baseline completed successfully and no unsafe operation changes.

| Candidate | Non-test before -> after | Tests before -> after | Disposition |
| --- | ---: | ---: | --- |
| unsafe_code = forbid | 7 -> 7 | 3 -> 3 | Reject: kernel-coarse clock and fd-safe write-clock stamping have no equivalent safe std operation preserving these semantics. |
| wildcard_enum_match_arm | 0 -> 0 | 3 -> 3 | Reject: fallback arms panic when tests see an unexpected lookup or workspace operation. |
| match_wildcard_for_single_variants | 0 -> 0 | 0 -> 0 | Reject: no witness. |
| cast_possible_truncation | 2 -> 2 | 1 -> 1 | Reject: bounded 90% eviction target and native nanoseconds; no demonstrated truncation defect in supported normal operation. |
| cast_sign_loss | 7 -> 6 | 1 -> 1 | Adopt only in blob_drift.rs: one cast removed, zero local false positives. Reject broad adoption for guarded counters, bounded retries and clock assumptions. |
| unwrap_used | 0 -> 0 | 1151 -> 1151 | Reject: non-test scope is clean; tests assert setup, successful operations and expected failures. |
| expect_used | 2 -> 2 | 10 -> 10 | Reject: atomic retry termination and memo-group initialization establish the invariants; do not restructure their runtime paths here. |
| redundant_clone | 0 -> 0 | 2 -> 2 | Reject: test copy cleanup is not a type/escape-hatch witness. |
| needless_pass_by_value | 1 -> 1 | 0 -> 0 | Reject: observe consumes an internal Observation as an ownership convention, not a false contract. |

## Selected trust removal

Class: **not a runtime defect; remove a guarded numeric cast**. SQLite drift
aggregates are read as i64, then exposed as u64 counters. The original
value.max(0) as u64 correctly clamped negative totals. Replace it with
u64::try_from(value).unwrap_or(0), and deny clippy::cast_sign_loss in this module.
The conversion now checks its own range rather than relying on a separate clamp.

For every i64 input, behavior is identical: negative maps to zero, and every
nonnegative i64 fits u64 with the same magnitude. The enclosing Result::map still
propagates rusqlite errors unchanged. No SQL, transaction, snapshot, write-lock,
schema, public signature or output field changes. Existing tests cover empty and
consistent tables, each positive drift class, byte totals and negative sums.

The module setting removes one unchecked cast with zero remaining false positives,
meeting the adoption bar of at most one per 5,000 lines. No broader setting meets
the same defect/trust-removal and density requirements in this slice.

Rejected-program witness: first add the module deny while retaining the old cast,
then run the following before replacing the cast:

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-store --lib --all-features --no-deps
```

```text
error: casting `i64` to `u64` may lose the sign of the value
  --> crates/kache-store/src/blob_drift.rs:63:47
63 |         row.get::<_, i64>(column).map(|value| value.max(0) as u64)
note: the lint level is defined here
  --> crates/kache-store/src/blob_drift.rs:3:9
3 | #![deny(clippy::cast_sign_loss)]
TYPE_WITNESS_EXIT=101
```

This is a Clippy witness, not a rustc compile_fail claim. The persistent module
deny rejects reintroduction; all positive checks use the checked conversion.
No new parser/predicate or runtime behavior requires new tests of standard TryFrom.

## Retained trust and deferred leads

- atomic::retry_transient has a fixed ten-attempt loop. Its final attempt cannot
  satisfy should_retry_transient, so error returns before the tail expect; success
  also returns. The caller's production transient predicate is always false.
  Removing the assertion without trading it for another one would restructure
  this bounded operation/error lifecycle. Keep the existing budget tests.
- cc_memo's current variant is installed when absent or when the row id changes,
  before as_mut().expect("opened above"). Recent history includes active C/C++
  memo/read-set changes (6781df66, 13f968f6, a2495b8b); no memo code is edited.
- Remaining sign casts clamp reclaim/size to zero, guard positive evidence cost,
  or clamp retry exponents to 0..=6. The eviction u128 percentage result is at
  most 90% of u64::MAX. These existing bounds are not demonstrated bugs.
- file_hash's CLOCK_REALTIME_COARSE path retains native timespec assumptions.
  Valid nanoseconds fit u32. Pre-epoch realtime behavior was not reproduced or
  verified; this audit does not claim its signed-second conversion is universally
  safe. Changing clock selection or fallback policy would change stamp behavior.
- link::touch_mtime_write_clock keeps O_NOFOLLOW/O_NONBLOCK and regular-file
  validation, OwnedFd cleanup, and futimens UTIME_NOW/UTIME_OMIT. Replacing these
  with a userspace SystemTime timestamp would change the kernel write-clock
  ordering contract. Test-only unsafe covers FIFO, effective uid and umask setup.
- Cargo.toml retains inherited permissions_set_readonly_false and
  too_many_arguments allowances. No new allowance, weaker profile, dependency,
  toolchain or shared configuration is introduced.

Matching-line escape-hatch counter over the 19 paths:
`\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\(`. It includes inactive
platform branches and comments, but excludes numeric casts and Cargo settings.

| Scope | Before | After |
| --- | ---: | ---: |
| Non-test | 9 | 9 |
| Tests | 1145 | 1145 |
| Total | 1154 | 1154 |

The one numeric cast removal is separate from this unchanged text count.

## Boundaries and consumers

Reviewed typed producers: blob metadata validation, SQL-only drift aggregates,
config's kache-core re-export, eviction features/policies/evidence cohorts,
index compaction outcomes, file fingerprints/lookup states, filesystem retainer
estimates, atomic publication, warning/session markers and invocation-local byte
scopes. Preserve optional uncertainty, read-only reporting, bounded backfill,
unknown compile costs, hardlink/reflink distinctions, native clock behavior,
warning sinks, ownership/Drop cleanup and the public ArtifactPolicy contract.
The benchmark's tiers and synthetic fixture inputs are not production wire data.

The selected public probe is re-exported from lib.rs, wrapped by
ArtifactStore::blob_refcount_drift, and consumed by root CLI/maintenance,
blob_heal and telemetry snapshots. Reviewed those call sites and the store tests
for read-only/unlocked probes. Workspace tests resolve the candidate through the
Cargo path dependency, not a released package. No cache key, store format,
wrapper protocol, CLI flag/exit code or report representation changes.
Performance workload is unaffected: reporting SQL and syscall counts do not
change; this conversion is not on the artifact restore or hash-lookup hot path.

## Verification

Toolchain: rustc 1.102.0-nightly (5b7b37aef 2026-11-08), Cargo 1.102.0-nightly
(c07ac2c4d 2026-11-06), Clippy 0.1.101 (5b7b37aefc 2026-11-08); Python 3.15.2
for scoped diagnostic/count analysis. Four of eight logical CPUs; no maintenance
installation needed. The unavailable todo CLI is replaced by the prior direct
backlog-edit convention. Close only DSLP-9, preserve Next: 103 and existing owner
tickets; no demonstrated new defect warrants an ID. No forbidden path is edited.

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache-store --all-features
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

```text
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.12s
STORE_TEST_EXIT=0
```

Cross-target Clippy, macOS execution and full oracle/project/performance campaigns
are not claimed. The resource-guard self-check's low-nofile warnings are deliberate
negative controls: the calling shell retains soft/hard limits 65536/524288.

Required gate output:

```text
test-resource guard: all checks passed
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 4.97s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
MAKE_CHECK_EXIT=0
```
