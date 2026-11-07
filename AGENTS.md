# Agent notes

`make check` is the pre-PR verification gate.

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
