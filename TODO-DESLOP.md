Prefix: DSLP
Next: 102

Deslop campaign backlog ([plan](https://github.com/laputa-systems/stuff/blob/main/docs/campaigns/deslop.md), tier D). One agent works this file with the `deslop` skill: take the lowest-numbered open ticket whose area does not start with `Needs owner ·`. After DSLP-5, work the open P1 and P2 findings, then fill in `## Report` below.

## Report

Not started.

## Tickets

### DSLP-5 · P2 · Dead code: crates/kache-config (1/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-config
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 5564 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-6 · P2 · Types: crates/kache-e2e bench, cc oracle, daemon (2/28)

- **Area:** deslop · types
- **Where:** crates/kache-e2e/src/assertions.rs, crates/kache-e2e/src/bench_host.rs, crates/kache-e2e/src/bench_otlp.rs, crates/kache-e2e/src/bench_profile.rs, crates/kache-e2e/src/bench_runner/tests.rs, crates/kache-e2e/src/bench_runner.rs, crates/kache-e2e/src/bin/e2e-fallback.rs, crates/kache-e2e/src/bin/kache-cc-bench.rs, crates/kache-e2e/src/bin/kache-cc-oracle.rs, crates/kache-e2e/src/bin/kache-cc.rs, crates/kache-e2e/src/bin/kache-cxx.rs, crates/kache-e2e/src/bin/kache-scenario.rs, crates/kache-e2e/src/cc_bench/activity.rs, crates/kache-e2e/src/cc_bench/mod.rs, crates/kache-e2e/src/cc_bench/stats.rs, crates/kache-e2e/src/cc_bench/workload.rs, crates/kache-e2e/src/cc_bench_cli.rs, crates/kache-e2e/src/cc_oracle/case.rs, crates/kache-e2e/src/cc_oracle/exec.rs, crates/kache-e2e/src/cc_oracle/gcov.rs, crates/kache-e2e/src/cc_oracle/judge.rs, crates/kache-e2e/src/cc_oracle/mod.rs, crates/kache-e2e/src/cc_oracle/pin.rs, crates/kache-e2e/src/cc_oracle/report.rs, crates/kache-e2e/src/cc_oracle_cli.rs, crates/kache-e2e/src/daemon.rs, crates/kache-e2e/src/disk_usage.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 18845 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-7 · P2 · Types: crates/kache-e2e fixture and scenario runners (3/28)

- **Area:** deslop · types
- **Where:** crates/kache-e2e/src/fixture.rs, crates/kache-e2e/src/fixture_runner.rs, crates/kache-e2e/src/lib.rs, crates/kache-e2e/src/phase.rs, crates/kache-e2e/src/report.rs, crates/kache-e2e/src/result.rs, crates/kache-e2e/src/runner.rs, crates/kache-e2e/src/scenario.rs, crates/kache-e2e/src/scenario_runner.rs, crates/kache-e2e/src/source.rs, crates/kache-e2e/tests/skip_summary.rs, crates/kache-e2e/tests/strict_gate.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 5586 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-8 · P2 · Types: crates/kache-fs (4/28)

- **Area:** deslop · types
- **Where:** crates/kache-fs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 4927 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-9 · P2 · Types: crates/kache-store core (5/28)

- **Area:** deslop · types
- **Where:** crates/kache-store/examples/bench-cc-memos.rs, crates/kache-store/src/atomic.rs, crates/kache-store/src/blob_drift.rs, crates/kache-store/src/blob_validation.rs, crates/kache-store/src/cc_memo.rs, crates/kache-store/src/config.rs, crates/kache-store/src/eviction.rs, crates/kache-store/src/eviction_evidence/tests.rs, crates/kache-store/src/eviction_evidence.rs, crates/kache-store/src/file_hash.rs, crates/kache-store/src/filesystem.rs, crates/kache-store/src/index_compaction.rs, crates/kache-store/src/lib.rs, crates/kache-store/src/link.rs, crates/kache-store/src/markers.rs, crates/kache-store/src/opcounts.rs, crates/kache-store/src/sharing.rs, crates/kache-store/src/store/compile_time_backfill.rs, crates/kache-store/src/store/tests/cost_backfill_tests.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 12970 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-10 · P2 · Types: crates/kache-store tests (6/28)

- **Area:** deslop · types
- **Where:** crates/kache-store/src/store/tests.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 14735 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-11 · P2 · Types: crates/kache-store remaining modules (7/28)

- **Area:** deslop · types
- **Where:** crates/kache-store/src/store.rs, crates/kache-store/src/test_support.rs, crates/kache-store/src/workspace.rs, crates/kache-store/tests/byte_counters.rs, crates/kache-store/tests/staged_publication.rs, crates/kache-store/tests/thread_store_counters.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 7306 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-12 · P2 · Types: scripts (8/28)

- **Area:** deslop · types
- **Where:** scripts
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 10830 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-13 · P2 · Types: support crates, launcher, scenarios (9/28)

- **Area:** deslop · types
- **Where:** crates/kache-client, crates/kache-core, crates/kache-format, crates/kache-ipc, crates/kache-log, crates/kache-proofs, crates/kache-protocol, crates/kache-shims, launcher, scenarios
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 14331 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-14 · P2 · Types: src/build_script.rs (10/28)

- **Area:** deslop · types
- **Where:** src/build_script.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 6814 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-15 · P2 · Types: src/cache_key submodules (11/28)

- **Area:** deslop · types
- **Where:** src/cache_key
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 18491 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-16 · P2 · Types: src/cache_key.rs (12/28)

- **Area:** deslop · types
- **Where:** src/cache_key.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 11446 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-17 · P2 · Types: src/cli submodules (13/28)

- **Area:** deslop · types
- **Where:** src/cli
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 6129 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-18 · P2 · Types: src/cli.rs (14/28)

- **Area:** deslop · types
- **Where:** src/cli.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 7174 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-19 · P2 · Types: src/compiler/cc tests (15/28)

- **Area:** deslop · types
- **Where:** src/compiler/cc/tests.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 10527 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-20 · P2 · Types: src/compiler (16/28)

- **Area:** deslop · types
- **Where:** src/compiler/cc.rs, src/compiler/flags.rs, src/compiler/mod.rs, src/compiler/platform.rs, src/compiler/rustc.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 17047 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-25 · P2 · Types: src native_archive, out_dir_alias, path_normalizer, prestage (21/28)

- **Area:** deslop · types
- **Where:** src/native_archive.rs, src/notice.rs, src/opcounts.rs, src/otel.rs, src/out_dir_alias.rs, src/path_normalizer.rs, src/phase_trace.rs, src/platform.rs, src/prestage.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 14123 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-26 · P2 · Types: src probe, report, savings, scheduler (22/28)

- **Area:** deslop · types
- **Where:** src/probe, src/probe_memo.rs, src/report, src/report.rs, src/savings, src/savings.rs, src/scheduler
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 13333 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-27 · P2 · Types: src/wrapper submodules (23/28)

- **Area:** deslop · types
- **Where:** src/wrapper
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 11557 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-28 · P2 · Types: src/wrapper.rs (24/28)

- **Area:** deslop · types
- **Where:** src/wrapper.rs
- **Problem:** Measure applicable strict-profile candidates and typing escape hatches; record witnesses and false positives. Scope contains approximately 8214 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: adopted profile clean, no increased escape hatches, witnesses recorded; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-33 · P2 · Tests: crates/kache-e2e bench, cc oracle, daemon (2/28)

- **Area:** deslop · tests
- **Where:** crates/kache-e2e/src/assertions.rs, crates/kache-e2e/src/bench_host.rs, crates/kache-e2e/src/bench_otlp.rs, crates/kache-e2e/src/bench_profile.rs, crates/kache-e2e/src/bench_runner/tests.rs, crates/kache-e2e/src/bench_runner.rs, crates/kache-e2e/src/bin/e2e-fallback.rs, crates/kache-e2e/src/bin/kache-cc-bench.rs, crates/kache-e2e/src/bin/kache-cc-oracle.rs, crates/kache-e2e/src/bin/kache-cc.rs, crates/kache-e2e/src/bin/kache-cxx.rs, crates/kache-e2e/src/bin/kache-scenario.rs, crates/kache-e2e/src/cc_bench/activity.rs, crates/kache-e2e/src/cc_bench/mod.rs, crates/kache-e2e/src/cc_bench/stats.rs, crates/kache-e2e/src/cc_bench/workload.rs, crates/kache-e2e/src/cc_bench_cli.rs, crates/kache-e2e/src/cc_oracle/case.rs, crates/kache-e2e/src/cc_oracle/exec.rs, crates/kache-e2e/src/cc_oracle/gcov.rs, crates/kache-e2e/src/cc_oracle/judge.rs, crates/kache-e2e/src/cc_oracle/mod.rs, crates/kache-e2e/src/cc_oracle/pin.rs, crates/kache-e2e/src/cc_oracle/report.rs, crates/kache-e2e/src/cc_oracle_cli.rs, crates/kache-e2e/src/daemon.rs, crates/kache-e2e/src/disk_usage.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 18845 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-34 · P2 · Tests: crates/kache-e2e fixture and scenario runners (3/28)

- **Area:** deslop · tests
- **Where:** crates/kache-e2e/src/fixture.rs, crates/kache-e2e/src/fixture_runner.rs, crates/kache-e2e/src/lib.rs, crates/kache-e2e/src/phase.rs, crates/kache-e2e/src/report.rs, crates/kache-e2e/src/result.rs, crates/kache-e2e/src/runner.rs, crates/kache-e2e/src/scenario.rs, crates/kache-e2e/src/scenario_runner.rs, crates/kache-e2e/src/source.rs, crates/kache-e2e/tests/skip_summary.rs, crates/kache-e2e/tests/strict_gate.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 5586 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-35 · P2 · Tests: crates/kache-fs (4/28)

- **Area:** deslop · tests
- **Where:** crates/kache-fs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 4927 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-36 · P2 · Tests: crates/kache-store core (5/28)

- **Area:** deslop · tests
- **Where:** crates/kache-store/examples/bench-cc-memos.rs, crates/kache-store/src/atomic.rs, crates/kache-store/src/blob_drift.rs, crates/kache-store/src/blob_validation.rs, crates/kache-store/src/cc_memo.rs, crates/kache-store/src/config.rs, crates/kache-store/src/eviction.rs, crates/kache-store/src/eviction_evidence/tests.rs, crates/kache-store/src/eviction_evidence.rs, crates/kache-store/src/file_hash.rs, crates/kache-store/src/filesystem.rs, crates/kache-store/src/index_compaction.rs, crates/kache-store/src/lib.rs, crates/kache-store/src/link.rs, crates/kache-store/src/markers.rs, crates/kache-store/src/opcounts.rs, crates/kache-store/src/sharing.rs, crates/kache-store/src/store/compile_time_backfill.rs, crates/kache-store/src/store/tests/cost_backfill_tests.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 12970 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-37 · P2 · Tests: crates/kache-store tests (6/28)

- **Area:** deslop · tests
- **Where:** crates/kache-store/src/store/tests.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 14735 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-38 · P2 · Tests: crates/kache-store remaining modules (7/28)

- **Area:** deslop · tests
- **Where:** crates/kache-store/src/store.rs, crates/kache-store/src/test_support.rs, crates/kache-store/src/workspace.rs, crates/kache-store/tests/byte_counters.rs, crates/kache-store/tests/staged_publication.rs, crates/kache-store/tests/thread_store_counters.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 7306 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-39 · P2 · Tests: scripts (8/28)

- **Area:** deslop · tests
- **Where:** scripts
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 10830 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-40 · P2 · Tests: support crates, launcher, scenarios (9/28)

- **Area:** deslop · tests
- **Where:** crates/kache-client, crates/kache-core, crates/kache-format, crates/kache-ipc, crates/kache-log, crates/kache-proofs, crates/kache-protocol, crates/kache-shims, launcher, scenarios
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 14331 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-41 · P2 · Tests: src/build_script.rs (10/28)

- **Area:** deslop · tests
- **Where:** src/build_script.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 6814 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-42 · P2 · Tests: src/cache_key submodules (11/28)

- **Area:** deslop · tests
- **Where:** src/cache_key
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 18491 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-43 · P2 · Tests: src/cache_key.rs (12/28)

- **Area:** deslop · tests
- **Where:** src/cache_key.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 11446 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-44 · P2 · Tests: src/cli submodules (13/28)

- **Area:** deslop · tests
- **Where:** src/cli
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 6129 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-45 · P2 · Tests: src/cli.rs (14/28)

- **Area:** deslop · tests
- **Where:** src/cli.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 7174 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-46 · P2 · Tests: src/compiler/cc tests (15/28)

- **Area:** deslop · tests
- **Where:** src/compiler/cc/tests.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 10527 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-47 · P2 · Tests: src/compiler (16/28)

- **Area:** deslop · tests
- **Where:** src/compiler/cc.rs, src/compiler/flags.rs, src/compiler/mod.rs, src/compiler/platform.rs, src/compiler/rustc.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 17047 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-52 · P2 · Tests: src native_archive, out_dir_alias, path_normalizer, prestage (21/28)

- **Area:** deslop · tests
- **Where:** src/native_archive.rs, src/notice.rs, src/opcounts.rs, src/otel.rs, src/out_dir_alias.rs, src/path_normalizer.rs, src/phase_trace.rs, src/platform.rs, src/prestage.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 14123 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-53 · P2 · Tests: src probe, report, savings, scheduler (22/28)

- **Area:** deslop · tests
- **Where:** src/probe, src/probe_memo.rs, src/report, src/report.rs, src/savings, src/savings.rs, src/scheduler
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 13333 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-54 · P2 · Tests: src/wrapper submodules (23/28)

- **Area:** deslop · tests
- **Where:** src/wrapper
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 11557 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-55 · P2 · Tests: src/wrapper.rs (24/28)

- **Area:** deslop · tests
- **Where:** src/wrapper.rs
- **Problem:** Audit boundaries and tiers, then measure coverage and scoped mutation; prove new tests with injected faults. Scope contains approximately 8214 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: critical survivors dispositioned, test cost budget holds, and scoped lane is qualified; run make test and make lint. Missing coverage or mutation tools are Needs environment.
- **Fix with:** deslop
- **Effort:** M

### DSLP-60 · P2 · Dead code: crates/kache-e2e bench, cc oracle, daemon (2/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-e2e/src/assertions.rs, crates/kache-e2e/src/bench_host.rs, crates/kache-e2e/src/bench_otlp.rs, crates/kache-e2e/src/bench_profile.rs, crates/kache-e2e/src/bench_runner/tests.rs, crates/kache-e2e/src/bench_runner.rs, crates/kache-e2e/src/bin/e2e-fallback.rs, crates/kache-e2e/src/bin/kache-cc-bench.rs, crates/kache-e2e/src/bin/kache-cc-oracle.rs, crates/kache-e2e/src/bin/kache-cc.rs, crates/kache-e2e/src/bin/kache-cxx.rs, crates/kache-e2e/src/bin/kache-scenario.rs, crates/kache-e2e/src/cc_bench/activity.rs, crates/kache-e2e/src/cc_bench/mod.rs, crates/kache-e2e/src/cc_bench/stats.rs, crates/kache-e2e/src/cc_bench/workload.rs, crates/kache-e2e/src/cc_bench_cli.rs, crates/kache-e2e/src/cc_oracle/case.rs, crates/kache-e2e/src/cc_oracle/exec.rs, crates/kache-e2e/src/cc_oracle/gcov.rs, crates/kache-e2e/src/cc_oracle/judge.rs, crates/kache-e2e/src/cc_oracle/mod.rs, crates/kache-e2e/src/cc_oracle/pin.rs, crates/kache-e2e/src/cc_oracle/report.rs, crates/kache-e2e/src/cc_oracle_cli.rs, crates/kache-e2e/src/daemon.rs, crates/kache-e2e/src/disk_usage.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 18845 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-61 · P2 · Dead code: crates/kache-e2e fixture and scenario runners (3/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-e2e/src/fixture.rs, crates/kache-e2e/src/fixture_runner.rs, crates/kache-e2e/src/lib.rs, crates/kache-e2e/src/phase.rs, crates/kache-e2e/src/report.rs, crates/kache-e2e/src/result.rs, crates/kache-e2e/src/runner.rs, crates/kache-e2e/src/scenario.rs, crates/kache-e2e/src/scenario_runner.rs, crates/kache-e2e/src/source.rs, crates/kache-e2e/tests/skip_summary.rs, crates/kache-e2e/tests/strict_gate.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 5586 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-62 · P2 · Dead code: crates/kache-fs (4/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-fs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 4927 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-63 · P2 · Dead code: crates/kache-store core (5/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-store/examples/bench-cc-memos.rs, crates/kache-store/src/atomic.rs, crates/kache-store/src/blob_drift.rs, crates/kache-store/src/blob_validation.rs, crates/kache-store/src/cc_memo.rs, crates/kache-store/src/config.rs, crates/kache-store/src/eviction.rs, crates/kache-store/src/eviction_evidence/tests.rs, crates/kache-store/src/eviction_evidence.rs, crates/kache-store/src/file_hash.rs, crates/kache-store/src/filesystem.rs, crates/kache-store/src/index_compaction.rs, crates/kache-store/src/lib.rs, crates/kache-store/src/link.rs, crates/kache-store/src/markers.rs, crates/kache-store/src/opcounts.rs, crates/kache-store/src/sharing.rs, crates/kache-store/src/store/compile_time_backfill.rs, crates/kache-store/src/store/tests/cost_backfill_tests.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 12970 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-64 · P2 · Dead code: crates/kache-store tests (6/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-store/src/store/tests.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 14735 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-65 · P2 · Dead code: crates/kache-store remaining modules (7/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-store/src/store.rs, crates/kache-store/src/test_support.rs, crates/kache-store/src/workspace.rs, crates/kache-store/tests/byte_counters.rs, crates/kache-store/tests/staged_publication.rs, crates/kache-store/tests/thread_store_counters.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 7306 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-66 · P2 · Dead code: scripts (8/28)

- **Area:** deslop · dead code
- **Where:** scripts
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 10830 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-67 · P2 · Dead code: support crates, launcher, scenarios (9/28)

- **Area:** deslop · dead code
- **Where:** crates/kache-client, crates/kache-core, crates/kache-format, crates/kache-ipc, crates/kache-log, crates/kache-proofs, crates/kache-protocol, crates/kache-shims, launcher, scenarios
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 14331 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-68 · P2 · Dead code: src/build_script.rs (10/28)

- **Area:** deslop · dead code
- **Where:** src/build_script.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 6814 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-69 · P2 · Dead code: src/cache_key submodules (11/28)

- **Area:** deslop · dead code
- **Where:** src/cache_key
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 18491 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-70 · P2 · Dead code: src/cache_key.rs (12/28)

- **Area:** deslop · dead code
- **Where:** src/cache_key.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 11446 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-71 · P2 · Dead code: src/cli submodules (13/28)

- **Area:** deslop · dead code
- **Where:** src/cli
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 6129 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-72 · P2 · Dead code: src/cli.rs (14/28)

- **Area:** deslop · dead code
- **Where:** src/cli.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 7174 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-73 · P2 · Dead code: src/compiler/cc tests (15/28)

- **Area:** deslop · dead code
- **Where:** src/compiler/cc/tests.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 10527 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-74 · P2 · Dead code: src/compiler (16/28)

- **Area:** deslop · dead code
- **Where:** src/compiler/cc.rs, src/compiler/flags.rs, src/compiler/mod.rs, src/compiler/platform.rs, src/compiler/rustc.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 17047 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-79 · P2 · Dead code: src native_archive, out_dir_alias, path_normalizer, prestage (21/28)

- **Area:** deslop · dead code
- **Where:** src/native_archive.rs, src/notice.rs, src/opcounts.rs, src/otel.rs, src/out_dir_alias.rs, src/path_normalizer.rs, src/phase_trace.rs, src/platform.rs, src/prestage.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 14123 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-80 · P2 · Dead code: src probe, report, savings, scheduler (22/28)

- **Area:** deslop · dead code
- **Where:** src/probe, src/probe_memo.rs, src/report, src/report.rs, src/savings, src/savings.rs, src/scheduler
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 13333 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-81 · P2 · Dead code: src/wrapper submodules (23/28)

- **Area:** deslop · dead code
- **Where:** src/wrapper
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 11557 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-82 · P2 · Dead code: src/wrapper.rs (24/28)

- **Area:** deslop · dead code
- **Where:** src/wrapper.rs
- **Problem:** Run applicable dead-code tools and inspect entry points, registrations and consumers before cuts. Scope contains approximately 8214 first-party source lines, excluding related consumers. Contracts: CLI, cache keys, store formats and wrapper protocol where used by this scope. Consumers: workspace crates, kache wrapper, fixtures and scripts calling the scoped modules.
- **Recommendation:** Accept: findings clean or reasoned allowlist, walk-backs ticketed Needs owner; run make test and make lint.
- **Fix with:** deslop
- **Effort:** M

### DSLP-88 · P2 · Make workspace tests independent of checkout-local temporary roots and ambient KACHE_CACHE_DIR

- **Area:** Needs environment · test isolation
- **Where:** src/cache_key/tests.rs, src/cli/tests.rs, src/out_dir_alias.rs, src/incremental_policy.rs, crates/kache-config/src/config/tests.rs
- **Problem:** With TMPDIR inside the checkout, make test fails 11 kache library tests because scratch paths inherit the checkout workspace, toolchain and path-normalization context; the same 11 fail at base aa8df06 (its deeper detached worktree adds two Unix socket path-length failures), and all pass with the system TMPDIR. Separately, with KACHE_CACHE_DIR exported (the isolation the deslop rules require), kache-config's config::tests::probe_memos_live_under_the_configured_cache_dir fails: its second probe_memo_dir() call reads the ambient variable instead of the default. The suite stops at the first failing test binary, so later binaries do not run.
- **Recommendation:** Make test fixtures explicitly control workspace ancestry, path-normalization roots and socket path length while keeping caches and configuration isolated; run make test with TMPDIR under .work/tmp.
- **Fix with:** test-audit
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

### DSLP-97 · P3 · Eight lib tests fail under host load through wall-clock waits

- **Area:** Tests
- **Where:** src/invocation/, src/probe/, src/daemon_execution.rs, src/daemon_lifecycle.rs, kache-store wait_for_committed_returns_false_without_an_owner
- **Problem:** Under load average 25-45 these fail with process-spawn timeouts and EAGAIN and pass alone; they wait on wall-clock bounds rather than readiness signals.
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
- **Problem:** Cross.toml has no in-repo consumer and [features] default = [] is unused (consumer questions); README links a missing bug_report issue template; TODO.md says 67 oracle cases where STATUS.md and cases.toml say 72; AGENTS.md lists DSLP-89 as open though cache_key_underkeying_test.rs:1166-1174 guards it.
- **Recommendation:** Confirm consumers then remove Cross.toml and the empty features table; fix the README link and the case count; drop the stale AGENTS.md gap.
- **Fix with:** dead-code-archaeology
- **Effort:** S

