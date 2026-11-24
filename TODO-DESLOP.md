Prefix: DSLP
Next: 126

Deslop ticket backlog. See the [roadmap](docs/roadmap.md).

## Tickets

### DSLP-88 · P2 · Make workspace tests independent of checkout-local temporary roots and ambient KACHE_CACHE_DIR

- **Area:** Needs environment · test isolation
- **Where:** src/cache_key/tests.rs, src/cli/tests.rs, src/out_dir_alias.rs, src/incremental_policy.rs, crates/kache-config/src/config/tests.rs
- **Problem:** With TMPDIR inside the checkout, make test fails 11 kache library tests because scratch paths inherit the checkout workspace, toolchain and path-normalization context; the same 11 fail at base aa8df06 (its deeper detached worktree adds two Unix socket path-length failures), and all pass with the system TMPDIR. Separately, with KACHE_CACHE_DIR exported (the isolation the deslop rules require), kache-config's config::tests::probe_memos_live_under_the_configured_cache_dir fails: its second probe_memo_dir() call reads the ambient variable instead of the default. The suite stops at the first failing test binary, so later binaries do not run.
- **Recommendation:** Make test fixtures explicitly control workspace ancestry, path-normalization roots and socket path length while keeping caches and configuration isolated; run make test with TMPDIR under .work/tmp.
- **Fix with:** test-audit
- **Effort:** M

### DSLP-125 · P2 · Keep Cargo test binary paths available through the installed rustc wrapper

- **Area:** Cargo integration
- **Where:** src/cache_key/tests.rs and src/build_script_inputs.rs
- **Problem:** On macOS arm64, the kache 2.0.0-rc.2 install recorded as release-2343033e183f5b0d72f4c023400cbdbc099a8450 rejects env! for CARGO_BIN_EXE_oxish-keygen during cargo test --locked --manifest-path client/Cargo.toml --test keygen in laputa-systems/ssh-build at b5046ca. RUSTC_WRAPPER= with the same command passes. Clippy on a second workspace test target hit the same error. Current source behavior still needs reproduction.
- **Recommendation:** Reproduce with the installed release and current source using a bin plus integration test pair, then require the wrapped build to see the same Cargo-provided binary path as a direct build after a cold invocation and a cache hit.
- **Fix with:** root-cause
- **Effort:** M

### DSLP-87 · P3 · Provision coverage and mutation tools for deslop verification

- **Area:** Needs environment · deslop
- **Where:** cargo-llvm-cov and cargo-mutants
- **Problem:** Neither cargo-llvm-cov nor cargo-mutants is on PATH on the Ground host, so the later coverage and bounded mutation measurements cannot run here.
- **Recommendation:** Provide the approved tools in the execution environment, then run the scoped coverage and mutation work in the component tickets.
- **Fix with:** deslop
- **Effort:** S

### DSLP-91 · P3 · ExtraInputsSnapshot optional fields are re-asserted Some by eight production expects

- **Area:** Types · extra inputs
- **Where:** src/extra_inputs.rs
- **Problem:** ExtraInputsSnapshot::digest and provider.package.crate_name are Option, while 8 production expect calls re-assert they are Some; the declaration-path list is also duplicated.
- **Recommendation:** Split the type into resolved and unresolved forms so the invariant is carried by the type, and keep one declaration-path list.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** M

### DSLP-92 · P3 · EventTailer is test-only yet the sole reader of the event-log rotation marker

- **Area:** Dead code · events
- **Where:** src/events.rs:607
- **Problem:** EventTailer exists only for tests but is the only code that reads the rotation marker, so the marker format is specified only by test code inside the production module.
- **Recommendation:** Move EventTailer and its tests to tests/ with the rotation format documented, or remove both if the marker has no consumer.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-93 · P3 · Process promptness tests use 2 s bounds sized for an idle host

- **Area:** Tests · invocation
- **Where:** src/invocation/process.rs:489, :565, :639, :815, :845
- **Problem:** The remaining promptness assertions bound child handling at 2 s; a sibling 3 s guard already failed once at load average ~50 (raised to 30 s in this pass).
- **Recommendation:** Assert the behaviour (child reaped, signal observed) with a generous hang guard instead of a tight elapsed-time bound.
- **Fix with:** root-cause
- **Effort:** S

