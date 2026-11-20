# cc-bench cc (acceptance)

Redacted 2026-11-20: process names only; see `redaction` in report.json.

- source: aa8df06a98d1d796afc6458576499c9c296bdfaa
- compiler: Apple clang version 21.0.0 (clang-2100.3.34.2)
- kache: kache 2.0.0-rc.2
- ccache: ccache version 4.14.3
- host: Apple M1 Max 10 CPUs, macos
- jobs: 10, seed: 20261117
- CPU and RSS: child-process accounting: cpu_ms and max_rss_kib cover the build's process tree only and omit the already-running kache daemon
- preflight: unrelated processes used 0.49 cores over 5.1 s (eligible)

## corpus-c-tiny

Acceptance: **INELIGIBLE** with 1 complete admitted pairs.

- host ineligible: unrelated processes above 0.5 cores, or activity unreadable
- 1 complete admitted pairs, 5 required
- cold vs ccache: Some(Inconclusive) (geomean 0.701, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 1.465, target 1.15)
- warm vs ccache: Some(Inconclusive) (geomean 1.479, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 0.758, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.121, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.168, target 1.15)
- revert-header-settled vs ccache: Some(Inconclusive) (geomean 1.425, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 0.750, target 1.15)
- revert-source-immediate vs ccache: Some(Inconclusive) (geomean 1.130, target 1.15)
- relocated vs ccache: Some(Inconclusive) (geomean 1.454, target 1.15)

Most unrelated activity around one timed build: 2.59 cores, busiest `softwareupdated`.

Validation (untimed, through compiler shims): code generations, then cache hits/misses/other per scenario.

| scenario | raw | ccache | kache |
|---|---|---|---|
| cold | 48 cg, 0h/0m/0o | 48 cg, 0h/48m/0o | 48 cg, 0h/48m/0o |
| warm | 48 cg, 0h/0m/0o | 0 cg, 48h/0m/0o | 0 cg, 48h/0m/0o |
| edit-source-settled | 1 cg, 0h/0m/0o | 1 cg, 0h/1m/0o | 1 cg, 0h/1m/0o |
| revert-source-settled | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/0o |
| edit-header-settled | 48 cg, 0h/0m/0o | 48 cg, 0h/48m/0o | 48 cg, 0h/48m/0o |
| revert-header-settled | 48 cg, 0h/0m/0o | 0 cg, 48h/0m/0o | 0 cg, 48h/0m/0o |
| edit-source-immediate | 1 cg, 0h/0m/0o | 1 cg, 0h/1m/0o | 1 cg, 0h/1m/0o |
| revert-source-immediate | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/0o |
| relocated | 48 cg, 0h/0m/0o | 0 cg, 48h/0m/0o | 0 cg, 48h/0m/0o |
| unit-daemon-start | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/0o |
| unit-steady | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/0o |

Timed replays: 3 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 3.57 to 6.59.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 1 | 0.701 | - | 1.1 | INCONCLUSIVE | 1568.5 | 2236.6 |
| cold | raw | 1 | 1.465 | - | 1.15 | INCONCLUSIVE | 1568.5 | 1070.8 |
| warm | ccache | 1 | 1.479 | - | 1.15 | INCONCLUSIVE | 286.9 | 194.0 |
| warm | raw | 1 | 0.268 | - | - | - | 286.9 | 1072.3 |
| edit-source-settled | ccache | 1 | 0.758 | - | 1.15 | INCONCLUSIVE | 46.7 | 61.6 |
| edit-source-settled | raw | 1 | 1.217 | - | - | - | 46.7 | 38.3 |
| revert-source-settled | ccache | 1 | 1.121 | - | 1.15 | INCONCLUSIVE | 21.6 | 19.3 |
| revert-source-settled | raw | 1 | 0.563 | - | - | - | 21.6 | 38.4 |
| edit-header-settled | ccache | 1 | 1.168 | - | 1.15 | INCONCLUSIVE | 2622.7 | 2244.7 |
| edit-header-settled | raw | 1 | 2.431 | - | - | - | 2622.7 | 1078.7 |
| revert-header-settled | ccache | 1 | 1.425 | - | 1.15 | INCONCLUSIVE | 275.8 | 193.6 |
| revert-header-settled | raw | 1 | 0.258 | - | - | - | 275.8 | 1071.1 |
| edit-source-immediate | ccache | 1 | 0.750 | - | 1.15 | INCONCLUSIVE | 46.9 | 62.5 |
| edit-source-immediate | raw | 1 | 1.190 | - | - | - | 46.9 | 39.4 |
| revert-source-immediate | ccache | 1 | 1.130 | - | 1.15 | INCONCLUSIVE | 21.5 | 19.0 |
| revert-source-immediate | raw | 1 | 0.547 | - | - | - | 21.5 | 39.4 |
| relocated | ccache | 1 | 1.454 | - | 1.15 | INCONCLUSIVE | 286.2 | 196.8 |
| relocated | raw | 1 | 0.267 | - | - | - | 286.2 | 1071.6 |
| unit-daemon-start | ccache | 1 | 5.467 | - | - | - | 103.2 | 18.9 |
| unit-daemon-start | raw | 1 | 2.653 | - | - | - | 103.2 | 38.9 |
| unit-steady | ccache | 1 | 1.258 | - | - | - | 21.6 | 17.1 |
| unit-steady | raw | 1 | 0.593 | - | - | - | 21.6 | 36.4 |

