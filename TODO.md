Prefix: KACH
Next: 24

### KACH-21 · P2 · Linux fixture scenarios run in no gate

- **Area:** Tests
- **Where:** `.github/workflows/ci.yml`, `Dockerfile:12`, `scenarios/e2e-cc-cross-gcc/scenario.toml:7`, `scenarios/e2e-cc-bench-flags-gnu/scenario.toml:15`
- **Problem:** CI runs `make e2e` and `make e2e-negative-control` only on macOS. The musl builder image has no `cmake`, `ninja`, `gcc` or the riscv64/arm cross compilers, so `--deny-missing-tools` would fail every supported fixture that needs them, and the two Linux-only fixtures (`e2e-cc-cross-gcc`, `e2e-cc-bench-flags-gnu`) never execute anywhere. Linux-specific remapping (`/proc/self/cwd`), hardlink and reflink restore paths, and the Linux delegation checks are covered only by unit and integration tests.
- **Recommendation:** Add `cmake`, `ninja` and `gcc` to the image, decide whether the cross-gcc fixture belongs in the gate (the Alpine image cannot provide glibc cross compilers), and run both e2e targets inside the image in the `linux-musl` job.
- **Fix with:** none
- **Effort:** M

### KACH-22 · P3 · Changed-line mutation testing runs in no gate

- **Area:** Tests
- **Where:** `Makefile:pr`, `Makefile:mutants-diff`, `.github/workflows/ci.yml`
- **Problem:** `make mutants-diff` exists and AGENTS.md relies on it, but no workflow runs it, so surviving mutants in key derivation, publication and restore branches are found only when a developer runs `make pr` with cargo-mutants installed.
- **Recommendation:** Run `make mutants-diff` against the merge base on pull requests that touch Rust, with the pinned cargo-mutants version.
- **Fix with:** none
- **Effort:** S

### KACH-23 · P4 · Fixture sources track build outputs every e2e run deletes

- **Area:** Tests
- **Where:** `scenarios/e2e-cc-memo-after-entry/source/build/`, `scenarios/e2e-cc-cl-same-content-sources/source/build/`
- **Problem:** #1392 committed object files and a linked `tool` under two fixtures' `build/` directories. Each fixture's clean step removes them, so every `make e2e` leaves the worktree dirty with deleted tracked files.
- **Recommendation:** Remove the tracked outputs and ignore those directories.
- **Fix with:** none
- **Effort:** S
