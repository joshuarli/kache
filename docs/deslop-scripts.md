# Deslop: scripts

Types, Tests and Dead code for DSLP-12, DSLP-39 and DSLP-66, scope `scripts/`
(52 files: 31 top-level, 11 in `bench/`, 9 in `bench/tests/`, 1 in `ci/`; about
6.6k lines of Python and shell at top level plus 4.3k in the `bench` package
and its tests). Base `11cbdba8`, branch `deslop/k2`, macOS arm64, Python
3.15.1. Tools were run from the stuff pypi mirror into a scratch
`UV_CACHE_DIR` with `uvx --index-url https://pypi.17166969.xyz/simple`: ruff
0.17.5, ty 0.0.104, vulture 2.16, coverage.py. `shellcheck` is installed.
Nothing in the repository's own configuration changed: there is no
`pyproject.toml`, ruff or ty config for `scripts/`.

## DSLP-12 Types

Escape hatches (`rg -c 'cast\(|# type: ignore|# ty: ignore|\bAny\b|# noqa'`):
4 before, 4 after, all `# noqa: E402` on the four one-line entry scripts that
insert `scripts/` into `sys.path` before importing `bench`.

Baseline `ty check scripts`: 30 diagnostics. Candidates measured:

| Candidate | Count | Decision |
| --- | --- | --- |
| ty default rules | 30 | **Adopted as the profile.** After the fixes below: 5 on this host, 4 with `--python-platform linux`. |
| ty, `os.sched_getaffinity` (`bench-contention.py:99`) | 1 | Rejected as a defect: the runner is Linux-only by design (`make bench-contention` builds and runs on Linux). The diagnostic is the host platform; it disappears with `--python-platform linux`. |
| ty, dict-literal inference in tests (`bench/tests/test_short.py:397` 3 diagnostics, `test-bench-contention.py:681` 1) | 4 | Rejected: ty types a literal mixing ints and nested dicts as a union and then objects to the mutation the test performs. Annotating them needs `dict[str, Any]`, which adds hatches to remove a false positive. |
| ruff `ANN` | 813 | Rejected: stdlib-only scripts without annotations; far above one false positive per 5k lines and no defect behind them. `ANN401`, `PGH003`, `TC`, `RET`, `F841`: 0. |
| ruff `B` | 4 | Rejected: B904 (`bench-contention.py:425`), B007 (`crates-io.py:109`) and two B905 `zip()` without `strict=` are style; `strict=True` would change what mismatched input does. |
| ruff `UP`, `SIM`, `PLR`, `E501` | 1, 13, 71, 259 | Rejected as style. |
| ruff `F401` | 2 | **Fixed** under Dead code. |
| `shellcheck` default severity | 5 | Rejected: 4x SC1007 in `test-unavailable-proc-labels.sh` (`RUSTC_WRAPPER= cargo ...` clears the variable on purpose) and SC2046 in `fetch-ccache.sh` (`set -- $(...)` splits a three-field pin on purpose). |

Fixes (25 diagnostics removed, 30 to 5, each witnessed by ty going clean for it and the
suites below passing):

1. Six scripts passed `spec_from_file_location(...)` straight to
   `module_from_spec` and dereferenced `spec.loader`, though both can be
   `None` (18 diagnostics): `test-ci-changes.py`, `test-crates-io.py`,
   `test-bench-contention.py`, `test-mutation-diff.py`,
   `bench/tests/test_edit_loop.py`, `bench/gate_local.py`. Each now raises
   `ImportError` naming the path. One shared helper was rejected: the
   production caller (`gate_local.renderer_from`) loads the report module from
   the staged copy of the commit being measured, which may predate a new helper.
2. `bench-contention.py`: `server_log.close()` on a value that is `None`
   unless the backend is sccache (now behind a check); `shutil.rmtree(**kwargs)`
   with a dict that matched no overload (now explicit `onexc`/`onerror`
   branches). 2 diagnostics.
3. `bench/short.py`: `records` was a list of heterogeneous dict literals, so
   `records[0]["result"]["git_ref"]` subscripted an inferred `int`/`float`.
   `SampleRecord` (a `TypedDict`, `result: dict[str, object]`) now declares
   the persisted shape, and `took` is annotated where ty read an empty
   literal. 3 diagnostics. The `ended_monotonic_s`/`started_monotonic_s`
   fields are write-only inside the repository (see Dead code).
