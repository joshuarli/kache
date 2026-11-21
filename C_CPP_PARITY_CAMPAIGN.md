# C/C++ parity campaign

Progress lives in `campaigns/cpp/STATUS.md`. This is an execution plan, not a
compatibility claim.
Created 2026-11-17. Kache baseline:
[4401c7a](https://github.com/joshuarli/kache/commit/4401c7a09498d2225ab22e7d1e74bef20211ca8f).
Tracking entry: KACH-26 in [TODO.md](TODO.md).

## Objective and scope

Make kache a dependable local C/C++ compiler cache on macOS and Linux, with
the safe cache coverage, observable compiler behavior, and operational
reliability of the pinned ccache baseline. Use ccache as the compatibility and
reuse oracle; use the real compiler as the correctness oracle. Preserve kache's
Rust support, cross-worktree reuse, local store, and scheduling.

Completion requires the matrix and final gates below to pass on one identified
kache revision. A successful uncached compilation proves fallback correctness;
it does not close a cache coverage gap. A passing corpus establishes measured
parity within its published scope, not a proof about every possible invocation.

**Remote storage is deferred.** Do not implement remote backends, credentials,
upload/download queues, network services, or remote-cache tests in this campaign.
The later roadmap is at the end. This ordering supersedes older remote-first
suggestions in coverage notes.

In scope: GCC, Clang, Apple Clang; C/C++; existing Objective-C, Objective-C++,
preprocessed assembly, and host-available clang-cl behavior; launchers/shims;
local cache management and diagnostics. Links and unsupported invocation shapes
must retain correct fallback. Ccache does not require caching links or
multi-source compilations. Native Windows/MSVC, CUDA/HIP, distributed compilation,
and general standard C++ module production are outside this campaign. Explicit
module consumers and narrower PCH/Clang-module cases remain in scope.

## Start here

1. Read [AGENTS.md](AGENTS.md), this plan, [TODO.md](TODO.md), and
   [scenarios/README.md](scenarios/README.md). Refresh source and workflows;
   baseline observations below can become stale.
2. If campaign state does not exist, bootstrap the pins and case inventory in
   **CCP-00**, then start **CCP-01**. Complete differential baseline capture once
   the runner exists; unavailable host cells do not block harness implementation.
   Otherwise resume the first unfinished dependency-ready item in the ledger.
3. Work in an isolated worktree. Keep test caches, configurations, compiler
   instrumentation, and outputs inside campaign scratch directories.
4. Establish existing gates on a provisioned host:

   ```sh
   make check
   make e2e
   make e2e-negative-control
   ```

   Record pre-existing failures separately. Missing required tools are not
   successful baselines.
5. Continue with the unattended loop below. Proposed oracle commands in this
   plan do not exist at the baseline; CCP-01 implements them.

## Baseline observations

These are source/document observations, not newly executed differential results.

| Area | Observation | Required work |
|---|---|---|
| Compiler model | [C/C++ docs](docs/getting-started/c-cpp.mdx) and [implementation](src/compiler/cc.rs) describe single-source objects/preprocessing, classified flags, dependency memos, and refusals. | Preserve existing coverage and establish differential contracts. |
| Cached shapes | Response files, `-S`, multiple architectures, PCH/modules, coverage, and split DWARF are documented passthroughs. | Implement the ccache-supported subset with complete invalidation/restoration. |
| Harmless edits | Keys use dependency fingerprints/read-set digests. | Measure reuse after equivalent preprocessing; rediscovering dependencies does not itself prove ccache-equivalent reuse. |
| Output policy | Kache injects an epoch and path maps unless configured otherwise. | Test transparent and reproducible profiles separately. |
| CI | [ci.yml](.github/workflows/ci.yml) runs fixture e2e and negative controls on macOS; Linux musl jobs run workspace tests. | Close KACH-21 with Linux fixture/oracle coverage and assigned cross-compiler jobs. |
| Harness | [kache-e2e](crates/kache-e2e/src) has kache/sccache/mbx benchmark arms and fixed fixture phases. | Add raw/ccache arms and state transitions. |
| Gates | Fixture `checks.measure` is advisory; fixture `assertions` and benchmark `checks.assert` block. | Oracle mismatches and coverage regressions must fail the command/CI. |
| Store/engine | Publication, restoration, daemon, memoization, and concurrency machinery already exist. | Exercise C/C++ contracts; reuse the architecture. |

Source wins over stale comments/docs. The C/C++ page, for example, contains a
link-caching paragraph alongside a link passthrough rule. Resolve contradictions
when touching that behavior; dormant helpers are not evidence of support.

## Frozen reference

Use **ccache 4.14.3**, tag `v4.14.3`, commit
`16464deae5188f8ef333cc693c58b63ee884e43e`:
[versioned manual](https://ccache.dev/manual/4.14.3.html),
[pinned source](https://github.com/ccache/ccache/tree/16464deae5188f8ef333cc693c58b63ee884e43e).
Record the binary digest/build options, compiler/SDK versions, and project
revisions. Updating the reference requires an explicit baseline-change commit
and differential rerun.

Ccache normally falls back from direct lookup to preprocessor lookup; depend
mode is a separate tradeoff. PCH requires special configuration, including
`pch_defines,time_macros` sloppiness. Limited implicit Clang modules require
`modules` sloppiness. General standard C++20 modules are unsupported, while
explicit module consumers have narrower support. Preserve these
[qualifications](https://ccache.dev/manual/4.14.3.html#_precompiled_headers);
do not omit all module/PCH work or enable relaxed assumptions globally.

Inventory relevant behaviors in the pinned
[test suites](https://github.com/ccache/ccache/tree/16464deae5188f8ef333cc693c58b63ee884e43e/test/suites),
[argument-processing tests](https://github.com/ccache/ccache/blob/16464deae5188f8ef333cc693c58b63ee884e43e/unittest/test_argprocessing.cpp),
and
[argument-tokenization tests](https://github.com/ccache/ccache/blob/16464deae5188f8ef333cc693c58b63ee884e43e/unittest/test_util_args.cpp).
Map individual behaviors to original kache case IDs. One happy path does not
cover an entire family. Each behavior is required, conditional with explicit
prerequisites, compiler-inapplicable with a capability probe, or out of scope
under the rules above.

Run the inventoried in-scope upstream suites against ccache in an external pinned
checkout to validate the reference installation. Its internal counters/layout
assertions are not a kache API. Author behavioral adapters and original kache
fixtures with provenance;
do not silently copy upstream implementation/test text into this repository.

## Required platform matrix

CCP-00 locks exact OS images, compiler binaries, SDKs, and supported current/prior
compiler releases. Native execution is required for host correctness/performance
claims; cross-compilation or emulation supplies additional evidence only. A
container on a host of the same architecture counts as native.

The matrix covers the targets kache releases: `aarch64-apple-darwin`,
`x86_64-unknown-linux-musl`, and `aarch64-unknown-linux-musl` (see
[release.yml](.github/workflows/release.yml)). Every required lane has a host
that exists today:

| Lane | Host and tools | Host source | Required workload |
|---|---|---|---|
| M1 | macOS arm64; Apple Clang and standalone LLVM Clang | `xcode-27` CI runner, or any Apple Silicon Mac with Xcode 27 | Full applicable corpus; SDK/frameworks; native and arm64+x86_64 universal objects. |
| L1 | Linux x86_64 glibc; GCC and Clang | `ubuntu-26.04` CI runner | Full applicable corpus and real-build integration. |
| L2 | Linux aarch64 glibc; GCC and Clang | `ubuntu-26.04-arm` CI runner | Full applicable corpus and real-build integration. |
| L3 | Alpine/musl x86_64; GCC and Clang | Alpine container on `ubuntu-26.04`, or any x86_64 Alpine host | Native kache/engine execution, deterministic core, fixtures, release/runtime checks. |
| L4 | Alpine/musl aarch64; GCC and Clang | Alpine container on `ubuntu-26.04-arm` | Same contract as L3. |

Intel macOS is not a release target, so it is not a required lane. Add it only
together with an `x86_64-apple-darwin` release and a native runner. Each lane
runs on a single machine with no access to other hosts or private services. A
host that also runs unrelated heavy work is fine for correctness cells. Never
take timed comparisons on such a host, and record host load with every
performance result.

Test the newest supported compiler major in primary lanes and one prior major
per compiler family in pinned extended lanes. Previous Apple Clang means an
older Xcode/SDK environment, not standalone LLVM. Lock current/minimum macOS
runtime versions consistent with release claims; a deployment target alone
does not prove execution on the older OS.

Frequent gates: M1 Apple Clang and L1 GCC/Clang, plus affected critical
key/restore cases elsewhere. Scheduled/manual jobs and final acceptance cover
the full required matrix. Every named cell runs the core; large projects and
repeated benchmarks belong on primary lanes. A lane whose host becomes
unavailable stays **BLOCKED** while independent work continues; dropping a lane
requires a plan-change commit that also narrows the release claims.

Assign existing cross-GCC fixtures to a glibc job if Alpine lacks their tools.
Compiling musl objects on glibc does not prove the host binary works on musl.
Exercise clang-cl compiler-only behavior where available; a missing Windows SDK
must not disable GCC/Clang coverage.

## Differential oracle

### Isolation and policy

Each case runs raw compiler, ccache, and kache with independent cache/config
state. Recreate identical source/output state at the same canonical absolute
fixture path for each arm, saving results outside it. This prevents incidental
debug-directory, `__FILE__`, and depfile differences. Relocation is an explicit
transition.

Record original/effective argv and environment, cwd, compiler/wrapper digests,
SDK/sysroot, locale, umask, filesystem, tool versions, and cache settings.
Remove inherited cache/wrapper settings before applying declared fixture
configuration. Use dedicated kache daemon/state directories.

| Profile | Contract |
|---|---|
| Transparent | `KACHE_CC_PATH_NORMALIZE=0` and `KACHE_CC_SOURCE_DATE_EPOCH=passthrough`; preserve raw flags/environment, including user maps/epoch. |
| Reproducible | Exercise kache's documented default maps/epoch; give raw and ccache the identical effective flags/environment and verify ordering. Report separately. |

Switch profiles against populated caches to test policy keying. Do not change
defaults to satisfy the oracle or present policy-aligned results as proof of
transparent defaults.

Run ccache's normal and forced-preprocessor configurations. Test depend mode
separately with exact settings; never inherit arbitrary sloppiness. Default
correctness includes system-header changes even where ccache depend mode with
`-MMD` omits them. Conditional PCH/module profiles declare assumptions and test
violations. A ccache hit under relaxed assumptions cannot justify wrong output.

### Compare observable behavior

Compare exit status, stdout/stderr with ordering within each stream, full output
inventory, observable file types/modes, object/assembly bytes, dependencies, and
sidecars. Detect unexpected as well as missing files. Check timestamps through
build-system behavior, not equality of sequential wall-clock times.

Compare deterministic output bytes first. Establish raw-versus-raw stability.
Any structured comparator needs a case-specific explanation and preserved raw
evidence; do not globally remove paths, debug sections, symbols, diagnostics,
or timestamps. Temporal cases use controlled epochs or explicit time/miss
assertions. Unresolved nondeterminism does not pass.

Parse depfiles with correct escaping/targets and consume them through Make/Ninja.
Link/run selected programs and inspect DWARF, universal slices, coverage, and
serialized diagnostics. These supplement byte comparison. Failure cases must
preserve compiler status/diagnostics and observable partial-output semantics;
never cache a failed compile as success or report an earlier object as a
successful cache hit. Preserve a pre-existing output when that exact failed raw
invocation leaves it intact. Fallback must preserve original behavior, signals,
and output-error handling.

### Transitions and hit evidence

For every applicable family, obtain a fresh raw reference for each state:

1. Empty cache/no outputs: compile and populate.
2. Delete outputs, retain cache: restore and prove a real hit.
3. Change a semantic input: prevent stale reuse; delete outputs again and verify
   reuse of the new state.
4. Revert the edit: reuse a retained correct earlier result.
5. Change only information irrelevant to the complete observable result:
   compare ccache's reuse, including preprocessor hits.
6. Relocate outputs/tree, switch toolchain/SDK/policy, restart the daemon, and
   evict selected entries as applicable.

Warm builds must schedule compiler work after output deletion. Build-system
no-ops are separate. Prove hits with per-invocation outcomes plus independent
compiler execution observation, distinguishing preprocessing/probes from code
generation. Instrumentation must preserve compiler identity/behavior and is
disabled during performance timing.

Negative controls must detect a deliberately stale result, omitted sidecar,
disabled invalidation, and cache-disabled execution of a required-hit case.
Also test output-error and reference-arm failures. Inject faults only in
disposable stores and restore each control immediately.

### Accounting

Keep correctness and reuse as separate axes for each case/transition/profile/
compiler/host.

| Outcome | Meaning |
|---|---|
| MATCH / VALID_HIT | Raw-equivalent behavior; independently verified reuse when required. |
| KACHE_WRONG_OUTPUT | Wrong status, diagnostics, outputs, dependencies, side effects, or stale hit. |
| KACHE_MISSED_REUSE | Correct output but missed/bypassed a required safe ccache reuse opportunity. |
| CCACHE_WRONG_OUTPUT | Reproducible disagreement with raw. Preserve a regression; raw wins. |
| COMPILER_NONDETERMINISM / HARNESS_ERROR | Unresolved comparison; fix the fixture/harness. |
| UNSUPPORTED_BY_COMPILER | Demonstrated capability absence with an explicit matrix disposition. |
| BLOCKED / NOT_RUN | Missing host/tool, infrastructure failure, timeout, or unexecuted work. |

Report invocation counts and uncached compiler time represented by eligible
hits/misses. Exclude links/probes from compile hit-rate denominators but retain
their correctness tests. Never improve a percentage by removing hard cases,
broadening allowlists, or counting passthrough as cached coverage.

## Ordered work packages

Execute in dependency order. New wrong-output failures outrank optimization and
feature expansion. Each package may land as several focused changes.

### CCP-00 — Lock inventory and establish both host families

Dependencies: none.

Create `campaigns/cpp/lock.toml`, `campaigns/cpp/cases.toml`, and
`campaigns/cpp/STATUS.md`. Lock reference/toolchain identities, matrix, case
families, project inputs, and budgets. Capture current behavior without changing
compiler flags to improve the baseline.

Provision dependencies through existing CI/container setup. Close KACH-21 by
assigning every gate fixture to a real macOS/Linux job; missing required tools
must fail. Keep `--deny-missing-tools`. Record unavailable hosts as specific
blockers instead of repeatedly attempting provisioning.

Bootstrap exit: pins and the initial in-scope inventory are sufficient to start
CCP-01; required cells have jobs or explicit blockers. Complete the inventory
and reproducible differential baseline after the runner exists. Blocked cells
do not prevent harness/core work and do not satisfy final acceptance.

### CCP-01 — Differential and transition runner

Dependencies: CCP-00 bootstrap inventory, not a completed differential baseline.

Extend `crates/kache-e2e` with raw/ccache adapters and a focused transition
driver sharing existing preparation, isolation, reporting, and artifact capture.
Keep deterministic clock/race hooks in process/integration tests where fixture
phases cannot express them. Avoid a second cache or general workflow engine.

Implement these proposed interfaces:

| Command | Contract |
|---|---|
| `make cc-oracle` | Current blocking cohort on the local required core matrix; strict tools; nonzero on contract failure. |
| `make cc-oracle CASE=<id>` | Replay a case and its transitions. |
| `make cc-oracle-full` | All locked applicable cases for the selected host/toolchain. |
| `make cc-oracle-negative-control` | Prove intentional faults are detected. |
| `make cc-bench` | Locked raw/ccache/kache performance protocol. |

Write versioned JSON and concise Markdown under ignored
`.work/tmp/cc-oracle/<run-id>/`: commands, inputs, output digests, per-transition
verdicts/reasons, compiler executions, duration, and reproducers. Zero selected
cases, dropped result rows, and failed reference runs cannot produce success.

Exit: minimal C/C++ transitions pass on macOS and Linux; each negative control
fails the intended assertion; a deliberate kache coverage gap is reported as
missed reuse instead of completed parity.

### CCP-02 — Input identity and invalidation

Dependencies: CCP-01. Reuse these regressions for every later feature.

Cover source/header changes, same-size edits with restored mtime, atomic file
replacement, symlink retargeting, header appearance/deletion/shadowing, include
search order, `__has_include[_next]`, macro-generated probes, and generated
headers. Macro-generated probes include header names built by token pasting,
which Chromium's headers use. Today kache refuses every such compile with
`cc: token pasting may construct a header availability probe`. Resolve those
probes against the read set, or key their results, so they hit without losing
correct invalidation when a probed header appears or disappears. Exercise `-I`, `-iquote`, `-isystem`, `-idirafter`, forced includes,
sysroots/frameworks, and include-path environment variables.

Test same-path compiler replacement, wrappers selecting another compiler, driver
mode, specs/config files, plugins, sanitizer ignorelists, VFS overlays, profile
inputs, and native CPU/target features. A flag naming a file needs content and
selection tracking, not merely its spelling. Test option order and user key
configuration.

Cover `__FILE__`, `__BASE_FILE__`, `__DATE__`, `__TIME__`, `__TIMESTAMP__`,
SDK changes, diagnostic/locale settings, and policy switches. Use deterministic
barriers for edits between hash, compile, and publication. Validate subsequent
quiescent states instead of treating a racing raw compile as deterministic truth.
Unmodeled assembler reads remain conservative refusals.

Exit: no stale output across required input/tool/policy transitions; regressions
cover real process paths as well as key derivation. Assign safe-hit gaps to later
packages explicitly.

### CCP-03 — Response files, flags, and invocation coverage

Dependencies: CCP-01 and applicable CCP-02 guards.

Implement compiler-dialect-aware response parsing/recursive expansion: quoting,
escaping, encoding, relative paths, option boundaries, and bounded cycles/depth.
Test nested/empty/missing files, spaces/backslashes, changed contents, and
`@foo` as an ordinary option value. Whitespace splitting is insufficient.

Expand the declarative flag model from observed ccache-cacheable invocations:
`-x`, `--`, joined/separate/repeated options, `-Wp,`, `-Xpreprocessor`,
`-Xclang`, dependency options, input charsets, optimization/debug, sanitizers,
and LTO compile modes. Add named-file assembly output (`-S`) and preprocessed
inputs where safely cached by the reference. Preserve kache's existing `-E`
behavior even where ccache does not cache it.

For each argument family identify input, output, diagnostic, host-dependence,
and key effects. Never broadly accept unknown flags or sort ordered arguments.

Start from branch `feat/cc-chromium-official-flags`. It models the 28 flags an
official Chromium macOS arm64 compile uses (clang 24, ThinLTO, PGO) as general
rules, with unit tests that fail on the baseline:

- the LTO compile modes `-flto=thin`, `-fsplit-lto-unit` and `-fwhole-program-vtables`;
- `-mllvm` pairs and the `-fsanitize*` family;
- `-fmodule-name` and `-mmacos-version-min`;
- `-fprofile-use` and `--warning-suppression-mappings`, keyed by file content;
- `-fcrash-diagnostics-dir`, which has no effect on outputs.

Its final edit is unverified and it has not passed `make check`. Rebase it,
verify it, and land it through this package.

Exit: required cases populate/restore; malformed/unsupported invocations match
raw; response/input changes invalidate correctly.

### CCP-04 — Equivalent-preprocessing reuse

Dependencies: CCP-02; CCP-03 for response-driven cases.

Measure comment-only changes, unused macros/options, and inactive conditional
edits against ccache. Select cases with unchanged complete observable results:
debug lines, warnings, dependencies, and macro expansions can make apparently
harmless edits significant.

If read-set digests force unnecessary compilation, add a secondary
preprocessed-result lookup or equivalent safe mechanism. Include code-generation
and diagnostic inputs; refresh dependency manifests even when reusing object
bytes. Preserve the unchanged-input memo fast path and valid compile-first
miss path. Measure rediscovery and compilation separately.

Exit: every locked safe ccache preprocessor-reuse case avoids kache code
generation with correct outputs; semantic edits still invalidate. Measure both
the recovered path and the original warm path.

### CCP-05 — Complete compiler output sets

Dependencies: CCP-02 and relevant CCP-03 parsing.

Implement coverage outputs such as `.gcno`, split DWARF `.dwo`,
serialized/structured diagnostics, and profile-guided compile inputs. Inventory
other reference-suite outputs, including dependency and compilation-database
fragments where supported. Runtime `.gcda`/`.profraw` are not compile outputs
to replay.

Model implicit/explicit naming, directories, absent sidecars, compiler
differences, and changed output paths. Validate the full restored set before
reporting a successful hit; do not report partial restoration as success.
Writable C/C++ outputs must not alias immutable cache data or other builds.

Exit: deleting all outputs then hitting restores the correct inventory; one
missing/corrupt blob triggers safe recovery; build/debug/coverage consumers
accept results; output failures preserve the defined compiler contract.

### CCP-06 — PCH and module consumers

Dependencies: CCP-02, CCP-03, CCP-05.

Cover GCC/Clang PCH creation/use where supported by the reference: lookup,
contents, macros/options, compiler identity, missing PCH, fallback headers,
timestamps, and diagnostics. Establish the exact assumptions before enabling
cached behavior.

Cover explicit `-fmodule-file=` and `-fprebuilt-module-path=` consumers,
including template-body edits, added/renamed/removed module files, directory
contents, compiler/SDK changes, and absent/unreadable inputs. Do not exclude
these consumers under a blanket “C++20 modules unsupported” label.

Implicit Clang modules are a separate conditional profile: module maps,
transitive inputs, module-cache state, and declared assumptions all need tests.
Never enable ccache-style sloppiness globally or silently weaken defaults.
Where general safe support is unavailable, document conditional scope and
correctly refuse outside it; unexplained refusals within the selected
ccache-cacheable profile remain open.

Exit: verified conditional hits/invalidation, documented limits, and correct
defaults. General standard module production remains separate.

### CCP-07 — Host integration and real builds

Dependencies: CCP-03 through CCP-06 as exercised by each project.

Implement macOS universal objects for supported multiple `-arch` and
`-Xarch_*` arguments. Model per-slice inputs/options and inspect every restored
slice. Test frameworks, SDK selection, deployment targets, debug paths, and
available Objective-C/Objective-C++ modes.

Exercise Linux glibc/musl native host execution, ARM64, timestamp granularity,
symlinks, copy/clone fallbacks, and cross-GCC fixtures. Use paths with spaces and
non-ASCII names on both host families.

Pin bounded integrations: zlib (C), a fixed LLVM/Clang target subset (C++),
a small mixed Rust/C/C++ `cc`-crate fixture, and a fixed Chromium target subset
built through `cc_wrapper` on M1. Use `//base` and one Blink target from
Google's official source archive for a pinned Chromium release, with frozen GN
args for an official macOS arm64 build (`is_official_build=true`,
`use_lld=true`, `use_thin_lto=true`, `chrome_pgo_phase=2` with the pinned
`mac-arm` profile). Record the archive digest and args in
`campaigns/cpp/lock.toml`. Exercise CMake/Ninja and Make,
plus small Meson/Autotools launcher checks. Freeze project revisions, target
lists, and configurations before measuring. Existing Firefox scenarios are
extended coverage, not the only route to completion.

Run cold, output-cleaned warm, edit/revert, and relocation phases. Validate
project tests or selected linked programs and collect actual compile eligibility.
Turn newly exposed flag gaps into focused regressions.

Exit: required project/lane cells build/validate; no required safe
ccache-cacheable invocation is silently bypassed.

### CCP-08 — Local reliability, diagnosis, and CI

Dependencies: CCP-01; build coverage alongside CCP-02 through CCP-07.

Test corrupt/truncated/missing entries, full/read-only cache, interrupted
publication, failed output materialization, stale locks, identical/different-key
concurrency, daemon crashes/restarts, cancellation/signals, and GC races.
Use disposable stores. Assert compiler outcomes and subsequent healthy reuse,
not only absence of crashes. Do not rerun compilation after an execution-commit
boundary where doing so would duplicate compiler side effects.

Exercise local size limits/eviction, cleanup, stats reset, disabled/read-only
modes where offered. Keep writable outputs independent of cached content.
Shared engine/store changes must preserve Rust behavior through existing gates.

Expose stable hit/miss/bypass/recovery reasons in existing diagnostic interfaces;
give each refusal a named test. Fix stale docs and wire blocking checks into
actual fork workflows, not references to missing upstream jobs.

Address KACH-22 with pinned changed-line mutation checks on relevant Rust
changes, especially key/restore/publication logic. Add discriminating tests or
remove dead branches for survivors; a mutation percentage is not proof of
correctness.

Exit: local fault/control suites pass on both OS families; required cases have
real gate assignments; evidence identifies the first actionable discrepancy.

### CCP-09 — Performance and final acceptance

Dependencies: CCP-02 through CCP-08.

Use release binaries, fixed local-storage budgets, equal jobs/resource limits,
identical project targets, and equivalent effective compiler flags. Downloads,
setup, and dependency fetching happen outside timing. “Cold” means empty compiler
cache; report filesystem-cache state without privileged page-cache flushing.
Separate daemon startup from steady state. A correctness failure invalidates the
performance observation.

Measure cold misses, warm restores, unchanged-input lookup, preprocessor reuse,
edit/revert, relocation, and concurrent builds. Use a fixed representative
translation-unit corpus plus bounded C and C++ project builds. Include tiny C
units and header-heavy C++; publish per-unit milliseconds so large builds cannot
hide startup overhead. Keep LLVM/Firefox integrations bounded; do not require
ten full browser/toolchain builds on every matrix cell. Do not pad workloads or
substitute build-system no-ops.

Randomize paired raw/ccache/kache ordering. Start with five valid pairs; increase
to at most ten only to resolve a concrete uncertainty. Record wall time, CPU,
peak memory, cache/artifact bytes, compiler executions, and time-weighted reuse.
Retain all valid samples; exclude only documented preparation/measurement
failures by rules common to all arms.

For each timed configuration/workload, first run an untimed instrumented
validation replay proving compiler activity and reuse. Timed runs use the same
validated command inventory, input/configuration digests, and output checks,
with instrumentation disabled and cache counters retained. A workload mismatch
invalidates the sample; do not time the observer's overhead or drop independent
hit evidence.

Initial engineering targets, frozen before implementation measurements:

| Measurement | Blocking target |
|---|---|
| Warm same-tree aggregate | kache/ccache <= 1.15 |
| Warm relocated aggregate, aligned reproducible profile | kache/ccache <= 1.15 |
| Cold aggregate | kache/ccache <= 1.10 |
| Representative compile-heavy cold builds | kache/raw <= 1.15 |
| Edit/revert and equivalent-preprocessing aggregates | kache/ccache <= 1.15 |
| Required safe-hit cases | Correct complete restoration and verified avoided code generation |

Apply targets per named project/corpus and primary performance lane, not just
a favorable global average. Lock the estimator, uncertainty calculation, sample
admission rules, and deterministic analysis seed in CCP-00; use paired ratios
and report uncertainty plus absolute times. Targets are engineering objectives,
not measured claims. Tiny single-unit latency is additionally reported in
absolute milliseconds and profiled separately.

Performance acceptance runs natively on M1 Apple Clang, L1 GCC/Clang, and L2
ARM64; other lanes retain correctness and startup regression checks. If the
interval crosses a threshold after the sample cap, mark **INCONCLUSIVE**;
a clear miss is **FAIL**. Profile/fix the bottleneck or retain the open criterion
and continue other work. Rerun only after a relevant change or resolved
measurement problem. Threshold changes require a justified plan-change commit,
never automatic relaxation.

## Unattended execution and durable progress

Use `campaigns/cpp/STATUS.md` as the short progress ledger. Track package/subtask,
state, dependencies, owner/worktree, code SHA, case/lane IDs, evidence
location/digest, next command, and concrete blocker/retry condition. States:
TODO, RUNNING, BLOCKED, FAIL, INCONCLUSIVE, DONE. All CCP items initially are
TODO; no proposed target or unexecuted baseline is green.

Adopt blocking tests monotonically. The full inventory always reports every
gap, and `cc-oracle-full` returns nonzero while required gaps remain. Frequent
integration gates start with the frozen verified cohort plus cases for the
feature being changed. Promote newly fixed cases to that cohort in the same
change; never remove a passing case or relax it to hide a regression. Known
future-feature failures remain explicit open work rather than preventing all
incremental fixes from landing. Wrong-output findings always block campaign
acceptance/release; final acceptance requires the entire required inventory.
Do not enable a newly modeled feature while its known wrong-output cases remain.

For each iteration:

1. Refresh main, ledger, failing evidence, and current diff. Reproduce inherited
   failures before accepting their diagnosis.
2. Select a dependency-ready discrepancy: wrong output first, missing safe
   coverage next, then measured performance and documentation.
3. Save a minimal reproducer with exact toolchain/configuration and seed.
   Add a regression that fails for that behavior.
4. Make the smallest coherent fix. Assign one owner per overlapping code area;
   independent agents may prepare fixtures or investigate other lanes.
   Integrate shared changes sequentially.
5. Run focused regression and opposite/invalidation cases, applicable oracle
   cells, and repository gates. Follow AGENTS.md for code changes/PRs; target
   main and rebase independent work after integration.
6. Commit verified slices with regressions and concise ledger updates. Preserve
   branch protections and unrelated WIP; do not force-push shared history or
   weaken assertions. Continue without asking about routine design choices.
7. On interruption/handoff, persist the exact next case/command. Invalidate
   affected evidence after code changes; final acceptance uses the integrated
   revision.

Use existing authorized runners/access. A missing host, compiler, permission, or
required external service blocks only affected cells. After one diagnostic
attempt and one justified transient retry, record evidence/retry conditions and
work on another ready task. Do not spin on infrastructure or relabel BLOCKED as
SKIP. Follow existing repository authorization for integration; this plan does
not bypass access controls or require a new release.

Lock per-case timeouts, job limits, memory/disk ceilings, and run budgets.
Initial exploration budget: 200 seeded transition sequences per primary
compiler lane, maximum 30 minutes per lane per revision. Save failures
immediately; minimize for up to 10 minutes and retain the original reproducer
if unfinished. Replay the finite saved corpus in gates. Expand exploration
only for a concrete coverage gap or changed feature.

Keep complete logs/artifacts in CI artifacts or campaign scratch, with IDs and
digests in the ledger. Commit small reproducers/manifests, not build outputs or
recurring large logs. Promotion to DONE requires a committed small case/manifest
and a retrievable report/artifact in an existing durable evidence location;
scratch-only results remain provisional. Record retention/expiry and preserve
the compact final report in git. On resume, missing or expired evidence becomes
unverified until reproduced; a stored digest alone is not retrievable evidence.
CI evidence storage is distinct from remote compilation-cache storage.

The final report may be committed after the tested revision. Name that tested
code SHA and verify that intervening changes affect documentation only; new
code, fixtures, build inputs, or configuration require affected checks again.

Stop when final criteria pass, the user stops work, or every remaining task has
a concrete blocker. In the last case leave a precise resumable status without
claiming completion. No indefinite soaking, repeated green tests, speculative
refactoring, or remote-storage work is a completion requirement.

## Final acceptance

All criteria must hold on the same integrated kache revision:

- Every locked applicable upstream behavior has a case/disposition; no required
  lane or case is absent, unresolved, or silently skipped.
- Raw-versus-kache behavior passes all required transitions in both policy
  profiles; zero unresolved wrong-output/stale-hit failures.
- Required safe ccache reuse opportunities are implemented. Correct fallback is
  reported separately for explicitly unsupported/out-of-scope behavior.
- Conditional PCH/module results state prerequisites and test their violations.
  Known ccache defects are recorded; kache must still match raw.
- M1 and L1–L4 requirements have execution evidence. Linux fixtures and
  negative controls run; architecture/runtime claims match actual tested hosts.
- `make check`, applicable e2e, oracle controls, local fault tests, and required
  mutation checks pass. Existing Rust functionality remains covered.
- Real-project validations and frozen performance gates pass with valid
  independently observed compiler activity and bounded measurement evidence.
- User docs describe supported modes/limits; diagnostics explain refusals;
  the ledger identifies the final revision, pins, matrix, reports, and reproducers.

Publish a compact report separating correctness, cached coverage, performance,
and unsupported scope. Do not turn this result into an unbounded claim about
every historical ccache feature or arbitrary compiler version.

## Later roadmap: remote storage

After local parity, consider remote storage as a separate campaign and explicit
activation decision. It is not a completion gate above.

1. Define a versioned artifact/manifest contract around existing local
   content-addressed storage, compiler identity, and complete output sets.
   Keep local operation usable without network configuration.
2. Select one backend from demonstrated deployment needs, such as HTTP or object
   storage. Compare its operational contract with ccache's remote backends.
3. Add bounded transfers, content verification, atomic publication, namespaces,
   authentication/credential handling, read-only clients, retention, and useful
   error reporting.
4. Test slow/unavailable services, interrupted transfers, corrupt/missing blobs,
   concurrent writers, incompatible toolchains, and local fallback. Measure
   end-to-end gains including network costs on macOS and Linux.
5. Enable it only after its own correctness and operational acceptance report.

General standard C++ module production, additional host platforms/toolchains,
and distributed execution are separate extensions. They must not displace
unfinished local C/C++ parity.
