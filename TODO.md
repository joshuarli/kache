Prefix: KACH
Next: 13
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
