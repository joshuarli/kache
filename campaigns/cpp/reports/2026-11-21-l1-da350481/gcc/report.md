# cc-bench gcc (acceptance)

- source: da350481a58373829051df9b6eff35400e0ba9a9
- compiler: gcc (Ubuntu 11.4.0-1ubuntu1~22.04.3) 11.4.0
- kache: kache 2.0.0-rc.2
- ccache: ccache version 4.14.3
- host: Intel(R) Xeon(R) Processor 8 CPUs, linux
- jobs: 4, seed: 20261117
- CPU and RSS: child-process accounting: cpu_ms and max_rss_kib cover the build's process tree only and omit the already-running kache daemon
- preflight: unrelated processes used 0.20 cores over 5.0 s (eligible)

## corpus-c-tiny

Acceptance: **INELIGIBLE** with 1 complete admitted pairs.

- host ineligible: unrelated processes above 0.5 cores, or activity unreadable
- 1 complete admitted pairs, 5 required
- cold vs ccache: Some(Inconclusive) (geomean 1.075, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 1.457, target 1.15)
- warm vs ccache: Some(Inconclusive) (geomean 2.983, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 1.060, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.553, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 0.983, target 1.15)
- revert-header-settled vs ccache: Some(Inconclusive) (geomean 2.381, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.086, target 1.15)
- revert-source-immediate vs ccache: Some(Inconclusive) (geomean 1.577, target 1.15)
- relocated vs ccache: Some(Inconclusive) (geomean 1.920, target 1.15)

Most unrelated activity around one timed build: 48.26 cores, busiest `u16:4-ext4-rsv-conversion`.

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
Host 1-minute load at the start of timed builds: 0.47 to 1.12.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 1 | 1.075 | - | 1.1 | INCONCLUSIVE | 1060.0 | 985.8 |
| cold | raw | 1 | 1.457 | - | 1.15 | INCONCLUSIVE | 1060.0 | 727.7 |
| warm | ccache | 1 | 2.983 | - | 1.15 | INCONCLUSIVE | 233.6 | 78.3 |
| warm | raw | 1 | 0.342 | - | - | - | 233.6 | 683.8 |
| edit-source-settled | ccache | 1 | 1.060 | - | 1.15 | INCONCLUSIVE | 29.0 | 27.4 |
| edit-source-settled | raw | 1 | 1.736 | - | - | - | 29.0 | 16.7 |
| revert-source-settled | ccache | 1 | 1.553 | - | 1.15 | INCONCLUSIVE | 8.6 | 5.5 |
| revert-source-settled | raw | 1 | 0.529 | - | - | - | 8.6 | 16.3 |
| edit-header-settled | ccache | 1 | 0.983 | - | 1.15 | INCONCLUSIVE | 1350.0 | 1373.1 |
| edit-header-settled | raw | 1 | 2.375 | - | - | - | 1350.0 | 568.3 |
| revert-header-settled | ccache | 1 | 2.381 | - | 1.15 | INCONCLUSIVE | 208.5 | 87.6 |
| revert-header-settled | raw | 1 | 0.353 | - | - | - | 208.5 | 591.4 |
| edit-source-immediate | ccache | 1 | 1.086 | - | 1.15 | INCONCLUSIVE | 29.1 | 26.8 |
| edit-source-immediate | raw | 1 | 1.728 | - | - | - | 29.1 | 16.9 |
| revert-source-immediate | ccache | 1 | 1.577 | - | 1.15 | INCONCLUSIVE | 8.9 | 5.6 |
| revert-source-immediate | raw | 1 | 0.469 | - | - | - | 8.9 | 19.0 |
| relocated | ccache | 1 | 1.920 | - | 1.15 | INCONCLUSIVE | 214.8 | 111.9 |
| relocated | raw | 1 | 0.313 | - | - | - | 214.8 | 686.4 |
| unit-daemon-start | ccache | 1 | 1.502 | - | - | - | 9.9 | 6.6 |
| unit-daemon-start | raw | 1 | 0.662 | - | - | - | 9.9 | 14.9 |
| unit-steady | ccache | 1 | 2.103 | - | - | - | 8.0 | 3.8 |
| unit-steady | raw | 1 | 0.542 | - | - | - | 8.0 | 14.8 |

Per unit (48 units): divide the medians by 48.

## corpus-cxx-headers

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs ccache: Some(Inconclusive) (geomean 1.023, target 1.1)
- warm vs ccache: Some(Fail) (geomean 1.657, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 1.119, target 1.15)
- revert-source-settled vs ccache: Some(Fail) (geomean 1.434, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.067, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 2.365, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.083, target 1.15)
- revert-source-immediate vs ccache: Some(Fail) (geomean 1.706, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 1.922, target 1.15)

Most unrelated activity around one timed build: 50.08 cores, busiest `u16:4-ext4-rsv-conversion`.

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