### DSLP-95 · P3 · src/scheduler/, launcher/, scenarios/ and crates/README.md are in no deslop component

- **Area:** deslop · scope
- **Where:** src/scheduler/, launcher/ (396 lines, 20 unsafe), scenarios/, crates/README.md
- **Problem:** These paths fall outside all 28 component scopes, so no Types/Tests/Dead code pass covered them; launcher/ alone holds 20 unsafe blocks.
- **Recommendation:** Add one component ticket group covering them and run the three steps.
- **Fix with:** deslop
- **Effort:** M

### DSLP-96 · P3 · 28 test early returns skip silently when a prerequisite is missing

- **Area:** Tests
- **Where:** tests/ (e.g. missing cc, running as root)
- **Problem:** Tests print 'skipping' and return, so a CI host missing cc or running as root passes them without exercising anything.
- **Recommendation:** Route them through a common::skip_unless helper that panics when KACHE_TEST_DENY_SKIPS is set, and set it in CI.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-97 · P3 · Lib tests fail under host load through wall-clock waits rather than readiness signals

- **Area:** Tests
- **Where:** src/invocation/, src/probe/, src/service.rs, src/daemon_execution.rs, src/daemon_lifecycle.rs, kache-store wait_for_committed_returns_false_without_an_owner
- **Problem:** Under load average 25-45 these fail with process-spawn timeouts and EAGAIN and pass alone; they wait on wall-clock bounds rather than readiness signals. A loaded run during the probe/report pass also failed service::tests::manager_command_deadline_bounds_wait_and_caps_diagnostics, daemon::execution::tests::*handoff_streams_two_compilers*, daemon::lifecycle_client::tests::abandoned_startup_reaps_the_child_without_terminating_it and cli::doctor_probe::tests::failed_and_stalled_compilers_return_details_and_finish (docs/deslop-src-probe-report.md, DSLP-53 Tests).
- **Recommendation:** Replace the waits with readiness signals or generous condition-wait bounds; prove each with a deterministic delay.
- **Fix with:** root-cause
- **Effort:** M

### DSLP-98 · P3 · Launcher build and install paths lack a decisive test

- **Area:** Tests · build
- **Where:** build.rs (link_flags), tests/build_script_launcher_test.rs
- **Problem:** link_flags is untested because build scripts are not test targets, and build_script_launcher_test still passes with a 0-byte embedded launcher.
- **Recommendation:** Move link_flags into launcher/compile.rs with unit tests, and assert the installed launcher bytes equal the embedded ones.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-99 · P3 · CLI reference and mutants config drift from the binary

- **Area:** Docs · tests
- **Where:** docs/commands/reference.mdx, .cargo/mutants.toml lines 1-250
- **Problem:** No test compares kache --help with the command reference (it documented a removed completions command), and the mutants.toml header still describes removed Windows and TUI exclusions.
- **Recommendation:** Add a test that diffs --help output against the reference, and rewrite the mutants.toml header.
- **Fix with:** single-source-of-truth
- **Effort:** S

### DSLP-100 · P3 · CI runs none of the client-crates, resource-guard, make pr or docs checks

- **Area:** Needs owner · CI
- **Where:** .github/workflows/, Makefile (client-crates-check, test-resource-guard-check, pr), scripts/check-docs.sh
- **Problem:** These local gates pass but no workflow runs them, so regressions they catch reach main.
- **Recommendation:** Owner decision on CI work: wire them into the CI workflow.
- **Fix with:** deslop
- **Effort:** S

### DSLP-104 · P3 · kache-e2e fixture results treat status and phase names as unchecked strings

- **Area:** Types · kache-e2e results
- **Where:** crates/kache-e2e/src/fixture.rs (FixtureResult.status), crates/kache-e2e/src/result.rs (PhaseResult.status, PhaseResult.phase), compared as literals at about 36 sites in runner.rs, fixture_runner.rs, result.rs, tests/strict_gate.rs and fixture.rs; docs/deslop-e2e-fixture.md (DSLP-7 Types)
- **Problem:** status holds a closed set (pass, fail, skip) and PhaseResult.phase holds a name Phase::name() already produces, but both are String compared as literals at about 36 sites, so a misspelled status compiles and matches nothing.
- **Recommendation:** Replace both with a #[non_exhaustive] enum carrying the same wire strings, behind a results.json compatibility decision.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### DSLP-105 · P3 · The kache-e2e negative-control verdict has no test because it sits beside process::exit

