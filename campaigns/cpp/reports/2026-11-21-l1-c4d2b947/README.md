# L1 acceptance rerun, 2026-11-21

Measured source: `c4d2b947f3cd1412decfed602a7ac8ded770df88`, clean fresh
worktree; includes #101 (Linux activity identity), #105 (time-macro sentinel
scan), and #109 (timestamp analysis memo). Evidence branch:
`devin/1795265219-cc-bench-l1-rerun`. No production code or benchmark rules changed.

**All six workloads are eligible, with 10 complete pairs each. All six FAIL
performance acceptance. Every untimed correctness/reuse validation passed.**

## Host, procedure and eligibility

Same native Linux x86_64 cloud VM as [#99's measurement](../2026-11-21-l1-da350481/README.md):
Ubuntu 22.04, glibc 2.35, kernel `6.8.0-1060-aws`, 8 CPUs, GCC 11.4,
Clang 14.0.0, pinned ccache 4.14.3. The campaign does not exclude native cloud
VMs. These older compiler versions do not close the newest-major L1 cells.

Built release tools from the clean measured revision, verified the pinned
ccache and project archives, and ran this command once for each compiler,
sequentially (`TOOLCHAINS=gcc`, then `TOOLCHAINS=clang`):

```sh
env -u TMPDIR -u KACHE_CACHE_DIR make cc-bench TOOLCHAINS=gcc \
  BENCH_ARGS="--acceptance --jobs 4 --workload corpus-c-tiny --workload corpus-cxx-headers --workload zlib"
```

Both variables were unset in the outer preflight/build/benchmark environment.
As in #99, the locked runner declares private per-arm `TMPDIR` values inside
its sandboxes; these appear in each JSON's declared environment. The caller
did not export a checkout-local `TMPDIR` or `KACHE_CACHE_DIR`.

A standalone five-second preflight using this revision's unchanged
`activity.rs` ran immediately before each compiler's command. Both first
attempts passed; no busy-host retries were necessary. The runner independently
performed its own preflight. The unrelated-CPU limit remains **0.5 cores**.

| Compiler | Standalone unrelated cores | Runner unrelated cores | Verdict |
|---|---:|---:|---|
| GCC | 0.021952 | 0.015970 | ELIGIBLE |
| Clang | 0.041824 | 0.013974 | ELIGIBLE |

Standalone outputs are in [preflight-gcc.txt](preflight-gcc.txt) and
[preflight-clang.txt](preflight-clang.txt). Runner preflights are in the raw
reports. Unlike #99, the sampler already keys Linux processes by PID and
kernel start time; command renames no longer charge lifetime counters.

| Compiler | Workload | Eligible/admitted replays | Individually eligible build windows |
|---|---|---:|---:|
| GCC | corpus-c-tiny | 30/30 | 266/330 |
| GCC | corpus-cxx-headers | 30/30 | 242/270 |
| GCC | zlib | 30/30 | 264/270 |
| Clang | corpus-c-tiny | 30/30 | 285/330 |
| Clang | corpus-cxx-headers | 30/30 | 252/270 |
| Clang | zlib | 30/30 | 264/270 |

Totals: **6/6 eligible workloads, 60/60 complete pairs, 180/180 eligible and
admitted arm replays, zero rejected replays**. The largest whole-replay
unrelated-CPU estimate is 0.366677 cores (GCC tiny C). Individually, 1573/1740
short build windows are eligible; 167 exceed the short-window limit. The
unchanged acceptance rule judges time-weighted activity over each whole replay,
not whether every short window passed. No interval was deleted, reclassified or
substituted. See `replays_eligible` in `crates/kache-e2e/src/cc_bench/mod.rs`.

All six workloads reached the existing ten-pair cap. No sample-count,
confidence-interval, admission, CPU limit or performance target was changed.
Both benchmark invocations returned performance failure (`Error 1` from the
runner, top-level make exit 2), not a correctness or host-eligibility failure.
Full invocation logs: [GCC](gcc-acceptance.txt), [Clang](clang-acceptance.txt).

## Results and comparison with #99

Ratios are the runner's paired geomeans, **kache/ccache**. Cold targets 1.10;
warm targets 1.15. A point estimate below target is not a PASS: the locked
verdict uses the entire 95% confidence interval. Full scenario ratios,
intervals, counters, provenance and all observations are in the unmodified
[GCC JSON](gcc/report.json) and [Clang JSON](clang/report.json), with generated
[GCC Markdown](gcc/report.md) and [Clang Markdown](clang/report.md).

| Compiler | Workload | Pairs | Eligible | Acceptance | Cold | Warm | #99 cold | #99 warm |
|---|---|---:|---|---|---:|---:|---:|---:|
| GCC 11 | Tiny C | 10 | yes | FAIL | 1.158 | 2.947 | 1.075* | 2.983* |
| GCC 11 | C++ headers | 10 | yes | FAIL | 0.974 | 3.507 | 1.023 | 1.657 |
| GCC 11 | zlib | 10 | yes | FAIL | 1.029 | 1.456 | 1.188* | 1.699* |
| Clang 14 | Tiny C | 10 | yes | FAIL | 1.293 | 3.284 | 1.675* | 3.222* |
| Clang 14 | C++ headers | 10 | yes | FAIL | 1.046 | 2.825 | 1.134 | 1.560 |
| Clang 14 | zlib | 10 | yes | FAIL | 1.453 | 1.278 | 1.142* | 1.166* |

`*` #99 tiny-C and zlib workloads were INELIGIBLE (one pair each on GCC,
five each on Clang); those historical ratios are diagnostic only. #99 had
2/6 eligible workloads, both FAIL, versus 6/6 eligible, all FAIL now. This
rerun does not establish passing L1 acceptance.

