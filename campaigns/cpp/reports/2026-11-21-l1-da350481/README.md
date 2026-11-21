# L1 acceptance attempt, 2026-11-21

Measured source: `da350481a58373829051df9b6eff35400e0ba9a9` (clean).
`origin/main` had advanced beyond #92 to #93 when this worktree was created.
No compiler/cache code, admission rule, threshold or sample was changed.

## Host and command

Ubuntu 22.04, Linux x86_64 glibc 2.35, AWS kernel `6.8.0-1060-aws`,
8 CPUs, GCC 11.4, Clang 14.0.0; pinned ccache 4.14.3. About 73 GiB was
free before the run. This is native same-architecture execution, not an
emulated or cross-compiled lane. The plan does not categorically exclude
cloud VMs: `lock.toml` even counts same-architecture containers as native.
Unrelated heavy work disqualifies timing, not correctness. These installed
older compilers do not close the plan's newest-primary-compiler cells.

`TMPDIR` and `KACHE_CACHE_DIR` were unset for every command. Each arm's
declared environment, binary digests, compiler versions, jobs, configuration
and source provenance are in the unmodified JSON reports. The Markdown
copies only trim the generator's extra empty line at EOF for git whitespace.

The CLI has no preflight-only switch. Before invoking `make`, an external
Rust harness imported this revision's unmodified `cc_bench/activity.rs`
and called `Snapshot::take`, waited five seconds, then called `until` with
zero build CPU and the same own-process prefix policy as `Config::is_ours`.
It did not run validation or a workload. Its output is [preflight.txt](preflight.txt):
**0.04185 unrelated cores, ELIGIBLE**, against the unchanged 0.5-core limit.

Then:

```sh
env -u TMPDIR -u KACHE_CACHE_DIR make cc-bench TOOLCHAINS="gcc clang" \
  BENCH_ARGS="--acceptance --jobs 4 --workload corpus-c-tiny --workload corpus-cxx-headers --workload zlib"
```

Four jobs is the lock's maximum half of eight cores; tiny C is deliberately
serial in the workload implementation. The runner performed another
five-second preflight per toolchain: GCC 0.1994 cores, Clang 0.03191 cores,
both eligible. It retained all admitted samples and checked activity per
replay. Downloads, benchmark-binary builds, configuration and validation
were untimed.

## Outcome

Both reports validated every workload and admitted every timed replay for
inventory/output/counter consistency. No correctness or reuse validation
failed. Admission is not host eligibility: four workloads were subsequently
marked **INELIGIBLE**, and the two eligible workloads **FAIL** performance
targets. There is no passing L1 acceptance claim.

Ratios below are paired log-geomeans, kache/ccache. Cold target is 1.10;
warm target is 1.15. Full 95% intervals and all scenarios are in the reports.

| Compiler | Workload | Complete admitted pairs | Eligible | Acceptance | Cold | Warm |
|---|---|---:|---|---|---:|---:|
| GCC 11 | tiny C | 1 | no | INELIGIBLE | 1.075 | 2.983 |
| GCC 11 | C++ headers | 10 | yes | FAIL | 1.023 | 1.657 |
| GCC 11 | zlib | 1 | no | INELIGIBLE | 1.188 | 1.699 |
| Clang 14 | tiny C | 5 | no | INELIGIBLE | 1.675 | 3.222 |
| Clang 14 | C++ headers | 10 | yes | FAIL | 1.134 | 1.560 |
| Clang 14 | zlib | 5 | no | INELIGIBLE | 1.142 | 1.166 |

The runner returned 3 for ineligibility; GNU make returned 2:

```text
make: *** [Makefile:203: cc-bench] Error 3
ACCEPTANCE_EXIT=2
```

- [GCC report.json](gcc/report.json), [report.md](gcc/report.md)
- [Clang report.json](clang/report.json), [report.md](clang/report.md)
- [Derived wall-loss ranking and event phase sums](analysis.json)

## Losses ranked by wall time

The ranking uses the arithmetic mean of each pair's kache wall milliseconds
minus ccache's for the same scenario, not a difference of medians. This
absolute-time ranking does not change the locked log-ratio estimator.

| Rank | Compiler / workload / scenario | Mean excess per replay | Pairs | Scope |
|---:|---|---:|---:|---|
| 1 | Clang / tiny C / cold | 1,708.6 ms | 5 | diagnostic only: ineligible |
| 2 | Clang / tiny C / settled header edit | 1,632.6 ms | 5 | diagnostic only: ineligible |
| 3 | Clang / C++ headers / cold | 872.8 ms | 10 | eligible |
| 4 | Clang / C++ headers / settled header edit | 436.2 ms | 10 | eligible |
| 5 | GCC / C++ headers / settled header edit | 402.7 ms | 10 | eligible |
| 6 | GCC / C++ headers / settled source edit | 268.1 ms | 10 | eligible |