- **Area:** Tests · kache-e2e
- **Where:** crates/kache-e2e/src/fixture_runner.rs (run: the want-pass, passed, verdict and exit-code decision); docs/deslop-e2e-fixture.md (DSLP-34 Tests)
- **Problem:** The negative-control verdict (VACUOUS, falsifiable, exit 1) is decided inside fixture_runner::run next to process::exit, so no test reaches it; the assertion surface has no guard.
- **Recommendation:** Extract the decision into a pure function over (want-pass, passed, verdict) and test its truth table, leaving run to map the result to an exit code.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-106 · P3 · kache-fs builds a Windows branch no repository build can reach

- **Area:** Needs owner · kache-fs
- **Where:** crates/kache-fs/src/home.rs (#[cfg(windows)] mod imp, 130 lines, five unsafe blocks); docs/deslop-fs.md (DSLP-62 Dead code)
- **Problem:** home.rs carries a 130-line #[cfg(windows)] module with five unsafe blocks, but the crate takes std::os::unix unconditionally, CI has no Windows job, and no other cfg(windows) exists in the crate, so no build the repository runs can reach it.
- **Recommendation:** Owner decision on platform support: delete the Windows branch, or restore Windows support across the crate and add a Windows job.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-107 · P3 · build_script's second-run settle path has no test

- **Area:** Tests · build script
- **Where:** src/build_script.rs:598-621 (run_cached, InputsChanged branch with reruns_settle); docs/deslop-build-script.md (DSLP-41 Tests)
- **Problem:** reruns_settle and mark_unsettled are unit-tested alone, but the branch that replays after a script rewrote a file it declared is reached by no lib test, because run_cached loads Config, opens the real store and spawns the preserved script.
- **Recommendation:** Add an integration test where a script rewrites a declared file, asserting one rerun and an unsettled: marker on the second failure.
- **Fix with:** test-audit
- **Effort:** M

### DSLP-108 · P3 · Nine scripts/ entry points have no caller in the repository

- **Area:** Needs owner · scripts
- **Where:** scripts/ci-changes.py, scripts/test-ci-changes.py, scripts/ci/resolve-version.sh, scripts/require-ci-green.sh, scripts/ci-monitor.sh, scripts/workspace-members.sh, scripts/check-coverage-scope.sh, scripts/volume-shards-check.py, scripts/test-unavailable-proc-labels.sh; docs/deslop-scripts.md (DSLP-66 Dead code)
- **Problem:** No workflow, Makefile target, doc or other script names these nine files, so nothing runs them. ci-changes.py and ci/resolve-version.sh decide CI groups (nix, helm, perf-gate) the workflows no longer have and only test-ci-changes.py references them; require-ci-green.sh claims to gate irreversible publishes on CI but nothing calls it; ci-monitor.sh defaults to Zondax/kunobi-frontend, not this repository; workspace-members.sh is called by no recipe and check-coverage-scope.sh only by it; volume-shards-check.py has no caller; test-unavailable-proc-labels.sh selects a live Rust test but has no target. Most implement release or CI policy, so removal is an owner decision.
- **Recommendation:** Owner decision per script: retire the ones whose policy is gone, and wire the ones still meant to run (volume-shards-check.py, test-unavailable-proc-labels.sh) into a Makefile target or workflow.
- **Fix with:** dead-code-archaeology
- **Effort:** M

### DSLP-113 · P3 · init printing 'Login service: configured (<path>)' has no test

- **Area:** Tests · CLI init
- **Where:** src/cli.rs (init login-service line, src/cli.rs:6212); tests/cli_commands_test.rs:2299 onward (Linux-only fake-systemd runs assert the install, skip and update lines only); docs/deslop-cli.md (DSLP-45 Tests)
- **Problem:** The installed-and-current service branch that prints 'Login service: configured (<path>)' is unasserted: the existing service tests are Linux-only fake-systemd runs that check the install, skip and update lines, so this line is provable on neither macOS nor the existing Linux fake.
- **Recommendation:** Extend the fake-systemd service fixture to an already-configured, current service and assert the configured line.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-117 · P3 · scheduler pressure ENABLED and set_enabled are test-only and the doc names a removed knob

- **Area:** Dead code · scheduler
- **Where:** src/scheduler/pressure.rs:36 (ENABLED, set_enabled), src/main.rs:1233 (InvocationPressureScope::enter(true)), src/scheduler.rs; docs/deslop-src-probe-report.md (DSLP-80 Dead code)
- **Problem:** pressure::ENABLED and pressure::set_enabled are only written by tests; production enters pressure through InvocationPressureScope::enter(true). The doc comment still names [cache] scheduler_memory_pressure, a knob removed in c2eca807. Cleaning it up spans main.rs and scheduler.rs, outside the component that found it.
- **Recommendation:** Remove ENABLED and set_enabled in favour of the invocation scope the production path already uses, and drop the stale knob from the doc comment.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-121 · P3 · Linux CRT driver probes carry 144 uncovered lines that macOS cannot exercise

- **Area:** Tests · cache key
- **Where:** src/cache_key.rs (record_crt_placement_memo, crt_placement_memo_for, probe_files_with_hash, print_file_name_for, probe_linux_crt_objects_memoized_for, place_linux_crt_objects, driver_search_dirs_for, driver_library_search_dirs_for); docs/deslop-cache-key.md (DSLP-43 Tests)
- **Problem:** 144 production lines of the Linux host CRT/libc identity probes are unreachable on the macOS host that measured coverage, and the record could not confirm that a Linux CI cell covers them rather than assuming it does.
- **Recommendation:** Run the instrumented suite in a Linux cell and record the probe coverage; add a focused test for any probe still uncovered there.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-122 · P3 · Four Python test suites under scripts/ run in no CI job or make target

- **Area:** Needs owner · CI
- **Where:** scripts/test-ci-changes.py, scripts/test-crates-io.py, scripts/test-mutation-diff.py, scripts/test-version-consistency.py; docs/deslop-scripts.md (DSLP-39 Tests)
- **Problem:** No .github/workflows job and no Makefile target runs these four suites, so a regression in ci-changes.py, crates-io.py publish ordering, mutation-diff.py or check-version-consistency.sh reaches main untested. Their local cost is under 3 s each except test-mutation-diff.py at 34 to 78 s.
- **Recommendation:** Owner decision on CI: add a workflow job (or a make target wired into one) that runs the four suites.
- **Fix with:** deslop
- **Effort:** S

### DSLP-90 · P4 · import_downloaded_entry follows symlinks in a downloaded entry (latent, no caller)

- **Area:** Needs owner · remote storage
- **Where:** crates/kache-store/src/store.rs: import_downloaded_entry, move_verified_blob, set_blob_readonly
- **Problem:** An entry directory holding artifact -> /outside/file whose bytes match the claimed hash passes validation (is_file and fs::metadata follow links); the blob is then renamed in as a symlink and set_permissions makes the outside file read-only. Reproduced against the library with a scratch crate under .work/tmp: blob_is_symlink=true, outside_mode=444 from 0644. No production code calls import_downloaded_entry or import_restored_entry (tests only) and AGENTS.md defers remote storage, so it is latent. Details: docs/deslop-diagnose.md.
- **Recommendation:** Owner decision: delete the unused import path in the dead-code pass, or before a remote layer returns check symlink_metadata is a regular file and open the blob without following links. Leave the code until decided.
- **Fix with:** os-boundaries
- **Effort:** S

### DSLP-94 · P4 · Small typing and dead-code leftovers from the args, extra inputs, daemon and incremental pass

- **Area:** Types · dead code
- **Where:** src/ (unreachable_pub in components 17/19/20), src/extra_inputs.rs, src/daemon_execution.rs:688, src/daemon.rs:1709, src/incremental_policy.rs:386, src/main.rs:1205, src/config.rs, .gitignore
- **Problem:** unreachable_pub flags 42/62/44 items in components 17/19/20; six #[cfg(test)] items sit outside mod tests in extra_inputs.rs; a pure truth-table test in daemon_execution.rs is Linux-gated; three redundant clones; config_path_lock is exported at two paths; .gitignore ignores docs/**/*.md so deslop docs need git add -f.
- **Recommendation:** Narrow visibility and adopt unreachable_pub; move the cfg(test) items into mod tests; drop the Linux gate; remove the clones; export config_path_lock once; add !docs/deslop-*.md to .gitignore.
- **Fix with:** deslop
- **Effort:** S

### DSLP-101 · P4 · Build, docs and config leftovers from the tests and configuration pass

- **Area:** Dead code · docs
- **Where:** Cross.toml, Cargo.toml [features], README.md, TODO.md:8, AGENTS.md (DSLP-89 gap)
- **Problem:** Cross.toml has no in-repo consumer and [features] default = [] is unused (consumer questions); README links a missing bug_report issue template; TODO.md says 67 oracle cases where docs/roadmap.md and cases.toml say 72; AGENTS.md lists DSLP-89 as open though cache_key_underkeying_test.rs:1166-1174 guards it.
- **Recommendation:** Confirm consumers then remove Cross.toml and the empty features table; fix the README link and the case count; drop the stale AGENTS.md gap.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-102 · P4 · kache-config's file-write feature, EnvOverrides and FileConfig writers have no consumer in the tree

- **Area:** Needs owner · kache-config
- **Where:** crates/kache-config/src/config.rs (update_file_config_at, save_file_config_to, EnvOverrides, load_raw_file_config_from, user_bypass_reason, cache_dir_for_path, user_config_path, host_config_path_for, config_file_fingerprint, volume_store_for, DEFAULT_* constants); docs/deslop-config.md and docs/deslop-kache-config-dead-code.md (DSLP-5 Dead code)
- **Problem:** kache-config is publishable and no workspace caller uses the file-write surface (Config::update_file_config_at, Config::save_file_config_to) or EnvOverrides; several public items have no reference anywhere in the tree (Config::cache_dir_for_path, DEFAULT_DAEMON_IDLE_TIMEOUT_SECS, DEFAULT_GC_MAX_AGE_HOURS, DEFAULT_MIN_STORE_COMPILE_MS), and the unused set looks like the remains of a config editor. Deleting them is a decision about supported API, not a dead-code cut.
- **Recommendation:** Decide whether file-write, EnvOverrides and the FileConfig writers are supported API; if they are not, delete them with their tests in one commit.
- **Fix with:** dead-code-archaeology
- **Effort:** M

### DSLP-103 · P4 · Small leftovers from the k1 pass: config typing, kache-e2e and kache-fs

- **Area:** Types · dead code
- **Where:** crates/kache-config/src/config.rs (7 helpers take Result<FileConfig> but only call .as_ref().ok(); misplaced doc comments on index_db_path and ignore_env_enabled), crates/kache-config/src/invocation.rs:77 (resolve's absolute-cwd filter is redundant, the final resolved.is_absolute() check enforces it), crates/kache-e2e/src/lib.rs (portable_path), crates/kache-e2e/src/result.rs (collect_meta), crates/kache-e2e/src/cc_oracle/judge.rs and exec.rs:626 (Verdict/Arm wildcard), crates/kache-e2e/src/source.rs:45 and :60 (discarded copy results), crates/kache-e2e/src/bin/kache-cc.rs and kache-cxx.rs (no test; need a sibling kache binary), crates/kache-e2e/src/bin/e2e-fallback.rs (no test), crates/kache-e2e/src/cc_bench_cli.rs and cc_oracle_cli.rs (Result<_, String>), crates/kache-fs/src/paths.rs:3 (strip_verbatim_prefix is an identity function), crates/kache-fs/src/lib.rs and ledger.rs (PinnedOutside not re-exported), crates/kache-fs/src/extents.rs (ACCOUNTABLE_FLAGS 0x3801 names two flags with no constant), crates/kache-fs/src/staging.rs (private struct Clone shadows the trait), crates/kache-fs copy.rs:34,62,67,89, home.rs:66,68,80, staging.rs:41,46,48, volume.rs:329,330, capacity.rs:65, apfs.rs:147 (unsafe without a SAFETY comment), crates/kache-fs home.rs (passwd fallback and XDG_CACHE_HOME untested); docs/deslop-config.md (DSLP-3 Types, DSLP-4 Tests), docs/deslop-e2e-bench.md (DSLP-6 Types, DSLP-33 Tests), docs/deslop-e2e-fixture.md (DSLP-7 Types, DSLP-61 Dead code), docs/deslop-fs.md (DSLP-8 Types, DSLP-35 Tests, DSLP-62 Dead code)
- **Problem:** Small leftovers the four k1 lanes recorded and did not change. config: seven helpers take Result<FileConfig> but only read .as_ref().ok() while the error is reported separately, two doc comments sit on the wrong items (index_db_path, ignore_env_enabled), and the absolute-cwd filter in resolve is redundant with the final resolved.is_absolute() check (mutant M10 is equivalent by inspection). kache-e2e: lib.rs::portable_path is an identity function (9 callers) and result::collect_meta is unreferenced; the oracle Verdict/Arm matches use _ for every other arm so a new Arm is silently treated as kache-like at exec.rs:626; the harness parsers return Result<_, String>; source.rs:45 and :60 discard the result of copying a pinned .cargo/config.toml, so a failed copy silently builds without the pin; kache-cc, kache-cxx (need a sibling kache binary) and e2e-fallback have no test; adoption of unwrap_used/expect_used is blocked on the 13 production unwrap() and 3 expect() in cc_bench/mod.rs, cc_oracle/gcov.rs, bench_profile.rs and bench_runner.rs; the E17 mutant (dropping found.sort()) survives on APFS because directory order is unobservable there, so only the ext4 CI cell kills it. kache-fs: paths::strip_verbatim_prefix is an identity function (Windows remnant) called by the root crate; ledger::PinnedOutside is returned but not re-exported; extents.rs names four flags in a comment but two have no constant; staging.rs's private struct Clone shadows the trait; 14 production unsafe sites have no SAFETY comment within four lines (copy.rs:34,62,67,89, home.rs:66,68,80, staging.rs:41,46,48, volume.rs:329,330, capacity.rs:65, apfs.rs:147); the $HOME-unset passwd fallback and XDG_CACHE_HOME are untested; six public methods (DirSizing::is_partial, DirSizing::shared_ratio, FileSizing::frees_nothing, VolumeUsage::used_ratio, CloneSketch::shares_with, InodeLedger::distinct_inodes) have no consumer outside the published crate.
- **Recommendation:** Take Option<&FileConfig> where the error is reported separately, fix the two doc comments and drop the redundant cwd filter; type the oracle Verdict/Arm matches exhaustively; delete portable_path, collect_meta and strip_verbatim_prefix; propagate the .cargo/config.toml copy failure in source.rs; convert the 16 production unwrap/expect and deny both crate-wide with allow-unwrap-in-tests; re-export PinnedOutside, name the two extents flags, rename the private struct Clone; add the SAFETY comments and test the passwd fallback and XDG_CACHE_HOME; confirm whether an out-of-tree consumer uses the six kache-fs methods.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### DSLP-109 · P4 · bench/short.py samples.json timestamps have no in-repo reader

- **Area:** Dead code · bench
- **Where:** scripts/bench/short.py and scripts/bench-contention.py:1198 (SampleRecord started_monotonic_s, ended_monotonic_s); docs/deslop-scripts.md (DSLP-66 Dead code)
- **Problem:** started_monotonic_s and ended_monotonic_s are written into samples.json but no reader in the repository consumes them, so whether they are an external contract or dead weight is unknown.
- **Recommendation:** Confirm whether the persisted samples are read outside the repository; if not, drop the two fields from SampleRecord.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-110 · P4 · Published kache-store methods and link helpers have no production caller

- **Area:** Needs owner · kache-store
- **Where:** crates/kache-store/src/store.rs (blobs_at_least, keys_for_crates, wait_for_committed_with_timeout, reconcile_blob_index, import_verified_restored_entries), crates/kache-store/src/link.rs:1387 (rebase_depinfo_content), crates/kache-store/src/link.rs:104 (set_shared_hardlink_restores); docs/deslop-kache-store-remaining.md (DSLP-65 Dead code), docs/deslop-kache-store-core.md (DSLP-63 Dead code)
- **Problem:** Five Store methods and two link.rs functions are called only from the crate's own tests; no production code, doc or other crate references them. kache-store is published, so none was removed. import_verified_restored_entries is the remote-storage import AGENTS.md defers (DSLP-90), and [cache] shared_hardlink_restores is applied through the invocation scope rather than by calling set_shared_hardlink_restores.
- **Recommendation:** Confirm whether an out-of-tree consumer uses any of the seven; if not, delete them with their tests, keeping import_verified_restored_entries until remote storage returns.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-111 · P4 · store.rs sign and truncation casts are mostly behind .max(0) but not fully audited

- **Area:** Types · kache-store
- **Where:** crates/kache-store/src/store.rs (cast_sign_loss 20 hits, cast_possible_truncation 8 hits; guarded at 5266, 5752, 5842, 5875-5878, 6336, 6338); docs/deslop-kache-store-remaining.md (DSLP-11 Types)
- **Problem:** Most of the 28 casts are i64-to-u64 behind .max(0), but the remaining unguarded ones were filed rather than changed because no witness shows a defect.
- **Recommendation:** Audit each unguarded cast in store.rs against its invariant and either add the missing guard or record why it cannot underflow.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### DSLP-112 · P4 · Support-crate public items have no reference outside the repository

- **Area:** Needs owner · support crates
- **Where:** crates/kache-protocol/src/fallback.rs:24 (Attempt::terminal_code), crates/kache-protocol/src/prediction.rs:22 (SharedPrediction, PlainRow), crates/kache-core/src/time.rs:506 (Parts::month_day_clock), crates/kache-core/src/time.rs:245 (UtcTime::format_sqlite), crates/kache-protocol/src/execution/process.rs:32 (Stdio::QUIET); docs/deslop-support-crates.md (DSLP-67 Dead code)
- **Problem:** Each of these five public items has no reference anywhere in the tree beyond its definition (format_sqlite only from a cli test, Stdio::QUIET only from a kache-ipc test). Every crate here is published, and SharedPrediction sits in a wire type module where removal could break a remote reader, so none was deleted.
- **Recommendation:** Confirm whether an out-of-tree consumer uses any of them; if not, delete the unused ones and keep SharedPrediction only if a remote reader needs it.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-114 · P4 · cache_key test module serializes on one lock and 27 tests can pass without asserting

- **Area:** Tests · cache key
- **Where:** src/cache_key/tests.rs (25 tests return when rustc is missing, 2 when the cache directory is unwritable; 126 tests share key_test_lock); docs/deslop-cache-key-submodules.md (DSLP-42 Tests)
- **Problem:** 126 tests take the process-wide key_test_lock to mutate environment variables, making the module effectively serial (47 to 56 s of test time). Separately, 25 tests start with 'if !rustc_available() { return; }' and 2 return after 'skipping: <dir> is not writable', so they pass without asserting in an environment that lacks the prerequisite.
- **Recommendation:** Inject the environment through KeyEnv and InvocationEnv (already used by the newer tests) so the module can run in parallel, and turn the rustc-missing returns into assertions since cargo test guarantees rustc on PATH.
- **Fix with:** test-audit
- **Effort:** M

### DSLP-115 · P4 · Redundant clones in the cache-key and build-script hot paths

- **Area:** Dead code · clones
- **Where:** src/cache_key.rs (path.clone() in workspace_tree_digest, resolved.clone() in the cc input lookup), src/build_script.rs:536 (options.clone() before Store::open_with_options); docs/deslop-cache-key.md (DSLP-43 Tests), docs/deslop-build-script.md (DSLP-14 Types)
- **Problem:** Three clones are the last use of their value, so clippy::redundant_clone fires and each is a needless allocation on a key or build-script path. They were left because dropping a clone is not a type, defect or dead-code change.
- **Recommendation:** Drop the three clones and keep clippy::redundant_clone at warn over the crate.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-116 · P4 · kache-protocol's Request::PublishCc has no unit test

- **Area:** Tests · protocol
- **Where:** crates/kache-protocol (Request::PublishCc, the one Request variant not constructed in only_build_requests_keep_the_machine_busy_and_only_some_declare_a_release); docs/deslop-support-crates.md (DSLP-40 Tests)
- **Problem:** The per-variant busy-ness and client-release table test covers every Request variant except PublishCc, which needs a BuildEvent to construct, so it stays covered only through the daemon.
- **Recommendation:** Construct a BuildEvent fixture and add PublishCc to the table test.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-118 · P4 · The input-race suppression truth table restates the production expression

- **Area:** Tests · wrapper
- **Where:** src/wrapper.rs (should_skip_cache_store_for_input_race), src/wrapper/tests.rs (input_race_store_suppression_truth_table); docs/deslop-src-wrapper.md (DSLP-55 Tests)
- **Problem:** The truth table copies the production expression (modified_input_guard && key_inputs_changed) instead of listing the eight outcomes, so it catches operator faults but not a wrong specification.
- **Recommendation:** Replace the expression-shaped assertion with explicit rows for all eight guard/input combinations.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-119 · P4 · The elapsed-millis cast idiom repeats 42 times across the wrapper and neighbours

- **Area:** Types · duration casts
- **Where:** src/wrapper.rs (42 elapsed().as_millis() as u64 and Duration conversions), src/cli.rs, src/daemon.rs, src/build_script.rs, src/wrapper/hit.rs:28 and rustc_hit.rs:49; docs/deslop-src-wrapper.md (DSLP-28 Types), docs/deslop-src-wrapper-submodules.md (DSLP-27 Types)
- **Problem:** Almost all 42 cast_possible_truncation/as_conversions hits in wrapper.rs are the same elapsed().as_millis() as u64 conversion of a value under 584 million years, repeated rather than shared; the rest are display and counter widths. Replacing them needs one helper spanning wrapper.rs, cli.rs, daemon.rs and build_script.rs, outside the lane that found them.
- **Recommendation:** Add one Duration-to-milliseconds helper in a shared module and route the 42 sites through it, so the lint can be adopted once.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### DSLP-120 · P4 · kache-ipc defines invalid and unknown identically in two modules

- **Area:** Dead code · kache-ipc
- **Where:** crates/kache-ipc/src/execution/unix_client.rs and crates/kache-ipc/src/execution/workspace.rs (invalid, unknown); docs/deslop-support-crates.md (DSLP-67 Dead code)
- **Problem:** The two helper functions are defined identically in both modules; the duplication was noted for the parent and not changed.
- **Recommendation:** Hoist the two helpers into one shared module and import them in both files.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### DSLP-123 · P4 · default_cache_executables is pinned only by scripts/check-docs.sh text, and its unit test compares against cfg!

- **Area:** Tests · kache-config
- **Where:** crates/kache-config/src/config/tests.rs:95 (default_cache_executables compares against cfg!), crates/kache-config/src/config.rs:1315, scripts/check-docs.sh:88 (pins the literal body); docs/deslop-config.md (DSLP-4 Tests)
- **Problem:** The unit test asserts default_cache_executables() equals a cfg! expression that restates the same condition, so it cannot catch a flipped body. The only real pin is check-docs.sh's literal-body text match, which nothing in CI runs (DSLP-100, DSLP-108).
- **Recommendation:** Assert the literal expected value in the unit test instead of a cfg! expression, so the body is pinned where the test actually runs.
- **Fix with:** test-audit
- **Effort:** S

### DSLP-124 · P4 · Small leftovers from the k4 pass: unreachable_pub, a silent cc skip, a last-use clone

- **Area:** Types · dead code
- **Where:** src/cache_key.rs and src/compiler/* (unreachable_pub: 25 in cache_key.rs, 81 in the compiler files, 24 in cli.rs), src/compiler/cc/tests.rs:830 (recognizes_unknown_wrapper_via_probe returns when no C compiler is installed), src/cli.rs:603 (machine.clone() at its last use), src/cli.rs:684 (render_stats is a #[cfg(test)] helper with three consumers); docs/deslop-cache-key.md (DSLP-16 Types), docs/deslop-compiler.md (DSLP-20 Types, DSLP-47 Tests), docs/deslop-cli.md (DSLP-18 Types, DSLP-72 Dead code), docs/deslop-cc-tests.md (DSLP-46 Tests)
- **Problem:** unreachable_pub reports 25 items in cache_key.rs, 81 across the five src/compiler files and 24 in cli.rs that are pub only inside private modules; the lanes rejected the lint as style and did not adopt it, so nothing keeps the count from growing. recognizes_unknown_wrapper_via_probe still returns silently at src/compiler/cc/tests.rs:830 when no C compiler is installed, which passes without asserting where a compiler is absent (the macOS false pass was fixed, this one was kept). cli.rs:603 clones machine at its last use for no contract reason.
- **Recommendation:** Adopt unreachable_pub once across src/ and narrow the flagged items to pub(crate); route the no-compiler skip through the common::skip_unless helper (DSLP-96) or assert the expected refusal; drop the last-use clone in cli.rs.
- **Fix with:** test-audit
- **Effort:** S

