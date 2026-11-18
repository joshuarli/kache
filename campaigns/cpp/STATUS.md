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
  inside the image (previously only `make test`).
- macOS gate unchanged (`e2e-macos`). All gate fixtures now execute on at
  least one Linux gate per arch family with `--deny-missing-tools`.
- Not yet observed: the new/changed jobs have not run (they run on this
  branch's PR). KACH-21 closes when the PR's Linux jobs are green with the
  cross-GCC fixture executing. Local docker (OrbStack) daemon is not running,
  so L3/L4 were not built or run locally.

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
| L1 | ubuntu-26.04 | ASSIGNED (not yet run) | e2e-linux-glibc x86_64 job added; GCC + Clang via apt. |
| L2 | ubuntu-26.04-arm | ASSIGNED (not yet run) | e2e-linux-glibc ARM64 job added; GCC + Clang via apt. |
| L3 | Alpine 3.24.2 musl x86_64 (container on ubuntu-26.04) | ASSIGNED (not yet run) | linux-musl job; in-image LLVM 23.1.5 + apk GCC 15.2.0. |
| L4 | Alpine 3.24.2 musl aarch64 (container on ubuntu-26.04-arm) | ASSIGNED (not yet run) | Same image; same job matrix. |

## Package ledger

| Package | State | Depends on | Evidence / next |
|---|---|---|---|
| CCP-00 | RUNNING | — | This ledger + lock.toml + cases.toml; Linux gate assignment above; exit needs: PR CI green on M1/L1–L4 gates (cross-GCC fixture executing), `make check` green on the integrated revision. |
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
3. **New CI jobs unproven.** e2e-linux-glibc (x86_64 + ARM64) and musl e2e
   are defined here but have never run; ubuntu-26.04 runner labels, apt
   package availability, and cross-GCC fixture behavior on real GCC are all
   unverified until the PR CI executes them.
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
2. If CCP-00 exit criteria are met (PR CI green incl. Linux glibc/musl e2e
   with the cross-GCC fixture executing), close CCP-00 in this ledger and
   start CCP-01 in a fresh worktree. Do not start CCP-01 before the gates
   run green or the user explicitly overrides.
3. All campaign work runs in isolated worktrees under `.work/wt/` (gitignored
   via `.git/info/exclude`); evidence needed after the fact lives in CI
   artifacts or committed manifests.