All six input digests, jobs=4, seed and compiler-version strings match #99.
The source revisions differ (`da350481` versus `c4d2b947`), so differences
cannot be attributed to one intervening PR alone. Clang tiny-C cold excess
fell from a diagnostic +1708.6 ms to an eligible +724.8 ms; its second
sentinel preprocessing pass is gone. Clang C++ cold excess fell from
+872.8 ms to +330.1 ms. Conversely, C++ warm performance regressed on both
compilers despite no compilation/preprocessing/probing on hits:

| Compiler | #99 warm kache/ccache median ms | Rerun warm kache/ccache median ms |
|---|---:|---:|
| GCC | 25.22 / 15.74 | 57.20 / 15.74 |
| Clang | 24.77 / 15.64 | 52.02 / 18.54 |

The prior single-TU timestamp-fix microbenchmark was against the newer #98
regression, not against #99. It is not evidence that this twelve-TU,
four-job acceptance path should match that microbenchmark. Its remaining
key/restore/concurrency costs need profiling; this run does not identify a
specific new defect as the cause of the warm regression.

## Top remaining wall losses and causes

Ranked by arithmetic **mean within-pair excess milliseconds**
`mean(kache.wall_ms - ccache.wall_ms)`, not by ratio or difference of medians.
Both leading rows have ten complete pairs and eligible workloads. The full
ranking and reproducible phase sums are in [analysis.json](analysis.json).
This is a ranking of point estimates, not a claim that the two losses are
statistically distinguishable from each other.

| Rank | Compiler / workload / scenario | Mean excess |
|---:|---|---:|
| 1 | Clang / tiny C / cold | +724.8 ms |
| 2 | GCC / C++ headers / settled header edit | +675.8 ms |

Events below are the retained final timed kache replay (zero-based pair 9),
not all ten pairs. Summed event durations are across units and cannot be
equated with a four-job build's elapsed wall time.

