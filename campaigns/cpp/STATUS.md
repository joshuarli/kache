# C/C++ parity campaign — progress ledger

Companion to [C_CPP_PARITY_CAMPAIGN.md](../../C_CPP_PARITY_CAMPAIGN.md).
This file is the durable resume point. Scratch evidence (full logs, artifacts)
lives in CI artifacts or in the repo's git history for small manifests — not
here. `.work/` is disposable; nothing this campaign needs may live only there.

Ledger states: `TODO` `RUNNING` `BLOCKED` `FAIL` `INCONCLUSIVE` `DONE`.

Created 2026-11-17. Kache baseline:
[4401c7a](https://github.com/joshuarli/kache/commit/4401c7a09498d2225ab22e7d1e74bef20211ca8f);
plan: [C_CPP_PARITY_CAMPAIGN.md](../../C_CPP_PARITY_CAMPAIGN.md); tracking
entry: KACH-26 in [TODO.md](../../TODO.md). Groundwork branch:
`feat/ccp00-campaign-groundwork`.

## Groundwork baseline (2026-11-17, this host)

Host: Apple Silicon, macOS 27.2.1 (26B131), Xcode 27.0 (27A266a), Apple clang
21.0.0 (clang-2100.3.34.2), CLT MacOSX.sdk (`-target-sdk-version=27.2`), GNU
Make 3.81, Homebrew cmake 4.4.2 and ninja 1.13.2. Host load at run time
(10 cores): 8.2; no timed comparisons were taken.

| Gate | Result | Notes |
|---|---|---|
| `make check` | PASS on rerun; first run FAILED one test | First run (fresh worktree, build under load 8.2): `tests/cached_units_regression_test.rs::record_less_misses_key_from_the_emitted_dep_info` failed (`results_for == []`, expected `["local_hit"]`). 4 isolated reruns of the binary pass; `make check` rerun exits 0. Treated as a load flake, not root-caused. Separately, main CI run 44232741312 (2026-11-18) failed `test (macOS)` on `daemon::execution::tests::disconnected_accepted_compiler_is_cancelled_and_reaped` — a different test of the same class; recorded, not fixed here. |
| `make e2e` | FAILED one fixture | `e2e-cmake-ninja-flagset`: `ninja: failed to spawn: …/target/release/kache/usr/bin/c++: Not a directory`. Root cause is the host-PATH `ninja` at `~/.local/bin/ninja` (gluon-managed reimplementation reporting 1.13.2) strips trailing variable whitespace, breaking CMake's `${LAUNCHER}` idiom in its ninja generator. Reproduced minimally; the fixture passes with Homebrew ninja (`=> e2e-cmake-ninja-flagset: pass`). All other executed fixtures pass (51 pass / 2 Linux-only skips on macOS). NOT a kache defect. |
| `make e2e-negative-control` | PASS (exit 0) | 52 executed, 2 skipped; every non-exempt fixture falsifiable. |
| `make e2e` (flagset, brew ninja) | PASS | `PATH=/opt/homebrew/bin:… kache-scenario --select name:e2e-cmake-ninja-flagset --deny-missing-tools` → pass. |

Follow-ups not taken here (need explicit authorization or are other items):
the two `test (macOS)` failures above are not root-caused; `make e2e` leaves
in-tree `build/` directories behind (KACH-23 class); this host's gluon-ninja
shadow should not be used for CMake fixtures (PATH with `/opt/homebrew/bin`
first).

## Linux gate assignment (this groundwork)

KACH-21 asked for Linux fixture scenarios in a gate with
`--deny-missing-tools`, and a decision on the cross-GCC fixture.

- `Dockerfile`: added `cmake=4.2.3-r0`, `gcc=15.2.0-r5`, `g++=15.2.0-r5`,
  `ninja-build=1.13.2-r1`, `ninja-is-really-ninja=1.13.2-r1` to the Alpine
  image (musl lanes L3/L4). Alpine v3.24 has no glibc cross compilers
  (checked both APKINDEX indexes: only bare-metal `*-none-elf` toolchains),
  so the cross-GCC fixture cannot run in the image.
- `e2e-cc-cross-gcc` is now declared `os = ["glibc"]` and assigned to new
  `e2e-linux-glibc` CI jobs (`ubuntu-26.04` and `ubuntu-26.04-arm`), which
  install `gcc-riscv64-linux-gnu` / `gcc-arm-linux-gnueabihf` and run
  `make e2e` + `make e2e-negative-control`. The harness learned libc os
  tokens: `"glibc"` / `"musl"` match the libc the harness binary was built
  for (musl CI builds it for musl). Fixture count is unchanged; the fixture
  runs wherever its tools exist and is not silently dropped elsewhere.
- `linux-musl` CI job now also runs `make e2e` + `make e2e-negative-control`
  inside the image (previously only `make test`), with the store/scratch
  moved onto the checkout filesystem via `TMPDIR=/kache/tmp` and
  `KACHE_E2E_SKIP_HARDLINK_CHECK=1` (the runner substrate clones instead of
  hardlinking, so the cross-tree hardlink contract cannot engage there).
- The glibc jobs do the same with `TMPDIR=<checkout>/tmp`. Both new gates
  upload `tmp/e2e/` as artifacts on failure.
- First CI results (run 44241684053, 2026-11-18): the glibc jobs fail on
  `e2e-rust-cross-tree` and `e2e-rust-transitive-native` (cmake passes on
  glibc); the musl jobs fail on those two plus `e2e-c-shims`,
  `e2e-cc-cl-xclang-deps` (x86_64 only) and the three `e2e-cmake-*` relocate
  assertions. No wrong output — all failures are reuse accounting. Detail:
  TODO.md **KACH-27** and the PR. The new Linux gates are `continue-on-error`
  (advisory) until KACH-27 closes; the musl `make test` step stays required.
  Cross-tree passes with the escape hatch.
- KACH-21 does not close until KACH-27 closes and the gates become required.
- 2026-11-18: the in-image pins moved. LLVM is now the llvm-prebuilt-musl
  prerelease `release-b29e4d07a2b9a5e72319b37429cf375149212755` (asset
  digests verified against the prerelease's SHA256SUMS), and gluon
  `release-fa8b6ace38683a17ee3f398d8b568341c7192fe` (musl, built from the
  main merge of laputa-systems/gluon PR #18) provides `ninja`, `meson` and
  `gluon-build` in place of apk `ninja-build`. Gluon's ninja is required for
  the kache-as-launcher scenario: CMake's TryCompile writes a `LAUNCHER`
  binding with a trailing space, and only gluon ≥ fa8b6aac preserves it
  (upstream ninja always did; gluon trimmed it). Verified by building the
  image on this host (OrbStack, linux/arm64) and running `make e2e` in it:
  the failing assertion set is the KACH-27 set with no new failures
  (arm64 additionally failed `e2e-cc-cl-xclang-deps`, matching the
  documented probe-resolution race). `make test in image` status is in PR
  CI. PR #28 CI (run 44255058098): both musl jobs passed `make test` and
  failed the advisory e2e step on exactly the five deterministic fixtures
  (`e2e-c-shims`, three `e2e-cmake-*`, `e2e-rust-transitive-native`); the
  intermittent cl-mode fixtures passed on both architectures.

## PR and CI status

- PR #26 (`feat/ccp00-campaign-groundwork`): groundwork + KACH-21 wiring.
  `make check` exits 0. `make e2e` exits 0 on this host with Homebrew ninja.
  macOS e2e and test jobs green on the PR. Dockerfile pinned-git bump
  (2.54.0-r0 → 2.54.1-r0) was required because the Alpine mirror moved the
  package; `-sfn` keeps `cc` on LLVM clang now that Alpine's gcc owns
  `/usr/bin/cc`.
- PR #27 (merged 2026-11-18, `fix/daemon-disconnect-error-kind`): accept
  `InvalidInput` alongside `BrokenPipe`/`ConnectionReset` in the
  disconnect-reaping fixture; the contract is that the daemon reaps the
  compiler, not which errno the raced dead-peer write produces.
- PR #28 (merged 2026-11-18, `feat/musl-image-gluon`): the musl image now
  pins the llvm-prebuilt-musl prerelease
  `release-b29e4d07a2b9a5e72319b37429cf375149212755` and gluon musl
  `release-fa8b6ace38683a17ee3f398d8b568341c7192fe` (merged
  laputa-systems/gluon PR #18), replacing apk `ninja-build`. In-image
  `make test` passed on both CI musl jobs (and on this host under
  OrbStack, native path); the advisory in-image e2e step fails only on
  the five deterministic KACH-27 fixtures.
- PR #29 (merged 2026-11-18, `fix/daemon-publish-stall-flake`): the
  slow-but-moving-worker drain fixture got 10x timing headroom (30 jobs
  of 40ms against a 400ms window); 30/30 isolated runs pass. Main CI run
  44252816334 (`test (macOS)`) was the report.
- `tests/cached_units_regression_test.rs::record_less_misses_key_from_the_emitted_dep_info`
  remains the only known unexplained flake (passes on rerun; load-suspect).

## Reference (see [lock.toml](lock.toml))

- ccache 4.14.3, tag `v4.14.3`, commit
  `16464deae5188f8ef333cc693c58b63ee884e43e` — verified 2026-11-17 by
  cloning and `rev-parse HEAD`. Source tarball sha256
  `e1999132cf7dbce48de7d946b767f3e26ea09adcb5daa4a7103a8fa09ee0637d`.
- Upstream prebuilt binaries locked per lane (darwin tarball fetched and
  digest-verified 2026-11-17; reports `ccache version 4.14.3`). We lock
  upstream's binaries rather than building from source; build options are
  upstream's, recorded as such.
- This M1 host: Apple clang 21.0.0, Xcode 27.0, macOS 27.2.1. Standalone
  LLVM Clang is NOT provisioned here — M1's full scope needs Apple Clang AND
  standalone LLVM; recorded as a gap below.

## Matrix status

| Lane | Host | State | Notes |
|---|---|---|---|
| M1 | xcode-27 CI runner / Apple Silicon Mac | RUNNING (partial) | This host covers Apple clang 21.0.0/Xcode 27.0. Standalone LLVM Clang missing — see gaps. |
| L1 | ubuntu-26.04 | RUNNING (advisory) | e2e-linux-glibc job runs; only `e2e-rust-transitive-native` fails (run 44244430288); cross-tree passes with escape hatch; gate assigned, advisory |
| L2 | ubuntu-26.04-arm | RUNNING (advisory) | Same job matrix; only `e2e-rust-transitive-native` fails; cross-tree passes with escape hatch; advisory |
| L3 | Alpine 3.24.2 musl x86_64 (container on ubuntu-26.04) | RUNNING (advisory) | linux-musl job; image LLVM 23.1.5 (prerelease b29e4d0) + gluon ninja 1.13.2 + apk GCC 15.2.0; e2e step advisory: `e2e-c-shims`, three `e2e-cmake-*`, `e2e-rust-transitive-native`, `e2e-cc-cl-xclang-deps` (intermittent) |
| L4 | Alpine 3.24.2 musl aarch64 (container on ubuntu-26.04-arm) | RUNNING (advisory) | Same image and matrix; e2e step advisory: `e2e-c-shims`, three `e2e-cmake-*`, `e2e-rust-transitive-native`, `e2e-cc-cl-same-content-sources` (intermittent) |

## Package ledger

| Package | State | Depends on | Evidence / next |
|---|---|---|---|
| CCP-00 | RUNNING | — | This ledger + lock.toml + cases.toml; Linux gate wiring landed; exit needs: KACH-27 triage plan recorded (tracked in TODO.md), PR CI green or explicitly advisory lanes, `make check` green. |
| CCP-01 | TODO | CCP-00 bootstrap | Differential/transition runner; `make cc-oracle*` targets do not exist yet. |
| CCP-02 | TODO | CCP-01 | Input identity & invalidation; token-paste probe refusal is a known gap (see cases.toml). |
| CCP-03 | TODO | CCP-01, CCP-02 | Includes rebasing/verifying `feat/cc-chromium-official-flags` (exists on origin, unverified by `make check`). |
| CCP-04 | TODO | CCP-02 | Equivalent-preprocessing reuse. |
| CCP-05 | TODO | CCP-02, CCP-03 | Complete output sets (.gcno/.dwo/serialized diagnostics). |
| CCP-06 | TODO | CCP-02, CCP-03, CCP-05 | PCH + explicit module consumers; conditional implicit-modules profile. |
| CCP-07 | TODO | CCP-03..CCP-06 | Host integration; zlib/LLVM/Chromium pins in lock.toml. Universal objects. |
| CCP-08 | TODO | CCP-01 | Reliability, diagnosis, CI; KACH-22 mutation coverage; gate assignments. |
| CCP-09 | TODO | CCP-02..CCP-08 | Performance protocol; estimator locked in lock.toml (seed 20261117). |

## Open gaps and blockers

1. **Standalone LLVM Clang (M1).** Not provisioned on this host; the M1 lane
   capability requires it. Candidate: `brew install llvm` locally and a CI
   step in e2e-macos (or a scheduled M1 job). Decision needed before the
   first M1 full-corpus run; not required to start CCP-01.
2. **Chromium archive digest.** `chromium-157.0.8103.80.tar.xz` is ~1.6 GB;
   lock.toml records the URL/version and `RECORD-AT-FETCH (CCP-07)`. First
   CCP-07 fetch must verify and commit the digest + the mac-arm PGO profile
   digest.
3. **New CI jobs have run (run 44241684053).** Both Linux gates
   exist and execute; both are advisory due to KACH-27. The cross-GCC
   fixture executes on the glibc lanes (glibc-only by declaration); its
   x86_64/aarch64 result was PASS (not in the failure lists — the gnu probe
   resolves).
4. **`make e2e` exit status is red on this host** (gluon-ninja shadow). The
   gate is green via CI (Homebrew/brew ninja on xcode-27; apt ninja-build on
   Linux) and on this host with `PATH=/opt/homebrew/bin:…`.
5. **Main is red**: CI run 44232741312 `test (macOS)` failed on
   `disconnected_accepted_compiler_is_cancelled_and_reaped`; this host's
   first `make check` failed on `record_less_misses_key_from_the_emitted_dep_info`
   (passes 4/4 isolated). Two unexplained failures; neither is addressed by
   this groundwork; both are recorded here so the campaign does not inherit
   a silently unverified baseline.
6. **Inventory completeness.** cases.toml is the initial CCP-00 inventory
   (134 entries); full behavioral enumeration against the pinned suites is
   owed by CCP-01/CCP-02 and is tracked there, not here.

## Resume instructions

1. Read this ledger, [lock.toml](lock.toml), [cases.toml](cases.toml), and
   the plan. Refresh `git fetch` and re-check gate status.
2. CCP-00 exit: KACH-27 triage is filed and the Linux gates are assigned
   (advisory is an acceptable interim disposition — see the Linux gate
   section). Closing the lanes to required status is KACH-27's acceptance
   test, not a prerequisite for starting CCP-01. Start CCP-01 in a fresh
   worktree once the user accepts the groundwork; do not start it without
   that go-ahead.
3. All campaign work runs in isolated worktrees under `.work/wt/` (gitignored
   via `.git/info/exclude`); evidence needed after the fact lives in CI
   artifacts or committed manifests.