Per unit (48 units): divide the medians by 48.

## corpus-cxx-headers

Acceptance: **INELIGIBLE** with 1 complete admitted pairs.

- host ineligible: unrelated processes above 0.5 cores, or activity unreadable
- 1 complete admitted pairs, 5 required
- cold vs ccache: Some(Inconclusive) (geomean 1.107, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 1.400, target 1.15)
- warm vs ccache: Some(Inconclusive) (geomean 0.993, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 0.948, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.340, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.054, target 1.15)
- revert-header-settled vs ccache: Some(Inconclusive) (geomean 1.393, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 0.962, target 1.15)
- revert-source-immediate vs ccache: Some(Inconclusive) (geomean 1.354, target 1.15)
- relocated vs ccache: Some(Inconclusive) (geomean 1.525, target 1.15)

Most unrelated activity around one timed build: 1.23 cores, busiest `fseventsd`.

Validation (untimed, through compiler shims): code generations, then cache hits/misses/other per scenario.

| scenario | raw | ccache | kache |
|---|---|---|---|
| cold | 12 cg, 0h/0m/0o | 12 cg, 0h/12m/0o | 12 cg, 0h/12m/0o |
| warm | 12 cg, 0h/0m/0o | 0 cg, 12h/0m/0o | 0 cg, 12h/0m/0o |
| edit-source-settled | 1 cg, 0h/0m/0o | 1 cg, 0h/1m/0o | 1 cg, 0h/1m/0o |
| revert-source-settled | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/0o |
| edit-header-settled | 12 cg, 0h/0m/0o | 12 cg, 0h/12m/0o | 12 cg, 0h/12m/0o |
| revert-header-settled | 12 cg, 0h/0m/0o | 0 cg, 12h/0m/0o | 0 cg, 12h/0m/0o |
| edit-source-immediate | 1 cg, 0h/0m/0o | 1 cg, 0h/1m/0o | 1 cg, 0h/1m/0o |
| revert-source-immediate | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/0o |
| relocated | 12 cg, 0h/0m/0o | 0 cg, 12h/0m/0o | 0 cg, 12h/0m/0o |

Timed replays: 3 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 5.08 to 8.00.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 1 | 1.107 | - | 1.1 | INCONCLUSIVE | 2707.5 | 2445.1 |
| cold | raw | 1 | 1.400 | - | 1.15 | INCONCLUSIVE | 2707.5 | 1933.6 |
| warm | ccache | 1 | 0.993 | - | 1.15 | INCONCLUSIVE | 64.5 | 65.0 |
| warm | raw | 1 | 0.034 | - | - | - | 64.5 | 1904.5 |
| edit-source-settled | ccache | 1 | 0.948 | - | 1.15 | INCONCLUSIVE | 915.4 | 965.2 |
| edit-source-settled | raw | 1 | 1.133 | - | - | - | 915.4 | 807.8 |
| revert-source-settled | ccache | 1 | 1.340 | - | 1.15 | INCONCLUSIVE | 31.5 | 23.5 |
| revert-source-settled | raw | 1 | 0.039 | - | - | - | 31.5 | 817.6 |
| edit-header-settled | ccache | 1 | 1.054 | - | 1.15 | INCONCLUSIVE | 2486.9 | 2358.8 |
| edit-header-settled | raw | 1 | 1.295 | - | - | - | 2486.9 | 1919.9 |
| revert-header-settled | ccache | 1 | 1.393 | - | 1.15 | INCONCLUSIVE | 89.1 | 64.0 |
| revert-header-settled | raw | 1 | 0.046 | - | - | - | 89.1 | 1954.9 |
| edit-source-immediate | ccache | 1 | 0.962 | - | 1.15 | INCONCLUSIVE | 918.2 | 954.9 |
| edit-source-immediate | raw | 1 | 1.130 | - | - | - | 918.2 | 812.4 |
| revert-source-immediate | ccache | 1 | 1.354 | - | 1.15 | INCONCLUSIVE | 32.6 | 24.1 |
| revert-source-immediate | raw | 1 | 0.039 | - | - | - | 32.6 | 826.7 |
| relocated | ccache | 1 | 1.525 | - | 1.15 | INCONCLUSIVE | 97.5 | 63.9 |
| relocated | raw | 1 | 0.051 | - | - | - | 97.5 | 1923.9 |

