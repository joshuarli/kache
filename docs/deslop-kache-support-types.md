# Deslop: support crates, launcher and scenario types

DSLP-13, Types component 9 of 28. Independent base:
`e0a7457ed4370cd09cdcda958375b51c2db41f58`. Linux x86_64, rustc
`1.102.0-nightly (5b7b37aef 2026-11-08)`, Clippy
`0.1.101 (5b7b37aefc 2026-11-08)`. No tools installed for this slice.

## Scope and inventory

Tracked source inventory includes Rust, C, C++, headers, assembly and shell;
it excludes manifests, locks, patches, documentation and external benchmark
repositories named by scenario manifests. Counts are from the base, including
tests and comments. Generated parallel-coalescing fixture functions are not
first-party tracked source.

| Area | Files | Lines | Escape-matching lines, before/after |
| --- | ---: | ---: | ---: |
| `crates/kache-client` | 9 | 1,351 | 118 / 118 |
| `crates/kache-core` | 5 | 954 | 11 / 11 |
| `crates/kache-format` | 2 | 402 | 3 / 3 |
| `crates/kache-ipc` | 9 | 3,984 | 389 / 389 |
| `crates/kache-log` | 1 | 532 | 15 / 15 |
| `crates/kache-proofs` | 1 | 50 | 0 / 0 |
| `crates/kache-protocol` | 11 | 1,948 | 22 / 22 |
| `crates/kache-shims` | 9 | 3,499 | 104 / 104 |
| `launcher` | 2 | 396 | 22 / 22 |
| `scenarios` | 121 | 1,240 | 24 / 24 |
| Total | 170 | 14,356 | 708 / 708 |

Text counter: `\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\(`,
counting matching lines once. It includes comments and is not a safety proof;
it does not count numeric casts, `unwrap_err`, or fallback combinators.
One unchecked narrowing cast was removed separately. No suppression or
panic-based conversion was added.

## Strict-profile measurement

Commands used four CPUs and cleared the dogfood wrapper. Cargo JSON messages
were deduplicated by code, primary file, line and column; dependency messages
were excluded. `--cap-lints warn` deliberately makes this a measurement, not
an assertion that the entire candidate profile passes with deny levels.

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-client -p kache-core -p kache-format -p kache-ipc \
  -p kache-log -p kache-proofs -p kache-protocol -p kache-shims \
  --all-targets --all-features --no-deps --message-format=json -- \
  --cap-lints warn -F unsafe_code \
  -W clippy::wildcard_enum_match_arm \
  -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::redundant_clone -W clippy::needless_pass_by_value
```

| Candidate | Eight crates before | After | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 71 | 71 | Retain Unix FFI, pointer/descriptor operations and test environment mutation. |
| `wildcard_enum_match_arm` | 8 | 8 | Retain intentional defaults; no missing-variant defect demonstrated. |
| `match_wildcard_for_single_variants` | 0 | 0 | No adoption witness. |
| `cast_possible_truncation` | 41 | 40 | Adopt only on `UtcTime::timestamp`; other sites discussed below. |
| `cast_sign_loss` | 10 | 10 | No behavior-preserving defect witness requiring a broad guard. |
| `unwrap_used` | 580 | 580 | Test assertions/setup dominate; not a global production policy witness. |
| `expect_used` | 14 | 14 | Ten test expectations and four build-time AST/output invariants. |
| `redundant_clone` | 3 | 3 | Test ownership/style findings, not typing defects. |
| `needless_pass_by_value` | 5 | 5 | Four IPC ownership-boundary helpers and one test helper; no type defect. |

Also measured the launcher's two separate compiler entry points with the same
flags, rather than assuming Cargo forwards its lints to rustc subprocesses:

```sh
clippy-driver --edition=2024 -Cpanic=abort --emit=metadata \
  -o LAUNCHER_METADATA launcher/build_script.rs --error-format=json PROFILE_FLAGS
clippy-driver --edition=2024 --crate-type=lib --test --emit=metadata \
  -o COMPILE_METADATA launcher/compile.rs --error-format=json PROFILE_FLAGS
