Prefix: KACH
Next: 24

### KACH-21 · P2 · Linux fixture scenarios run in no gate

- **Area:** Tests
- **Where:** `.github/workflows/ci.yml`, `Dockerfile:12`, `scenarios/e2e-cc-cross-gcc/scenario.toml:7`, `scenarios/e2e-cc-bench-flags-gnu/scenario.toml:15`
- **Problem:** CI runs `make e2e` and `make e2e-negative-control` only on macOS. The musl builder image has no `cmake`, `ninja`, `gcc` or the riscv64/arm cross compilers, so `--deny-missing-tools` would fail every supported fixture that needs them, and the two Linux-only fixtures (`e2e-cc-cross-gcc`, `e2e-cc-bench-flags-gnu`) never execute anywhere. Linux-specific remapping (`/proc/self/cwd`), hardlink and reflink restore paths, and the Linux delegation checks are covered only by unit and integration tests.
- **Recommendation:** Add `cmake`, `ninja` and `gcc` to the image, decide whether the cross-gcc fixture belongs in the gate (the Alpine image cannot provide glibc cross compilers), and run both e2e targets inside the image in the `linux-musl` job.
- **Fix with:** none
- **Effort:** M

### KACH-20 · P3 · Removed miss diagnostics still advertised and half-present

- **Area:** Dead code
- **Where:** `src/wrapper.rs:7075`, `src/wrapper.rs:7212`, `src/wrapper.rs:2441`, `src/cli.rs:1777`, `src/cli/miss_diagnosis.rs:54`, `src/miss_chain.rs`, `docs/commands/reference.mdx:224`, `docs/commands/reference.mdx:241`, `docs/commands/reference.mdx:251`, `docs/how-it-works/cache-key.mdx:228`, `docs/getting-started/quick-start.mdx:68`, `docs/getting-started/troubleshooting.mdx:59`
- **Problem:** e5d57ab removed `explain_miss` ("no miss diagnostics") by turning its gates into `if true`/`if false`, so key-group digests, extern digests, unit ids and env-dependency hashes are never recorded. `kache explain` still prints "set explain_miss = true under [cache], then rebuild" for every miss, and six doc passages describe the checkout comparison, dependency cascade and env hints as available behind that setting. A config file with `explain_miss = true` is accepted silently and changes nothing. About 2.4k lines of `miss_chain` analyze only events written by older versions. The `if false` around `arm_too_new_guard` is the same leftover from the removed input guard.
- **Recommendation:** Delete the neutered branches, the recording plumbing they guard, the `dependency_chain`/`checkout_comparison`/hint output and `miss_chain`, and the doc passages; surface the JSON output change.
- **Fix with:** delete
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
