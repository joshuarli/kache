# cc-bench clang (acceptance)

- source: ee2402bfb20c2d018c4036bec5e53a1084f79a9a
- compiler: Ubuntu clang version 14.0.0-1ubuntu1.1
- kache: kache 2.0.0-rc.2
- ccache: ccache version 4.14.3
- host: Intel(R) Xeon(R) Processor 8 CPUs, linux
- jobs: 4, seed: 20261117
- CPU and RSS: child-process accounting: cpu_ms and max_rss_kib cover the build's process tree only and omit the already-running kache daemon
- preflight: unrelated processes used 0.03 cores over 5.0 s (eligible)

## corpus-c-tiny

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs raw: Some(Fail) (geomean 1.343, target 1.15)
- warm vs ccache: Some(Fail) (geomean 2.448, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.426, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.158, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 2.427, target 1.15)
- revert-source-immediate vs ccache: Some(Fail) (geomean 1.597, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 2.287, target 1.15)

Most unrelated activity around one timed build: 1.73 cores, busiest `amazon-ssm-agen`.

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

Timed replays: 30 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 0.12 to 0.50.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 0.716 | 0.619..0.827 | 1.1 | PASS | 1850.0 | 2438.9 |
| cold | raw | 10 | 1.343 | 1.160..1.554 | 1.15 | FAIL | 1850.0 | 1341.5 |
| warm | ccache | 10 | 2.448 | 2.120..2.826 | 1.15 | FAIL | 189.8 | 72.4 |
| warm | raw | 10 | 0.139 | 0.126..0.155 | - | - | 189.8 | 1373.4 |
| edit-source-settled | ccache | 10 | 0.662 | 0.593..0.739 | 1.15 | PASS | 43.7 | 62.6 |
| edit-source-settled | raw | 10 | 1.125 | 1.037..1.219 | - | - | 43.7 | 39.0 |
| revert-source-settled | ccache | 10 | 1.426 | 1.127..1.804 | 1.15 | INCONCLUSIVE | 8.8 | 5.7 |
| revert-source-settled | raw | 10 | 0.219 | 0.181..0.265 | - | - | 8.8 | 37.5 |
| edit-header-settled | ccache | 10 | 1.158 | 1.027..1.307 | 1.15 | INCONCLUSIVE | 2603.5 | 2293.0 |
| edit-header-settled | raw | 10 | 1.968 | 1.770..2.188 | - | - | 2603.5 | 1381.2 |
| revert-header-settled | ccache | 10 | 2.427 | 2.004..2.939 | 1.15 | FAIL | 182.3 | 77.1 |
| revert-header-settled | raw | 10 | 0.141 | 0.122..0.163 | - | - | 182.3 | 1355.6 |
| edit-source-immediate | ccache | 10 | 0.759 | 0.577..0.998 | 1.15 | PASS | 45.0 | 59.4 |
| edit-source-immediate | raw | 10 | 1.384 | 1.197..1.601 | - | - | 45.0 | 35.4 |
| revert-source-immediate | ccache | 10 | 1.597 | 1.165..2.190 | 1.15 | FAIL | 8.9 | 5.5 |
| revert-source-immediate | raw | 10 | 0.256 | 0.212..0.309 | - | - | 8.9 | 35.0 |
| relocated | ccache | 10 | 2.287 | 1.718..3.044 | 1.15 | FAIL | 180.8 | 72.4 |
| relocated | raw | 10 | 0.152 | 0.136..0.170 | - | - | 180.8 | 1306.2 |
| unit-daemon-start | ccache | 10 | 18.326 | 14.561..23.066 | - | - | 75.4 | 3.7 |
| unit-daemon-start | raw | 10 | 2.636 | 2.132..3.259 | - | - | 75.4 | 27.6 |
| unit-steady | ccache | 10 | 1.845 | 1.609..2.116 | - | - | 7.5 | 4.2 |
| unit-steady | raw | 10 | 0.261 | 0.219..0.310 | - | - | 7.5 | 30.3 |

Per unit (48 units): divide the medians by 48.

## corpus-cxx-headers

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs ccache: Some(Inconclusive) (geomean 1.058, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 1.055, target 1.15)
- warm vs ccache: Some(Fail) (geomean 1.339, target 1.15)
- revert-source-settled vs ccache: Some(Fail) (geomean 1.341, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 1.585, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.229, target 1.15)
- revert-source-immediate vs ccache: Some(Fail) (geomean 1.906, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 1.973, target 1.15)

