Prefix: KACH
Next: 20

### KACH-19 · P3 · Removed cache settings remain in operator instructions and fixtures

- **Area:** Docs
- **Where:** `docs/getting-started/configuration.mdx:119`, `docs/getting-started/configuration.mdx:388`, `src/out_dir_alias.rs:10`, `src/out_dir_alias.rs:980`, `tests/event_log_contention_test.rs:97`, `crates/kache-config/src/config.rs:67`, `crates/kache-config/src/config.rs:2273`
- **Problem:** Source-verified contract drift at `79d0449`. Configuration docs still tell operators to set `gc_evict_shared`/`KACHE_GC_EVICT_SHARED` and `out_dir_alias_root`/`KACHE_OUT_DIR_ALIAS_ROOT`. Production conversion hardcodes shared eviction to false and OUT_DIR aliasing always passes `None` for the configurable root. The alias module header still advertises the removed override. The contention fixture writes `event_log_max_size = "1B"`, although production uses the fixed 64 MiB threshold. Removing tests for those retired settings is appropriate, but leaving the documented controls and ignored fixture values implies behaviors that no longer exist.
- **Recommendation:** Remove or rewrite the retired-setting instructions and ignored fixture values to describe the supported fixed policies. Delete the unreachable configured-root branch and its obsolete tests if no supported caller remains. If retaining a process-level rotation test, explicitly arrange an event log beyond the real threshold; preserve the existing busy-ledger retry unit coverage. Do not restore removed configuration solely to satisfy historical docs or tests.
- **Fix with:** none
- **Effort:** S
