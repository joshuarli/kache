# cc-bench gcc (acceptance)

- source: c4d2b947f3cd1412decfed602a7ac8ded770df88
- compiler: gcc (Ubuntu 11.4.0-1ubuntu1~22.04.3) 11.4.0
- kache: kache 2.0.0-rc.2
- ccache: ccache version 4.14.3
- host: Intel(R) Xeon(R) Processor 8 CPUs, linux
- jobs: 4, seed: 20261117
- CPU and RSS: child-process accounting: cpu_ms and max_rss_kib cover the build's process tree only and omit the already-running kache daemon
- preflight: unrelated processes used 0.02 cores over 5.0 s (eligible)

## corpus-c-tiny

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs ccache: Some(Inconclusive) (geomean 1.158, target 1.1)
- cold vs raw: Some(Fail) (geomean 1.865, target 1.15)
- warm vs ccache: Some(Fail) (geomean 2.947, target 1.15)
- edit-source-settled vs ccache: Some(Inconclusive) (geomean 1.228, target 1.15)
- revert-source-settled vs ccache: Some(Fail) (geomean 1.669, target 1.15)
- edit-header-settled vs ccache: Some(Fail) (geomean 1.509, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 3.325, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.143, target 1.15)
- revert-source-immediate vs ccache: Some(Fail) (geomean 1.769, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 3.350, target 1.15)

Most unrelated activity around one timed build: 2.34 cores, busiest `devin-remote`.

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
Host 1-minute load at the start of timed builds: 0.10 to 0.84.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 1.158 | 1.054..1.273 | 1.1 | INCONCLUSIVE | 1197.9 | 1066.5 |
| cold | raw | 10 | 1.865 | 1.633..2.129 | 1.15 | FAIL | 1197.9 | 622.4 |
| warm | ccache | 10 | 2.947 | 2.505..3.468 | 1.15 | FAIL | 247.1 | 81.1 |
| warm | raw | 10 | 0.384 | 0.343..0.430 | - | - | 247.1 | 660.2 |
| edit-source-settled | ccache | 10 | 1.228 | 1.070..1.408 | 1.15 | INCONCLUSIVE | 35.1 | 25.8 |
| edit-source-settled | raw | 10 | 1.445 | 1.074..1.945 | - | - | 35.1 | 20.7 |
| revert-source-settled | ccache | 10 | 1.669 | 1.385..2.012 | 1.15 | FAIL | 9.7 | 5.7 |
| revert-source-settled | raw | 10 | 0.419 | 0.298..0.589 | - | - | 9.7 | 20.0 |
| edit-header-settled | ccache | 10 | 1.509 | 1.336..1.704 | 1.15 | FAIL | 1518.5 | 975.7 |
| edit-header-settled | raw | 10 | 2.242 | 1.931..2.602 | - | - | 1518.5 | 649.4 |
| revert-header-settled | ccache | 10 | 3.325 | 2.761..4.004 | 1.15 | FAIL | 251.4 | 80.0 |
| revert-header-settled | raw | 10 | 0.385 | 0.308..0.482 | - | - | 251.4 | 677.1 |
| edit-source-immediate | ccache | 10 | 1.143 | 0.969..1.348 | 1.15 | INCONCLUSIVE | 31.0 | 25.6 |
| edit-source-immediate | raw | 10 | 1.527 | 1.296..1.800 | - | - | 31.0 | 19.3 |
| revert-source-immediate | ccache | 10 | 1.769 | 1.689..1.853 | 1.15 | FAIL | 9.7 | 5.6 |
| revert-source-immediate | raw | 10 | 0.482 | 0.408..0.569 | - | - | 9.7 | 18.2 |
| relocated | ccache | 10 | 3.350 | 3.016..3.721 | 1.15 | FAIL | 261.1 | 76.6 |
| relocated | raw | 10 | 0.384 | 0.329..0.449 | - | - | 261.1 | 652.6 |
| unit-daemon-start | ccache | 10 | 2.794 | 2.448..3.188 | - | - | 10.9 | 4.2 |
| unit-daemon-start | raw | 10 | 0.674 | 0.523..0.869 | - | - | 10.9 | 14.9 |
| unit-steady | ccache | 10 | 2.173 | 2.006..2.354 | - | - | 8.7 | 4.0 |
| unit-steady | raw | 10 | 0.524 | 0.434..0.633 | - | - | 8.7 | 15.6 |

Per unit (48 units): divide the medians by 48.

## corpus-cxx-headers

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs ccache: Some(Inconclusive) (geomean 0.974, target 1.1)
- warm vs ccache: Some(Fail) (geomean 3.507, target 1.15)
- revert-source-settled vs ccache: Some(Fail) (geomean 2.217, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.103, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 3.082, target 1.15)
- revert-source-immediate vs ccache: Some(Fail) (geomean 3.152, target 1.15)
- relocated vs ccache: Some(Fail) (geomean 3.671, target 1.15)

