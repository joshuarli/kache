Prefix: KACH
Next: 20

### KACH-14 · P1 · Failed publication still conflates empty references and metadata generations

- **Area:** Data
- **Where:** `crates/kache-store/src/store.rs:1602`, `crates/kache-store/src/store.rs:2468`, `crates/kache-store/src/store.rs:2834`, `crates/kache-store/src/store.rs:2951`, `crates/kache-store/src/store.rs:3027`, `crates/kache-store/src/store.rs:5982`, `crates/kache-store/src/store/tests.rs:4124`, `src/wrapper.rs:5787`
- **Problem:** Reachable defects, reproduced against `79d0449552294a363fe66974771f30a59832bd65` through public store APIs with an injected SQLite registration failure. The original nonempty A/B reproduction is fixed: independent read-only and writable lookups miss and B survives. Two valid transitions still violate the publication contract. (1) Publish committed B with a blob and committed A with zero output files; the current writer accepts this (`store.rs:2867-2951`), and A has a non-NULL content hash but legitimately no `entry_blobs` rows. Remove A's metadata, acquire its ordinary repair claim, and replace A with B's artifact. Abort the entry INSERT after metadata replacement (`store.rs:3027-3033`): SQL rolls back to the empty A generation, but the new metadata remains. Both direct `remove_entry(A)` and the writable lookup's mismatch eviction treat the empty committed reference set as legacy (`store.rs:5982-6001`) and subtract the uncommitted file's B reference. In separate runs, B remained committed while its live blob was deleted. No forged or manually deleted reference rows were needed. (2) Publish A with an artifact and old stdout/stderr; repair missing metadata with identical artifact names, hashes, sizes and modes but changed stdout/stderr, then inject the same registration failure. `meta_matches_committed_generation` compares the file-only dedup hash (`store.rs:1602`, `store.rs:2490`), so both independent readers return the new uncommitted diagnostic streams while SQLite retains the old generation. Those streams are observable through diagnostic replay (`wrapper.rs:5787`). The new checked-in regression also performs writable `get(A)` before its read-only and explicit-remove assertions (`store/tests.rs:4124-4138`); lookup has already evicted A, so those later assertions do not exercise the split state independently.
- **Recommendation:** Distinguish a known modern empty reference set from unknown legacy accounting, and never decrement references inferred from metadata that does not belong to the committed generation; uncertain legacy accounting must fail closed or reconcile safely. Either represent zero-output publications explicitly or reject them before publication if the API contract excludes them. Bind lookup to an authoritative generation identifier or digest covering the complete replayed metadata, including stdout/stderr, while retaining the file-only hash for its dedup purpose. Add actual registration-failure regressions for empty/nonempty outputs and unchanged artifacts with changed diagnostics; use fresh scenarios for read-only lookup, writable eviction and direct removal. Verify old-complete/new-committed/miss behavior, preservation of unrelated committed blobs, and reopen/recovery. Use process-death fault points as well as ordinary error returns; exception cleanup alone cannot establish crash recovery.
- **Fix with:** durable-state
- **Effort:** M

### KACH-5 · P2 · Stale explain_miss config commands and tests reference a removed knob

- **Area:** Config
- **Where:** `tests/cli_commands_test.rs:252`, `tests/config_commands_test.rs:25`
- **Problem:** `bare_config_lists_values_without_a_terminal` and three `config_commands_test` cases fail with `unknown configuration key cache.explain_miss`. The knob was removed but its tests and `config set` coverage were not.
- **Recommendation:** Delete or repoint the stale assertions to a surviving key.
- **Fix with:** none
- **Effort:** S

### KACH-6 · P2 · ENV_FILE_KEYS omits KACHE_HEARTBEAT_SECS

- **Area:** Config
- **Where:** `crates/kache-config/src/config.rs:451`
- **Problem:** `every_env_var_config_reads_maps_to_a_config_key` fails: `KACHE_HEARTBEAT_SECS` is read but has no `ENV_FILE_KEYS` entry, so `kache doctor` cannot name its override.
- **Recommendation:** Add the missing entry or mark the variable operational.
- **Fix with:** none
- **Effort:** S

