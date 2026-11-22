# Warm-path diagnostic checkpoint, 2026-11-22

Measured endpoints: `da350481a58373829051df9b6eff35400e0ba9a9` (A) and
`c4d2b947f3cd1412decfed602a7ac8ded770df88` (B).
Current-main measurements: `9553a432f80b8186bd89ed9443ee9c91f29ef16c`.
Each step fetched origin first. Main remained at that revision throughout.

**These are diagnostics, not formal acceptance. Acceptance rules are unchanged.**
This checkpoint neither replaces the historical FAIL verdicts nor establishes passing
L1 performance. M1 acceptance, newer L1 compiler cells, native ARM64 L2 acceptance,
and LLVM subset performance acceptance remain open. CI correctness is separate evidence.
No timestamp, prefix-map, language-option, hidden-input or race safeguard was removed.

## Findings

1. Both historical endpoints selected overlong sockets. The large warm regression
   reproduces in standalone fallback, not at comparable magnitude with verified daemons.
2. `438c5f04` introduces the large standalone regression through repeated timestamp
   scanning and allocation over the complete memo read set.
3. Main still has a standalone penalty. Its measured hotspot has moved to loading
   and deserializing persisted header analyses into a process-local cache.
4. Main's native Clang tiny-C cold loss is smaller but measurable. Serial per-invocation
   bookkeeping contributes approximately 210 ms beyond ccache in separate profiles.
5. **The historical GCC header-edit excess remains unassigned.** Its current paired
   interval includes both zero and the historical +675.8 ms. The profiles do not support
   attributing that amount to preprocessing count, daemon queueing or permit waits.

## Host and shared controls

Native Ubuntu 22.04 x86_64 cloud VM, glibc 2.35, kernel `6.8.0-1060-aws`, eight CPUs.
Fixed compilers: `/usr/bin/x86_64-linux-gnu-g++-11` (GCC 11.4) and
`/usr/lib/llvm-14/bin/clang` (Clang 14.0.0). The reference is pinned ccache 4.14.3.
[Provenance](provenance.json) records compiler, frontend, engine and fixture digests.
Compiler, ccache and input BLAKE3 digests match the historical reports.

Each revision has its own release worktree, target, cache and configuration.
Native samples use explicit short 0700 runtimes. Startup checks require exit zero,
live status and a PID whose executable path and SHA-256 match that revision's engine.
No native sample may silently use standalone execution. Every owned daemon was stopped
and observed exited. Every measured stage checks object bytes against a raw compiler
replay. Warm stages require twelve hits and zero compiler, preprocessor and probe runs.

Quiet preflights precede timing blocks. No builds run concurrently with timing.
[Retained preflights](quiet-preflights.json) record the host checks, not formal
per-replay acceptance eligibility. Step 1 used 0.008–0.030 active cores.
Step 2 used 0.022–0.190. Step 3's retained final primary blocks used 0.020 and 0.006.
Step 3 overwrote its earlier preflight JSON when extending five pairs to ten.
The earlier successful gate checks are not independently recoverable from this package.

Timings exclude object hashing, event-log quiescence and daemon management.
Profiling is separate from the uninstrumented paired observations. No complete successful
timing sample was excluded. Bootstrap intervals use 50,000 paired percentile resamples.
Steps 1–2 use seed 20261117. Step 3 uses seed 20261122.

## Step 1: endpoint execution route explains where the regression reproduces

The historical reports are [A](../2026-11-21-l1-da350481/README.md) and
[B](../2026-11-21-l1-c4d2b947/README.md). Their declared default socket lengths are:

| Endpoint | GCC socket bytes | Clang socket bytes |
|---|---:|---:|
| A | 117 | 119 |
| B | 123 | 125 |

All exceed Linux's 107-byte limit. Neither report declares a runtime or socket override.
Both revisions' `cc_bench::replay` checks whether daemon start launched, not its exit
status. Config defaults runtime to cache, and failed handoff permits standalone fallback.
[Explicit reproductions](historical-sockets.json) fail at all four lengths with exit 1.
The historical reports contain no process traces, so route attribution uses their
declared configuration, the reproduced failures and the fallback dispatch code.