4. `test-bench-contention.py`: `child.stdout.readline()` on an Optional pipe
   (narrowed with `assert`), and `sample < context` where `context` can be
   `None` (the comparison now requires `context is not None`, which is what the
   loop's data already guaranteed). 2 diagnostics.

Suites run after the changes, all exit 0: `python3 -m unittest discover -s
scripts/bench/tests -t scripts` (95 tests), `scripts/test-bench-contention.py`
(29), `test-ci-changes.py` (10), `test-crates-io.py` (10 before the tests
below), `test-version-consistency.py` (4), `test-mutation-diff.py` (12).

## DSLP-39 Tests

Costs on this host (load 25 to 50): the `bench` suite 4 to 9 s, contention 6
to 9 s, `test-mutation-diff.py` 34 to 78 s, the rest under 3 s. **None runs in
CI**: `.github/workflows` call `make test`, `make lint`, `make e2e`,
`make cc-oracle` and their negative controls only, and `make test` is
`cargo test`. Four suites have no `make` target at all
(`test-ci-changes.py`, `test-crates-io.py`, `test-mutation-diff.py`,
`test-version-consistency.py`). Filed below.

Coverage, branch, product code only (`coverage run --branch --source=scripts`
over the five suites above plus `test-mutation-diff.py`, tests omitted):
2,849 statements, 60% line+branch overall.

| Script | Cover | Contract | Guarding tests | Disposition |
| --- | --- | --- | --- | --- |
| `bench/stats.py`, `bench/design.py`, `bench/report.py`, `bench/short.py`, `bench/phases.py`, `bench/inference.py` | 92, 84, 88, 84, 88, 77% | Sample validity, paired verdicts, cold-reuse design, the perf-gate comment | `bench/tests/test_short.py`, `test_inference.py`, `test_phases.py`, `test_gate_local.py` | Asserted |
| `bench/gate_local.py` | 53% | Stage the merge base and head, compare | `bench/tests/test_gate_local.py` | Partly: the git/cargo staging paths run only by hand |
| `bench-contention.py` | 61% | Six overlapping Cargo jobs, admission and counts | `test-bench-contention.py` (29), `bench/tests/test_contention_counts.py` | Asserted for the pure parts; the Linux-only runner body is by hand |
| `mutation-diff.py` | 79% | Which lines enter the mutation lane | `test-mutation-diff.py` (12) | Asserted |
| `ci-changes.py`, `check-version-consistency.sh` | 95% / by `test-version-consistency.py` | Which CI groups a PR needs; release version agreement | `test-ci-changes.py`, `test-version-consistency.py` | Asserted |
| `crates-io.py` | 29% before, see below | Discovery, ordering, config, **publish** | `test-crates-io.py` | `cmd_publish` was `none`: **test added** |
| `volume-shards-check.py` | 0% | Real compiler routing on bind mounts | none | Needs a harness (it drives a real compiler and volumes); filed |
| `cc-project-autotools.py`, `cc-*.sh`, `with-test-resources.sh` | 0% / none | C/C++ project oracles and resource guards | `make test-resource-guard-check` for the guard; the oracles run in their own CI lanes | Not unit-testable here |

New tests (`test-crates-io.py`, class `PublishGateTests`, 6 tests): the version
mismatch refusal publishes nothing; publishing goes in dependency order and
waits for each index entry; an already-published version is skipped; an
unregistered crate stops the run with exit 1 before any publish; a dry run
publishes and waits for nothing; an entry that never reaches the index fails
the run before the next crate.

Injected fault (critical decision: never republish a published version). In
`scripts/crates-io.py` `cmd_publish` change `if client.version_exists(crate.name,
version):` to `if False and client.version_exists(...):`;
`python3 scripts/test-crates-io.py` exits 1 with
`FAIL: test_an_already_published_version_is_not_published_again`
(`AssertionError: 1 != 0`). Restored from a saved copy; `cmp` exit 0, the
suite passes (16 tests), `git status` shows only the new test file.

Needs environment: mutation testing. `mutmut` 3.8.1 is on the mirror but loads
importable modules from `source_paths`; the entry scripts here have hyphenated
file names and are loaded by path, so a run needs a copied tree with renamed
modules, which this lane did not set up. Python coverage is recorded above
(scratch, not a CI lane). Warm `make test` is unchanged (no Rust tests added
here).

## DSLP-66 Dead code

Tools and results: ruff `F401`/`F841` found two unused `import importlib.util`
(`bench/tests/test_gate_local.py:6`, `bench/tests/test_short.py:6`), **removed**
and both suites pass; vulture 2.16 at `--min-confidence 60`: 1 finding,
`bench-contention.py:947` `extent_count`, a `ctypes.Structure` field assigned
for the `FS_IOC_FIEMAP` ioctl (`:931`, `:948`), so it is read by the kernel and
allowlisted by reason, not removed. After the TypedDict change vulture also
reports `started_monotonic_s`, `ended_monotonic_s`, `cold_reused` of
`SampleRecord` at 60%: TypedDict fields are not attribute reads (false
positive), but see the first row below.

Reference scan (`rg -F <name> --hidden`, outside `.git`, `target`, `.work` and
the file itself) found eight scripts nothing in the repository calls: no
workflow, Makefile target, doc or other script names them. None is deleted: a
script can be run by hand or by another repository, and several implement
release or CI policy.

| Script | Evidence | Disposition |
| --- | --- | --- |
| `ci-changes.py`, `test-ci-changes.py`, `ci/resolve-version.sh` | Only the test references `ci-changes.py` and `resolve-version.sh`; `.github/workflows` do not call them. The groups they decide (`nix`, helm, perf-gate) describe CI the workflows no longer have | Consumer question: is this CI-split logic retired? |
| `require-ci-green.sh` | Header says it gates irreversible publishes on CI; no workflow, recipe or script calls it | Consumer question, security-relevant |
| `ci-monitor.sh` | 584 lines; default repo is `Zondax/kunobi-frontend`, not this repository | Consumer question |
| `check-docs.sh` | No caller | Consumer question |
| `workspace-members.sh`, `check-coverage-scope.sh` | The first is called by no recipe; the second only by the first | Consumer question: coverage-scope gate with no coverage lane (DSLP-87) |
| `volume-shards-check.py` | No caller | Consumer question |
| `test-unavailable-proc-labels.sh` | No Makefile or workflow target; the Rust test it selects exists (`src/daemon_execution.rs:675`, module `daemon::execution`) | Live but unwired; consumer question |
| `started_monotonic_s`, `ended_monotonic_s` (`bench/short.py`, `bench-contention.py:1198`) | Written into `samples.json`; no reader in the repository | Artifact fields, possibly consumed outside: consumer question |

Platforms: scripts were analysed on macOS; Linux-only code
(`bench-contention.py`, `test-unavailable-proc-labels.sh`) was read, not run,
and `make bench-contention` was not run (Full tier).