### KACH-7 · P2 · Host status misattributes cache.daemon_idle_timeout_secs

- **Area:** Config
- **Where:** `crates/kache-config/src/config/tests.rs:999`
- **Problem:** `host_status_names_the_variable_config_reads` expects `Env("KACHE_DAEMON_IDLE_TIMEOUT")` but reports `Host` for `cache.daemon_idle_timeout_secs`.
- **Recommendation:** Fix the status attribution or the test's expectation.
- **Fix with:** none
- **Effort:** S

### KACH-8 · P2 · Cross-device ingest advisory test fails

- **Area:** Tests
- **Where:** `crates/kache-store/src/store/tests.rs`, `injected_cross_device_ingest_advises_but_other_errors_do_not`
- **Problem:** Fails on main; needs triage whether the advisory regressed or the fixture is environment-sensitive.
- **Recommendation:** Reproduce, then fix the advisory or the fixture.
- **Fix with:** none
- **Effort:** S

### KACH-9 · P2 · cc_admission test fails on main

- **Area:** Tests
- **Where:** `tests/cc_passthrough_args_test.rs`, `cc_admission_reports_a_skip_reason_and_recompiles_until_admitted`
- **Problem:** Fails on main; needs triage whether admission reporting regressed or the test is flaky.
- **Recommendation:** Reproduce, then fix the reporting or the test.
- **Fix with:** none
- **Effort:** S

### KACH-10 · P2 · Warm-target extra-inputs test fails on main

- **Area:** Tests
- **Where:** `tests/extra_inputs_warm_target_test.rs`, `declared_extra_input_invalidates_a_real_warm_cargo_target`
- **Problem:** Fails on main; needs triage.
- **Recommendation:** Reproduce, then fix the invalidation or the test.
- **Fix with:** none
- **Effort:** S

### KACH-11 · P2 · Cross-store proc-macro bytes test fails on main

- **Area:** Tests
- **Where:** `tests/out_dir_alias_test.rs`, `proc_macro_bytes_and_dependent_keys_match_across_stores`
- **Problem:** Fails on main (alias lands under the store instead of the shared root); needs triage.
- **Recommendation:** Reproduce, then fix the alias root handling or the test.
- **Fix with:** none
- **Effort:** S

### KACH-12 · P2 · Automatic-GC integration test proves its size bound through manual GC

- **Area:** Tests
- **Where:** `tests/integration_test.rs:3365`, `tests/integration_test.rs:3377`, `tests/integration_test.rs:3392`, `crates/kache-config/src/config.rs:2273`, `src/daemon/tests.rs:1998`, `src/daemon/tests.rs:2403`
- **Problem:** Source-verified assurance gap at `79d0449`; the revised test passes locally. `test_auto_gc_bounds_store_size` checks only a log message that an automatic sweep was spawned or handed off (`integration_test.rs:3365-3369`), then explicitly runs synchronous `kache gc` (`:3377-3389`) before asserting the size bound (`:3392-3416`). Manual collection can satisfy that assertion even when the Cargo-triggered automatic sweep never completes. Target-held bytes are legitimately excluded from pressure now that `gc_evict_shared` is fixed to false, so removing the obsolete knob is correct. Existing daemon tests still cover held-byte policy and socket-hint/background eviction; the missing evidence is the full Cargo-build-to-automatic-completion composition, not all GC behavior.
- **Recommendation:** Split the current test into a truthfully named manual-GC-after-releasing-targets case and an automatic-completion case. Give the latter enough aged, reclaimable data to exceed the effective budget, trigger it through a real Cargo build, and observe eventual reclamation without invoking manual GC. Keep the held-byte policy tests. A disabled automatic worker must make the automatic case fail even if its trigger log is emitted.
- **Fix with:** none
- **Effort:** S

### KACH-16 · P2 · Failed entries monopolize every bounded durability sweep