Per unit (12 units): divide the medians by 12.

## zlib

Acceptance: **INELIGIBLE** with 1 complete admitted pairs.

- host ineligible: unrelated processes above 0.5 cores, or activity unreadable
- 1 complete admitted pairs, 5 required
- cold vs ccache: Some(Inconclusive) (geomean 1.067, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 0.835, target 1.15)
- warm vs ccache: Some(Inconclusive) (geomean 1.360, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 0.970, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.202, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.111, target 1.15)
- revert-header-settled vs ccache: Some(Inconclusive) (geomean 1.376, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.187, target 1.15)
- revert-source-immediate vs ccache: Some(Inconclusive) (geomean 1.345, target 1.15)
- relocated vs ccache: Some(Inconclusive) (geomean 1.471, target 1.15)

Most unrelated activity around one timed build: 8.46 cores, busiest `OrbStack Helper`.

Validation (untimed, through compiler shims): code generations, then cache hits/misses/other per scenario.

| scenario | raw | ccache | kache |
|---|---|---|---|
| cold | 17 cg, 0h/0m/0o | 17 cg, 0h/17m/0o | 17 cg, 0h/17m/2o |
| warm | 17 cg, 0h/0m/0o | 0 cg, 17h/0m/0o | 0 cg, 17h/0m/2o |
| edit-source-settled | 1 cg, 0h/0m/0o | 1 cg, 0h/1m/0o | 1 cg, 0h/1m/2o |
| revert-source-settled | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/2o |
| edit-header-settled | 9 cg, 0h/0m/0o | 9 cg, 0h/9m/0o | 9 cg, 0h/9m/2o |
| revert-header-settled | 9 cg, 0h/0m/0o | 0 cg, 9h/0m/0o | 0 cg, 9h/0m/2o |
| edit-source-immediate | 1 cg, 0h/0m/0o | 1 cg, 0h/1m/0o | 1 cg, 0h/1m/2o |
| revert-source-immediate | 1 cg, 0h/0m/0o | 0 cg, 1h/0m/0o | 0 cg, 1h/0m/2o |
| relocated | 17 cg, 0h/0m/0o | 0 cg, 17h/0m/0o | 0 cg, 17h/0m/2o |

Timed replays: 3 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 3.59 to 6.55.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 1 | 1.067 | - | 1.1 | INCONCLUSIVE | 418.3 | 392.1 |
| cold | raw | 1 | 0.835 | - | 1.15 | INCONCLUSIVE | 418.3 | 501.1 |
| warm | ccache | 1 | 1.360 | - | 1.15 | INCONCLUSIVE | 151.2 | 111.2 |
| warm | raw | 1 | 0.252 | - | - | - | 151.2 | 600.6 |
| edit-source-settled | ccache | 1 | 0.970 | - | 1.15 | INCONCLUSIVE | 173.9 | 179.2 |
| edit-source-settled | raw | 1 | 0.983 | - | - | - | 173.9 | 176.8 |
| revert-source-settled | ccache | 1 | 1.202 | - | 1.15 | INCONCLUSIVE | 115.3 | 95.9 |
| revert-source-settled | raw | 1 | 0.562 | - | - | - | 115.3 | 205.1 |
| edit-header-settled | ccache | 1 | 1.111 | - | 1.15 | INCONCLUSIVE | 411.9 | 370.7 |
| edit-header-settled | raw | 1 | 0.868 | - | - | - | 411.9 | 474.6 |
| revert-header-settled | ccache | 1 | 1.376 | - | 1.15 | INCONCLUSIVE | 139.3 | 101.2 |
| revert-header-settled | raw | 1 | 0.280 | - | - | - | 139.3 | 497.9 |
| edit-source-immediate | ccache | 1 | 1.187 | - | 1.15 | INCONCLUSIVE | 208.1 | 175.3 |
| edit-source-immediate | raw | 1 | 1.116 | - | - | - | 208.1 | 186.5 |
| revert-source-immediate | ccache | 1 | 1.345 | - | 1.15 | INCONCLUSIVE | 126.0 | 93.7 |
| revert-source-immediate | raw | 1 | 0.609 | - | - | - | 126.0 | 206.9 |
| relocated | ccache | 1 | 1.471 | - | 1.15 | INCONCLUSIVE | 162.4 | 110.4 |
| relocated | raw | 1 | 0.275 | - | - | - | 162.4 | 590.1 |

