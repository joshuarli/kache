# Roadmap

This is the single plan for this repository: all current and future work, and the handoff for resuming it. Campaigns are sections below, and their items interleave in **Sequence**. "Resume the roadmap" means continue from **Status**.

## Status

- **active**: C/C++ parity, converting planned inventory into blocking cases and fixing the exposed gaps (KACH-26).
- Last accepted revision: `fc43a9a005ecfe122a8db72fdb64889d3f631a39`; the ledger reports earlier integrated main checks green, but integration at this revision is not established here.
- No parked branch or retry condition is recorded for the current slice. Earlier work branches are historical; check Git before reuse.
- Next: choose the first dependency-ready planned case in `campaigns/cpp/cases.toml`, add its oracle and fix any correctness failure on L3.
- Performance acceptance is waived; macOS CI is disabled and Linux CI is reconciled after local L3 verification and merge, per the 2026-11-23 owner ruling. Missing matrix evidence remains missing.

## Sequence

| ID | Campaign | Owned files | Prerequisites | Acceptance command |
|---|---|---|---|---|
| CCP-02 | C/C++ parity | `src/cache_key/`, `src/compiler/`, `campaigns/cpp/` | CCP-01 runner | `make cc-oracle CASE=<id>` |
| CCP-03 | C/C++ parity | `src/compiler/`, `campaigns/cpp/` | CCP-01 and relevant CCP-02 guards | `make cc-oracle CASE=<id>` |
| CCP-04 | C/C++ parity | `src/compiler/`, `campaigns/cpp/` | CCP-02; CCP-03 for response cases | `make cc-oracle CASE=<id>` |
| CCP-05 | C/C++ parity | `src/compiler/`, `src/wrapper/`, `campaigns/cpp/` | CCP-02 and relevant CCP-03 parsing | `make cc-oracle CASE=<id>` |
| CCP-06 | C/C++ parity | `src/compiler/`, `campaigns/cpp/` | CCP-02, CCP-03, CCP-05 | `make cc-oracle CASE=<id>` |
| CCP-07 | C/C++ parity | `campaigns/cpp/`, `scenarios/`, launcher and compiler paths | applicable CCP-03–06 | `make cc-project-autotools TOOLCHAINS="gcc clang cc"` and applicable project gates |
| CCP-08 | C/C++ parity | `src/`, `crates/kache-store/`, `campaigns/cpp/` | CCP-01; alongside CCP-02–07 | `make cc-oracle CASE=<id>` and affected store/wrapper tests |

## Campaign: C/C++ parity

### Goal and state

KACH-26: match the pinned ccache's safe local compiler-cache coverage and observable behavior on macOS and Linux. Ccache is the reuse reference; the raw compiler is the correctness oracle. Preserve Rust support, local storage, scheduling and cross-worktree reuse. Correct passthrough is not cached parity. Work the [ticket backlog](../TODO.md); the separate [deslop backlog](../TODO-DESLOP.md) retains its tickets. Its former tier-D instruction selected the lowest-numbered open ticket outside `Needs owner ·`, then open P1/P2 findings after DSLP-5; its report had not started.

CCP-00 bootstrap and CCP-01 runner exist. CCP-02–08 have landed substantial coverage but remain incomplete; the inventory is authoritative. The latest ledger records 196 inventory entries: 143 oracle, 43 planned, 6 conditional and 4 out of scope; 139 cases block and four remain open: `cc-eqp-comment-only`, `cc-eqp-unused-macro`, `cc-eqp-inactive-conditional`, and `cc-input-token-paste-probe`. Recount from the inventory before relying on these figures.

- CCP-02: cover header appearance, shadowing, symlinks, generated probes, compiler replacement, tool/config files, time macros, policy changes and input races. Resolve token-pasted header probes without stale hits.
- CCP-03: cover response quoting, nesting, encoding, paths and failure behavior; driver options, flags and invocation shapes must either key all inputs or preserve fallback.
- CCP-04: compare safe equivalent-preprocessing reuse with ccache while preserving observable warnings, debug information and time-sensitive behavior.
- CCP-05: restore complete object, dependency, diagnostic and sidecar sets; exercise missing, corrupt and partial output behavior.
- CCP-06: exercise supported PCH and explicit module consumers with their prerequisites and conservative invalidation.
- CCP-07: validate universal objects, SDKs, frameworks and launchers, plus pinned zlib, LLVM, Chromium, `cc`-crate, Meson and Autotools builds through cold, warm, edit, revert and relocation phases.
- CCP-08: cover store damage, eviction, concurrency, cancellation, interrupted publication and diagnostics. KACH-22 tracks mutation testing.

### Decisions