```

Here `PROFILE_FLAGS` is the complete argument list after Cargo's `--` above.
The native launcher reports 18 unsafe sites and no other named candidate;
the compiler helper reports two test unwraps. Two standalone `dead_code`
warnings arise because the helper has no containing build-script caller in
this probe. Both metadata compilations exited zero. Native launcher safety
still depends on libc signatures, NUL-terminated argv/environment/record
fields, buffer capacities and the documented legacy record fallback; a
crate-wide unsafe prohibition cannot preserve that no_std launcher contract.

Scenario Rust inputs contain 55 Cargo manifests in 28 independent roots.
Measured copies under `.work/dslp-13-scenario-audit.Ct0EOg`, never the tracked
fixtures, using `cargo clippy --offline --manifest-path MANIFEST --workspace
--all-targets --no-deps --message-format=json -- PROFILE_FLAGS`. This allows
Cargo to write fixture lockfiles and target artifacts without changing the
scenario contracts. Initially 26 roots compiled: multi-dep needed an uncached
`memchr 2.8.0`, and parallel-coalesce needs its generated heavy members.
The latter source generator is the `gen_heavy` function and three calls in
`parallel-build.sh`; only that generation section was run in the copy, not
the scenario's two release builds. The two roots were retried after preparing
those inputs: multi-dep with online Cargo dependency resolution, and generated
parallel-coalesce still offline. All 28 roots then exited zero. Distinct
findings were eight unwraps, 11 expects, four unsafe sites and three narrowing
casts. Other named candidates had zero hits. Three default
`unnecessary_min_or_max` diagnostics are the intentional macro-input fixture
expressions described below. Generated functions added no findings.
This is compiler-profile measurement, not execution of scenario
cold/warm/coalescing assertions. Non-Rust C/C++ fixture inputs were inventoried
and left intact; Rust lints are not a C/C++ strict profile.

## Adopted witness and contract proof

`UtcTime::timestamp` returns whole signed seconds, truncating fractions toward
zero and saturating at i64 bounds. The old implementation clamped an i128
quotient then asserted the range with `as i64`. The new implementation makes
the narrowing checked, choosing the same signed bound when it does not fit.
If the quotient fits, TryFrom returns exactly its value. Otherwise its sign
agrees with `self.nanos`, so the fallback equals the old clamp. No input,
including `i128::MIN`, changes its result. There is no optional/absent value to
invent, no allocation, no new public type and no new error or panic path.

The function-local `deny(clippy::cast_possible_truncation)` removes one
unchecked numeric trust site with zero local false positives. It does not
claim the global candidate meets the one-false-positive-per-5k density bar.
Returning to the old cast with the guard enabled failed:

```text
error: casting `i128` to `i64` may truncate the value
error: could not compile `kache-core` (lib) due to 1 previous error
TYPE_WITNESS_EXIT=101
```

Added `whole_seconds_truncate_and_saturate`: zero, subsecond values on both
sides of the epoch, negative fractional seconds, exact signed bounds and both
i128 overflow directions. There is no empty/error result for this total API.
Injecting the wrong positive fallback for negative overflow killed the test:

```text
assertion `left == right` failed
  left: 9223372036854775807
 right: -9223372036854775808
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.00s
INJECTED_FAULT_EXIT=101
```

The fault was removed. Initial verification found a new
`unnecessary_lazy_evaluations` warning in the fallback; replaced the lazy
closure with `unwrap_or` and reran the affected checks. No allow was added.

## Retained boundaries, callers and limitations

- Time producers include `now`, parsed RFC3339, integer constructors, `MIN`
  and saturating duration arithmetic. Consumers include event segment
  cutoffs in `src/events.rs`, report store reads in `src/report.rs`, telemetry
  in `src/otel.rs`, `src/since.rs` and native local-zone lookup in `time.rs`.
  Each receives the same i64 seconds. Timestamp JSON formatting/parsing,
  millisecond/microsecond functions and date/parts layout were not changed.
  Their clamp/modulo/calendar casts are not globally prohibited. The separate
  `saturating_duration_since` nanosecond narrowing also remains; changing its
  large-span behavior is not this behavior-preserving conversion's scope.
- IPC descriptor counts, libc CMSG lengths, credentials and control-buffer
  pointer lifetimes remain native boundary obligations. BPF constants and
  timeval casts in tests model kernel interfaces. The workspace path helper
  decodes owned NativeString, rejects NUL/relative paths and consumes the same
  protocol payload even where Clippy prefers borrowing; no signature change.
- Protocol request defaults (`client_version`, fallback outcomes, terminal
  message checks) intentionally have catch-all behavior. NativeString keeps
  Unix bytes and rejects NUL. Versioned Prepare/Commit and completion states,
  execution version 2 and commit/no-local-retry semantics are unchanged.
- Format `EntryMeta` retains defaulted legacy fields, key schema 34, unknown
  emit coverage and compatibility fixtures. No cache recipe, wire field or
  persisted representation changed.
- Shim selection and farm decision catch-alls represent force/ownership and
  unknown executable cases; exhaustive rewrites have no demonstrated witness.
  Logger Option/file sink and thread-local routing retain meaningful absent
  states and intentionally ignored poisoned/unavailable sinks.
- Scenario unwraps/expects express generated-input and Cargo environment
  assumptions; FFI examples intentionally exercise native linking. The
  cascade fixtures' path-length casts feed `.min(0)` on purpose, retaining
  source-path-sensitive macro inputs without changing their numeric output.
- `kache-proofs`' ordinary tests compiled. `cfg(kani)` is a separate harness,
  not exercised by `--all-features`; no Kani installation or proof run.
- Linux-only measurement does not type-check macOS audit-token/getumask/ABI
  branches or Windows core time-zone code. External consumers were not run.
  No oracle/project campaigns, scenario assertions or full performance gate
  were run. An isolated optimized timestamp IR comparison produced different
  instruction shapes, not an identity proof or performance verdict. No speed
  improvement is claimed. No forbidden compiler/probe/campaign paths changed.

No new owner decision or public API retirement is required for the adopted
conversion; no new ticket was filed, and `Next: 103` is retained.

## Verification

The eight-crate all-feature test command passed (168 passed, eight ignored
across unit/integration binaries) before the fallback's lint-only cleanup.
Final source then passed the core tests and the required workspace gate:

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache-core --all-features
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

```text
CLIPPY_FINAL_EXIT=0
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
CORE_TEST_EXIT=0
test-resource guard: all checks passed
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.71s
MAKE_CHECK_EXIT=0
```

The final scoped profile has the counts in the table, with no new default
Clippy finding. No visual surface changed. No PR or main push is part of this
slice.