The fixed reduced driver measures twelve C++ TUs with `make -j4`. It prepares a fresh
cache with an untimed cold build, deletes objects, then measures first and second warm
builds. It retains 300 ms event quiescence outside timing. It omits unrelated edit and
relocation scenarios and the full runner's generic 1.1-second make-timestamp tick.
This is not a substitute acceptance driver.

Each row below has ten alternating A/B pairs. Values are first-warm medians and 95% CIs.
Ratios and deltas use marginal medians with paired resampling, not a median of pair ratios.

| Mode | Compiler | A ms [CI] | B ms [CI] | B/A [CI] |
|---|---|---:|---:|---:|
| Standalone fallback | GCC | 22.14 [21.19, 29.78] | 54.85 [50.85, 65.14] | 2.478 [1.940, 2.747] |
| Standalone fallback | Clang | 22.76 [21.86, 27.74] | 53.44 [46.44, 59.42] | 2.348 [1.859, 2.649] |
| Working daemon | GCC | 21.67 [20.86, 26.33] | 22.24 [20.15, 23.60] | 1.026 [0.851, 1.067] |
| Working daemon | Clang | 23.25 [21.64, 25.36] | 23.36 [21.97, 27.64] | 1.005 [0.928, 1.179] |

Native B minus A is +0.57 ms [−3.89, +1.45] on GCC and +0.11 ms [−1.71, +4.21] on
Clang. These intervals do not prove equivalence or exclude smaller regressions.
Second-warm results have no comparable native regression. Ccache controls remain
approximately 15–17 ms. All 160 measured warm builds satisfy the hit and object checks.

All 40 native instances passed startup, executable identity and shutdown checks.
Native sockets are 68 bytes for GCC and 70 for Clang. Eight separate twelve-hit traces
cover every endpoint, compiler and route combination. Each fallback trace has twelve
distinct engine PIDs. Each native trace has only its verified daemon PID:

| Compiler | A daemon PID | B daemon PID |
|---|---:|---:|
| GCC | 3032568 | 3032876 |
| Clang | 3042486 | 3042801 |

Both endpoints were already affected. This is not an A-native to B-standalone transition.
The execution route exposes the standalone regression but does not identify its commit.

## Step 2: first slower standalone revision and introducing cost

The bounded GCC bisect varies only kache and its matching engine. Each probe alternates
against the latest good lower bound. Every timed build has a separate identity build
with twelve distinct engine PIDs. The interval contains twenty commits after A through B.

| Previous good | Probe | Pairs | Good median ms [CI] | Probe median ms [CI] |
|---|---|---:|---:|---:|
| `da350481` | `7829cedd` | 5 | 25.72 [22.30, 26.75] | 78.14 [72.65, 82.10] |
| `da350481` | `934bba05` | 5 | 28.12 [25.89, 35.09] | 27.35 [23.16, 36.01] |
| `934bba05` | `92cb3b3e` | 5 | 28.00 [26.31, 54.49] | 21.93 [21.66, 33.28] |
| `92cb3b3e` | `438c5f04` | 10 | 27.70 [25.82, 30.48] | 84.06 [74.77, 93.99] |

First slower: **`438c5f04080fc8427eb47bb08c873f2763461eb8`,
`fix(cc): key timestamp expansions by input mtimes`**.
Its immediate parent is `92cb3b3e27ad871bc8aab2866b3cd0cd0555be0e`.
Child minus parent: **+56.36 ms [47.10, 66.32]**.
Child/parent: **3.035 [2.681, 3.453]**.
Good probes exclude the large endpoint effect, not every smaller possible change.
The boundary is compiler-independent timestamp code, with no suspicious GCC-only result
requiring a second compiler bisect.

At the slower revision, `src/compiler/cc.rs:9520–9528` supplies the hit's memo read set.
The converged key path calls `cc_timestamp_key` at :9602–9606.
The scanner at :8150–8172 rereads every input, allocates a filtered Vec, searches for
`__TIMESTAMP__` and checks metadata. Each TU has 275 inputs. Twelve invocations cause
47.3 MiB of logical rereading, mostly shared headers. This is not physical disk traffic.

Separate phase traces cover 120 hits per revision. Values below are means per hit,
not four-job wall allocations. Key includes probe and memo lookup.

