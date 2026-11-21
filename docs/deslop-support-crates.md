# Deslop: support crates, launcher, scenarios

Component 9 of the deslop campaign (DSLP-13, DSLP-40, DSLP-67). Scope:
`crates/kache-{client,core,format,ipc,log,proofs,protocol,shims}`, `launcher/`
and `scenarios/`. Base `11cbdba8`, branch `deslop/k3`, macOS arm64,
`nightly-2026-11-09`. The scope is 13,173 Rust lines in the crates and launcher,
plus 499 lines of e2e fixture sources under `scenarios/*/source`.

Contracts in this scope: the daemon wire protocol (`kache-protocol`), the entry
format and key-schema constant (`kache-format`), the descriptor-passing and
jobserver handoff (`kache-ipc`). None changed. A change to any of them is a stop
condition for this lane.

Host notes. Builds on this host were killed several times by SIGKILL or SIGTERM
under load (average 45 to 70); each was retried and passed. Commands that
compile `kache` itself ran with the system `TMPDIR`, because a checkout-local
`TMPDIR` fails 4 `cache_key` tests (DSLP-88). `RUSTC_WRAPPER=` was cleared so
no installed `kache` sat between cargo and clippy.

## DSLP-13: Types

Commands (all from the worktree root):

```sh
cargo clippy --workspace --all-targets -- -D warnings            # baseline, exit 0
RUSTFLAGS="--cap-lints warn" CARGO_TARGET_DIR=.work/tmp/target-lint \
  cargo clippy --workspace [--all-targets] --message-format=json -- \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value -W unsafe_code -W unreachable_pub \
  -W clippy::indexing_slicing -W clippy::as_conversions -W clippy::cast_possible_wrap \
  -W clippy::cast_lossless -W clippy::fn_params_excessive_bools \
  -W clippy::struct_excessive_bools -W clippy::if_not_else
```

The JSON was bucketed by primary span. The launcher and scenarios are outside
the cargo workspace, so the launcher was measured with `clippy-driver` on
`launcher/build_script.rs` directly and the scenarios are fixtures (see below).

Candidate counts for the eight crates (production = lib and bins only;
all = with tests, benches and build scripts):

| Candidate | Prod | All | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 39 | 72 | Adopted as `forbid` in `kache-format`, `kache-log`, `kache-protocol`, `kache-proofs` (0 hits, 0 false positives). Rejected for `kache-client` (8), `kache-core` (1), `kache-ipc` (29), `kache-shims` (1): each is hand-written FFI (`SCM_RIGHTS`, `fcntl`, `fork`, `getpeereid`, `localtime_r`, `geteuid`) with no safe std equivalent and no dependency may be added. |
| `unwrap_used` | 0 | 579 | Rejected. With `allow-unwrap-in-tests` the production count is 0, so the lint rejects nothing today; every hit is a test assertion. |
| `expect_used` | 4 | 14 | Rejected. With `allow-expect-in-tests` the 4 hits are all `crates/kache-client/build.rs`, a build script that must abort the build (4 false positives in 13k lines exceeds one per 5k). |
| `wildcard_enum_match_arm` | 8 | 8 | Rejected. Each arm is a deliberate safe default (`Slot::Other` falls to `Skip`, `Request::client_version` and `Attempt::terminal_code` fall to `None`, `Error::source`, the deser `Atom`, reply mismatches that `bail!`). Listing variants would add no defect detection; `Request::client_version` and `is_build_activity` now have a per-variant table test instead (DSLP-40). |
| `match_wildcard_for_single_variants` | 0 | 0 | Nothing to adopt. |
| `cast_possible_truncation` | 25 | 35 | Rejected. Bounded by construction (`CMSG_SPACE` of at most `MAX_DESCRIPTORS` ints, `as u8` on clamped calendar fields); a `try_from` would add error arms for unreachable states. |
| `cast_sign_loss` / `cast_possible_wrap` | 8 / 7 | 10 / 10 | Rejected for the same reason (`libc` width conversions, `i128` nanosecond arithmetic). |
| `cast_lossless`, `as_conversions`, `indexing_slicing` | 42, 99, 19 | 46, 136, 65 | Rejected: style or idiom, no witness. The 19 production slices were not individually audited and none has a witness. |
| `needless_pass_by_value` | 4 | 5 | Rejected: owned arguments on `NativeString`/`Spawn` handlers and `impl ToString`, kept by value for the call sites. |
| `redundant_clone` | 0 | 3 | Rejected: nursery lint, all three in test fixtures. |
| `unreachable_pub` | 1 | 1 | Rejected: the one hit is `imp::local_offset_seconds` in the private platform module of `time.rs`, an intentional `pub` inside a private module. |
| `fn_params_excessive_bools`, `struct_excessive_bools`, `if_not_else` | 0 | 0 | Nothing to adopt. |