- **Area:** Data
- **Where:** `crates/kache-store/src/store.rs:4393`, `crates/kache-store/src/store.rs:4400`, `crates/kache-store/src/store.rs:4414`, `src/daemon.rs:715`, `src/daemon.rs:1643`, `src/cli.rs:2659`
- **Problem:** Reachable defect, reproduced against `221cf7d` with 65 pending entries and three sweeps. Pending entries implicitly include both retryable failures and never-attempted work, but each sweep selects only the oldest `limit` rows (`store.rs:4307-4308`). Unreadable, changed or corrupt entries return false and retain their selection position (`store.rs:4349-4356`, `store.rs:4376-4379`). If the oldest 64 entries reference damaged bytes, every daemon sweep selects those same entries (`daemon.rs:724`; batch size `cli.rs:2659`), and healthy entry 65 never gets flushed. A single shared damaged blob suffices to block all 64. The reproducer kept all 65 pending after three bounded sweeps; an explicit unbounded GC-style flush reached and promoted the healthy entry. This breaks bounded background progress and leaves later hits paying mandatory hashing until failed rows are removed or another path flushes them. The existing corruption-continuation test (`store/tests.rs:2775-2824`) puts the good entry inside the same two-item batch, so it misses starvation across the batch boundary. This is distinct from KACH-12's automatic-GC integration coverage. Replayed against `79d0449`: all 65 entries still remained pending after three bounded sweeps, and the unbounded flush reached the healthy entry. KACH-13's repair now correctly queues reconstructed entries as pending, so queue fairness also matters after recovery.
- **Recommendation:** Make failed work yield its place while preserving bounded work per sweep: advance a persistent or daemon-owned cursor, apply fair retry scheduling, or retire definitively corrupt entries through the existing safe removal path. Keep transient failures retryable. Add more-than-one-batch tests with corrupt, unreadable and transiently failing early entries; prove later healthy entries eventually flush and a recovered failed entry is revisited. Explicit CLI GC (`src/cli.rs:2676`) is a recovery path, not a substitute for fair periodic progress.
- **Fix with:** durable-state
- **Effort:** M

### KACH-17 · P2 · Release integration tests ignore the distributable binary selector

- **Area:** Tests
- **Where:** `Makefile:2`, `Makefile:31`, `tests/common/mod.rs:204`, `tests/common/mod.rs:256`, `tests/common/mod.rs:288`, `.github/workflows/release.yml:98`, `.github/workflows/release.yml:116`, `tests/cc_family_probe_test.rs:29`
- **Problem:** Source-verified testing defect at `221cf7d`, independent of the currently failing CI tests. `make test-ci` checks that the dist binary exists and exports `KACHE_TEST_BINARY` (`Makefile:31-34`), but no test/helper reads that variable. The common helper either selects Cargo's own binary via `KACHE_TEST_USE_CARGO_BIN_EXE` or runs an unprofiled `cargo build` and returns `debug/kache` (`tests/common/mod.rs:256-295`); other integration tests use `CARGO_BIN_EXE_kache` directly. The release workflow invokes this target, so its CLI integration results do not validate the exact `dist/kache` and neighboring `dist/kache-engine` pair being shipped. Running the test harness with `--release` does not fix this binary-selection mismatch.
- **Recommendation:** Give process tests one authoritative explicit binary selector and use it throughout, including daemon startup/teardown and the engine companion. Release verification must exercise the actual distributable pair, with an assertion identifying the selected paths/build identity. A regression pointing the override at a missing or deliberately failing stub must fail instead of silently rebuilding another binary. Keep normal developer bootstrap behavior when no explicit override is supplied.
- **Fix with:** none
- **Effort:** M

### KACH-18 · P2 · Semantic verification suites exist without an active enforcement path