| Phase, ms/hit | Parent | First slower |
|---|---:|---:|
| Recorded startup, whole-ms quantization | 0.03 | 0.06 |
| Probe lookup | 0.103 | 0.053 |
| Memo lookup | 1.461 | 1.330 |
| **Key, inclusive** | **2.227** | **19.260** |
| Restore | 0.451 | 0.367 |
| Entire backend | 4.454 | 21.340 |

Thirty separately CPU-profiled builds per revision yield 360 verified invocation PIDs.
The slower revision has **4,351/8,298 samples (52.43%)** in the scanner's filtered-Vec
construction. The parent has zero. SQLite sample counts are similar, 178 versus 171,
and memo lookup does not grow. No compiler, preprocessor or probe reruns occur.
These checks locate the added approximately 17 ms/hit in timestamp scanning and
allocation, not memo waiting, startup, restore or daemon serialization.
The timestamp correctness repair must remain intact.

### Main retains a different standalone cost

Five alternating mode pairs on main use the same workload and GCC binary:

| Mode | Median ms [95% CI] |
|---|---:|
| Standalone | 65.57 [57.50, 79.01] |
| Working daemon | 24.10 [20.84, 26.39] |

Standalone excess: **+41.47 ms [31.11, 58.17]**.
Standalone/daemon: **2.721 [2.179, 3.791]**.
Main rejects overlong explicit endpoints during configuration. Its standalone control
instead uses an uncreatable `/proc/self/...sock` endpoint. Fallback warning writes are
included. This within-main mode comparison is not evidence of another historical regression.

Commit `60d8ecdd` replaced raw timestamp scanning with verified persisted analyses.
Main's converged timestamp-key path at `src/compiler/cc.rs:10078` still obtains full
header analyses through :8589–8612. The parsed cache at :7218–7252 is process-local.
Each standalone engine starts empty and loads and deserializes persisted analyses at
:7257–7325, especially :7293–7299. A daemon reuses the parsed objects.

Main's separately traced key mean is **11.413 ms/hit standalone versus 2.193 native**.
Memo validation is similar, 1.315 versus 1.342. Restore is not slower standalone.
A separate DWARF profile contains **775/2,511 samples (30.86%)** under
`cc_parsed_header_scans`. **573/2,511 (22.82%)** include both header-analysis and
deserializer frames. Those categories overlap. The old allocating scanner has zero
samples. This corroborates the current loading path rather than assuming it from code.
The retained cache has 286 analyses, 581,213 serialized bytes, all timestamp-insensitive.

## Step 3: native-daemon attribution of the two L1 absolute losses

Tiny-C has 48 units, `-O2`, and **one job**, as forced by `workload.rs`.
C++ has twelve units, `-std=c++17 -O1`, and four jobs.
The reduced C++ replay follows cold, warm, settled source edit, settled source revert,
then settled header edit. The header appends `extern int kache_bench_edit_settled;`.
It preserves the runner's 1.1-second make tick and 2.5-second settling waits outside
timing. It omits the resource-meter process and unrelated later scenarios.

Ten alternating kache/ccache pairs per target retain every observation.
Here, excess is the **median of paired differences**, unlike the Step 1–2 estimator.
It need not equal the difference between the two marginal medians.

| Scenario | Kache median ms [CI] | ccache median ms [CI] | Paired excess ms [CI] |
|---|---:|---:|---:|
| Clang 14 tiny-C cold | 2754.61 [2394.92, 2997.19] | 2475.82 [2295.14, 2625.57] | **+332.48 [62.89, 430.10]** |
| GCC 11 settled header edit | 5697.83 [5378.47, 6191.29] | 5359.96 [5162.40, 6279.72] | **+138.60 [−632.25, 904.14]** |

Current mean paired excesses are +287.23 ms and +25.75 ms. The historical means are
+724.8 ms and +675.8 ms. Revision and execution route differ, so these changes cannot
be attributed to daemon routing alone. The GCC interval neither proves parity nor
excludes the old excess.

Twenty primary daemons and four profile daemons use 64-byte or 67-byte sockets.
**Daemon startup and readiness are outside cold timing**, taking roughly 97–157 ms.
The first compiler request, broker setup and compiler calibration remain inside cold.
Status checks do not warm compiler records.

### Clang: serial cache and broker overhead, not extra content preprocessors

