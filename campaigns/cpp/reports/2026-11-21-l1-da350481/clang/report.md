# cc-bench clang (acceptance)

- source: da350481a58373829051df9b6eff35400e0ba9a9
- compiler: Ubuntu clang version 14.0.0-1ubuntu1.1
- kache: kache 2.0.0-rc.2
- ccache: ccache version 4.14.3
- host: Intel(R) Xeon(R) Processor 8 CPUs, linux
- jobs: 4, seed: 20261117
- CPU and RSS: child-process accounting: cpu_ms and max_rss_kib cover the build's process tree only and omit the already-running kache daemon
- preflight: unrelated processes used 0.03 cores over 5.0 s (eligible)

## corpus-c-tiny

Acceptance: **INELIGIBLE** with 5 complete admitted pairs.

- host ineligible: unrelated processes above 0.5 cores, or activity unreadable
- cold vs ccache: Some(Fail) (geomean 1.675, target 1.1)
- cold vs raw: Some(Fail) (geomean 3.166, target 1.15)
- warm vs ccache: Some(Fail) (geomean 3.222, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 1.377, target 1.15)
- revert-source-settled vs ccache: Some(Fail) (geomean 1.791, target 1.15)
- edit-header-settled vs ccache: Some(Fail) (geomean 1.644, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 2.940, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.548, target 1.15)
- revert-source-immediate vs ccache: Some(Fail) (geomean 1.772, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 2.928, target 1.15)

Most unrelated activity around one timed build: 60.01 cores, busiest `u16:4-ext4-rsv-conversion`.

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

Timed replays: 15 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 0.22 to 0.67.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 5 | 1.675 | 1.341..2.094 | 1.1 | FAIL | 4219.6 | 2596.5 |
| cold | raw | 5 | 3.166 | 2.794..3.587 | 1.15 | FAIL | 4219.6 | 1353.5 |
| warm | ccache | 5 | 3.222 | 2.987..3.476 | 1.15 | FAIL | 227.0 | 71.5 |
| warm | raw | 5 | 0.174 | 0.137..0.221 | - | - | 227.0 | 1412.3 |
| edit-source-settled | ccache | 5 | 1.377 | 0.882..2.150 | 1.15 | INCONCLUSIVE | 92.7 | 65.2 |
| edit-source-settled | raw | 5 | 2.666 | 2.128..3.339 | - | - | 92.7 | 34.1 |
| revert-source-settled | ccache | 5 | 1.791 | 1.348..2.380 | 1.15 | FAIL | 9.0 | 5.4 |
| revert-source-settled | raw | 5 | 0.264 | 0.209..0.334 | - | - | 9.0 | 33.6 |
| edit-header-settled | ccache | 5 | 1.644 | 1.417..1.907 | 1.15 | FAIL | 4239.1 | 2635.6 |
| edit-header-settled | raw | 5 | 3.413 | 3.183..3.659 | - | - | 4239.1 | 1220.6 |
| revert-header-settled | ccache | 5 | 2.940 | 2.582..3.348 | 1.15 | FAIL | 219.9 | 78.6 |
| revert-header-settled | raw | 5 | 0.151 | 0.108..0.210 | - | - | 219.9 | 1469.2 |
| edit-source-immediate | ccache | 5 | 1.548 | 1.028..2.331 | 1.15 | INCONCLUSIVE | 86.9 | 58.8 |
| edit-source-immediate | raw | 5 | 2.597 | 1.649..4.090 | - | - | 86.9 | 34.2 |
| revert-source-immediate | ccache | 5 | 1.772 | 1.297..2.421 | 1.15 | FAIL | 8.9 | 5.0 |
| revert-source-immediate | raw | 5 | 0.276 | 0.220..0.347 | - | - | 8.9 | 34.2 |
| relocated | ccache | 5 | 2.928 | 2.637..3.251 | 1.15 | FAIL | 226.2 | 76.6 |
| relocated | raw | 5 | 0.168 | 0.142..0.200 | - | - | 226.2 | 1307.4 |
| unit-daemon-start | ccache | 5 | 3.050 | 2.461..3.780 | - | - | 11.4 | 3.7 |
| unit-daemon-start | raw | 5 | 0.397 | 0.324..0.487 | - | - | 11.4 | 28.7 |
| unit-steady | ccache | 5 | 2.722 | 1.979..3.742 | - | - | 9.1 | 3.8 |
| unit-steady | raw | 5 | 0.287 | 0.126..0.653 | - | - | 9.1 | 28.3 |

Per unit (48 units): divide the medians by 48.

## corpus-cxx-headers

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs ccache: Some(Inconclusive) (geomean 1.134, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 1.255, target 1.15)
- warm vs ccache: Some(Fail) (geomean 1.560, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 1.112, target 1.15)
- revert-source-settled vs ccache: Some(Fail) (geomean 1.635, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.065, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 2.068, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.055, target 1.15)
- revert-source-immediate vs ccache: Some(Fail) (geomean 1.517, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 2.046, target 1.15)