- **Area:** Tests
- **Where:** `.github/workflows/ci.yml:41`, `.github/workflows/ci.yml:79`, `.github/workflows/lint.yml:35`, `Makefile:79`, `Makefile:107`, `Makefile:114`, `AGENTS.md:1`, `crates/kache-proofs/src/lib.rs:16`, `scripts/check-coverage-scope.sh:1`, `crates/kache-e2e/src/fixture_runner.rs:117`, `tests/integration_test.rs:3104`, `tests/integration_test.rs:3122`
- **Problem:** Source-verified assurance gap at `221cf7d`; this is about which checks execute, not the existing red CI jobs. Current workflows run workspace tests and clippy, but never invoke the 54 fixture scenarios, their disabled-cache negative control, changed-line mutation testing, Kani, or the workspace coverage-scope check. `AGENTS.md:1-15` and Makefile comments still describe a mutation gate that is absent. `make check` also omits the fixture suite, and `make e2e` does not pass `--deny-missing-tools`, so supported fixtures missing compilers/tools are skipped successfully. The dormant proof harness has already drifted: it imports `checked_nonempty_contiguous_region`, which no longer exists in the included `src/checked_regions.rs`; ordinary `cargo test` sees zero tests in the `cfg(kani)` crate. The static suite inventory and old claims therefore do not establish that the strongest semantic checks run on a release candidate. No current line-coverage percentage or mutation score was measured in this audit. These gates remain unchanged at `79d0449`. The changed legacy-depinfo integration fixture also now exercises an earlier rejection: it renames an artifact only in `meta.json` (`tests/integration_test.rs:3104-3105`), so the new generation hash check rejects it before the wrapper's missing-depinfo-role check. Its new `now cached` assertion (`:3122`) and passing recompile/warm-hit sequence do not prove the named artifact-role rejection or explanation.
- **Recommendation:** Define and wire a lean production acceptance gate: the existing workspace tests, representative real-compiler cold/warm/relocation/coalescing scenarios on supported platforms with required tools enforced, and their negative controls; run the full fixture matrix before release. Add targeted mutation coverage for key derivation, publication/recovery, restore and scheduling rather than treating an unchecked exclusion file as a guarantee. Repair or retire the stale Kani harness for the removed helper, and run any retained proofs explicitly. If reporting coverage, invoke the existing workspace-scope validator. Repair the legacy-depinfo fixture so its committed metadata/index agree and the intended role check is reached, or split/rename it while keeping a separate role-rejection integration case. Treat `KACHE_VERIFY` as a report producer: qualification must reject unacceptable divergence/recompile-failure reports and require a nonzero verified count, since the wrapper itself deliberately continues. Update AGENTS/scenario coverage documentation to match the actual gates and archive executed/skipped counts and binary identity. Do not expand back into removed remote or Windows features merely because old documents mention them.
- **Fix with:** none
- **Effort:** M

### KACH-19 · P3 · Removed cache settings remain in operator instructions and fixtures

- **Area:** Docs
- **Where:** `docs/getting-started/configuration.mdx:119`, `docs/getting-started/configuration.mdx:388`, `src/out_dir_alias.rs:10`, `src/out_dir_alias.rs:980`, `tests/event_log_contention_test.rs:97`, `crates/kache-config/src/config.rs:67`, `crates/kache-config/src/config.rs:2273`
- **Problem:** Source-verified contract drift at `79d0449`. Configuration docs still tell operators to set `gc_evict_shared`/`KACHE_GC_EVICT_SHARED` and `out_dir_alias_root`/`KACHE_OUT_DIR_ALIAS_ROOT`. Production conversion hardcodes shared eviction to false and OUT_DIR aliasing always passes `None` for the configurable root. The alias module header still advertises the removed override. The contention fixture writes `event_log_max_size = "1B"`, although production uses the fixed 64 MiB threshold. Removing tests for those retired settings is appropriate, but leaving the documented controls and ignored fixture values implies behaviors that no longer exist.
- **Recommendation:** Remove or rewrite the retired-setting instructions and ignored fixture values to describe the supported fixed policies. Delete the unreachable configured-root branch and its obsolete tests if no supported caller remains. If retaining a process-level rotation test, explicitly arrange an event log beyond the real threshold; preserve the existing busy-ledger retry unit coverage. Do not restore removed configuration solely to satisfy historical docs or tests.
- **Fix with:** none
- **Effort:** S
