# Agent notes

`make check` is the pre-PR verification gate.

## Verification

`make clean` removes this checkout’s build output in `./target`. It refuses symlinks and ordinary files at those paths.

- Edit (config): `cargo test -p kache-config --all-features --lib NAME`; seconds including incremental build.
- Change (config component): `cargo test -p kache-config --all-features`; 117 tests, 0.05s execution / 1.73s total
  on Linux x86_64, capped to 4 of 8 CPUs. Explicit invocation snapshots and private temp files; no live-env tests added.
- Change (consumers): `make check` is the pre-PR gate: lint, client graph, resource guard, workspace tests.
  Warm `make test` 269.79s → 269.99s here (4 CPUs); costs and fault proofs: `docs/deslop-kache-config-tests.md`.
  Use system `TMPDIR`, unset `KACHE_CACHE_DIR`: checkout-local temp roots / ambient cache roots fail tests (DSLP-88).
- Scenario: PR CI runs workspace `make test` (`ci.yml`, Linux musl and macOS), `make lint` (`lint.yml`), and e2e/oracles.
  macOS jobs run only while the repository variable `KACHE_MACOS_CI` is not `off`
  (`gh variable set KACHE_MACOS_CI --body on` re-enables them); each macOS negative control is its own job.
  `make test` runs every test binary (`--no-fail-fast`). The oracle fails a case whose compiler, ccache or kache
  invocation runs past 300 s instead of stalling the run.
- Full: performance/project/oracle campaigns and `make pr` are manual/on request; not run in the config tests slice.
- CLI flags and exit codes: `tests/cli_commands_test.rs` (`argument_errors_exit_two`); wrapper invocation:
  `tests/integration_test.rs` (`test_wrapper_hello_world`, `test_rust_restored_outputs_allow_build_without_wrapper`).
- Cache key (a wrong hit silently serves a stale artifact): `tests/cache_key_underkeying_test.rs`, `src/cache_key/tests.rs`. Known gap: linker input files named by
  `-C link-arg` (DSLP-89).
- On-disk store format, integrity, concurrent writers: `crates/kache-store/src/store/tests.rs`; entry schema in `crates/kache-format`.
- Wrapper/daemon wire protocol: `crates/kache-protocol/src/execution.rs`
  (`native_strings_round_trip_without_loss_and_reject_nuls`, `execution_diagnostics_reject_legacy_and_foreign_protocols`).
- Performance workload: `make perf-gate-local` (HEAD against its merge base) and `make bench-short PROJECT=<project>`
  (cold and warm builds); neither ran in Ground.
- Review findings and scope: `docs/deslop-diagnose.md`.
- No vendored trees. DSLP-4 installed `cargo-llvm-cov 0.9.2` and `cargo-mutants 27.1.0`; scoped scratch recipes
  and coverage limits are in the audit. No new coverage/mutation CI lane is qualified by that local audit.
- Components, one deslop slice each (about 5k to 19k lines; scopes in `TODO-DESLOP.md`): `crates/kache-config`,
  `kache-e2e` (2), `kache-fs`, `kache-store` (3), the small `crates/kache-*`, top-level `src` module groups,
  `src/cache_key`, `src/compiler`, `src/wrapper`, `src/cli`, `tests` (2), `scripts`, `test-projects`, build and docs.

## C/C++ parity campaign

Roadmap: [docs/roadmap.md](docs/roadmap.md) holds all current and future work;
its Status is the resume point. The C/C++ campaign supplements the verification
and PR rules below.

## Before `gh pr create`

```sh
make check
```

That is the clippy gate, the client-crates graph, the resource-guard self-check, and the workspace tests. Do not open the PR until it exits 0.

For end-to-end scenario verification, `make e2e` runs gate scenarios with `--deny-missing-tools` to ensure required compilers and tools are present.

`make pr` runs `make check` plus `make mutants-diff` against `origin/main`. If you use it:

```sh
cargo install --locked cargo-mutants --version 27.1.0
```

Do not invoke `cargo mutants` with `RUSTC_WRAPPER=kache`. The Makefile clears it (unless `KACHE_SELF_HOST=1`). Hand-running mutants with a dogfood wrapper wraps kache in itself and the unmutated baseline dies.

## Verification tiers

Run the smallest tier that covers the change. CI is the cross-platform gate:
it runs a superset of every local lane below on every PR.

| Tier | What | When | Cost on an M1 |
| --- | --- | --- | --- |
| Edit | The regression: `cargo test -p kache --lib NAME` or `cargo test --test BINARY -- NAME`. Apply the mutation by hand and watch it fail | While iterating | Seconds to a minute |
| Change | `make check`. When the change touches C/C++ keying, restore, outputs or diagnostics, also `make cc-oracle CASE=ID` for the cases it affects, or `make cc-oracle` and `make cc-oracle-negative-control` when it is broad | Before `gh pr create` | About 12 min for `make check`; the full cohort with its negative controls adds about 8 min (both estimated from one run) |
| Scenario | CI on the PR: the oracle and its negative controls on macOS and four Linux cells, `e2e` and `e2e-negative-control`, and the Linux musl workspace tests | Every PR. Merge only when green | Not on the local host |
| Full | `make cc-oracle-full`, `make cc-project-zlib`, `make cc-project-llvm`, `make cc-bench`, `make pr` | Only on request | Tens of minutes to hours |

Do not rerun the oracle cohort, `make e2e` and a Linux container locally before
every push. CI runs them on more cells, and a local container that runs only
the oracle misses the Linux workspace tests. When a CI job fails, reproduce
that job's target in its cell, for Linux musl in the `kache-builder` image the
way `ci.yml` runs it, and rerun only what failed.

## How mutants die here

Prefer tests and deleting dead branches over skip annotations.

- `if !s.is_empty() { print!("{s}") }` — printing `""` is a no-op, so `delete !` is missed. Call a helper that already has empty vs nonempty tests (`replay_cached_diagnostics`).
- `a && b` / `a || b` — one test where the left is false, one where the right is false.
- Named size constants built with `*` (`5 * 1024 * 1024 * 1024`) — do not `assert_eq!` against that same constant. Use an independent literal (`5 << 30`).
- `u64::try_from` on a value that is already `u64` — Linux clippy `-D warnings`. macOS clippy does not type-check `#[cfg(target_os = "linux")]`.
- Unix-only test helpers under `-D warnings` on Windows are unused. Mark them `#[cfg(unix)]`.
- macOS-only helpers Linux never calls: `#[cfg(any(test, target_os = "macos"))]`.
- `Drop` / file locks — a second process must acquire after the first drops.
- New event-log fields — grep `event.schema` in the same commit and bump the tests.
- A test that passes is not a test that kills the mutant. Apply the mutation by hand, watch the test fail, then revert. A stale-record test passed under both `==` and `!=` because a single client refreshed its own record before the branch was ever reached; it took two clients to make the branch observable.
- Moving code without changing it still marks every moved line as changed, so the lane mutates lines your change never touched. Wrapping a 200-line block in a `loop` cost six survivors, four of them in code that predated the branch. Extract a function instead: the call site is one changed line and the body is none.
- Tests that read the live process environment race the rest of the suite, which mutates it (`set_env_for_test`). Take a snapshot and pass it in, or the test passes alone and fails in `make check`.

## PRs

Every PR targets `main`. Do not stack PRs on each other's branches. After one merges, rebase the others onto `main`.

Work in a git worktree. Do not mix unrelated dirty-tree WIP into the PR.

People keep running `cargo`. There is no `kache build`. Caching is `kache init` / `RUSTC_WRAPPER`.