Both tools execute 48 compiles and 48 content preprocessors.
Kache additionally executes one epoch-test preprocessor and two driver queries.
The event counters group the two queries as one probe and exclude the epoch test from
the content-preprocessor counter. Clang 14's recorded epoch-support verdict is `no`.
A direct check with SOURCE_DATE_EPOCH=946684800 prints the current date on Clang 14,
while GCC prints January 1, 2000.

`cc_direct_key_eligible` at `src/compiler/cc.rs:8664–8670` requires epoch support,
so this compiler takes the preprocessing path. The verified time-macro-free proof
at :3244–3291 avoids a second sentinel expansion. There are not 96 content preprocessors.
Since ccache also runs 48, process count alone does not explain the relative excess.

Two separate profiles find **210–211 ms** more non-child invocation work in kache,
approximately **4.4 ms per serial TU**. The first profile partitions the serial chain:

| Component, ms | Kache | ccache |
|---|---:|---:|
| Compiler processes | 1365.32 | 1310.41 |
| Preprocessors, including kache epoch test | 888.48 | 919.61 |
| Driver-query processes | 37.04 | 0 |
| Other invocation work | 425.03 | 214.00 |
| Make gaps and tail | 42.66 | 45.96 |

Kache's other work includes 114 ms frontend/IPC, 113 ms key analysis outside children,
74 ms compiler-output/broker completion work, and 87 ms store opening, memo publication
and storing. These are components of kache's budget, not individually isolated net
differences against ccache's internal bookkeeping. Permit and flight phases total
approximately 4 ms. Restore is zero. SQLite and broker stacks corroborate distributed
bookkeeping cost, not one function explaining the entire gap.

### GCC: compiler lifetimes dominate the completion chain

Both tools execute **twelve preprocessors and twelve compiles**, with no driver queries
in the target scenario. Kache's twelve `dup` results are compiles whose output blobs
already exist, not hits. Ccache's five-stage replay has 25 misses and 13 hits.
The invalidated existing memo takes the intentional preprocessing fallback at
`src/compiler/cc.rs:9993–10020`. GCC's epoch-support verdict is `ok`.

The analysis backtracks the observed four-job resource-release chain from the last
completion. Each chain below contains three TUs, not twelve summed overlapping units.
The disjoint budgets include make dispatch and tail and close to each profile's wall.

| Completion-chain component, ms | Kache profile 1 | ccache profile 1 | Kache profile 2 | ccache profile 2 |
|---|---:|---:|---:|---:|
| Compiler processes | **4958.59** | **5084.55** | **6306.83** | **5126.24** |
| Preprocessors | 162.84 | 160.22 | 212.22 | 155.43 |
| Key analysis outside children | 85.28 | included below | 100.82 | included below |
| Other invocation work | 31.79 | 63.91 | 37.35 | 72.91 |
| Make dispatch and tail | 5.31 | 4.56 | 4.76 | 4.44 |
| Wall | 5243.81 | 5313.23 | 6661.99 | 5359.01 |

Profile 2's +1303 ms difference contains +1181 ms inside compiler process lifetimes.
Compiler CPU totals move similarly: 24.10 versus 19.78 CPU-seconds, compared with
19.35 versus 19.62 in profile 1. The reason for compiler-speed variation is unverified.
Both profiles reach four concurrent backends and four concurrent compilers.

Preprocessing-result analysis is the largest verified kache-side analysis cost:
85–101 ms on those completion chains. Of 381 daemon CPU samples, 53.28% include
`preprocess_hash`, 18.64% the hidden-assembler directive scanner, 8.14% token walking
and 5.51% prefix mapping. Categories overlap and are not wall shares.
The standalone header-analysis loader has zero samples on this native path.
Frontend/IPC occupies 6–8 ms, permit plus flight 0.39–0.50 ms, store about 4 ms,
and memo commit 10–12 ms on the completion chain. Restore is zero.

**No verified dominant cause for the historical +675.8 ms excess follows from these
measurements.** The point estimate is noisy, ccache also preprocesses twelve units,
and neither queueing, waits nor store work accounts for hundreds of milliseconds.
Compiler-normalized or separately controlled historical-route evidence remains necessary.

## Ranked next actions

