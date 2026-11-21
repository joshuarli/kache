# DSLP-4: kache-config tests

Base: `4ef7293d9c4b3f679ba0168fe64da4e58f0a025b`.

## Scope and pyramid

Reviewed `config.rs`, `config/tests.rs`, `invocation.rs`, `policy.rs`, crate
features, consumer boundaries, `AGENTS.md`, and the existing mutation exclusions.
No production behavior, public API, dependency, toolchain, Make target, or CI
lane changes. Compiler/probe/C++ campaign sources are untouched.

| Tier | Selection and dependency | Cost on this Linux x86_64 host (4 of 8 CPUs) |
| --- | --- | --- |
| Edit | `cargo test -p kache-config --all-features --lib NAME`; pure policy and snapshots, or private temp files/program fixtures | Seconds including incremental build |
| Change/component | `cargo test -p kache-config --all-features`; TOML, file-write, precedence, path policy, invocation construction and one tiny child-process fixture | Baseline 112 tests, 0.02s test execution in coverage lane |
| Change/consumer | Required `make check`: lint, client dependency graph, resource guard and workspace library/integration tests | See final gate and timing below |
| Scenario | PR CI: Linux musl and macOS workspace tests plus glibc/macOS e2e; CLI exit codes and actual wrapper builds are owned by root integration tests | Not executed locally in this slice; retain CI |
| Full | `make pr`, full oracle/project/performance campaigns | Manual/on request; not run |

There is no literal `none` boundary in the base `AGENTS.md` Verification
section. Config owns TOML editing/parsing and invocation inputs, not CLI exit
codes, cache/store encoding, or wrapper/daemon wire serialization. Existing
CLI/wrapper, cache-key, store, and wire tests remain named there. Linker-key gap
DSLP-89 and ambient-environment isolation DSLP-88 are not silently claimed fixed.

Most tests are cheap pure/unit or temp-file component tests. Global config
legacy tests serialize through `config_path_lock`; retain them for compatibility
and public entry-point coverage. New tests use explicit `InvocationEnv::from_parts`
or pure provider closures, never mutable live environment reads. Do not move
format, policy, or concurrency checks into an e2e-only tier merely to reduce counts.

## Coverage

Installed user-local `cargo-llvm-cov 0.9.2`, `cargo-mutants 27.1.0` (locked), and
`llvm-tools-preview`; no installation failures. Rust/Cargo are the repository's
pinned nightly. Branch instrumentation is explicitly experimental (`--branch`).

Before command (substitute `after` for the second measurement):

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER -u TMPDIR -u KACHE_CACHE_DIR CARGO_BUILD_JOBS=4 \
  cargo llvm-cov -p kache-config --all-features --branch --html \
  --output-dir /home/ubuntu/repos/kache/.work/dslp-4-coverage-before \
  --ignore-filename-regex 'crates/(kache-core|kache-fs|kache-log|kache-shims)/|/config/tests.rs|/testing.rs'
cargo llvm-cov report -p kache-config \
  --ignore-filename-regex 'crates/(kache-core|kache-fs|kache-log|kache-shims)/|/config/tests.rs|/testing.rs'
