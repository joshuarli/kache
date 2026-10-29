# Kache simplification campaign

Make Kache a cache again, not a second build system around Cargo. Delete target-directory
management, adaptive incremental compilation, rustdoc caching, stale remote/prefetch
accounting, and experimental config; narrow C/C++; consolidate diagnostics; replace the
daemon with direct execution only on measured parity. Correctness gates (cache keys,
copy-on-write restores, e2e) stay strict throughout.

## Status

- Goal: land deletion slices S1–S9 on `main`, each with narrow acceptance green.
- Base: `2822d12` (2026-10-29). Tree clean except this doc.
- Accepted: none. Red on baseline: `fallback_recovery_test::sandbox_preflight_bypasses_denied_server_but_keeps_allowed_server` (see Baseline). Next: S1.
- Resume: start S1 on `main`; acceptance `cargo clippy -p kache --all-targets` + `cargo test -p kache --lib`.

## Charter

- Decide and keep going. Record a decision only when a later slice depends on it.
- Within the execution grant, proceed without waiting. A plan grants no missing permission. Park any item needing authority outside the grant.
- Hard stops (park the item): irreversible or outside-the-repo actions, boundaries in §Execution boundary, a loop that cannot check anything. Pushing is always forbidden; the parent opens PRs.
- Fix egregious breakage met on the way in its own commit (anything failing clippy or a test); record lesser debt and leave it.
- Parity before optimization. No optimization slice may land before S1–S7 are accepted.
- One writer, serial slices, on `main` or a short branch merged before the next slice. Worktrees, if needed, at `.work/wt/<lane>` (never `/tmp`); `.work/` is git-ignored.
- Every commit is a Conventional Commit scoped to the workstream (`refactor(target)!`, `refactor(policy)!`, …) with the slice ID in the body. Code comments never cite this doc.
- Rust changes follow AGENTS.md mutant rules: independent literals (not the named constant), both sides of every `&&`/`||` tested, hand-apply each mutation and watch the test fail. CI mutation shards are the gate; the parent opens the draft PR, the executor kills survivors.
- Parent steering checkpoint (orchestrate) after each accepted slice.

## Decisions

Owner-decided (2026-10-29):
- C/C++: narrow but keep, including link caching. Delete only unsupported modes and platform variants outside the release matrix.
- Build-script result replay: measure first on a script-heavy profile; delete only on <10% warm-build contribution, else keep with the numbers recorded.
- Daemon: replace with direct execution only on measured parity (warm-hit ±10%, zero e2e regressions); otherwise park with the numbers and keep the daemon.