Launcher: `clippy::undocumented_unsafe_blocks` reported 0 (every unsafe block
carries a `SAFETY` comment), `multiple_unsafe_ops_per_block` 6, `indexing_slicing`
4, `as_conversions` 1. All are inherent to a `no_std` libc shim and were
rejected. Scenarios: the 116 hatches under `scenarios/` are fixture build scripts
and `#[no_mangle]` FFI fixtures whose bytes are test inputs; not measured further.

Escape hatches (`unsafe`, `#[allow]`, `.unwrap()`, `.expect(`, `git grep` over
the scope at `11cbdba8` and now): production hatches 0 added; test-only
`.unwrap()` +1 (the new protocol table test); `unreachable!()` 4 to 3.

Fixes, one commit each:

| Commit | Finding | Witness |
| --- | --- | --- |
| `39819a11` fix(core) | `UtcTime::parse_rfc3339` sliced the offset digits by byte index, so `...05+a\u{e9}x` panicked inside an event-log parse; it also accepted `+-5` as five hours west. Only ASCII digit pairs are accepted now. | `time::tests::rfc3339_rejects_malformed_offsets_without_panicking` failed before the fix with `end byte index 2 is not a char boundary` and passes after. |
| `d387c0cc` refactor(ipc) | `exchange` tested the channel kind with `matches!`, then re-matched it with `unreachable!()` to read `terminal`. One match now carries the flag beside the broker files. | Fault: splice offset `3 + terminal` changed to `3 + !terminal` fails `execution::tests::client_process_broker_transfers_live_pipes_and_reaps_the_native_child`; restored, `cmp` exit 0. |
| `5a4351f5` build(crates) | `unsafe_code = "forbid"` in four crates. | `unsafe { zeroed() }` added to `kache-log/src/lib.rs` failed `cargo check -p kache-log` with `usage of an unsafe block`; restored, `cmp` exit 0. |

Contract check: `cargo test -p kache-protocol -p kache-format` stayed green
(entry v31 representation, legacy defaults, request round trips); no key or
format byte changed.

## DSLP-40: Tests

Tiers and cost (system `TMPDIR`, warm, `cargo test -p kache-client -p kache-core
-p kache-format -p kache-ipc -p kache-log -p kache-protocol -p kache-shims
-p kache-proofs`, 27.6s wall including a rebuild, about 1.1s of test time):
client 15 (+3 ignored), core 10, format 9 + 4, ipc 59 (+2 ignored), log 10,
proofs 2, protocol 8 + 6, shims 46. The launcher is exercised by
`tests/launcher_personality_test.rs` (4 tests, 0.6s) and
`tests/build_script_launcher_test.rs` (2 tests, 183s on a busy host: they
compile the launcher and run Cargo against it).

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Entry metadata serialization and legacy defaults | `crates/kache-format/tests/compatibility.rs:4,17,36,42` | Guarded. |
| Cache-key, blob-hash, artifact-name trust predicates | `crates/kache-format/src/lib.rs` `is_valid_cache_key_rejects_traversal_and_malformed`, `is_safe_stored_artifact_name_allows_only_build_script_trees_beyond_one_component`, `covers_requested_emit_semantics` | Guarded (a duplicate I drafted was dropped after the existing test showed up). |
| `CACHE_KEY_VERSION = 34` | none in the crate; the key tests in `src/cache_key/tests.rs` hash it | Intentionally unpinned: a literal assertion would only restate the constant. |
| Request/response wire shapes | `crates/kache-protocol/tests/compatibility.rs:8,33,49,63,73` | Guarded. |
| Request busy-ness and declared client release | `only_build_requests_keep_the_machine_busy_and_only_some_declare_a_release` (new), `src/daemon/tests.rs:820-836` | Gap closed: the crate tested one variant of each. `PublishCc` is the one variant not constructed (needs a `BuildEvent`); it stays covered only through the daemon. |
| Execution handoff, native strings, diagnostics | `crates/kache-protocol/src/execution.rs` tests, `crates/kache-ipc/src/execution.rs` (7) | Guarded; the new fault below. |
| Descriptor passing, jobserver, deadline pool | `kache-ipc` `descriptors.rs` (16), `jobserver.rs` (9), `deadline_io.rs` (3) | Guarded. |
| Shim farm, frontend selection, companion naming | `kache-shims` `farm/tests.rs` (29), `select/tests.rs` (8), `compiler_name.rs` (3) | Guarded. |
| Time parsing and formatting | `kache-core` `time.rs` (6, one new) | Gap closed by the offset test. |
| Log filter dialect and routing | `kache-log` (10) | Guarded. |
| `checked_file_region` proof | `kache-proofs` test plus Kani harness | Kani is `Needs environment`. |
| Launcher personality selection | `launcher/compile.rs` tests (3), `tests/launcher_personality_test.rs` | Guarded; fault below. |
| Scenario fixtures | `make e2e` (needs real compilers, `--deny-missing-tools`) | `Needs environment`; not run. |

