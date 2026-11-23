# Formal L1 acceptance rerun, 2026-11-23

Measured source: **`ee2402bfb20c2d018c4036bec5e53a1084f79a9a`**, clean main
after #141. Evidence branch: `devin/1795408350-l1-acceptance-rerun`.
This report changes no production code, benchmark protocol or acceptance rule.

**All six workloads are eligible, with ten complete pairs each. All six FAIL
overall performance acceptance.** Every correctness/reuse validation passed;
all complete-pair outputs equal raw. Clang 14 tiny-C **cold vs ccache passes**,
but its cold-vs-raw, warm and relocation targets do not. A passing cold cell is
not passing L1 acceptance.

Post-merge main checks on this exact source also finished **11 passed, 0 failed,
0 pending, 0 skipped**: [check record](main-ci.txt).

## Host and unchanged procedure

Same native Linux x86_64 cloud VM as [#99](../2026-11-21-l1-da350481/README.md)
and [#114](../2026-11-21-l1-c4d2b947/README.md): Ubuntu 22.04, glibc 2.35,
kernel `6.8.0-1060-aws`, eight logical Xeon CPUs, about 32 GiB RAM, ext4, no cgroup
CPU quota. GCC 11.4.0, Clang 14.0.0, pinned **ccache 4.14.3**. These older
compiler versions do not close the campaign's newest-primary-major L1 cells.

Fresh main-based worktree, release tools, pinned archives verified before
timing. About 13 GiB disk remained available. Same three workloads and jobs 4
as #99/#114; tiny C remains deliberately serial. Commands, run sequentially
for `TOOLCHAINS=gcc`, then `TOOLCHAINS=clang`:

```sh
env -u TMPDIR -u KACHE_CACHE_DIR make cc-bench TOOLCHAINS=gcc \
  CC_ORACLE_CCACHE_DIR=/home/ubuntu/repos/kache/.work/wt/cc-bench-l1-rerun/.work/tmp/ccache \
  BENCH_ARGS="--acceptance --jobs 4 --workload corpus-c-tiny --workload corpus-cxx-headers --workload zlib"
```

The ccache directory override reuses the already pinned executable, **not an
arm's cache**. Every timed arm still has its own private store. Logs:
[GCC](gcc-acceptance.txt), [Clang](clang-acceptance.txt). Both make invocations
return 2 because the locked runner returns 1 for performance failure; neither
is a correctness or eligibility failure.

Same input digests, seed 20261117, paired randomized arm ordering, untimed
instrumented validations, observer-free timed runs, retained cache counters
and output checks. The estimator remains the paired log-geomean with a 95%
Student-t interval on log ratios, **not a bootstrap interval**. The runner
starts with five admitted pairs and escalates only while an interval crosses
a target, up to ten. All six reached that existing cap. All 180 timed arm
replays are retained and admitted; no timing observation was removed or rerun.
Filesystem caches are naturally warm; no privileged page-cache flushing.
Downloads, builds, daemon startup and preparation remain outside steady-state
timing; the explicit single-unit daemon-start scenario is separately labelled.

As before, the caller unsets `TMPDIR` and `KACHE_CACHE_DIR`; the runner declares
private per-arm temp/store values inside each sandbox. All effective flags,
declared environments, host details, binary BLAKE3 digests, observations and
intervals are in the unmodified [GCC JSON](gcc/report.json) and
[Clang JSON](clang/report.json). Generated Markdown copies
([GCC](gcc/report.md), [Clang](clang/report.md)) only trim blank lines at EOF.
CPU/max-RSS are the unchanged **build-process-tree** measurements; they omit
the already-running daemon, as `resource_accounting` explicitly records.

## Quiet-host admission and native daemon verification

An external five-second preflight imports this revision's unmodified
`cc_bench/activity.rs` ([source](preflight.rs)); the runner independently repeats
its own preflight. The unrelated-CPU limit stays **0.5 cores**.

| Compiler | Standalone unrelated cores | Runner unrelated cores | Verdict |
|---|---:|---:|---|
| GCC 11 | 0.031940 | 0.013966 | ELIGIBLE |
| Clang 14 | 0.025950 | 0.033933 | ELIGIBLE |

The first external GCC preflight was busy, **0.704695 cores**, and launched
**no benchmark**. It chiefly observed `devin-remote` (1,570 ms CPU).
Twenty minutes later the second attempt passed. Both attempts are retained:
[initial refusal](preflight-gcc-attempt-01.txt), [admitted GCC preflight](preflight-gcc.txt),
[Clang preflight](preflight-clang.txt). This is a preparation retry, not removal
of an acceptance sample.

All **180/180** whole replays are eligible; the maximum time-weighted unrelated
activity is **0.381263 cores**. Individually, **1568/1740** short build windows
pass. The 172 short-window failures remain in the data: the unchanged rule
judges time-weighted activity over the **whole replay**, not each short window.
See `replay_activity` in [analysis.json](analysis.json).

This run uses #140's short private `/tmp/kache-cc-*` runtimes. Each daemon
launch is followed by the runner's mandatory successful readiness check,
matching daemon version and build epoch to the invoking CLI. Preparation or
readiness failure aborts rather than silently admitting a fallback result.
Additional passive samples record **18 GCC and 44 Clang native daemon PIDs**,
their live `/proc/PID/exe` SHA256, private mode 0700 runtime and both socket
paths. Maximum sampled socket length is **32 bytes**, below 104. Records:
[GCC](daemon-observations-gcc-recovered.jsonl),
[Clang](daemon-observations-clang.jsonl), [monitor source](daemon_watch.py).

After **all acceptance timing**, separate strace identity checks run the
original twelve-TU warm make command with only its twelve derived objects
removed. **Each compiler: 12 requests, 12 hits, zero compiler/preprocessor runs,
zero standalone-engine executions**; requests use the already-started native
daemon. [GCC evidence](gcc-warm-identity.json), [Clang evidence](clang-warm-identity.json),
[trace log](identity.txt), [script](identity.py). These untimed diagnostics
contribute no performance samples. An initial auxiliary identity attempt used
`make -B` and failed its all-warm assertion; the final checks above use the
original invocation inventory instead. That diagnostic failure does not remove
or replace any formal validation or timed replay.

### Auxiliary-monitor correction

The first passive monitor hashed the initial combined Cargo build **before**
`make cc-bench` rebuilt binaries. Its stale expected hash assertion failed,
not the benchmark's daemon readiness or correctness checks. Initial digests
and failure remain [here](initial-build-binaries.sha256) and
[here](daemon-watch-initial.txt). The corrected monitor compares each live
engine inode with the then-current executable. It was restarted during GCC;
the GCC samples therefore cover only the latter part of that run. Every
formal replay still enforced the runner's readiness check. Clang used the
corrected monitor throughout. [Final SHA256 digests](measured-binaries.sha256)
are supplementary to the runner's recorded binary BLAKE3 identities.

The outer GCC orchestration stopped **after** the complete GCC report because
its auxiliary monitor had exited ([log](gcc-orchestration.txt)); Clang was then
launched separately ([log](clang-orchestration.txt)). The GCC acceptance data
were preserved, not rerun. This was an evidence-helper error, not a relaxed
admission rule or hidden host/daemon failure.

## Results against the frozen targets

Ratios are **kache/ccache geomeans [95% CI]**. Targets: cold <=1.10,
warm/relocated <=1.15. `I` means INCONCLUSIVE, not PASS. Every workload's overall
verdict is **FAIL**, because at least one required interval clearly misses.
Full per-scenario target verdicts are in the generated reports and analysis.

| Compiler / workload | Cold | Warm | Relocated | Overall |
|---|---|---|---|---|
| GCC 11 tiny C | 1.239 [1.167,1.315] FAIL | 2.468 [2.370,2.571] FAIL | 2.356 [2.171,2.556] FAIL | FAIL |
| GCC 11 C++ headers | 1.086 [0.985,1.197] I | 1.307 [1.175,1.454] FAIL | 1.441 [0.986,2.107] I | FAIL |
| GCC 11 zlib | 1.192 [1.086,1.308] I | 1.304 [1.192,1.427] FAIL | 1.112 [0.981,1.259] I | FAIL |
| Clang 14 tiny C | **0.716 [0.619,0.827] PASS** | 2.448 [2.120,2.826] FAIL | 2.287 [1.718,3.044] FAIL | FAIL |
| Clang 14 C++ headers | 1.058 [0.978,1.144] I | 1.339 [1.236,1.451] FAIL | 1.973 [1.604,2.427] FAIL | FAIL |
| Clang 14 zlib | 1.332 [1.204,1.473] FAIL | 1.213 [1.037,1.420] I | 1.305 [1.159,1.470] FAIL | FAIL |

Cold-vs-raw target <=1.15 applies independently:

| Compiler / workload | kache/raw [95% CI] | Verdict |
|---|---|---|
| GCC 11 tiny C | 1.994 [1.884,2.110] | FAIL |
| GCC 11 C++ headers | 1.102 [0.982,1.236] | INCONCLUSIVE |
| GCC 11 zlib | 1.286 [1.171,1.412] | FAIL |
| Clang 14 tiny C | 1.343 [1.160,1.554] | FAIL |
| Clang 14 C++ headers | 1.055 [0.918,1.213] | INCONCLUSIVE |
| Clang 14 zlib | 1.585 [1.406,1.788] | FAIL |

Absolute medians, milliseconds per whole build (kache / ccache):

| Compiler / workload | Cold | Warm |
|---|---:|---:|
| GCC 11 tiny C,48 objects | 1115.85 /890.85 | 173.31 /70.88 |
| GCC 11 C++ headers,12 objects | 5995.40 /5464.40 | 24.87 /17.95 |
| GCC 11 zlib,17 objects | 678.24 /589.63 | 83.68 /67.88 |
| Clang 14 tiny C,48 objects | 1849.97 /2438.86 | 189.79 /72.44 |
| Clang 14 C++ headers,12 objects | 6425.45 /6117.06 | 22.90 /16.65 |
| Clang 14 zlib,17 objects | 806.69 /588.68 | 106.30 /80.39 |

Tiny-C single-unit steady medians are **7.10 /3.78 ms (GCC)** and
**7.53 /4.20 ms (Clang)**. Explicit daemon-start medians are 62.74 ms and
75.43 ms respectively. These additional latency scenarios have no separate
blocking ratio target; they do not substitute for the 48-TU aggregate.

## Comparison and remaining work

Compared with #114, cold/warm kache/ccache ratios move:

| Compiler / workload | #114 cold /warm | This run cold /warm |
|---|---:|---:|
| GCC 11 tiny C | 1.158 /2.947 | 1.239 /2.468 |
| GCC 11 C++ headers | 0.974 /3.507 | 1.086 /1.307 |
| GCC 11 zlib | 1.029 /1.456 | 1.192 /1.304 |
| Clang 14 tiny C | 1.293 /3.284 | 0.716 /2.448 |
| Clang 14 C++ headers | 1.046 /2.825 | 1.058 /1.339 |
| Clang 14 zlib | 1.453 /1.278 | 1.332 /1.213 |

This is a formal current-source acceptance result, not an isolated causal A/B
of #138, #139 or #141. #114's overlong default sockets caused standalone
fallback; the working-daemon change is material. Do not attribute the complete
warm improvement to scanner microbenchmarks or header-analysis deserialization.

The largest current mean within-pair wall losses include Clang C++ immediate
source edit **+534.0 ms**, GCC C++ cold **+504.9 ms**, Clang tiny-C settled
header edit **+389.0 ms** and Clang C++ cold **+364.9 ms**. This absolute-time
ranking is diagnostic only, not a replacement estimator or CPU attribution.
GCC C++ settled-header edit now averages **-38.5 ms** vs ccache; that does not
explain the historical unassigned +675.8 ms or establish a root cause.
Last-replay [events](events/) are retained for follow-up. On hits,
`compile_time_ms` is historical; use actual `compiler_runs`, not that field,
when measuring executed work.

Remaining: all six named L1 workloads still fail overall, especially warm,
revert/relocation, GCC tiny-C cold and Clang zlib cold. Profile current losses
before another targeted change/rerun. The remaining standalone header-analysis
cost is separate from this verified-daemon acceptance run. **Eligible M1,
newer-L1 compilers, native ARM64 L2 and LLVM subset performance acceptance
remain unmeasured.** Provision M1 standalone LLVM Clang before claiming that
missing compiler cell. The prior 6–18 hour two-compiler LLVM estimate remains
planning only; check the unchanged 2 GB per-case/20 GB campaign scratch limits
before launching. Retained benchmark scratch here is approximately260 MiB;
the largest retained workload is about22 MiB. These are end-of-run sizes,
not continuously sampled peak bounds.

## Reproduce the evidence analysis

From this report directory, with the adjacent historical #114 report present:

```sh
python analyze.py
```

The standard-library-only [analysis script](analyze.py) independently reproduces
all 116 reported paired estimates/95% log-t intervals and locked target verdicts;
checks the exact clean revision, seed/jobs/compiler/input-digest continuity;
verifies all 180 retained replays, 1740 observer-free observations and complete-pair
raw-object equivalence; and compares the derived [analysis.json](analysis.json).
The live orchestration scripts retain original absolute worktree paths as
provenance; use a fresh output/worktree and update those paths before another
live run. They are not required to analyze the committed data. Acceptance rules
and `C_CPP_PARITY_CAMPAIGN.md` are unchanged.

The final local gate and dataset checks are recorded in
[verification.txt](verification.txt).

The first exact `env -u TMPDIR -u KACHE_CACHE_DIR make check` hit the known
test-fixture socket-length constraint in this long worktree path:

```text
error: socket path is 108 bytes, but a Unix socket path can be at most 107 on this platform
```

It failed `cached_check_outputs_allow_cargo_and_clippy_fix`, not a compiler
correctness or benchmark check. The final gate uses the existing documented
test-scratch override, without changing `TMPDIR`, any source or shared config:

```sh
env -u TMPDIR -u KACHE_CACHE_DIR \
  KACHE_TEST_SCRATCH_DIR=/home/ubuntu/kache-test-scratch make check
```

This affects only local test fixtures, after acceptance timing. It does not
change the benchmark's short private runtimes or any acceptance observation.
