Prefix: KACH
Next: 5
### KACH-4 · P3 · Collapse remaining two-knobs-one-policy pairs and micro-knobs

- **Area:** Config
- **Where:** `crates/kache-config/src/config.rs:100`, `crates/kache-config/src/config.rs:101`, `crates/kache-config/src/config.rs:103`, `crates/kache-config/src/config.rs:272`, `crates/kache-config/src/config.rs:276`, `crates/kache-config/src/config.rs:314`
- **Problem:** `event_log_max_size` + `event_log_keep_lines` tune one rotation policy; `compression_level` (zstd 1-22, default 3) is a micro-knob; `trust_daemon_execution` + `daemon_execution_concurrency` gate a niche pipeline; `scheduler_memory_pressure` and `auto_recover_min_free_bytes` overlap `max_size` pressure. Each adds docs, env, file, and test surface for no measured win.
- **Recommendation:** Fix rotation to constants, fix compression to 3, hard-code daemon concurrency sizing, merge or delete the pressure toggles. Delete env/file keys and tests for each.
- **Fix with:** none
- **Effort:** S
