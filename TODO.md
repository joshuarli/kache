Prefix: KACH
Next: 36

### KACH-26 · P1 · Reach ccache-level local C/C++ compatibility

- **Area:** Compiler compatibility, correctness, tests
- **Where:** campaigns/cpp/STATUS.md (progress ledger), campaigns/cpp/cases.toml (inventory), campaigns/cpp/oracle/ (cases)
- **Problem:** Kache does not yet match pinned ccache's local C/C++ compatibility: inventory rows remain planned or open, and some lanes are unexecuted. Pinned ccache is the compatibility and reuse oracle and raw compilers are the correctness ground truth, across response files and flags, invalidation, equivalent-preprocessing reuse, complete output sets, conditional PCH/module support, host integration and local reliability. Remote storage stays on the later roadmap.
- **Recommendation:** Execute the campaign until every final acceptance criterion in the plan passes on one integrated revision; safe passthrough, missing tools and unexecuted lanes do not count as cached parity. KACH-22 (mutants in no gate) stays open alongside.
- **Fix with:** reference-parity
- **Effort:** L
- **Design:** [C_CPP_PARITY_CAMPAIGN.md](C_CPP_PARITY_CAMPAIGN.md)

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

### KACH-33 · P3 · Clang -ivfsoverlay compiles are never cached

- **Area:** Compiler compatibility
- **Where:** src/compiler/cc.rs CC_FLAGS (no -ivfsoverlay row); inventory row cc-input-vfs-overlay
- **Problem:** A compile with -ivfsoverlay <file> is refused as an unsupported flag, so builds that use VFS overlays (Swift/Clang module setups, some Bazel toolchains) never cache. Keying the overlay file alone is unsafe: the overlay redirects reads to external files that the depfile names only by their virtual paths, so the read set does not see the real inputs.
- **Recommendation:** Parse the overlay (YAML/JSON roots, external-contents) and key the overlay bytes plus every external-contents file it maps, or refuse when the overlay cannot be fully resolved; add cc-input-vfs-overlay as a blocking oracle case.
- **Fix with:** reference-parity
- **Effort:** M

### KACH-34 · P3 · GCC -fprofile-use with a profile directory is never cached

- **Area:** Compiler compatibility
- **Where:** src/compiler/cc.rs cache key loop for -fprofile-use= (bails for GCC), and bare -fprofile-use (unsupported flag)
- **Problem:** GCC reads <dir>/<mangled object path>.gcda for -fprofile-use[=dir], a file the key cannot see from the flag, so kache refuses these compiles; PGO builds with GCC therefore never cache.
- **Recommendation:** Derive the .gcda path GCC reads from the object path, cwd and -fprofile-dir/-fprofile-prefix-path the way GCC mangles it, key that file (absent vs present), and cover it with an oracle case that edits the profile.
- **Fix with:** reference-parity
- **Effort:** M

### KACH-35 · P4 · No read-only store mode for shared or CI caches

- **Area:** Store
- **Where:** crates/kache-config/src/config.rs (no read-only setting); store writes in src/wrapper.rs (cc publish and daemon handoff), src/daemon_publish.rs, the rustc store path, preprocess memo and probe-record writes, and access-time updates on hits; inventory row cc-rel-readonly-disabled
- **Problem:** Kache can be disabled (KACHE_DISABLED) but cannot use a store read-only: a CI job or a shared cache that should restore but never write has no supported mode, and the inventory row for read-only behaviour has nothing to exercise.
- **Recommendation:** Add a read-only setting that restores hits, skips publication and leaves the store untouched, and cover it with an oracle case that asserts misses compile and nothing is stored.
- **Fix with:** none
- **Effort:** M

