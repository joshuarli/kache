# Agent notes

`make check` is the pre-PR verification gate.

## C/C++ parity campaign

When executing the C/C++ parity campaign, follow
[C_CPP_PARITY_CAMPAIGN.md](C_CPP_PARITY_CAMPAIGN.md). It defines the pinned
ccache reference, raw-compiler correctness oracle, macOS/Linux matrix, ordered
work packages, evidence requirements, and unattended resume loop.

Remote storage is deferred to that plan's later roadmap. Prioritize local C/C++
compatibility and correctness; older coverage notes do not override this scope.
The campaign supplements the verification and PR rules below.

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
