Prefix: KACH
Next: 19

### KACH-13 · P1 · Index reconstruction certifies unverified blobs as durable

- **Area:** Data
- **Where:** `crates/kache-store/src/store.rs:1663`, `crates/kache-store/src/store.rs:2188`, `crates/kache-store/src/store.rs:2488`, `crates/kache-store/src/store.rs:3688`, `crates/kache-store/src/store.rs:3712`
- **Problem:** Reachable defect, reproduced against `221cf7dfd43620bb110315b084c8ec5717f7f0dd` with the public store API on Linux. The publication lifecycle spans `entries.committed`, `entries.durable`, `meta.json` and CAS blobs: a pending publication must pass content verification and fsync before becoming durable. Store an entry without flushing, change a blob without changing its length, then lose/rebuild the index or reopen a corrupt index. Reconstruction checks names, hash syntax and file lengths (`store.rs:3688-3700`), but its INSERT omits `durable` (`store.rs:3712`), inheriting the migration default of 1 (`store.rs:1663`). Automatic recovery uses that same reconstruction (`store.rs:2208-2209`). `get` consequently skips mandatory pending-entry hashing under the default `VerifyRestores::Off` policy (`store.rs:2488-2512`) and returns the damaged artifact as a hit. Both manual and automatic recovery reproduced this; the intact-index control correctly missed. Existing pending-corruption tests (`store/tests.rs:2183`, `store/tests.rs:2775`) and reconstruction tests (`store/tests.rs:3875`, `store/tests.rs:3972`) do not combine the transitions.
- **Recommendation:** Reconstruct entries as pending (`durable = 0`) until the existing verified flush succeeds, or verify and sync the complete publication before certifying it. Add manual and automatic index-recovery regressions with same-size corrupted blobs, intact pending blobs, and read-only readers. Assert damaged entries miss, intact reconstructed entries remain pending until verified promotion, and failed verification never promotes a row.
- **Fix with:** durable-state
- **Effort:** M

### KACH-14 · P1 · Failed same-key publication leaves metadata and index generations split

- **Area:** Data
- **Where:** `crates/kache-store/src/store.rs:2431`, `crates/kache-store/src/store.rs:2575`, `crates/kache-store/src/store.rs:2941`, `crates/kache-store/src/store.rs:2466`, `crates/kache-store/src/store.rs:5671`, `crates/kache-store/src/store.rs:5881`, `src/wrapper.rs:3296`, `src/wrapper.rs:3743`
- **Problem:** Reachable defect, reproduced against `221cf7d` using public `claim_build`/put/get/remove APIs and a SQLite trigger that aborts the replacement INSERT. One publication is split between the filesystem's `meta.json` and SQLite's entry/content-hash/blob-reference rows. Begin with committed A and B; remove A's metadata so ordinary missing-entry recovery claims A (`store.rs:2431-2432`, `store.rs:2579-2586`, wrapper call sites above). Replace A using another generation's artifacts. The put atomically replaces `meta.json` (`store.rs:2941`) before registering/committing its row (`store.rs:2944-2947`). An error there rolls back SQL but leaves the new metadata. `get(A)` reads the new file against the old committed row without checking their generation (`store.rs:2466-2490`); the reproducer returned the unpublished generation. `remove(A)` then derives decrements from that file (`store.rs:5671-5680`, `store.rs:5881-5897`), and deleted a blob still referenced by committed B. The existing concurrent-publication and refcount tests (`store/tests.rs:6386`, `store/tests.rs:6435`, `store/tests.rs:7261`) do not inject failure between metadata replacement and SQL registration.
- **Recommendation:** Make the committed SQLite generation authoritative for metadata lookup and reference release. Reject or reconcile a metadata/row mismatch before serving or reclaiming it, and decrement the transaction's recorded `entry_blobs` references rather than an unverified filesystem generation. Make same-key metadata publication crash-recoverable; an exception-only cleanup is insufficient for process death. Add fault injection at metadata rename, row registration and commit, including missing-metadata repair, reopen, and shared-blob removal. Every result must be the old complete generation, the new committed generation, or a miss; unrelated committed entries must survive.
- **Fix with:** durable-state
- **Effort:** M

### KACH-15 · P1 · Discarding a shared staging link makes published cache blobs writable

