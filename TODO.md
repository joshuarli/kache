Prefix: KACH
Next: 32

### KACH-26 · P1 · Reach ccache-level local C/C++ compatibility

- **Area:** Compiler compatibility, correctness, tests
- **Plan:** [C_CPP_PARITY_CAMPAIGN.md](C_CPP_PARITY_CAMPAIGN.md)
- **Status:** Running. Differential oracle, 67 oracle cases (63 blocking), required macOS/Linux gates and the zlib/LLVM project checks have landed; a provisional timed baseline (`make cc-bench`) ranks the losses; a quiet-host rerun is next. Progress: [campaigns/cpp/STATUS.md](campaigns/cpp/STATUS.md).
- **Goal:** Execute the unattended campaign using pinned ccache as the compatibility/reuse oracle and raw compilers as correctness ground truth, with native macOS and Linux coverage.
- **Scope:** Response files and flags, invalidation, equivalent-preprocessing reuse, complete output sets, conditional PCH/module support, host integration, local reliability, and measured performance. Remote storage stays on the later roadmap.
- **Coordination:** KACH-21 and KACH-27 closed when the Linux gates became required (PR #50). KACH-22 (mutants in no gate) stays open. Evidence lives in the campaign ledger.
- **Done when:** Every final acceptance criterion in the plan passes on the same integrated revision; safe passthrough, missing tools, and unexecuted lanes do not count as cached parity.

### KACH-29 · P3 · Narrow native C/C++ compiler probe environment keys — implemented

- **Area:** Performance
- **Where:** `src/probe/cache.rs`, `src/probe/mod.rs`, `src/compiler/cc.rs`
- **Problem:** The `cc -###` probe record is keyed on every environment variable except `KACHE_*` and the per-command shell and build-driver variables (`PER_COMMAND_ENV`: `_`, `SHLVL`, `MAKEFLAGS`, …). Any other variable that changes per invocation therefore reruns the probe (about 30 ms) on every hit and writes one more record under `probes/`. Under hyperfine (`HYPERFINE_ITERATION`), a warm hit took 42 ms instead of 14, and 68 runs left 68 records. Builds that export a per-job counter or a timestamp pay the same.
- **Resolution:** Probe schema 6 uses documented GCC/Clang environment inputs only after the probe identifies a native ELF/Mach-O compiler; executable realpaths and invocation names must not name known launchers. Scripts, unknown drivers, launchers and other binary formats keep whole-environment keying. The driver and preprocessor memos share a conservative superset of GCC's environment documentation and Clang driver source inputs, including loader, configuration, include, locale, target and temporary-directory families. User `key_env_vars` patterns join both probe modes and override internal-variable exclusions. Preprocessing/direct-capability memo schemas also advance so old records cannot alias the expanded input set.
- **Fix with:** native-driver table tests assert that each input re-probes, while `HYPERFINE_ITERATION` preserves one record; a shell wrapper named `gcc` still re-probes on arbitrary inputs. Mutation tests remove `CPATH` and accept scripts as native. Hyperfine's environment-unsetting workaround is unnecessary for qualifying native GCC/Clang, but remains necessary when timing opaque wrappers.
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
