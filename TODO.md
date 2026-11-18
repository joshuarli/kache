Prefix: KACH
Next: 27

### KACH-26 · P1 · Reach ccache-level local C/C++ compatibility

- **Area:** Compiler compatibility, correctness, tests
- **Plan:** [C_CPP_PARITY_CAMPAIGN.md](C_CPP_PARITY_CAMPAIGN.md)
- **Status:** Planned; implementation and differential baseline have not started.
- **Goal:** Execute the unattended campaign using pinned ccache as the compatibility/reuse oracle and raw compilers as correctness ground truth, with native macOS and Linux coverage.
- **Scope:** Response files and flags, invalidation, equivalent-preprocessing reuse, complete output sets, conditional PCH/module support, host integration, local reliability, and measured performance. Remote storage stays on the later roadmap.
- **Coordination:** CCP-00/CCP-08 cover the relevant KACH-21/KACH-22 work; retain those entries until their gates actually run. Start at CCP-00 and persist evidence in the campaign ledger specified by the plan.
- **Done when:** Every final acceptance criterion in the plan passes on the same integrated revision; safe passthrough, missing tools, and unexecuted lanes do not count as cached parity.

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

### KACH-24 · P3 · Make release-tag advice match the dispatch-only release workflow

- **Area:** release
- **Where:** Makefile:279-281, .github/workflows/release.yml:4,38
- **Problem:** make release-tag pushes a tag and says that starts the release pipeline, but release.yml declares only workflow_dispatch and requires the default branch, so following the Makefile publishes nothing and leaves an unused tag.
- **Recommendation:** Replace the release-tag target and its message with the actual procedure: gh workflow run release.yml on the default branch, then verify the run's head SHA and the release-<sha> prerelease it creates; remove the tag push.
- **Fix with:** make -n release-tag no longer pushes a tag, and its text names gh workflow run release.yml; the release workflow is unchanged.
- **Effort:** S

### KACH-25 · P3 · Point the documentation install pages at this fork's releases

- **Area:** docs
- **Where:** docs/index.mdx, docs/getting-started/installation.mdx
- **Problem:** The README now installs joshuarli/kache release-<sha> builds, but docs/index.mdx and docs/getting-started/installation.mdx still lead with cargo install kache --locked and cargo-binstall, which install Kunobi's crates.io distribution rather than this fork, along with Homebrew, APT, AUR, mise, and Nix packages that are also upstream.
- **Recommendation:** Lead those pages with this fork's release archives and source install (both kache and kache-engine), and keep the upstream channels only as attributed alternatives, or drop them.
- **Fix with:** rg -n "cargo install kache --locked|cargo-binstall" docs/ finds no primary install instruction that selects the upstream distribution.
- **Effort:** S


### KACH-23 · P4 · Fixture sources track build outputs every e2e run deletes

- **Area:** Tests
- **Where:** `scenarios/e2e-cc-memo-after-entry/source/build/`, `scenarios/e2e-cc-cl-same-content-sources/source/build/`
- **Problem:** #1392 committed object files and a linked `tool` under two fixtures' `build/` directories. Each fixture's clean step removes them, so every `make e2e` leaves the worktree dirty with deleted tracked files.
- **Recommendation:** Remove the tracked outputs and ignore those directories.
- **Fix with:** none
- **Effort:** S