Timed replays: 30 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 1.31 to 2.58.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 1.023 | 0.931..1.124 | 1.1 | INCONCLUSIVE | 5486.2 | 5415.2 |
| cold | raw | 10 | 0.977 | 0.852..1.119 | 1.15 | PASS | 5486.2 | 5498.6 |
| warm | ccache | 10 | 1.657 | 1.410..1.947 | 1.15 | FAIL | 25.2 | 15.7 |
| warm | raw | 10 | 0.005 | 0.004..0.006 | - | - | 25.2 | 5314.7 |
| edit-source-settled | ccache | 10 | 1.119 | 0.946..1.324 | 1.15 | INCONCLUSIVE | 1788.4 | 1740.1 |
| edit-source-settled | raw | 10 | 1.113 | 0.956..1.297 | - | - | 1788.4 | 1703.0 |
| revert-source-settled | ccache | 10 | 1.434 | 1.175..1.750 | 1.15 | FAIL | 10.1 | 7.1 |
| revert-source-settled | raw | 10 | 0.007 | 0.006..0.007 | - | - | 10.1 | 1701.3 |
| edit-header-settled | ccache | 10 | 1.067 | 0.944..1.207 | 1.15 | INCONCLUSIVE | 5658.3 | 5414.5 |
| edit-header-settled | raw | 10 | 1.055 | 0.937..1.188 | - | - | 5658.3 | 5275.8 |
| revert-header-settled | ccache | 10 | 2.365 | 1.785..3.133 | 1.15 | FAIL | 36.6 | 17.4 |
| revert-header-settled | raw | 10 | 0.007 | 0.005..0.010 | - | - | 36.6 | 5263.6 |
| edit-source-immediate | ccache | 10 | 1.083 | 0.948..1.238 | 1.15 | INCONCLUSIVE | 1786.8 | 1701.6 |
| edit-source-immediate | raw | 10 | 1.054 | 0.972..1.144 | - | - | 1786.8 | 1702.8 |
| revert-source-immediate | ccache | 10 | 1.706 | 1.427..2.040 | 1.15 | FAIL | 12.6 | 6.7 |
| revert-source-immediate | raw | 10 | 0.007 | 0.007..0.008 | - | - | 12.6 | 1684.0 |
| relocated | ccache | 10 | 1.922 | 1.687..2.190 | 1.15 | FAIL | 33.2 | 15.4 |
| relocated | raw | 10 | 0.006 | 0.006..0.007 | - | - | 33.2 | 5422.3 |

Per unit (12 units): divide the medians by 12.

## zlib

Acceptance: **INELIGIBLE** with 1 complete admitted pairs.

- host ineligible: unrelated processes above 0.5 cores, or activity unreadable
- 1 complete admitted pairs, 5 required
- cold vs ccache: Some(Inconclusive) (geomean 1.188, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 1.510, target 1.15)
- warm vs ccache: Some(Inconclusive) (geomean 1.699, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 1.421, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.170, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 0.980, target 1.15)
- revert-header-settled vs ccache: Some(Inconclusive) (geomean 1.121, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.057, target 1.15)
- revert-source-immediate vs ccache: Some(Inconclusive) (geomean 1.288, target 1.15)
- relocated vs ccache: Some(Inconclusive) (geomean 0.856, target 1.15)

Most unrelated activity around one timed build: 7.37 cores, busiest `u16:4-ext4-rsv-conversion`.

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
Host 1-minute load at the start of timed builds: 0.54 to 1.06.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 1 | 1.188 | - | 1.1 | INCONCLUSIVE | 822.0 | 692.0 |
| cold | raw | 1 | 1.510 | - | 1.15 | INCONCLUSIVE | 822.0 | 544.6 |
| warm | ccache | 1 | 1.699 | - | 1.15 | INCONCLUSIVE | 111.7 | 65.7 |
| warm | raw | 1 | 0.174 | - | - | - | 111.7 | 642.2 |
| edit-source-settled | ccache | 1 | 1.421 | - | 1.15 | INCONCLUSIVE | 229.1 | 161.2 |
| edit-source-settled | raw | 1 | 1.892 | - | - | - | 229.1 | 121.0 |
| revert-source-settled | ccache | 1 | 1.170 | - | 1.15 | INCONCLUSIVE | 69.2 | 59.1 |
| revert-source-settled | raw | 1 | 0.497 | - | - | - | 69.2 | 139.1 |
| edit-header-settled | ccache | 1 | 0.980 | - | 1.15 | INCONCLUSIVE | 508.1 | 518.4 |
| edit-header-settled | raw | 1 | 0.818 | - | - | - | 508.1 | 621.5 |
| revert-header-settled | ccache | 1 | 1.121 | - | 1.15 | INCONCLUSIVE | 76.4 | 68.2 |
| revert-header-settled | raw | 1 | 0.155 | - | - | - | 76.4 | 493.5 |
| edit-source-immediate | ccache | 1 | 1.057 | - | 1.15 | INCONCLUSIVE | 140.0 | 132.4 |
| edit-source-immediate | raw | 1 | 1.138 | - | - | - | 140.0 | 123.0 |
| revert-source-immediate | ccache | 1 | 1.288 | - | 1.15 | INCONCLUSIVE | 104.0 | 80.7 |
| revert-source-immediate | raw | 1 | 0.811 | - | - | - | 104.0 | 128.2 |
| relocated | ccache | 1 | 0.856 | - | 1.15 | INCONCLUSIVE | 81.2 | 94.8 |
| relocated | raw | 1 | 0.126 | - | - | - | 81.2 | 644.2 |
