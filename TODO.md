Prefix: KACH
Next: 28

### KACH-27 · P2 · Linux e2e diverges on cache reuse for relocated and probe invocations

- **Area:** Compiler compatibility, correctness, tests
- **Status:** Open; discovered by the Linux e2e gates added for KACH-21 (PR #26).
- **Found by:** `make e2e` on the new `e2e-linux-glibc` and musl `linux-musl` jobs; CI runs 44241684053 and 44244430288. Reference artifacts from the first run: artifacts `e2e-results-glibc-{x86_64,ARM64}`, `e2e-results-musl-{x86_64,ARM64}` on run 44241684053.
- **Flakiness:** the failing set is not stable run-to-run on musl — `e2e-cc-cl-xclang-deps` failed on x86_64 in the first run and passed in the second, and `e2e-cc-cl-same-content-sources` failed on aarch64 in the second after passing in the first — consistent with a probe-resolution race rather than fixed per-fixture breakage. `e2e-rust-transitive-native` fails deterministically on both Linux lanes; the cmake fixtures fail deterministically on musl and pass on glibc and macOS.
- **Findings (macOS passes every one of these):**
  1. **Relocation adds one wrap miss per relocation on Linux.** `e2e-rust-transitive-native` fails `assertions.relocate.max_misses` (`<= 0`, actual 1) on both Linux lanes: the `native` crate's `build_script_run` at the relocated `OUT_DIR` misses with a fresh key while its outputs are byte-identical (store duplicates), so the portable-`OUT_DIR` check classifies the same bytes differently on Linux than on macOS. Supplements the cc-crate `#1182` contract — treat as a missed-reuse finding, not wrong output.
  2. **The prebuilt musl LLVM clang (23.1.5) does not hit probe-keyed work across cmake's random `TryCompile` directories.** The `CMakeCXXCompilerABI.cpp` probe key changes per configure (`e2e-cmake-ninja-flagset`, `e2e-cmake-out-of-tree`, `e2e-cmake-file-macro-oot` relocate phase: `max_misses` `<= 0`, actual 1), which trips the cmake fixtures on the musl image. 2026-11-18 trace (debug kache, `KACHE_LOG_FILE=trace`; the harness's `KACHE_E2E_KEYTRACE` filter string is not a level `kache_log` parses, so it logs nothing): the only key inputs that differ between configures are the TryCompile prefix-map root, already mapped to `/proc/self/cwd`, and `depinfo ... target=CMakeFiles/cmTC_<random>.dir/...o`. That target comes from CMake's `-MT` and appears in the restored `.d`, so keying on it is correct; the miss is not stale-output risk. Still open: why the same probe is not counted as a relocate miss on glibc gcc and macOS. Options are to rewrite the `-MT` target in restored dep-info or to exempt CMake's TryCompile probes in the fixture assertion.
  3. **Fixed by PR #35: not a kache defect.** `e2e-c-shims` recorded zero cold entries because the musl image exports `CC=/opt/llvm-musl/bin/clang` and the harness removed `CC` only from the fixture's declared env, so `make` ran clang directly. `e2e-cc-cl-xclang-deps` on musl x86_64 is a separate, intermittent signature.
  4. `e2e-rust-cross-tree` requires a hardlink carrier rlib; the Linux runners clone instead (verified with the store on the checkout filesystem), so the fixture documents `KACHE_E2E_SKIP_HARDLINK_CHECK=1` for this substrate — the FD-stability contract stays armed wherever a hardlink carrier exists.
- **Impact:** The new Linux gates are advisory (`continue-on-error`) until this closes; `make e2e` exits nonzero on both Linux lanes. No observed wrong output — all failing assertions are reuse accounting.
- **Unblock:** Root-cause each divergence on a Linux host (or in a Linux container) with instrumented kache builds; fix kache keying/portability detection or the probe path as root cause demands; then the four fixtures pass on both Linux lanes without `KACHE_E2E_SKIP_HARDLINK_CHECK=1`, the advisory flags are removed, and the gates become required.
- **Local repro (2026-11-18, OrbStack):** `docker build --platform linux/arm64 -f Dockerfile -t kache-builder:aarch64-unknown-linux-musl .` then `docker run --rm --init --platform linux/arm64 -e CI -e TMPDIR=/kache/tmp -e KACHE_E2E_SKIP_HARDLINK_CHECK=1 -v <checkout>:/kache -w /kache kache-builder:aarch64-unknown-linux-musl sh -ec 'mkdir -p /kache/tmp && make e2e'` reproduces the arm64 set: `e2e-c-shims`, `e2e-cc-cl-same-content-sources`, `e2e-cc-cl-xclang-deps`, the three `e2e-cmake-*`, `e2e-rust-transitive-native`. The LLVM-prerelease and gluon-ninja pins introduce no new failing assertion.
- **Fix with:** campaign scope (CCP-02/CCP-03 territory); do not paper over by weakening assertions.
- **Effort:** M

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
- **Progress (2026-11-17):** Addressed in PR #26: the musl image gained `cmake`, `gcc`, `g++`, `ninja-build`; `linux-musl` runs both e2e targets in-container; `e2e-cc-cross-gcc` is declared `os = ["glibc"]` and runs in the new `e2e-linux-glibc` jobs with the glibc cross compilers; the harness learned libc os tokens. The lanes execute the gate corpus but are advisory until KACH-27 closes (Linux-specific reuse divergences found by the new gates).

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