```

The package selector on `report` matters: without it the root-package report is
empty, not 0% coverage. The table below is the nonempty scoped report. Inline
test modules in `config.rs`, `invocation.rs`, and `policy.rs` remain in LLVM's
file-level denominators; this is not a production-only percentage. The separate
config test file and optional testing helpers/dependencies are excluded equally
before and after. Added inline tests can change the denominator.

| File | Before lines | After lines | Before branches | After branches |
| --- | --- | --- | --- | --- |
| config.rs | 1344/1417 (94.85%) | 1348/1417 (95.13%) | 191/230 (83.04%) | 195/230 (84.78%) |
| invocation.rs | 283/295 (95.93%) | 337/342 (98.54%) | 16/16 (100%) | 16/16 (100%) |
| policy.rs | 29/29 (100%) | 52/52 (100%) | 0/0 (not measured) | 2/2 (100%, new test branches) |
| Total | 1656/1741 (95.12%) | 1737/1811 (95.91%) | 207/246 (84.15%) | 213/248 (85.89%) |

Baseline coverage wall time: 6.29s, 112 tests / 0.02s; after: 1.85s, 117 tests /
0.05s. Baseline was a cold instrumented build, so these wall times are not a
performance comparison. Gains in config are the no-op, missing-file and malformed
TOML editor paths. Invocation now exercises relative PATH rooting. Remaining
uncovered source regions include legacy process-global fallbacks, host status
diagnostics, unusual path/glob expansion cases, inaccessible-cwd fallback, and
consumer-only adapters. They are not claimed covered by this package-only run;
workspace/consumer tests and platform CI remain necessary. No behavior change is
justified from a coverage percentage alone.

## Bounded mutation

Two disjoint baseline cohorts, identical union after changes; deterministic
function selection, no random sampling. Each uses `-p kache-config --all-features`
and the existing `.cargo/mutants.toml`, baseline run, one worker, 4 jobserver
tasks, 10s tests and 300s build timeouts. `RUSTC_WRAPPER` is unset, never kache.

```sh
RE='Config::update_file_config_at|parse_layered_file_config_for|running_in_ci_with|env_truthy|InvocationEnv::var_os|InvocationEnv::validate|InvocationEnv::command|InvocationEnv::set_os|sanitize_trust_domain|normalize_key_env_vars'
taskset -c 0-3 env -u RUSTC_WRAPPER -u TMPDIR -u KACHE_CACHE_DIR \
  CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=2 cargo mutants -p kache-config --all-features \
  --re "$RE" --baseline run --gitignore true --timeout 10 --build-timeout 300 \
  -j 1 --jobserver-tasks 4 --output /home/ubuntu/repos/kache/.work/dslp-4-mutation-after