Most unrelated activity around one timed build: 43.97 cores, busiest `u16:4-ext4-rsv-conversion`.

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
Host 1-minute load at the start of timed builds: 1.42 to 2.60.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 1.134 | 1.051..1.223 | 1.1 | INCONCLUSIVE | 7387.4 | 6303.1 |
| cold | raw | 10 | 1.255 | 1.114..1.413 | 1.15 | INCONCLUSIVE | 7387.4 | 5652.8 |
| warm | ccache | 10 | 1.560 | 1.302..1.869 | 1.15 | FAIL | 24.8 | 15.6 |
| warm | raw | 10 | 0.004 | 0.004..0.005 | - | - | 24.8 | 5873.2 |
| edit-source-settled | ccache | 10 | 1.112 | 0.989..1.251 | 1.15 | INCONCLUSIVE | 2110.6 | 1945.8 |
| edit-source-settled | raw | 10 | 1.170 | 1.007..1.360 | - | - | 2110.6 | 1875.4 |
| revert-source-settled | ccache | 10 | 1.635 | 1.186..2.253 | 1.15 | FAIL | 10.1 | 6.9 |
| revert-source-settled | raw | 10 | 0.006 | 0.005..0.008 | - | - | 10.1 | 1890.1 |
| edit-header-settled | ccache | 10 | 1.065 | 0.930..1.219 | 1.15 | INCONCLUSIVE | 6438.6 | 6140.2 |
| edit-header-settled | raw | 10 | 1.082 | 0.949..1.234 | - | - | 6438.6 | 6132.5 |
| revert-header-settled | ccache | 10 | 2.068 | 1.895..2.257 | 1.15 | FAIL | 34.3 | 17.0 |
| revert-header-settled | raw | 10 | 0.006 | 0.005..0.006 | - | - | 34.3 | 6273.6 |
| edit-source-immediate | ccache | 10 | 1.055 | 0.926..1.202 | 1.15 | INCONCLUSIVE | 2052.0 | 1998.1 |
| edit-source-immediate | raw | 10 | 1.080 | 0.946..1.232 | - | - | 2052.0 | 1910.8 |
| revert-source-immediate | ccache | 10 | 1.517 | 1.229..1.873 | 1.15 | FAIL | 11.2 | 7.0 |
| revert-source-immediate | raw | 10 | 0.006 | 0.005..0.006 | - | - | 11.2 | 1920.0 |
| relocated | ccache | 10 | 2.046 | 1.793..2.336 | 1.15 | FAIL | 33.7 | 16.9 |
| relocated | raw | 10 | 0.006 | 0.005..0.007 | - | - | 33.7 | 5984.9 |

Per unit (12 units): divide the medians by 12.

## zlib

Acceptance: **INELIGIBLE** with 5 complete admitted pairs.

- host ineligible: unrelated processes above 0.5 cores, or activity unreadable
- cold vs ccache: Some(Inconclusive) (geomean 1.142, target 1.1)
- cold vs raw: Some(Fail) (geomean 1.607, target 1.15)
- warm vs ccache: Some(Inconclusive) (geomean 1.166, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 1.157, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.015, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.051, target 1.15)
- revert-header-settled vs ccache: Some(Inconclusive) (geomean 1.117, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.212, target 1.15)
- revert-source-immediate vs ccache: Some(Inconclusive) (geomean 1.167, target 1.15)
- relocated vs ccache: Some(Inconclusive) (geomean 1.257, target 1.15)

Most unrelated activity around one timed build: 9.68 cores, busiest `devin-remote`.

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

Timed replays: 15 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 0.27 to 1.23.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 5 | 1.142 | 1.011..1.290 | 1.1 | INCONCLUSIVE | 782.8 | 684.7 |
| cold | raw | 5 | 1.607 | 1.522..1.696 | 1.15 | FAIL | 782.8 | 480.2 |
| warm | ccache | 5 | 1.166 | 0.776..1.752 | 1.15 | INCONCLUSIVE | 103.6 | 84.8 |
| warm | raw | 5 | 0.206 | 0.149..0.285 | - | - | 103.6 | 552.2 |
| edit-source-settled | ccache | 5 | 1.157 | 1.071..1.250 | 1.15 | INCONCLUSIVE | 198.7 | 168.7 |
| edit-source-settled | raw | 5 | 1.275 | 0.959..1.696 | - | - | 198.7 | 140.2 |
| revert-source-settled | ccache | 5 | 1.015 | 0.843..1.221 | 1.15 | INCONCLUSIVE | 87.9 | 78.5 |
| revert-source-settled | raw | 5 | 0.578 | 0.429..0.778 | - | - | 87.9 | 134.3 |
| edit-header-settled | ccache | 5 | 1.051 | 0.882..1.252 | 1.15 | INCONCLUSIVE | 585.1 | 543.2 |
| edit-header-settled | raw | 5 | 1.272 | 1.066..1.517 | - | - | 585.1 | 446.5 |
| revert-header-settled | ccache | 5 | 1.117 | 1.028..1.214 | 1.15 | INCONCLUSIVE | 97.2 | 87.7 |
| revert-header-settled | raw | 5 | 0.189 | 0.148..0.242 | - | - | 97.2 | 463.1 |
| edit-source-immediate | ccache | 5 | 1.212 | 0.878..1.674 | 1.15 | INCONCLUSIVE | 211.9 | 162.0 |
| edit-source-immediate | raw | 5 | 1.626 | 1.374..1.925 | - | - | 211.9 | 128.3 |
| revert-source-immediate | ccache | 5 | 1.167 | 0.969..1.406 | 1.15 | INCONCLUSIVE | 88.5 | 78.6 |
| revert-source-immediate | raw | 5 | 0.706 | 0.621..0.803 | - | - | 88.5 | 127.5 |
| relocated | ccache | 5 | 1.257 | 1.013..1.561 | 1.15 | INCONCLUSIVE | 101.4 | 77.6 |
| relocated | raw | 5 | 0.201 | 0.173..0.234 | - | - | 101.4 | 477.1 |