Prescribed (overridable with reason):
- `kache cargo` stays as a command but becomes a thin passthrough (exec cargo with wrapper env, no seeding/tracking). Reason: removing the subcommand is a product contract change; the machinery behind it is the actual boundary violation.
- `EntryMeta::emit_kinds` empty-means-unknown bypass stays. Reason: rejecting pre-gate entries mass-invalidates existing caches (kunobi-ninja/kache#325); format-break invalidation is out of scope.
- No crate reorganization. The draft's four-component shape is logical only; moving code marks every moved line changed and multiplies mutation load for zero behavior gain.
- Deletion slices delete completely: implementation, tests, config knobs, docs, exports. No tombstones, no compat aliases for removed flags (dead-code-archaeology: no live consumer can exist for a deleted feature).

Delegated (executor decides inside the bound, logs it in the slice commit):
- S6 profile choice: heaviest build-script profile runnable offline; bound: existing harness, `ARGS="--skip-clone"` if subjects cached.
- S7 archaeology: a C/C++ mode goes only with evidence (no callers, no e2e scenario, outside release matrix); link caching and ordinary object compilation are out of scope by owner decision.
- S8 parity call inside the ±10% / zero-regression bound; outside it, park.

## Baseline

- Known red (do not mistake for own breakage): `fallback_recovery_test::sandbox_preflight_bypasses_denied_server_but_keeps_allowed_server` fails on `2822d12` (`left: "fallback\ndirect\n"`, sandbox preflight bypass). Retry condition: passes alone AND in full suite after the sandbox/env fix lands; currently fails in full `cargo test --workspace`.
- Loop timings (measured 2026-10-29, macOS): incremental `cargo build -p kache` 10.6s; full `cargo test --workspace --lib --tests` ~5.5min; `make check` / `make e2e` / bench timings unmeasured — S1 records them before any expensive run.
- Verified against the tree: Windows-support removal already landed (platform removal + `39abc1f`, e2e expectations dropped in `79d9e3a`); remaining `cfg(windows)` is benign platform-conditional helpers. Rust 1.99 rustdoc passthrough already documented — S1 removes a mostly-dead adapter. No non-test producers of `PrefetchHit`/`RemoteHit` remain (remote backend removed in `b8505c2`); S2 confirms readers of old logs before deleting variants.

## Sequence

Serial, one writer; no parallel pairs (all structural slices touch `src/cli.rs` or `src/main.rs` wiring — revisit after S1).

- S1 rustdoc deletion. Owns `src/compiler/rustdoc.rs`, rustdoc branches in `src/compiler/mod.rs` + `src/main.rs`, `rustdoc_*` fns in `src/cli.rs`, `src/wrapper_config.rs` rustdoc refs, `docs/getting-started/rustdoc.mdx` + cross-refs. Prereq: none. Acceptance: `cargo clippy -p kache --all-targets` + `cargo test -p kache --lib`; record `make check`/`make e2e` wall times.
- S2 stale remote/prefetch accounting. Owns `PrefetchAccounting`/`PrefetchOrigin`/`remote_wait_ms` in `kache-core/timeline.rs`, `kache-protocol` prefetch/remote fields, readers in `src/cli.rs` display + `src/sessions.rs` + `src/miss_chain.rs`. Prereq: S1. Bound: audit old-log readers first; keep `EventResult` variants only if persisted logs require them. Acceptance: narrow tests for touched crates + clippy.
- S3 config surface. Owns compat aliases + duplicate env overrides + experimental knobs in `crates/kache-config`, `src/cli_args.rs`, `src/config.rs`, `docs/getting-started/configuration.mdx`. Prereq: S2. Bound: every removed flag/env is a contract change — list each in the slice report for the owner; never silently keep a hidden alias. Acceptance: config/cli test targets + `sh scripts/check-docs.sh`.
- S4 target management. Owns `src/target_cleanup.rs`, `src/target_dedup.rs`, `src/target_liveness.rs`, `src/target_seed.rs` (+`src/target_seed/`), `src/target_use.rs`, `src/cargo_proxy.rs` internals; `kache cargo` becomes thin passthrough; `src/main.rs` + `src/cli.rs` wiring only. Prereq: S3. Bound: Kache owns its cache dir, Cargo owns `target/`; reflink restore stays. Acceptance: affected tests + `make e2e` (cold/warm/noop must pass — restore path is the parity proof).
- S5 adaptive incremental. Owns `src/incremental_policy.rs` (+`src/incremental_policy/`), adaptive branches in `src/wrapper.rs` + `src/cache_key.rs` policy fold. Prereq: S4. Replacement: cache stateless eligible invocations; pass incremental through unchanged. `cache_key.rs` correctness logic is untouchable beyond the policy fold. Acceptance: wrapper/cache-key tests + `make e2e`, incl. `e2e-rust-cross-tree`.
- S6 build-script verdict. Owns bench run + (conditionally) `src/build_script.rs`, `src/build_script/`, `launcher/`. Prereq: S5. Measure script-heavy profile cold/warm; ≥10% warm contribution keeps it (record numbers, slice ends), else delete replay, keep `build.rs` compilation caching. Acceptance: the bench command itself + affected tests if deleted.
- S7 C/C++ narrowing. Owns unsupported modes + off-matrix platform variants in `src/compiler/cc/` + `src/link_probe.rs`. Prereq: S5 (independent of S6 outcome). Bound: ordinary object compilation and link caching stay per owner. Acceptance: `e2e-cc-*` + `e2e-c-*` scenarios green.
- S8 direct execution. Prototype direct wrapper + per-key file locks + throttled-maintenance GC in `.work/wt/direct`; run e2e suite and ≥2 bench profiles (one Rust, one native) against both paths. Prereq: S6, S7. Replace only on warm-hit ±10% + zero e2e regressions; else park branch with numbers and continue on daemon tree. Acceptance: the head-to-head numbers, recorded in the slice report.
- S9 diagnostics consolidation. Owns `src/events.rs`, `src/report.rs`, `src/otel.rs`, `src/miss_chain.rs`, `src/sessions.rs`, `src/explain.rs` surface. Prereq: S8 (verdict either way). Keep hit/miss/passthrough counters + concise miss reasons; delete session archaeology and exporters with no readers. Acceptance: explain/report tests + docs check.
- S10 close. Dead-code sweep of leftovers from S1–S9, docs sync, `ultrareview` P1/P2 fixed, final report with the collected contract-change list. Prereq: S9. Acceptance: `make check` + `make e2e` green except Baseline red.

Failing item: 3 attempts max (root-cause: repro first, one hypothesis at a time), then park on branch with cause + retry condition. Shared env blocker gets one owner escalation, then Needs environment.

## Acceptance

- Per slice: its listed command(s) on the integrated tree, plus the narrowest affected test target. Green = pass with no new failures vs Baseline.
- Milestones (full suite, grouped to amortize cost): after S3 and S7 — `make check`; after S4, S5, S8 — `make check` + `make e2e`. Close (S10) runs both plus `sh scripts/check-docs.sh`.
- Reference corpus (never shrunk to pass): `scenarios/e2e-*` cold/warm/noop, `e2e-rust-cross-tree`, bench cold/warm on one Rust + one native profile at S8. Parity gate for S8 is head-to-head, not absolute.

## Needs environment

- `make bench-contention` (six overlapping Cargo jobs) is Linux-only; macOS numbers do not substitute. Owner command on Linux: `make bench-contention ARGS="--output tmp/bench/contention-<date>"`.
- Bench subjects requiring fresh clones need network (`make bench PROFILE=<p>` without `--skip-clone`). Offline slices use cached subjects only.
- CI mutation shards (`Mutation testing (diff k/n)`) run on Linux runners after the parent opens the draft PR; a laptop run at 1–3 jobs takes hours and flakes fixture tests.

## Execution boundary and non-goals

- Authorized: delete/refactor code, tests, config knobs, docs in-repo; run builds, tests, e2e, bench (offline subjects); create worktrees under `.work/wt/`.
- Forbidden: pushes (parent opens PRs), `RUSTC_WRAPPER=kache` dogfooding (kills mutant baselines; Makefile already unexports it), release tags, `make bump`, crates.io publishes, CI workflow edits, formatters.
- Non-goals: `cache_key.rs` correctness rewrite; rusqlite replacement; crate reorganization; Rust-only scope (owner kept C/C++); full build-script deletion without the S6 numbers; `emit_kinds` rejection; Windows removal (already landed — verify only).
- Contract changes (owner reviews at close; each recorded in its slice report): removed CLI flags/env vars (S3), `kache cargo` losing seed/track behavior (S4), incremental modes collapsing to passthrough (S5), conditional build-script replay removal (S6), removed diagnostics surface (S9).

## Reference

- Entry: `src/frontend.rs` (CLI) → `src/main.rs` dispatch → `src/wrapper.rs` (rustc/cc classify, key, restore-or-compile) → `src/daemon.rs` service. Store: `crates/kache-store`, index SQLite, `crates/kache-format` entry meta. Config: `crates/kache-config` + `src/cli_args.rs` + `src/cli.rs` (380KB render/report monolith — every structural slice touches it; that is why slices are serial).
- Removal targets by size: `src/cache_key.rs` 475KB PROTECTED; `src/cli.rs` 380KB shared wiring; `src/build_script.rs` 272KB + `src/build_script/` 212KB; `crates/kache-config/src/config.rs` 159KB; `src/compiler/rustdoc.rs` 149KB dead; `src/incremental_policy.rs` 129KB; `src/daemon_execution*.rs` ~81KB; `src/target_*.rs` + `cargo_proxy.rs` ~530KB combined.
