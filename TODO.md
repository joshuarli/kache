Prefix: KACH
Next: 79

### KACH-26 · P1 · Reach ccache-level local C/C++ compatibility

- **Area:** Compiler compatibility, correctness, tests
- **Where:** docs/roadmap.md (status), campaigns/cpp/cases.toml (inventory), campaigns/cpp/oracle/ (cases)
- **Problem:** Kache does not yet match pinned ccache's local C/C++ compatibility: inventory rows remain planned or open, and some lanes are unexecuted. Pinned ccache is the compatibility and reuse oracle and raw compilers are the correctness ground truth, across response files and flags, invalidation, equivalent-preprocessing reuse, complete output sets, conditional PCH/module support, host integration and local reliability. Remote storage stays on the later roadmap.
- **Recommendation:** Execute the campaign until every final acceptance criterion in the plan passes on one integrated revision; safe passthrough, missing tools and unexecuted lanes do not count as cached parity. KACH-22 (mutants in no gate) stays open alongside.
- **Fix with:** reference-parity
- **Effort:** L
- **Design:** [C/C++ parity](docs/roadmap.md#campaign-cc-parity)

### KACH-50 · P2 · Make workspace tests independent of checkout-local temporary roots and ambient KACHE_CACHE_DIR (was DSLP-88)

- **Area:** Needs environment · test isolation
- **Where:** src/cache_key/tests.rs, src/cli/tests.rs, src/out_dir_alias.rs, src/incremental_policy.rs, crates/kache-config/src/config/tests.rs
- **Problem:** With TMPDIR inside the checkout, library tests fail because scratch paths inherit workspace, toolchain and path-normalization context; deep worktrees can also exceed Unix socket path limits. With KACHE_CACHE_DIR exported, config::tests::probe_memos_live_under_the_configured_cache_dir reads the ambient value instead of the expected default.
- **Recommendation:** Make test fixtures explicitly control workspace ancestry, path-normalization roots and socket path length while keeping caches and configuration isolated; run make test with TMPDIR under .work/tmp.
- **Fix with:** test-audit
- **Effort:** M

### KACH-77 · P2 · KACHE_CC_SOURCE_DATE_EPOCH=passthrough preprocesses every hit twice

- **Area:** C/C++ performance
- **Where:** src/compiler/cc.rs (time-macro handling on the memo path)
- **Problem:** With KACHE_CC_SOURCE_DATE_EPOCH=passthrough, every C/C++ hit runs the preprocessor twice (events report preprocessor_runs=2) even for translation units the header scan has proven time-free. On 59 Chromium compiles the second warm pass took 5 s against 1 s with the default epoch handling; ccache keeps direct mode unless the source actually uses __DATE__ or __TIME__.
- **Recommendation:** Let proven time-free translation units use the direct memo under passthrough, and preprocess only those whose scan finds a time macro. Acceptance: a warm pass under passthrough reports preprocessor_runs=0 for time-free inputs, and an oracle case with __TIME__ still misses when the time changes.
- **Fix with:** make-it-fast
- **Effort:** M

### KACH-78 · P2 · rustc --emit=dep-info=PATH,link from GN passes through

- **Area:** Rust compatibility
- **Where:** src/args.rs (explicit_emit_output), src/compiler/rustc.rs (rustc_refuse_reasons)
- **Problem:** GN's rustc_wrapper.py passes --emit=dep-info=<out>.d,link -o <out>, so every Rust target of a Chromium build passes through with 'rustc explicit --emit output path — not cacheable' (observed on Chromium 156 for serde, hashbrown, miniz_oxide, url and the rest of its Rust graph).
- **Recommendation:** Cache an explicit dep-info path the way -o is cached: store the dep-info file as an output and restore it to the requested path, keeping other explicit emit kinds refused. Acceptance: a GN-style rustc invocation misses then hits with byte-identical rlib and dep-info at the requested paths.
- **Fix with:** reference-parity
- **Effort:** M

### KACH-22 · P3 · Changed-line mutation testing runs in no gate

- **Area:** Tests
- **Where:** `Makefile:pr`, `Makefile:mutants-diff`, `.github/workflows/ci.yml`
- **Problem:** `make mutants-diff` exists and AGENTS.md relies on it, but no workflow runs it, so surviving mutants in key derivation, publication and restore branches are found only when a developer runs `make pr` with cargo-mutants installed.
- **Recommendation:** Run `make mutants-diff` against the merge base on pull requests that touch Rust, with the pinned cargo-mutants version.
- **Fix with:** none
- **Effort:** S

### KACH-24 · P3 · Make release-tag advice match the dispatch-only release workflow

- **Area:** release
- **Where:** Makefile:279-281, .github/workflows/release.yml:4,38
- **Problem:** make release-tag pushes a tag and says that starts the release pipeline, but release.yml declares only workflow_dispatch and requires the default branch, so following the Makefile publishes nothing and leaves an unused tag.
- **Recommendation:** Replace the release-tag target and its message with the actual procedure: gh workflow run release.yml on the default branch, then verify the run's head SHA and the release-<sha> prerelease it creates; remove the tag push.
- **Fix with:** make -n release-tag no longer pushes a tag, and its text names gh workflow run release.yml; the release workflow is unchanged.
- **Effort:** S

### KACH-25 · P3 · Point the documentation install pages at this fork's releases

- **Area:** docs
- **Where:** docs/index.mdx, docs/getting-started/installation.mdx
- **Problem:** The README now installs joshuarli/kache release-<sha> builds, but docs/index.mdx and docs/getting-started/installation.mdx still lead with cargo install kache --locked and cargo-binstall, which install Kunobi's crates.io distribution rather than this fork, along with Homebrew, APT, AUR, mise, and Nix packages that are also upstream.
- **Recommendation:** Lead those pages with this fork's release archives and source install (both kache and kache-engine), and keep the upstream channels only as attributed alternatives, or drop them.
- **Fix with:** rg -n "cargo install kache --locked|cargo-binstall" docs/ finds no primary install instruction that selects the upstream distribution.
- **Effort:** S

### KACH-37 · P3 · ExtraInputsSnapshot optional fields are re-asserted Some by eight production expects (was DSLP-91)

- **Area:** Types · extra inputs
- **Where:** src/extra_inputs.rs
- **Problem:** ExtraInputsSnapshot::digest and provider.package.crate_name are Option, while 8 production expect calls re-assert they are Some; the declaration-path list is also duplicated.
- **Recommendation:** Split the type into resolved and unresolved forms so the invariant is carried by the type, and keep one declaration-path list.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** M

### KACH-38 · P3 · EventTailer is test-only yet the sole reader of the event-log rotation marker (was DSLP-92)

- **Area:** Dead code · events
- **Where:** src/events.rs:607
- **Problem:** EventTailer exists only for tests but is the only code that reads the rotation marker, so the marker format is specified only by test code inside the production module.
- **Recommendation:** Move EventTailer and its tests to tests/ with the rotation format documented, or remove both if the marker has no consumer.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-39 · P3 · Launcher build and install paths lack a decisive test (was DSLP-98)

- **Area:** Tests · build
- **Where:** build.rs (link_flags), tests/build_script_launcher_test.rs
- **Problem:** link_flags is untested because build scripts are not test targets, and build_script_launcher_test still passes with a 0-byte embedded launcher.
- **Recommendation:** Move link_flags into launcher/compile.rs with unit tests, and assert the installed launcher bytes equal the embedded ones.
- **Fix with:** test-audit
- **Effort:** S

### KACH-40 · P3 · kache-e2e fixture results treat status and phase names as unchecked strings (was DSLP-104)

- **Area:** Types · kache-e2e results
- **Where:** crates/kache-e2e/src/fixture.rs (FixtureResult.status), crates/kache-e2e/src/result.rs (PhaseResult.status, PhaseResult.phase), compared as literals at about 36 sites in runner.rs, fixture_runner.rs, result.rs, tests/strict_gate.rs and fixture.rs
- **Problem:** status holds a closed set (pass, fail, skip) and PhaseResult.phase holds a name Phase::name() already produces, but both are String compared as literals at about 36 sites, so a misspelled status compiles and matches nothing.
- **Recommendation:** Replace both with a #[non_exhaustive] enum carrying the same wire strings, behind a results.json compatibility decision.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### KACH-41 · P3 · The kache-e2e negative-control verdict has no test because it sits beside process::exit (was DSLP-105)

- **Area:** Tests · kache-e2e
- **Where:** crates/kache-e2e/src/fixture_runner.rs (run: the want-pass, passed, verdict and exit-code decision)
- **Problem:** The negative-control verdict (VACUOUS, falsifiable, exit 1) is decided inside fixture_runner::run next to process::exit, so no test reaches it; the assertion surface has no guard.
- **Recommendation:** Extract the decision into a pure function over (want-pass, passed, verdict) and test its truth table, leaving run to map the result to an exit code.
- **Fix with:** test-audit
- **Effort:** S

### KACH-42 · P3 · build_script's second-run settle path has no test (was DSLP-107)

- **Area:** Tests · build script
- **Where:** src/build_script.rs:598-621 (run_cached, InputsChanged branch with reruns_settle)
- **Problem:** reruns_settle and mark_unsettled are unit-tested alone, but the branch that replays after a script rewrote a file it declared is reached by no lib test, because run_cached loads Config, opens the real store and spawns the preserved script.
- **Recommendation:** Add an integration test where a script rewrites a declared file, asserting one rerun and an unsettled: marker on the second failure.
- **Fix with:** test-audit
- **Effort:** M

### KACH-46 · P3 · scheduler pressure ENABLED and set_enabled are test-only and the doc names a removed knob (was DSLP-117)

- **Area:** Dead code · scheduler
- **Where:** src/scheduler/pressure.rs:36 (ENABLED, set_enabled), src/main.rs:1233 (InvocationPressureScope::enter(true)), src/scheduler.rs
- **Problem:** pressure::ENABLED and pressure::set_enabled are only written by tests; production enters pressure through InvocationPressureScope::enter(true). The doc comment still names [cache] scheduler_memory_pressure, a knob removed in c2eca807. Cleaning it up spans main.rs and scheduler.rs, outside the component that found it.
- **Recommendation:** Remove ENABLED and set_enabled in favour of the invocation scope the production path already uses, and drop the stale knob from the doc comment.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-51 · P3 · Process promptness tests use 2 s bounds sized for an idle host (was DSLP-93)

- **Area:** Tests · invocation
- **Where:** src/invocation/process.rs:489, :565, :639, :815, :845
- **Problem:** Process promptness tests use tight elapsed-time bounds that can fail on a loaded host even when child cleanup succeeds.
- **Recommendation:** Assert the behaviour (child reaped, signal observed) with a generous hang guard instead of a tight elapsed-time bound.
- **Fix with:** root-cause
- **Effort:** S

### KACH-52 · P3 · 28 test early returns skip silently when a prerequisite is missing (was DSLP-96)

- **Area:** Tests
- **Where:** tests/ (e.g. missing cc, running as root)
- **Problem:** Tests print 'skipping' and return, so a CI host missing cc or running as root passes them without exercising anything.
- **Recommendation:** Route them through a common::skip_unless helper that panics when KACHE_TEST_DENY_SKIPS is set, and set it in CI.
- **Fix with:** test-audit
- **Effort:** S

### KACH-53 · P3 · Lib tests fail under host load through wall-clock waits rather than readiness signals (was DSLP-97)

- **Area:** Tests
- **Where:** src/invocation/, src/probe/, src/service.rs, src/daemon_execution.rs, src/daemon_lifecycle.rs, kache-store wait_for_committed_returns_false_without_an_owner
- **Problem:** Under heavy load, process and daemon tests have failed with spawn timeouts and EAGAIN while passing alone. Affected tests include manager_command_deadline_bounds_wait_and_caps_diagnostics, execution_handoff_streams_two_compilers_without_serializing_or_mixing_clients, abandoned_startup_reaps_the_child_without_terminating_it, and failed_and_stalled_compilers_return_details_and_finish.
- **Recommendation:** Replace the waits with readiness signals or generous condition-wait bounds; prove each with a deterministic delay.
- **Fix with:** root-cause
- **Effort:** M

### KACH-58 · P3 · CLI reference and mutants config drift from the binary (was DSLP-99)

- **Area:** Docs · tests
- **Where:** docs/commands/reference.mdx, .cargo/mutants.toml lines 1-250
- **Problem:** No test compares kache --help with the command reference (it documented a removed completions command), and the mutants.toml header still describes removed Windows and TUI exclusions.
- **Recommendation:** Add a test that diffs --help output against the reference, and rewrite the mutants.toml header.
- **Fix with:** single-source-of-truth
- **Effort:** S

### KACH-59 · P3 · CI omits client graph, resource guard and documentation checks (was DSLP-100)

- **Area:** Needs owner · CI
- **Where:** .github/workflows/, Makefile (client-crates-check, test-resource-guard-check), scripts/check-docs.sh
- **Problem:** No workflow runs the client-crates graph, resource-guard self-check, or documentation checks, so regressions detected by those local gates can reach main.
- **Recommendation:** Decide whether to run these checks in the CI workflow.
- **Fix with:** none
- **Effort:** S

### KACH-60 · P3 · kache-fs builds a Windows branch no repository build can reach (was DSLP-106)

- **Area:** Needs owner · kache-fs
- **Where:** crates/kache-fs/src/home.rs (#[cfg(windows)] mod imp, 130 lines, five unsafe blocks)
- **Problem:** home.rs carries a 130-line #[cfg(windows)] module with five unsafe blocks, but the crate takes std::os::unix unconditionally, CI has no Windows job, and no other cfg(windows) exists in the crate, so no build the repository runs can reach it.
- **Recommendation:** Owner decision on platform support: delete the Windows branch, or restore Windows support across the crate and add a Windows job.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-61 · P3 · Nine scripts/ entry points have no caller in the repository (was DSLP-108)

- **Area:** Needs owner · scripts
- **Where:** scripts/ci-changes.py, scripts/test-ci-changes.py, scripts/ci/resolve-version.sh, scripts/require-ci-green.sh, scripts/ci-monitor.sh, scripts/workspace-members.sh, scripts/check-coverage-scope.sh, scripts/volume-shards-check.py, scripts/test-unavailable-proc-labels.sh
- **Problem:** No workflow, Makefile target, doc or other script names these nine files, so nothing runs them. ci-changes.py and ci/resolve-version.sh decide CI groups (nix, helm, perf-gate) the workflows no longer have and only test-ci-changes.py references them; require-ci-green.sh claims to gate irreversible publishes on CI but nothing calls it; ci-monitor.sh defaults to Zondax/kunobi-frontend, not this repository; workspace-members.sh is called by no recipe and check-coverage-scope.sh only by it; volume-shards-check.py has no caller; test-unavailable-proc-labels.sh selects a live Rust test but has no target. Most implement release or CI policy, so removal is an owner decision.
- **Recommendation:** Owner decision per script: retire the ones whose policy is gone, and wire the ones still meant to run (volume-shards-check.py, test-unavailable-proc-labels.sh) into a Makefile target or workflow.
- **Fix with:** dead-code-archaeology
- **Effort:** M

### KACH-62 · P3 · init printing 'Login service: configured (<path>)' has no test (was DSLP-113)

- **Area:** Tests · CLI init
- **Where:** src/cli.rs (init login-service line, src/cli.rs:6212); tests/cli_commands_test.rs:2299 onward (Linux-only fake-systemd runs assert the install, skip and update lines only)
- **Problem:** The installed-and-current service branch that prints 'Login service: configured (<path>)' is unasserted: the existing service tests are Linux-only fake-systemd runs that check the install, skip and update lines, so this line is provable on neither macOS nor the existing Linux fake.
- **Recommendation:** Extend the fake-systemd service fixture to an already-configured, current service and assert the configured line.
- **Fix with:** test-audit
- **Effort:** S

### KACH-63 · P3 · Linux CRT driver probes carry 144 uncovered lines that macOS cannot exercise (was DSLP-121)

- **Area:** Tests · cache key
- **Where:** src/cache_key.rs (record_crt_placement_memo, crt_placement_memo_for, probe_files_with_hash, print_file_name_for, probe_linux_crt_objects_memoized_for, place_linux_crt_objects, driver_search_dirs_for, driver_library_search_dirs_for)
- **Problem:** 144 production lines of the Linux host CRT/libc identity probes are unreachable on the macOS host that measured coverage, and the record could not confirm that a Linux CI cell covers them rather than assuming it does.
- **Recommendation:** Run the instrumented suite in a Linux cell and record the probe coverage; add a focused test for any probe still uncovered there.
- **Fix with:** test-audit
- **Effort:** S

### KACH-64 · P3 · Four Python test suites under scripts/ run in no CI job or make target (was DSLP-122)

- **Area:** Needs owner · CI
- **Where:** scripts/test-ci-changes.py, scripts/test-crates-io.py, scripts/test-mutation-diff.py, scripts/test-version-consistency.py
- **Problem:** No .github/workflows job and no Makefile target runs these four suites, so a regression in ci-changes.py, crates-io.py publish ordering, mutation-diff.py or check-version-consistency.sh reaches main untested. Their local cost is under 3 s each except test-mutation-diff.py at 34 to 78 s.
- **Recommendation:** Owner decision on CI: add a workflow job (or a make target wired into one) that runs the four suites.
- **Fix with:** none
- **Effort:** S

### KACH-66 · P3 · Verify ownership of a pre-existing fallback cache directory

- **Area:** Cache directory security
- **Where:** crates/kache-config/src/config.rs:1349, crates/kache-fs/src/home.rs:32
- **Problem:** If neither an environment nor passwd home is available, the default cache falls back to /tmp/kache. The behavior when that directory already belongs to a different user was not verified; a shared or attacker-controlled cache could affect stored artifacts.
- **Recommendation:** Exercise the fallback with a pre-existing directory owned by another account and require safe ownership and symlink handling or a private per-user fallback.
- **Fix with:** security
- **Effort:** M

### KACH-75 · P3 · Consolidate unsafe process-liveness probes in integration tests

- **Area:** Tests and process handling
- **Where:** tests/unknown_compiler_probe_test.rs:20, tests/engine_bootstrap_test.rs:416
- **Problem:** Two test files independently call libc::kill with signal zero to check process liveness, and the unsafe blocks lack a safety explanation.
- **Recommendation:** Share a process-liveness helper with a documented safety invariant and preserve both test cases.
- **Fix with:** lifecycle-concurrency
- **Effort:** S

### KACH-76 · P3 · Remove fallible unwraps from end-to-end harness code

- **Area:** End-to-end harness
- **Where:** crates/kache-e2e/src/cc_bench/mod.rs, crates/kache-e2e/src/cc_oracle/gcov.rs, crates/kache-e2e/src/bench_profile.rs, crates/kache-e2e/src/bench_runner.rs
- **Problem:** Production unwrap and expect calls in benchmark and oracle paths can abort the harness rather than report an error, and block crate-wide checks for these operations.
- **Recommendation:** Check each call's invariant, propagate recoverable failures, and adopt the applicable lint when the paths are covered.
- **Fix with:** error-model
- **Effort:** M

### KACH-35 · P4 · No read-only store mode for shared or CI caches

- **Area:** Store
- **Where:** crates/kache-config/src/config.rs (no read-only setting); store writes in src/wrapper.rs (cc publish and daemon handoff), src/daemon_publish.rs, the rustc store path, preprocess memo and probe-record writes, and access-time updates on hits; inventory row cc-rel-readonly-disabled
- **Problem:** Kache can be disabled (KACHE_DISABLED) but cannot use a store read-only: a CI job or a shared cache that should restore but never write has no supported mode, and the inventory row for read-only behaviour has nothing to exercise.
- **Recommendation:** Add a read-only setting that restores hits, skips publication and leaves the store untouched, and cover it with an oracle case that asserts misses compile and nothing is stored.
- **Fix with:** none
- **Effort:** M

### KACH-36 · P4 · Reject symlinked artifacts when importing downloaded entries (was DSLP-90)

- **Area:** Store imports
- **Where:** crates/kache-store/src/store.rs:3252, crates/kache-store/src/store.rs:3457
- **Problem:** Downloaded entry validation follows symlinks while checking and hashing artifacts, then can move the link into the blob store and change permissions on its target. The path currently has no production caller but is exposed by the published store library.
- **Recommendation:** Decide whether to remove the unused import path or harden it against symlinks and replacement races before using it with remote data; cover the outside-file permission and mutable-blob cases.
- **Fix with:** os-boundaries
- **Effort:** S

### KACH-43 · P4 · bench/short.py samples.json timestamps have no in-repo reader (was DSLP-109)

- **Area:** Dead code · bench
- **Where:** scripts/bench/short.py and scripts/bench-contention.py:1198 (SampleRecord started_monotonic_s, ended_monotonic_s)
- **Problem:** started_monotonic_s and ended_monotonic_s are written into samples.json but no reader in the repository consumes them, so whether they are an external contract or dead weight is unknown.
- **Recommendation:** Confirm whether the persisted samples are read outside the repository; if not, drop the two fields from SampleRecord.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-44 · P4 · Published kache-store methods and link helpers have no production caller (was DSLP-110)

- **Area:** Needs owner · kache-store
- **Where:** crates/kache-store/src/store.rs (blobs_at_least, keys_for_crates, wait_for_committed_with_timeout, reconcile_blob_index, import_verified_restored_entries), crates/kache-store/src/link.rs:1387 (rebase_depinfo_content), crates/kache-store/src/link.rs:104 (set_shared_hardlink_restores),
- **Problem:** Five Store methods and two link.rs functions are called only from the crate's own tests; no production code, doc or other crate references them. kache-store is published, so none was removed. import_verified_restored_entries is the remote-storage import AGENTS.md defers, and [cache] shared_hardlink_restores is applied through the invocation scope rather than by calling set_shared_hardlink_restores.
- **Recommendation:** Confirm whether an out-of-tree consumer uses any of the seven; if not, delete them with their tests, keeping import_verified_restored_entries until remote storage returns.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-45 · P4 · cache_key test module serializes on one lock and 27 tests can pass without asserting (was DSLP-114)

- **Area:** Tests · cache key
- **Where:** src/cache_key/tests.rs (25 tests return when rustc is missing, 2 when the cache directory is unwritable; 126 tests share key_test_lock)
- **Problem:** 126 tests take the process-wide key_test_lock to mutate environment variables, making the module effectively serial (47 to 56 s of test time). Separately, 25 tests start with 'if !rustc_available() { return; }' and 2 return after 'skipping: <dir> is not writable', so they pass without asserting in an environment that lacks the prerequisite.
- **Recommendation:** Inject the environment through KeyEnv and InvocationEnv (already used by the newer tests) so the module can run in parallel, and turn the rustc-missing returns into assertions since cargo test guarantees rustc on PATH.
- **Fix with:** test-audit
- **Effort:** M

### KACH-47 · P4 · The input-race suppression truth table restates the production expression (was DSLP-118)

- **Area:** Tests · wrapper
- **Where:** src/wrapper.rs (should_skip_cache_store_for_input_race), src/wrapper/tests.rs (input_race_store_suppression_truth_table)
- **Problem:** The truth table copies the production expression instead of listing expected results for the sixteen combinations, so it can catch operator faults but not a wrong specification.
- **Recommendation:** Replace the expression-shaped assertion with explicit expected results for all sixteen combinations.
- **Fix with:** test-audit
- **Effort:** S

### KACH-48 · P4 · default_cache_executables is pinned only by scripts/check-docs.sh text, and its unit test compares against cfg! (was DSLP-123)

- **Area:** Tests · kache-config
- **Where:** crates/kache-config/src/config/tests.rs:95 (default_cache_executables compares against cfg!), crates/kache-config/src/config.rs:1315, scripts/check-docs.sh:88 (pins the literal body)
- **Problem:** The unit test asserts default_cache_executables() equals a cfg! expression that restates the same condition, so it cannot catch a flipped body. The only real pin is check-docs.sh's literal-body text match, which nothing in CI runs.
- **Recommendation:** Assert the literal expected value in the unit test instead of a cfg! expression, so the body is pinned where the test actually runs.
- **Fix with:** test-audit
- **Effort:** S

### KACH-49 · P4 · Surface failed toolchain and Cargo config copies in fixture worktrees (was DSLP-103)

- **Area:** End-to-end fixtures
- **Where:** crates/kache-e2e/src/source.rs:41
- **Problem:** Fixture relocation ignores errors copying the toolchain pin and the nearest Cargo configuration, so a failed copy can run a build without the intended toolchain or registry configuration.
- **Recommendation:** Propagate copy and directory creation errors when a pin or configuration exists, and test the failed-copy behavior.
- **Fix with:** error-model
- **Effort:** S

### KACH-54 · P4 · Support-crate public items have no reference outside the repository (was DSLP-112)

- **Area:** Needs owner · support crates
- **Where:** crates/kache-protocol/src/fallback.rs:24 (Attempt::terminal_code), crates/kache-protocol/src/prediction.rs:22 (SharedPrediction, PlainRow), crates/kache-core/src/time.rs:506 (Parts::month_day_clock), crates/kache-core/src/time.rs:245 (UtcTime::format_sqlite), crates/kache-protocol/src/execution/process.rs:32 (Stdio::QUIET)
- **Problem:** Each of these five public items has no reference anywhere in the tree beyond its definition (format_sqlite only from a cli test, Stdio::QUIET only from a kache-ipc test). Every crate here is published, and SharedPrediction sits in a wire type module where removal could break a remote reader, so none was deleted.
- **Recommendation:** Confirm whether an out-of-tree consumer uses any of them; if not, delete the unused ones and keep SharedPrediction only if a remote reader needs it.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-55 · P4 · kache-protocol's Request::PublishCc has no unit test (was DSLP-116)

- **Area:** Tests · protocol
- **Where:** crates/kache-protocol (Request::PublishCc, the one Request variant not constructed in only_build_requests_keep_the_machine_busy_and_only_some_declare_a_release)
- **Problem:** The per-variant busy-ness and client-release table test covers every Request variant except PublishCc, which needs a BuildEvent to construct, so it stays covered only through the daemon.
- **Recommendation:** Construct a BuildEvent fixture and add PublishCc to the table test.
- **Fix with:** test-audit
- **Effort:** S

### KACH-56 · P4 · kache-ipc defines invalid and unknown identically in two modules (was DSLP-120)

- **Area:** Dead code · kache-ipc
- **Where:** crates/kache-ipc/src/execution/unix_client.rs and crates/kache-ipc/src/execution/workspace.rs (invalid, unknown)
- **Problem:** The two helper functions are defined identically in both modules; the duplication was noted for the parent and not changed.
- **Recommendation:** Hoist the two helpers into one shared module and import them in both files.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-65 · P4 · The elapsed-millis cast idiom repeats 42 times across the wrapper and neighbours (was DSLP-119)

- **Area:** Types · duration casts
- **Where:** src/wrapper.rs (42 elapsed().as_millis() as u64 and Duration conversions), src/cli.rs, src/daemon.rs, src/build_script.rs, src/wrapper/hit.rs:28 and rustc_hit.rs:49
- **Problem:** The wrapper and neighboring modules repeatedly narrow elapsed milliseconds to u64 for telemetry. A shared conversion could express the intended range policy consistently.
- **Recommendation:** Add one Duration-to-milliseconds helper in a shared module and route the 42 sites through it, so the lint can be adopted once.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### KACH-67 · P4 · kache-config's file-write feature, EnvOverrides and FileConfig writers have no consumer in the tree (was DSLP-102)

- **Area:** Needs owner · kache-config
- **Where:** crates/kache-config/src/config.rs (update_file_config_at, save_file_config_to, EnvOverrides, load_raw_file_config_from, user_bypass_reason, cache_dir_for_path, user_config_path, host_config_path_for, config_file_fingerprint, volume_store_for, DEFAULT_* constants)
- **Problem:** kache-config is publishable and no workspace caller uses the file-write surface (Config::update_file_config_at, Config::save_file_config_to) or EnvOverrides; several public items have no reference anywhere in the tree (Config::cache_dir_for_path, DEFAULT_DAEMON_IDLE_TIMEOUT_SECS, DEFAULT_GC_MAX_AGE_HOURS, DEFAULT_MIN_STORE_COMPILE_MS), and the unused set looks like the remains of a config editor. Deleting them is a decision about supported API, not a dead-code cut.
- **Recommendation:** Decide whether file-write, EnvOverrides and the FileConfig writers are supported API; if they are not, delete them with their tests in one commit.
- **Fix with:** dead-code-archaeology
- **Effort:** M

### KACH-68 · P4 · store.rs sign and truncation casts are mostly behind .max(0) but not fully audited (was DSLP-111)

- **Area:** Types · kache-store
- **Where:** crates/kache-store/src/store.rs (cast_sign_loss 20 hits, cast_possible_truncation 8 hits; guarded at 5266, 5752, 5842, 5875-5878, 6336, 6338)
- **Problem:** Most of the 28 casts are i64-to-u64 behind .max(0), but the remaining unguarded ones were filed rather than changed because no witness shows a defect.
- **Recommendation:** Audit each unguarded cast in store.rs against its invariant and either add the missing guard or record why it cannot underflow.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### KACH-69 · P4 · Redundant clones in the cache-key and build-script hot paths (was DSLP-115)

- **Area:** Dead code · clones
- **Where:** src/cache_key.rs (path.clone() in workspace_tree_digest, resolved.clone() in the cc input lookup), src/build_script.rs:536 (options.clone() before Store::open_with_options)
- **Problem:** Three clones are the last use of their value, so clippy::redundant_clone fires and each is a needless allocation on a key or build-script path. They were left because dropping a clone is not a type, defect or dead-code change.
- **Recommendation:** Drop the three clones and keep clippy::redundant_clone at warn over the crate.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-70 · P4 · Narrow leftover visibility and remove redundant helpers (was DSLP-94)

- **Area:** Types · dead code
- **Where:** src/extra_inputs.rs, src/daemon_execution.rs, src/daemon.rs, src/incremental_policy.rs, src/main.rs, src/config.rs
- **Problem:** Several src modules expose items beyond their callers, six test-only helpers live outside the tests module, a pure truth-table test is Linux-gated, three clones appear redundant, and config_path_lock is exported at two paths.
- **Recommendation:** Verify callers, narrow visibility, move test-only helpers, run the pure test on all supported hosts, remove redundant clones, and choose one config_path_lock export.
- **Fix with:** none
- **Effort:** S

### KACH-71 · P4 · Resolve config, fixture and filesystem typing and test gaps (was DSLP-103)

- **Area:** Types · dead code
- **Where:** crates/kache-config/src/config.rs, crates/kache-config/src/invocation.rs, crates/kache-e2e/src/lib.rs, crates/kache-e2e/src/result.rs, crates/kache-e2e/src/cc_oracle/, crates/kache-e2e/src/bin/, crates/kache-fs/src/
- **Problem:** Config helpers accept Result<FileConfig> but discard errors reported separately, two doc comments appear misplaced, and an absolute-cwd check is redundant. The end-to-end harness has identity and unused helpers, wildcard oracle matches, string errors, and untested wrapper binaries. Filesystem code retains an identity path helper, an unexported returned type, unnamed extent flags, a type named Clone, unsafe operations without safety explanations, untested passwd and XDG fallbacks, and six public methods without known consumers: DirSizing::is_partial, DirSizing::shared_ratio, FileSizing::frees_nothing, VolumeUsage::used_ratio, CloneSketch::shares_with and InodeLedger::distinct_inodes. A directory sorting mutation is observable on ext4 but not APFS.
- **Recommendation:** Check callers and invariants before narrowing config types or deleting identity helpers; make oracle matches exhaustive, review error types and binary coverage, clarify filesystem names and safety invariants, test home resolution, and determine whether the public methods have external consumers.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** L

### KACH-72 · P4 · Narrow visibility and cover compiler-probe skips (was DSLP-124)

- **Area:** Types · dead code
- **Where:** src/cache_key.rs and src/compiler/* (unreachable_pub: 25 in cache_key.rs, 81 in the compiler files, 24 in cli.rs), src/compiler/cc/tests.rs:830 (recognizes_unknown_wrapper_via_probe returns when no C compiler is installed), src/cli.rs:603 (machine.clone() at its last use), src/cli.rs:684 (render_stats is a #[cfg(test)] helper with three consumers)
- **Problem:** Several cache-key, compiler and CLI items are public within private modules. recognizes_unknown_wrapper_via_probe silently returns when no C compiler is installed, and a CLI value appears to be cloned at its last use.
- **Recommendation:** Adopt unreachable_pub once across src/ and narrow the flagged items to pub(crate); route the no-compiler skip through the common::skip_unless helper or assert the expected refusal; drop the last-use clone in cli.rs.
- **Fix with:** test-audit
- **Effort:** S

### KACH-73 · P4 · Resolve unused cross configuration and empty default feature (was DSLP-101)

- **Area:** Build configuration
- **Where:** Cross.toml, Cargo.toml [features]
- **Problem:** The cross configuration has no repository caller and the root package declares an empty default feature. External consumers of the configuration and published feature contract are unknown.
- **Recommendation:** Confirm external consumers before removing either unused configuration.
- **Fix with:** dead-code-archaeology
- **Effort:** S

### KACH-74 · P4 · Fix README links to missing issue templates (was DSLP-101)

- **Area:** Documentation
- **Where:** README.md:132, .github/ISSUE_TEMPLATE
- **Problem:** The README links to bug and feature request templates that are absent from the repository.
- **Recommendation:** Point the links to the supported issue creation flow.
- **Fix with:** technical-writing
- **Effort:** S