- **Area:** Data
- **Where:** `crates/kache-store/src/store.rs:278`, `crates/kache-store/src/store.rs:3850`, `crates/kache-store/src/store.rs:4017`, `crates/kache-store/src/store.rs:4083`, `crates/kache-store/src/atomic.rs:74`, `src/compiler_store.rs:8`
- **Problem:** Reachable defect, reproduced against `221cf7d` on Linux tmpfs with the pinned rustc. The staging lifecycle loses whether its inode is private or already shared with a published blob. On a filesystem without reflinks, put an eligible Rust artifact, then put the same source inode under another key. The second staged hardlink shares the first CAS inode; the existing-blob branch discards it (`store.rs:4017-4019`) through `unlink_blob`, which clears the inode's read-only mode before unlinking (`store.rs:278-294`, `store.rs:4083-4085`). The surviving CAS name and build output become writable. The production artifact policy permits `.o` and `.rlib` hardlinks (`src/compiler_store.rs:9-14`, `src/compiler_store.rs:74-82`). After flushing and restoring a hardlink, ordinary unwrapped `rustc --emit=obj` rewrote the restored `.o` in place, preserving its inode and length while changing the CAS bytes; the old key still hit under default verification. `atomic.rs:77-87` already preserves permissions when discarding an inode with multiple links, but this cleanup path does not. Existing snapshot/publication tests do not assert that discarding a second link preserves the first publication's seal.
- **Recommendation:** Use one shared-inode-aware unlink operation for staged files and blobs: unlinking one name must never make remaining CAS/restore names writable. Reuse the existing multiple-link rule rather than adding lifecycle flags. Test repeated puts, deduplicated output names and lost-publication cleanup on a forced hardlink substrate; assert all surviving modes, then use a real compiler overwrite as a negative control. Cover both a successful unlink and a transient unlink failure.
- **Fix with:** durable-state
- **Effort:** S

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

### KACH-12 · P2 · auto_gc budget test fails on main

- **Area:** Tests
- **Where:** `tests/integration_test.rs`, `test_auto_gc_bounds_store_size`
- **Problem:** Fails on main; needs triage.
- **Recommendation:** Reproduce, then fix the GC bound or the test.
- **Fix with:** none
- **Effort:** S

### KACH-16 · P2 · Failed entries monopolize every bounded durability sweep

- **Area:** Data
- **Where:** `crates/kache-store/src/store.rs:4303`, `crates/kache-store/src/store.rs:4349`, `crates/kache-store/src/store.rs:4376`, `src/daemon.rs:715`, `src/daemon.rs:1643`, `src/cli.rs:2659`
- **Problem:** Reachable defect, reproduced against `221cf7d` with 65 pending entries and three sweeps. Pending entries implicitly include both retryable failures and never-attempted work, but each sweep selects only the oldest `limit` rows (`store.rs:4307-4308`). Unreadable, changed or corrupt entries return false and retain their selection position (`store.rs:4349-4356`, `store.rs:4376-4379`). If the oldest 64 entries reference damaged bytes, every daemon sweep selects those same entries (`daemon.rs:724`; batch size `cli.rs:2659`), and healthy entry 65 never gets flushed. A single shared damaged blob suffices to block all 64. The reproducer kept all 65 pending after three bounded sweeps; an explicit unbounded GC-style flush reached and promoted the healthy entry. This breaks bounded background progress and leaves later hits paying mandatory hashing until failed rows are removed or another path flushes them. The existing corruption-continuation test (`store/tests.rs:2775-2824`) puts the good entry inside the same two-item batch, so it misses starvation across the batch boundary. This is distinct from KACH-12's disk-budget failure.
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
- **Where:** `.github/workflows/ci.yml:41`, `.github/workflows/ci.yml:79`, `.github/workflows/lint.yml:35`, `Makefile:79`, `Makefile:107`, `Makefile:114`, `AGENTS.md:1`, `crates/kache-proofs/src/lib.rs:16`, `scripts/check-coverage-scope.sh:1`, `crates/kache-e2e/src/fixture_runner.rs:117`
- **Problem:** Source-verified assurance gap at `221cf7d`; this is about which checks execute, not the existing red CI jobs. Current workflows run workspace tests and clippy, but never invoke the 54 fixture scenarios, their disabled-cache negative control, changed-line mutation testing, Kani, or the workspace coverage-scope check. `AGENTS.md:1-15` and Makefile comments still describe a mutation gate that is absent. `make check` also omits the fixture suite, and `make e2e` does not pass `--deny-missing-tools`, so supported fixtures missing compilers/tools are skipped successfully. The dormant proof harness has already drifted: it imports `checked_nonempty_contiguous_region`, which no longer exists in the included `src/checked_regions.rs`; ordinary `cargo test` sees zero tests in the `cfg(kani)` crate. The static suite inventory and old claims therefore do not establish that the strongest semantic checks run on a release candidate. No current line-coverage percentage or mutation score was measured in this audit.
- **Recommendation:** Define and wire a lean production acceptance gate: the existing workspace tests, representative real-compiler cold/warm/relocation/coalescing scenarios on supported platforms with required tools enforced, and their negative controls; run the full fixture matrix before release. Add targeted mutation coverage for key derivation, publication/recovery, restore and scheduling rather than treating an unchecked exclusion file as a guarantee. Repair or retire the stale Kani harness for the removed helper, and run any retained proofs explicitly. If reporting coverage, invoke the existing workspace-scope validator. Update AGENTS/scenario coverage documentation to match the actual gates and archive executed/skipped counts and binary identity. Do not expand back into removed remote or Windows features merely because old documents mention them.
- **Fix with:** none
- **Effort:** M