Injected fault in a critical decision (launcher export selection):
`found.is_some_and(|previous| previous != symbol)` changed to `== symbol` in
`launcher/compile.rs`. `cargo test -p kache --test launcher_personality_test`
exit 101, `launcher::tests::refuses_missing_unsupported_or_conflicting_personalities`
FAILED (3 passed, 1 failed); restored, `cmp` exit 0. The new protocol table was
proved with two faults: removing `| Request::Health` from `is_build_activity` and
turning `Request::CompileStarted(req) => req.client_version.as_deref()` into
`None` each failed `only_build_requests_keep_the_machine_busy_and_only_some_declare_a_release`.

Coverage. `cargo-llvm-cov` is not installed, but the pinned toolchain ships
`llvm-profdata` and `llvm-cov`, so coverage was measured by hand:

```sh
RUSTC_WRAPPER= CARGO_TARGET_DIR=.work/tmp/target-cov RUSTFLAGS="-C instrument-coverage" \
  cargo test -p kache --lib --no-run     # and the eight crates below
LLVM_PROFILE_FILE=$PWD/.work/tmp/cov/%p-%m.profraw cargo test -p kache --lib
LLVM_PROFILE_FILE=... cargo test -p kache-client -p kache-core -p kache-format \
  -p kache-ipc -p kache-log -p kache-protocol -p kache-shims -p kache-proofs
llvm-profdata merge -sparse .work/tmp/cov/*.profraw -o .work/tmp/cov.profdata
llvm-cov export --format=lcov --instr-profile=.work/tmp/cov.profdata --object <each test binary>
```

Lines inside `#[cfg(test)]` items were removed from the lcov before counting, so
the figures are production lines only. Unit and in-crate integration tests only:
the root crate's `tests/*.rs` binaries were not instrumented, and the launcher
runs in a compiled child. One unrelated test failed under instrumentation
(`daemon::tests::detached_daemon_survives_sigint_to_the_callers_group`, outside
this scope, not investigated; it spawns a detached daemon under a loaded host).

| Crate | Production lines covered | Notes |
| --- | --- | --- |
| `kache-client` | 177/269 (65.8%) | `frontend.rs` 19/98: its tests spawn child processes whose counters are not merged here. |
| `kache-core` | 360/401 (89.8%) | |
| `kache-format` | 46/46 (100%) | |
| `kache-ipc` | 882/954 (92.5%) | |
| `kache-log` | 152/173 (87.9%) | |
| `kache-protocol` | 294/320 (91.9%) | `fallback.rs` 0/5 and `execution/process.rs` 0/8 are the unused, untested items listed under DSLP-67. |
| `kache-shims` | 798/821 (97.2%) | |
| `kache-proofs` | 0 production lines | The proof module is `include!`d under `cfg(any(kani, test))`. |
| Total | 2,709/2,984 (90.8%) | |

Needs environment: `cargo-mutants` (DSLP-87) and branch coverage (the
instrumented build did not enable it). With no mutation run there is no survivor
list, so the scoped lane is unqualified and the acceptance "no undispositioned
survivor" cannot be claimed for this scope; the uncovered production lines above
and the hand-injected faults are the substitute evidence. Kani and `make e2e`
are also absent.

## DSLP-67: Dead code

Tools and results:

| Tool | Result |
| --- | --- |
| rustc `dead_code` through `cargo clippy --workspace --all-targets -- -D warnings` | Clean at base and after. |
| `cargo` `unused_dependencies` manifest lint (nightly-2026-11-09, warn by default) over `cargo check --workspace --all-targets` | No warnings. Negative control: adding an unused `zstd = "0.14"` to `kache-log` produced `warning: unused dependency zstd`; reverted with `git checkout`. |
| `unreachable_pub`, workspace-wide | 1 hit in scope (`imp::local_offset_seconds`, a private-module `pub`), not dead. |
| Whole-word search for every `pub` item of the scope across the tree (`.rs`, `.md`, `.toml`, `.sh`, `.py`, `.yml`) | 3 items referenced only at their definition, listed below. |
| `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat` | `Needs environment`: not installed. The cargo lint above covers the dependency question. |

Consumer questions (not deleted: every crate here except `kache-proofs` is a
published library, and these are public items):

| Item | Where | Evidence |
| --- | --- | --- |
| `Attempt::terminal_code` | `crates/kache-protocol/src/fallback.rs:24` | `rg -w terminal_code` finds only the definition. |
| `SharedPrediction` (and `PlainRow`, reachable only through it) | `crates/kache-protocol/src/prediction.rs:22` | No other reference in the tree. It sits in a wire type module, so removal could break a remote reader. |
| `Parts::month_day_clock` | `crates/kache-core/src/time.rs:506` | No caller anywhere. |
| `UtcTime::format_sqlite` | `crates/kache-core/src/time.rs:245` | Only `src/cli/tests.rs:1723` calls it. |
| `Stdio::QUIET` | `crates/kache-protocol/src/execution/process.rs:32` | Only `crates/kache-ipc/src/execution/unix_client.rs:617` (a test) uses it. |

Private dead code found and removed: none. Allowlist: none needed.
Duplication noted for the parent, not changed: `invalid` and `unknown` are defined
identically in `kache-ipc/src/execution/unix_client.rs` and
`kache-ipc/src/execution/workspace.rs`.