**The first two share a measured expensive path: Clang 14 preprocessing
fallback and its time-macro safety check.** The final retained tiny-C kache
replay (pair 4, zero-based) records:

| Scenario | Compiler runs | Preprocessor runs | Probe runs | Summed key ms | Summed store ms |
|---|---:|---:|---:|---:|---:|
| cold | 48 | 96 | 1 | 2,186 | 139 |
| settled header edit | 48 | 96 | 0 | 2,125 | 108 |
| warm | 0 | 0 | 0 | 0 | 0 |

The retained [epoch-support memo](events/clang-corpus-c-tiny-epoch.txt)
for that actual driver/environment says `no`.
`cc_direct_key_eligible` requires `cc_source_date_epoch_is_pinned`, so
Clang 14 takes `preprocess_hash`. That function runs `-E` once for the key
and, without a trusted epoch, again with sentinel date/time macros to
prove the TU is time-independent. The events record exactly those two
preprocessor executions per miss. Warm/restored entries do not re-probe or
generate code. Validation gives raw, ccache and kache 48 code generations
for each cold/header-edit phase, so this is not a missing-hit gap relative
to ccache; `dup` means kache compiled and found identical output bytes,
not that code generation was avoided.

The same path is present in the largest **eligible** losses: the last
Clang C++ replay has 12 compiler runs and 24 preprocessor runs in both cold
and header-edit phases, with summed key times of 2,830 and 2,141 ms.
Those builds use four jobs, so summed per-unit times are not wall time.
This identifies the expensive kache path, not an exact allocation of all
paired excess: ccache preprocessing time was not separately retained, and
these detailed events cover the last replay rather than all pairs.

The eligible warm C++ failures remain a separate smaller fixed-hit-cost
loss: medians are 25.2 vs 15.7 ms (GCC) and 24.8 vs 15.6 ms (Clang) for
12 hits. Every hit has zero compiler/preprocessor/probe runs, including
relocation. The old relocated re-probe explanation is not supported here.

Detailed last-replay events are retained under [events/](events/).
`analysis.json` slices them in report scenario order using each phase's
hits + misses + other count. `compile_time_ms` on a hit is cached historical
cost and must not be counted as executed work; use `compiler_runs`.
Event milliseconds are rounded; zero does not prove no sub-millisecond work.

## Eligibility measurement limitation

The unchanged runner reports the kernel worker PID `1009580` alternately
as `u16:4-writeback` and `u16:4-ext4-rsv-conversion`. In the GCC tiny-C
kache replay it charges this worker 1,000 ms in, for example, a 21.44 ms
build interval, while its system-wide estimate for that interval is 23 ms.
Linux `ps` CPU time is only whole-second precision. Also,
`Snapshot::until` discards a PID's earlier CPU counter whenever its full
command string changes, charging its entire later cumulative CPU time.

An external deterministic fixture using an otherwise unchanged copy of
the activity module confirms the rename defect: a same-PID worker at
1,000 ms in both snapshots costs zero with an unchanged command, but
1,000 ms after changing only the workqueue name. See
[sampler-fixture.txt](sampler-fixture.txt). A brief live probe did not catch
a fresh same-tick rename. The reports do not preserve the before/after
per-process snapshots, so the exact contributions of renaming, quantized
counter boundaries and real background activity to historical rejections
cannot be separated. No sample was reclassified or removed.

Before another acceptance attempt, fix/test Linux activity accounting with
high-resolution per-PID CPU and stable process identity, retaining the
0.5-core limit and the locked sample rules. The host must also remain quiet
throughout every timed replay, not merely its five-second preflight. Do
not infer that all rejected observations were actually quiet.

## LLVM estimate and decision

LLVM was **not run**. The entry-point script already downloaded and verified
its 159 MB archive outside timing; its full uncompressed tar is about
2.0 GB. Nine relevant LLVM CMake source lists contain 343 `.cpp` entries,
before accounting for conditional/transitive targets. The pinned targets
are `llvm-tblgen` and `LLVMCore`.

At four jobs, extrapolating the observed C++ cold cost gives a rough
multi-hour budget (about 2–6 hours for both compilers, not a measured
runtime). Even at five pairs, validation plus three arms entails 18 cold
builds per compiler, then edit/revert phases and repeated configuration;
at ten pairs it is 33. Estimated source/build/cache scratch is several
GiB per compiler (roughly 4–12 GiB combined, uncertain but below 73 GiB
available and subject to the 20 GiB campaign ceiling). The three smaller
workloads currently occupy about 238 MiB together, including the archives.
Disk alone is not a blocker. Further expensive acceptance timing is not
justified until the activity-accounting limitation is resolved. No retry
or shortened LLVM workload was substituted.