Most unrelated activity around one timed build: 1.55 cores, busiest `snap`.

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
Host 1-minute load at the start of timed builds: 1.08 to 2.35.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 0.974 | 0.849..1.116 | 1.1 | INCONCLUSIVE | 5523.9 | 5731.7 |
| cold | raw | 10 | 1.007 | 0.922..1.098 | 1.15 | PASS | 5523.9 | 5542.6 |
| warm | ccache | 10 | 3.507 | 2.965..4.148 | 1.15 | FAIL | 57.2 | 15.7 |
| warm | raw | 10 | 0.010 | 0.008..0.012 | - | - | 57.2 | 5751.6 |
| edit-source-settled | ccache | 10 | 1.000 | 0.946..1.056 | 1.15 | PASS | 1831.6 | 1806.5 |
| edit-source-settled | raw | 10 | 1.024 | 0.926..1.131 | - | - | 1831.6 | 1691.1 |
| revert-source-settled | ccache | 10 | 2.217 | 1.883..2.611 | 1.15 | FAIL | 19.2 | 8.2 |
| revert-source-settled | raw | 10 | 0.011 | 0.010..0.012 | - | - | 19.2 | 1673.3 |
| edit-header-settled | ccache | 10 | 1.103 | 0.977..1.246 | 1.15 | INCONCLUSIVE | 6066.3 | 5716.2 |
| edit-header-settled | raw | 10 | 1.152 | 0.996..1.333 | - | - | 6066.3 | 5261.6 |
| revert-header-settled | ccache | 10 | 3.082 | 2.131..4.457 | 1.15 | FAIL | 58.4 | 16.8 |
| revert-header-settled | raw | 10 | 0.011 | 0.010..0.012 | - | - | 58.4 | 5449.1 |
| edit-source-immediate | ccache | 10 | 0.969 | 0.873..1.076 | 1.15 | PASS | 1735.3 | 1823.5 |
| edit-source-immediate | raw | 10 | 1.041 | 0.970..1.118 | - | - | 1735.3 | 1705.6 |
| revert-source-immediate | ccache | 10 | 3.152 | 2.365..4.200 | 1.15 | FAIL | 22.3 | 7.6 |
| revert-source-immediate | raw | 10 | 0.015 | 0.011..0.019 | - | - | 22.3 | 1658.5 |
| relocated | ccache | 10 | 3.671 | 3.131..4.303 | 1.15 | FAIL | 63.4 | 16.9 |
| relocated | raw | 10 | 0.012 | 0.011..0.013 | - | - | 63.4 | 5404.6 |

Per unit (12 units): divide the medians by 12.

## zlib

Acceptance: **FAIL** with 10 complete admitted pairs.

- cold vs ccache: Some(Inconclusive) (geomean 1.029, target 1.1)
- cold vs raw: Some(Inconclusive) (geomean 1.176, target 1.15)
- warm vs ccache: Some(Fail) (geomean 1.456, target 1.15)
- revert-source-settled vs ccache: Some(Inconclusive) (geomean 1.099, target 1.15)
- edit-header-settled vs ccache: Some(Inconclusive) (geomean 1.140, target 1.15)
- revert-header-settled vs ccache: Some(Fail) (geomean 1.406, target 1.15)
- edit-source-immediate vs ccache: Some(Inconclusive) (geomean 1.009, target 1.15)
- relocated vs ccache: Some(Inconclusive) (geomean 1.275, target 1.15)

Most unrelated activity around one timed build: 1.14 cores, busiest `amazon-ssm-agen`.

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
Host 1-minute load at the start of timed builds: 0.08 to 1.03.

| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |
|---|---|---|---|---|---|---|---|---|
| cold | ccache | 10 | 1.029 | 0.927..1.142 | 1.1 | INCONCLUSIVE | 656.4 | 625.8 |
| cold | raw | 10 | 1.176 | 0.999..1.385 | 1.15 | INCONCLUSIVE | 656.4 | 538.2 |
| warm | ccache | 10 | 1.456 | 1.360..1.559 | 1.15 | FAIL | 101.7 | 68.8 |
| warm | raw | 10 | 0.182 | 0.168..0.198 | - | - | 101.7 | 534.6 |
| edit-source-settled | ccache | 10 | 1.045 | 0.963..1.134 | 1.15 | PASS | 137.3 | 130.6 |
| edit-source-settled | raw | 10 | 1.048 | 0.894..1.229 | - | - | 137.3 | 142.3 |
| revert-source-settled | ccache | 10 | 1.099 | 0.867..1.395 | 1.15 | INCONCLUSIVE | 71.4 | 77.8 |
| revert-source-settled | raw | 10 | 0.681 | 0.558..0.831 | - | - | 71.4 | 116.0 |
| edit-header-settled | ccache | 10 | 1.140 | 0.992..1.311 | 1.15 | INCONCLUSIVE | 552.3 | 500.9 |
| edit-header-settled | raw | 10 | 1.199 | 1.067..1.347 | - | - | 552.3 | 471.8 |
| revert-header-settled | ccache | 10 | 1.406 | 1.156..1.709 | 1.15 | FAIL | 89.1 | 67.1 |
| revert-header-settled | raw | 10 | 0.208 | 0.168..0.259 | - | - | 89.1 | 432.1 |
| edit-source-immediate | ccache | 10 | 1.009 | 0.881..1.156 | 1.15 | INCONCLUSIVE | 137.7 | 136.3 |
| edit-source-immediate | raw | 10 | 1.178 | 0.989..1.402 | - | - | 137.7 | 115.3 |
| revert-source-immediate | ccache | 10 | 0.962 | 0.807..1.147 | 1.15 | PASS | 69.3 | 71.7 |
| revert-source-immediate | raw | 10 | 0.604 | 0.522..0.699 | - | - | 69.3 | 111.9 |
| relocated | ccache | 10 | 1.275 | 1.077..1.508 | 1.15 | INCONCLUSIVE | 94.3 | 74.0 |
| relocated | raw | 10 | 0.188 | 0.167..0.211 | - | - | 94.3 | 515.0 |