1. **Directive scanner:** investigate `memchr_iter(b'.', ...)` in
   `cc_directive_operands` at `src/compiler/cc.rs:2786–2806`.
   Keep exact case-insensitive matches, boundaries, operands, decoded-string scanning,
   computed-asm refusals and macro-directive checks. This is a measured hotspot, not
   a demonstrated 676 ms recovery. No implementation or gain is claimed here.
2. **Clang before 16:** investigate compile-first for certain cold misses whose complete
   captured inputs and effective argv are proven time-macro-free. Only Clang 14 was
   measured. Retain capability probing rather than treating a version range as proof.
   Preserve capture-race, include-resolution and hidden-input checks. If proof fails,
   deliver the compiled object without publishing a reusable entry or memo.
   Keep preprocessing for uncertain lookups and existing-memo/revert cases.
   The serial profile has approximately 0.87 seconds of content preprocessing available
   to avoid before replacement-proof costs. This is an investigated opportunity,
   not a measured optimization. Never mark unsupported epoch behavior safe globally.
3. **Standalone header-analysis loading:** reduce full-analysis loading and deserialization
   for timestamp sensitivity while retaining content-bound verdicts and invalidation.
   Preserve timestamp, prefix-map and language-option correctness repairs.
4. **LLVM acceptance:** after targeted fixes and quiet preflight, run the locked acceptance
   protocol with verified working daemons. Keep the historical 6–18 hour two-compiler
   estimate as high-uncertainty planning evidence, not a measured new runtime.
   Check scratch ceilings first. Complete the missing compiler and host cells separately.

## Evidence inventory and limits

The report is a reference checkpoint. JSON tables retain unprofiled samples, counters,
raw-object comparison results, process identities and binary provenance.
No perf.data, compiled binaries, object files or full stacks are committed.
The complete machine-local captures remain in `/home/ubuntu/warm-endpoint-ab-evidence`,
`/home/ubuntu/warm-bisect-evidence` and `/home/ubuntu/l1-loss-phase-evidence`.
Those paths are historical locations, not dependencies of the portable analysis.

| Files | Content |
|---|---|
| `endpoint-samples.json`, `endpoint-controls.json` | All endpoint timings, including second warm and cold preparation |
| `bisect-samples.json` | All bounded probes, parent comparison and main mode pairs |
| `loss-samples.json` | Both loss targets and the preceding C++ scenario history |
| `endpoint-analysis.json`, `bisect-analysis.json`, `loss-analysis.json` | Original estimates and extracted phase budgets |
| `historical-sockets.json`, `readsets.json`, `current-cpu.json`, `loss-cpu.json` | Socket failures, read sets and aggregate CPU evidence |
| `provenance.json`, `quiet-preflights.json`, `source-sha256.json` | Measurement provenance, retained quiet gates and source evidence hashes |
| `analyze.py`, `analysis-summary.json` | Portable recomputation of timing estimates and aggregate accounting checks |
| `*-analysis-source.py` | Unmodified original analysis source snapshots for auditing extraction logic |

From the repository root, run the portable analysis:

```sh
python3 campaigns/cpp/reports/2026-11-22-warm-checkpoint/analyze.py
```

It recomputes all reported timing medians and CIs using only this directory and Python's
standard library. It checks retained counter invariants, completion-chain budget closure
and CPU percentages. It does not repeat builds, revalidate old PIDs or reconstruct phase
and CPU extraction from omitted captures. The original source snapshots require their
machine-local helper modules and captures and are not standalone runnable tools.

Observer shims and perf appear only in separate profiles. Frontend/IPC is an upper bound
including startup, queueing, response handling and diagnostic overhead, not an isolated
queue-wait measurement. Step 3's CPU-profiled tiny replay briefly overlapped an analysis
job. Its wall comparison is not quiet timing evidence. The primary samples and separate
non-perf profile did not overlap that job. Non-kache counter fields in uninstrumented
loss rows are null, not measured zeros. Ccache counts come from stats and process profiles.

Summed per-unit phases are not build wall time. Nested phases and CPU categories overlap.
The observed completion-chain partitions describe the profiles, not an exact additive
allocation of the primary paired median excess. No macOS or musl measurement is added.
No formal acceptance, compiler correctness oracle or campaign rule is changed.

Packaging verification and the full `make check` tail are in
[verification.md](verification.md). Those checks validate this docs-only checkpoint,
not performance acceptance.