Most unrelated activity around one timed build: 1.37 cores.

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
Host 1-minute load at the start of timed builds: 1.36 to 2.47.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 1.058 | 0.978..1.144 | 1.1 | INCONCLUSIVE | 6425.4 | 6117.1 |
| cold | raw | 10 | 1.055 | 0.918..1.213 | 1.15 | INCONCLUSIVE | 6425.4 | 5821.1 |
| warm | ccache | 10 | 1.339 | 1.236..1.451 | 1.15 | FAIL | 22.9 | 16.7 |
| warm | raw | 10 | 0.004 | 0.003..0.004 | - | - | 22.9 | 5891.4 |
| edit-source-settled | ccache | 10 | 1.046 | 0.993..1.101 | 1.15 | PASS | 2053.1 | 1935.1 |
| edit-source-settled | raw | 10 | 1.057 | 0.962..1.163 | - | - | 2053.1 | 1874.4 |
| revert-source-settled | ccache | 10 | 1.341 | 1.274..1.413 | 1.15 | FAIL | 9.9 | 7.3 |
| revert-source-settled | raw | 10 | 0.005 | 0.005..0.006 | - | - | 9.9 | 1856.5 |
| edit-header-settled | ccache | 10 | 1.009 | 0.935..1.089 | 1.15 | PASS | 6354.2 | 5969.6 |
| edit-header-settled | raw | 10 | 1.009 | 0.894..1.139 | - | - | 6354.2 | 5934.5 |
| revert-header-settled | ccache | 10 | 1.585 | 1.373..1.830 | 1.15 | FAIL | 27.1 | 17.1 |
| revert-header-settled | raw | 10 | 0.005 | 0.004..0.005 | - | - | 27.1 | 5868.1 |
| edit-source-immediate | ccache | 10 | 1.229 | 1.019..1.482 | 1.15 | INCONCLUSIVE | 2147.5 | 1935.3 |
| edit-source-immediate | raw | 10 | 1.255 | 1.095..1.437 | - | - | 2147.5 | 1892.7 |
| revert-source-immediate | ccache | 10 | 1.906 | 1.408..2.580 | 1.15 | FAIL | 11.2 | 7.2 |
| revert-source-immediate | raw | 10 | 0.007 | 0.006..0.010 | - | - | 11.2 | 1851.1 |
| relocated | ccache | 10 | 1.973 | 1.604..2.427 | 1.15 | FAIL | 28.9 | 15.5 |
| relocated | raw | 10 | 0.005 | 0.004..0.006 | - | - | 28.9 | 5801.6 |

Per unit (12 units): divide the medians by 12.

## zlib

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs ccache: Some(Fail) (geomean 1.332, target 1.1)
- cold vs raw: Some(Fail) (geomean 1.585, target 1.15)
- warm vs ccache: Some(Inconclusive) (geomean 1.213, target 1.15)
- edit-source-settled vs ccache: Some(Fail) (geomean 1.278, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.208, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.149, target 1.15)
- revert-header-settled vs ccache: Some(Inconclusive) (geomean 1.346, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.214, target 1.15)
- revert-source-immediate vs ccache: Some(Inconclusive) (geomean 1.225, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 1.305, target 1.15)

Most unrelated activity around one timed build: 0.71 cores, busiest `devin-remote-0.`.

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

Timed replays: 30 admitted, 0 rejected.
Host 1-minute load at the start of timed builds: 0.25 to 1.17.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 1.332 | 1.204..1.473 | 1.1 | FAIL | 806.7 | 588.7 |
| cold | raw | 10 | 1.585 | 1.406..1.788 | 1.15 | FAIL | 806.7 | 484.1 |
| warm | ccache | 10 | 1.213 | 1.037..1.420 | 1.15 | INCONCLUSIVE | 106.3 | 80.4 |
| warm | raw | 10 | 0.211 | 0.198..0.224 | - | - | 106.3 | 483.3 |
| edit-source-settled | ccache | 10 | 1.278 | 1.161..1.406 | 1.15 | FAIL | 213.0 | 166.2 |
| edit-source-settled | raw | 10 | 1.566 | 1.392..1.761 | - | - | 213.0 | 134.2 |
| revert-source-settled | ccache | 10 | 1.208 | 1.111..1.313 | 1.15 | INCONCLUSIVE | 89.0 | 73.6 |
| revert-source-settled | raw | 10 | 0.651 | 0.563..0.752 | - | - | 89.0 | 131.7 |
| edit-header-settled | ccache | 10 | 1.149 | 1.108..1.191 | 1.15 | INCONCLUSIVE | 593.8 | 518.1 |
| edit-header-settled | raw | 10 | 1.294 | 1.218..1.375 | - | - | 593.8 | 467.6 |
| revert-header-settled | ccache | 10 | 1.346 | 1.106..1.639 | 1.15 | INCONCLUSIVE | 102.4 | 78.2 |
| revert-header-settled | raw | 10 | 0.232 | 0.195..0.276 | - | - | 102.4 | 466.7 |
| edit-source-immediate | ccache | 10 | 1.214 | 1.131..1.303 | 1.15 | INCONCLUSIVE | 197.9 | 161.1 |
| edit-source-immediate | raw | 10 | 1.504 | 1.365..1.658 | - | - | 197.9 | 130.0 |
| revert-source-immediate | ccache | 10 | 1.225 | 1.073..1.399 | 1.15 | INCONCLUSIVE | 90.8 | 76.6 |
| revert-source-immediate | raw | 10 | 0.696 | 0.632..0.766 | - | - | 90.8 | 135.8 |
| relocated | ccache | 10 | 1.305 | 1.159..1.470 | 1.15 | FAIL | 89.9 | 70.5 |
| relocated | raw | 10 | 0.197 | 0.177..0.219 | - | - | 89.9 | 465.2 |