```

The first baseline excludes the last two names; the disjoint key cohort contains
only those names. Together: 38 mutants, **36 caught, 1 missed, 0 timeout,
1 unviable**, 24.08s + 18.33s wall time, including baseline builds. This is bounded
critical-contract evidence, not a claim about every possible crate mutant.
Targets cover layered TOML, safe CLI editing, independent CI policy, invocation
validation/environment/command construction, trust-domain boundaries and key-env
normalization. Existing exact-output tests killed every key/policy mutant.

The survivor sets the `NotFound` match guard to false in
`Config::update_file_config_at`. Existing read-error and changed-file tests never
try creating a missing file with unchanged/default values. Add a test of that
contract, not an exclusion. The unviable `InvocationEnv::command ->
Ok(Default::default())` fails E0277: `std::process::Command` has no `Default`.
No equivalent/dead/platform-unobservable survivor is being hidden by an exclusion;
`.cargo/mutants.toml` is unchanged. No timeout needs a longer rerun.

Coverage and mutation are scratch audit recipes, **not newly qualified CI lanes**.
No new CI/Make lane was invented: cross-platform recipe qualification and a
reproducible empty-selection negative control would be required first. Manual
fault injections below establish test sensitivity, not an original runtime defect.

## False passes and fault witnesses

`update_file_config_leaves_an_unchanged_file_byte_for_byte` changed `cc` from
`None` to `Some(default)` and asserted only the reloaded value. It never compared
bytes and never took the no-op branch (zero baseline coverage on its early return).
Correct the fixture to pass identical values and compare the original file bytes,
including comments and an unknown table. Keep the separate changed-fields test.

Five added tests plus the corrected no-op assertion were each run against a
hand-edited production fault. All six produced **0 passed, 1 failed, exit 101**,
not a compilation failure; all faults were restored before positive runs.
Faults were applied together; each test filters to exactly one case and reaches
its own listed changed expression (no fault relies on another test running).

| Test | Injected fault and observed failure |
| --- | --- |
| `update_file_config_leaves_an_unchanged_file_byte_for_byte` (corrected) | Change no-op `existing.is_some()` to `is_none()`: serializer rewrites file; exact bytes lose comments/unknown table |
| `update_file_config_creates_a_missing_file_even_when_values_match` | Make `NotFound` match guard false (actual survivor): unwrap fails with contextual missing-file read error |
| `update_file_config_rejects_invalid_existing_bytes_without_writing` | Skip existing-TOML validation: unwrap_err receives `Ok(())`; test also checks invalid UTF-8, exact error context and unchanged bytes |
| `rewriting_a_snapshot_removes_duplicate_keys_without_changing_its_clone` | Retain every prior variable in `set_os`: exact list contains stale duplicates, not just EMPTY and replacement; also checks clone isolation, last-wins and missing key |
| `relative_search_paths_are_rooted_in_each_invocations_directory` | Filter to absolute PATH entries rather than rooting each in invocation cwd: `None` instead of exact private executable path; both client roots and missing executable are asserted (Unix fixture) |
| `ci_truth_values_are_case_insensitive_but_not_whitespace_or_arbitrary_strings` | Remove ASCII case folding: false instead of true for `GITHUB_ACTIONS=TrUe`; table covers all three providers, 1/TrUe/YES, empty/0/false/spaces/arbitrary values and false earlier providers followed by true CI |

Positive scoped check after restoration:

```text
test result: ok. 117 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
Doc-tests kache_config: 0 tests
real 1.73
```

After mutation uses the unchanged 38-mutant union: **37 caught, 0 missed,
0 timeout, 1 unviable**, 27.55s wall time. No new exclusion needed. All critical
survivors in the bounded cohort are dispositioned; the only baseline survivor
is now caught. Including both before runs, after run and four-core cap, the
maximum CPU-affinity budget is under five aggregate core-minutes; well under
the ten-minute mutation-trial budget. No unresolved timeout or platform-only
survivor remains. Linux x86_64 executed; macOS-specific behavior is not locally
executed and needs the existing PR CI lane.

Tools: rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`, Cargo
`1.102.0-nightly (c07ac2c4d 2026-11-06)`, cargo-llvm-cov `0.9.2`, cargo-mutants
`27.1.0`, LLVM `23.1.5-rust-1.102.0-nightly`. No runtime fix or new out-of-scope
defect was demonstrated. DSLP-4 is removed from the backlog following prior slices.

## Workspace cost and final gate

Before: cold `make test` 382.93s; immediately repeated warm `make test` **269.79s**.
Both used system temp, unset cache/wrapper environment, `taskset -c 0-3`,
`CARGO_BUILD_JOBS=4`, and `/usr/bin/time -p`. After is measured from the emitted
`make test` recipe through process exit inside the required `make check`, so
lint/client-graph/resource-guard time is excluded and the same workspace tests
are not redundantly rerun. No competing audit/install process runs during this
after measurement. This is an observed host budget, not a product benchmark.

The first gate attempt was interrupted by the timing/log wrapper's strict UTF-8
decoder when an existing test emitted byte `0xff`. Lint, client graph and resource
guard had completed, but that attempt is **not a successful gate or timing**.
The wrapper was changed to copy raw bytes; no repository code was changed in
response. The complete rerun below is the gate evidence.

After warm `make test`: **269.99s**, +0.20s (+0.074%). Within both +20% and
+30s limits; this small difference is not evidence of a real speed change.
Complete `make check`: **270.61s, exit 0**, including `make lint`, client checks,
resource-guard self-check and all workspace/consumer tests. Only documentation
of the measured result was edited after this gate; no test inputs changed.

```text
test-resource guard: all checks passed
test result: ok. 117 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
MAKE_CHECK_EXIT=0
MAKE_CHECK_SECONDS=270.61
WARM_MAKE_TEST_AFTER_SECONDS=269.99
```

Audit outputs are under the checkout's ignored `.work/dslp-4-*`; evidence above
is retained here rather than requiring those disposable HTML/log files. No PR
CI or actual performance campaign is represented as locally passing.
