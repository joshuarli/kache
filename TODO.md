Prefix: KACH
Next: 32

### KACH-31 · P4 · A put killed between its meta.json rename and its commit strands the entry directory

- **Area:** Store
- **Where:** `crates/kache-store/src/store.rs` (`put_with_compile_time_policy` Phase 2; `sweep_orphan_blobs`)
- **Problem:** Phase 2 renames `meta.json` into `store/<key>/` inside the transaction, then commits the row. A kill between the rename and the commit leaves a `meta.json` with no row. `get` misses on it, which is correct. Its blobs have no rows and the orphan-blob sweep reclaims them. The directory itself is reclaimed only by a later put of the same key or by an index rebuild. Each occurrence leaks a small directory.
- **Recommendation:** Do not delete such directories. A `meta.json` with no row is also what index recovery (#415, `rebuild_index_from_store`) rebuilds from after quarantining a corrupt index, and that rebuild is best-effort. A sweep that deleted rowless directories would destroy the only source left after a failed rebuild. Re-adopt instead, through the rebuild path, under `gc.lock` and `BEGIN IMMEDIATE`: a directory whose `meta.json` parses and whose blobs all exist and verify gets its rows back. Remove only a directory whose `meta.json` cannot be read. The orphan-blob sweep can unlink a stranded put's blobs after its grace, so re-adoption must check every blob first.
- **Fix with:** store tests for three cases: a rowless entry with intact blobs is re-adopted and then hits; one with a missing blob is a miss that leaks nothing; one with an unreadable `meta.json` is removed.

### KACH-26 · P1 · Reach ccache-level local C/C++ compatibility

- **Area:** Compiler compatibility, correctness, tests
- **Plan:** [C_CPP_PARITY_CAMPAIGN.md](C_CPP_PARITY_CAMPAIGN.md)
- **Status:** Running. Differential oracle, 67 oracle cases (63 blocking), required macOS/Linux gates and the zlib/LLVM project checks have landed; a provisional timed baseline (`make cc-bench`) ranks the losses; a quiet-host rerun is next. Progress: [campaigns/cpp/STATUS.md](campaigns/cpp/STATUS.md).
- **Goal:** Execute the unattended campaign using pinned ccache as the compatibility/reuse oracle and raw compilers as correctness ground truth, with native macOS and Linux coverage.
- **Scope:** Response files and flags, invalidation, equivalent-preprocessing reuse, complete output sets, conditional PCH/module support, host integration, local reliability, and measured performance. Remote storage stays on the later roadmap.
- **Coordination:** KACH-21 and KACH-27 closed when the Linux gates became required (PR #50). KACH-22 (mutants in no gate) stays open. Evidence lives in the campaign ledger.
- **Done when:** Every final acceptance criterion in the plan passes on the same integrated revision; safe passthrough, missing tools, and unexecuted lanes do not count as cached parity.

### KACH-29 · P3 · A C/C++ compiler probe is keyed on the whole environment

- **Area:** Performance
- **Where:** `src/probe/cache.rs` (`probe_key_for`, `fingerprint_env`)
- **Problem:** The `cc -###` probe record is keyed on every environment variable except `KACHE_*` and the per-command shell and build-driver variables (`PER_COMMAND_ENV`: `_`, `SHLVL`, `MAKEFLAGS`, …). Any other variable that changes per invocation therefore reruns the probe (about 30 ms) on every hit and writes one more record under `probes/`, which nothing prunes. Under hyperfine (`HYPERFINE_ITERATION`), a warm hit took 42 ms instead of 14, and 68 runs left 68 records. Builds that export a per-job counter or a timestamp pay the same.
- **Recommendation:** Key the probe on the variables the driver reads (`SDKROOT`, `DEVELOPER_DIR`, `MACOSX_DEPLOYMENT_TARGET`, `COMPILER_PATH`, `GCC_EXEC_PREFIX`, the include and library path variables, `PATH` for tool lookup). An allowlist must be proven complete against both drivers' documentation, or it miskeys. A cheaper step is to prune probe records in GC.
- **Fix with:** a test that varies an unrelated variable and keeps one probe record, and one per listed variable that rekeys.
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