- Wrong output and stale hits outrank reuse and feature expansion. Preserve exact compiler status, diagnostics, side effects and complete output sets. Failed compiles cannot become hits.
- Equivalent preprocessing reuse for comment-only and similar edits is declined where observable debug or warning behavior makes it unsound; preserve the open cases and distinguish correct misses from parity.
- Response-file cycles hang pinned ccache; `cc-rsp-cycle` disables that reference arm and requires raw compiler agreement. GCC rejects a UTF-8 BOM that Clang accepts.
- Profile inputs, path-named Clang config and GCC specs files, and supported VFS overlays are content-keyed. Bare searched configs, nested config `@` files, specs `%include`, and overlays with `use-external-names: false` pass through. Sidecars under `-S`, implicit modules and module production pass through.
- CCP-08 store race and damage regressions cover materialization, missing/corrupt entries, cancellation and stale locks. `cc-rel-size-limit-eviction` is constrained by eviction grace; stats reset does not apply to event-log-derived statistics. Read-only mode remains KACH-35.
- Quoted rooted macro collision was fixed with a capture guard; keep the shared-cache cross-checkout regression. Do not repeat the already completed warm-regression bisect absent new evidence.
- The 2026-11-23 owner ruling waives performance acceptance and macOS CI; Linux CI runs in the background after local L3 verification. It supersedes older timing and CI requirements. Feature completion comes first. Include gluon Meson/Ninja launcher integration in CCP-07.

### Baseline

The initial source baseline was `4401c7a09498d2225ab22e7d1e74bef20211ca8f`. L3 was Alpine 3.25 x86_64 musl, GCC 15.2, Clang 23.1.4 and ccache 4.14.3. Autotools/libtool passed locally on Ubuntu 22.04 GCC 11.4 and Clang 14; macOS and musl cells remain unverified. The last formal L1 performance rerun at `ee2402bf` failed all six workloads; no formal result includes the later connection pool. M1 timing was ineligible and L2 and newer L1 timings were missing. These historical performance results do not establish current performance. [Reports](../campaigns/cpp/reports/) retain dated evidence.

### Acceptance

CCP-01 is the differential and transition runner: `make cc-oracle`, `make cc-oracle CASE=<id>`, `make cc-oracle-full`, and `make cc-oracle-negative-control`. Each arm has isolated state and identical fixture paths. Compare raw, ccache and kache status, ordered diagnostics, output inventory, bytes, dependencies and sidecars. Prove actual hits and avoided code generation after output deletion; exercise semantic edit, revert, irrelevant edit, relocation and policy/toolchain changes as applicable. Negative controls must detect stale results, missing sidecars, disabled invalidation and disabled caching. Report wrong output, missed reuse, reference defects, nondeterminism and unavailable tools separately.

Use transparent (`KACHE_CC_PATH_NORMALIZE=0`, `KACHE_CC_SOURCE_DATE_EPOCH=passthrough`) and reproducible profiles with equivalent effective flags. Never enable ccache sloppiness globally; PCH and limited implicit modules need declared assumptions and violation tests. Raw behavior wins on ccache defects. Establish raw-versus-raw stability before interpreting byte differences.

For final correctness, classify every applicable pinned behavior, run its transitions in both profiles without unresolved stale hits, demonstrate required safe reuse, validate real projects and local fault controls, and document unsupported cases. Run `make check`, applicable `make e2e`, `make e2e-negative-control`, oracle controls and affected tests under the governing verification rules. Preserve Rust coverage. Required correctness matrix evidence covers M1 Apple and standalone LLVM Clang, L1/L2 glibc and L3/L4 musl with GCC and Clang, current and prior supported compiler versions; unrun cells are not passes. The owner's CI and performance waiver above applies to completion gates, not to claims of measured coverage.

### Needs environment

Native L3 is provisioned for correctness. Standalone LLVM Clang on M1 was unavailable; retry that cell when installed. Autotools macOS/musl, Chromium and Meson launcher cells need their provisioned hosts and pinned project inputs. Native ARM64 and M1 evidence cannot be inferred from cross-compilation. Timing, if later reinstated, needs quiet native hosts and the locked paired protocol; the earlier loaded M1 run was ineligible.

### Execution boundary and non-goals

Prioritize local C/C++ compatibility: GCC, Clang, Apple Clang, C/C++, existing Objective-C, Objective-C++, preprocessed assembly and available clang-cl; preserve correct fallback for unsupported shapes. Native Windows/MSVC, CUDA/HIP, distributed execution, general C++ module production and remote storage are outside this campaign. PCH and explicit module consumers remain in scope. Use isolated worktrees and disposable stores. Follow current repository verification and PR rules.

### Reference

Pinned [ccache 4.14.3](https://ccache.dev/manual/4.14.3.html), tag `v4.14.3`, commit `16464deae5188f8ef333cc693c58b63ee884e43e`; identities, matrix and project pins are in [lock.toml](../campaigns/cpp/lock.toml), cases in [cases.toml](../campaigns/cpp/cases.toml), and fixtures in [oracle](../campaigns/cpp/oracle/). Reference updates require an explicit baseline change and differential rerun. Use ccache's pinned upstream suites to validate its installation, and write original behavioral adapters rather than copying implementation text.

## Later

- Remote storage: after local parity, decide whether to pursue versioned artifacts, backend selection, verified transfers, authentication, retention and independent operational acceptance.
- Firefox cross-checkout: productionize and upstream the cache-agnostic patches; address vendored checksums, system-NSS, bare-Cargo and Windows support, and consider the `remote_settings` residual (#167).
- Firefox: consider a config-file `path_only_env_vars` setting and dedicated test, retaining the include-only safety gate.
- KACH-22, KACH-24, KACH-25, KACH-35 and DSLP tickets: see [TODO.md](../TODO.md) and [TODO-DESLOP.md](../TODO-DESLOP.md).