1. **Clang 14 preprocessing-key fallback remains expensive, but no longer
   performs two expansions per miss.** The actual retained
   [epoch-support record](events/clang-corpus-c-tiny-epoch.txt) is `no`.
   `cc_direct_key_eligible` in `src/compiler/cc.rs` requires
   `cc_source_date_epoch_is_pinned`, which checks that record, so this
   compiler takes `preprocess_hash` rather than compile-first discovery.
   [Events](events/clang-corpus-c-tiny.jsonl) show 48 cold misses, 48 actual
   compiler runs, **48 preprocessor runs** (96 in #99), one probe,
   **1061 ms summed key time**, and 120 ms summed store time. The ordinary
   source/dependency scan in `cc_time_macro_free_inputs` now avoids the
   sentinel expansion in `preprocess_hash`; the remaining first expansion
   and miss bookkeeping are still paid. Validation shows all arms generate
   the expected 48 objects: this is not a missed warm hit or incorrect
   reuse. These counters locate the expensive kache path; ccache's internal
   preprocessing was not independently profiled and not all +724.8 ms is
   assigned to preprocessing alone.
2. **GCC header invalidation takes the existing-memo preprocessing fallback,
   then recompiles all affected units.** The retained
   [GCC epoch-support record](events/gcc-corpus-cxx-headers-epoch.txt) is `ok`;
   this is not Clang's epoch-support limitation. Cold events have 12 compiler
   runs and zero preprocessors; after a recorded read set is invalidated by
   the header edit, the deferral guard
   `!ctx.file_hasher.cc_preprocess_memo_recorded(memo_key)` in
   `CcCompiler::cache_key_under` prevents a new compile-first capture. The
   fallback can still find older states, preserving revert reuse.
   [Header-edit events](events/gcc-corpus-cxx-headers.jsonl) show **12 compiler
   runs plus 12 preprocessors**, zero probes, **1429 ms summed key time**,
   64 ms summed store time and 12 `dup` results. `dup` means compiled output
   matched existing blobs, not that compilation was avoided. Untimed
   validation shows raw and ccache also compile the expected 12 units.
   Thus preprocessing/key work before the necessary compilation is the
   demonstrated extra kache path, rather than missing expected cache hits.
   Exact attribution of the +675.8 ms excess between keying, scheduling and
   other miss-path costs remains unprofiled.

For context, retained GCC C++ warm/relocated events have 12 local hits and
zero compiler/preprocessor/probe runs, with summed key times 110/154 ms and
3,367,304 copied bytes per build (zero reflink/hardlink bytes). Clang C++
warm/relocated summed key times are 110/131 ms, also with no compiler,
preprocessor or probe execution. These are separate hit-path losses that
sentinel elimination alone cannot remove.

## Evidence layout and verification

- `gcc/report.json` and `clang/report.json` are byte-identical copies of
  generated reports. Markdown differs only by trimming trailing blank lines.
- `events/` retains all six final timed streams unchanged, plus the two
  actual epoch-support records used above. The runner recreates state each
  replay, so these are representative final-replay streams, not an event
  history for all pairs.
- For zlib only, derived phase sums exclude 30 untimed configure events per
  compiler (28 `ztest*.c` probes and two output-less invocations), retaining
  every original event in the committed streams. Phase boundaries then match
  every observed hit/miss/other counter. Warm/relocated zlib includes two
  expected compiler-driver link passthroughs for `example` and `minigzip`,
  but no code generation, preprocessing or probes for restored objects.
- `analysis.json` contains all 58 scenario excesses, comparison ratios,
  admission/activity counts and final-replay event phase sums. Individual
  short-window flags and whole-replay eligibility are kept distinct.
- `verification.txt` records the evidence consistency check and source/input
  equality checks. No production code changed; `make check` was not rerun
  for this evidence-only commit. No PR was opened.

## LLVM estimate and decision

**LLVM was not run. Defer it until the remaining small-workload losses,
especially the C++ warm regression, have been profiled.** The activity
accounting blocker is resolved; all smaller workloads now have eligible
data, but none meets acceptance. A larger build is useful for eventual
matrix completion, not the most efficient next diagnostic.

The same pinned 159 MB archive is verified; #99's nine CMake lists contain
343 `.cpp` entries before conditional/transitive targets. Targets remain
`llvm-tblgen` and `LLVMCore`. Current twelve-TU C++ cold builds take roughly
5–7 seconds; a naive linear 343-unit extrapolation is already about 2–3.5
minutes per full build, before LLVM's heavier code/Release optimization,
generated prerequisites, linking and repeated configuration.

Unlike counting only the cold rows (18–33 per compiler in #99's estimate),
raw warm and header-edit/revert phases also compile work. Depending on header
fanout, validation plus 5–10 pairs of this non-relocatable workload can cost
roughly **24–88 full-build equivalents per compiler**: per replay raw pays
two full builds plus two header-affected fractions, and each cache pays one
full build plus one header-affected fraction. Source edits add further work.
Reserve roughly **6–18 hours for both compilers at four jobs**, with high
uncertainty rather than treating #99's 2–6 hours as a bound.

The earlier rough **4–12 GiB combined scratch** estimate is still unmeasured;
about 48 GiB is currently free. Disk alone is not an obvious host blocker,
but an actual CMake inventory/scratch estimate and the locked 2 GB per-case /
20 GB campaign ceilings must be checked before launch, not silently waived.
No estimate here substitutes for measured LLVM results or authorizes a
shortened workload, relaxed gate or budget increase.
