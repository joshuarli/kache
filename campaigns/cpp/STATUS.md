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
| L1 | ubuntu-26.04 | RUNNING (required, PR #50) | e2e-linux-glibc job runs; its only failure, `e2e-rust-transitive-native`, is fixed by PR #49; cross-tree passes with escape hatch; gate assigned, advisory |
| L2 | ubuntu-26.04-arm | RUNNING (required, PR #50) | Same job matrix and fix (PR #49); cross-tree passes with escape hatch; advisory |
| L3 | Alpine 3.24.2 musl x86_64 (container on ubuntu-26.04) | RUNNING (required, PR #50) | linux-musl job; image LLVM 23.1.5 (prerelease b29e4d0) + gluon ninja 1.13.2 + apk GCC 15.2.0; the e2e failures it showed are fixed by PRs #47–#49 (pending CI on main) |
| L4 | Alpine 3.24.2 musl aarch64 (container on ubuntu-26.04-arm) | RUNNING (required, PR #50) | Same image and matrix; locally all green with PRs #44–#49 (52 executed, 0 failures) |

## Package ledger

| Package | State | Depends on | Evidence / next |
|---|---|---|---|
| CCP-00 | DONE (bootstrap) | — | Bootstrap exit met: pins, initial inventory, Linux gates assigned (advisory under KACH-27). Remaining inventory enumeration is carried by CCP-01..08 as the plan allows. Go-ahead for CCP-01: user, 2026-11-18 ("complete the campaign unattended"). |
| CCP-01 | DONE | CCP-00 bootstrap | Merged in PR #31 (a2ea56fd). CI evidence: run 44306769765 (logs and on-failure report artifacts retained 90 days, so until about 2027-02-16). `cc-oracle` and negative controls passed in `e2e (macOS)` (Apple clang), `cc-oracle (Linux x86_64/ARM64 glibc)` (apt GCC + clang) and `test (Linux x86_64/ARM64 musl)` (image GCC 15 + clang 23). Exit criteria met: minimal transitions pass on macOS and Linux, each control fails as intended, and the deliberate `-S` gap reports `KACHE_MISSED_REUSE`. `make cc-bench` moves to CCP-09. |
| CCP-02 | RUNNING | CCP-01 | Branch `feat/ccp02-input-identity`: 20 input-identity cases; see "CCP-02 input identity" below. Not yet covered: same-path compiler replacement, wrapper/driver-mode selection, VFS overlays, PGO profiles, -march=native, user key config, diagnostic locale, profile switches against a populated cache, hash/compile/publish race barriers. |
| CCP-03 | RUNNING | CCP-01, CCP-02 | Branch `feat/ccp03-chromium-flags`: Chromium official flags verified and landed from the WIP branch; `-S -o` cached. Open: response files (`@file`), clang `--config`, GCC `-specs`, `-fsanitize-ignorelist`, `-x` and `-Wp,`/`-Xpreprocessor` audits. See "CCP-03..CCP-08 progress" below. |
| CCP-04 | LIMIT | CCP-02 | Comment-only reuse is declined: comment text and width are observable (see "CCP-04 finding"). ccache's reuse is wrong output when a comment changes width before code (`cc-eqp-column-significance`) and when a same-width reword changes GCC's fallthrough warning (`cc-eqp-fallthrough-comment`, blocking, ccache wrong in both modes). `cc-eqp-comment-only` stays open as the recorded gap. |
| CCP-05 | RUNNING | CCP-02, CCP-03 | `.gcno`, `.dwo` and `.dia` sidecars cached (PR #52); `cc-out-gcno` (GCC and clang), `cc-out-split-dwarf`, `cc-out-serialized-diagnostics` blocking. See "CCP-05 sidecars" below. Diagnostic formatting keyed per entry (`cc-out-diagnostics-format*`). Colored entries serve colorless builds (PR #56). GCC `-fdiagnostics-format=json-file` cached as a sidecar (`cc-out-diagnostics-json-file`, blocking); SARIF outputs refused for good (see "CCP-05 GCC diagnostic files"). Open: sidecars under `-S`. |
| CCP-06 | RUNNING | CCP-02, CCP-03, CCP-05 | clang PCH builds and `-include-pch` consumers cached by default (`cc-pch-clang-use`, `cc-pch-clang-define-variants` blocking); see "CCP-06 PCH" below. Clang explicit module consumers (`-fmodule-file=`, `-fprebuilt-module-path=`) cached; `cc-module-fmodule-file`, `cc-module-body-edit`, `cc-module-prebuilt-path`, `cc-module-add-remove` blocking on Linux clang 23; see "CCP-06 modules" below. Open: GCC implicit `.gch` use through `-include`, implicit modules, module production. |
| CCP-07 | RUNNING | CCP-03..CCP-06 | Blocking `cc-host-objc`, `cc-host-objcxx`, `cc-host-universal-objects`, `cc-host-universal-slice-headers` (universal objects keyed slice by slice: the driver refuses `-E` with several `-arch`, and a compile's depfile lists only the last slice's headers). `-Xarch_<arch>` operands keyed per slice (`cc-host-xarch`, blocking): each slice is re-parsed with its own operands, and dropping the other slices' operands is what separates them (keeping all fails `edit-x86_64-header` with `KACHE_WRONG_OUTPUT`). `-Xarch_*` outside a universal compile, or naming no built slice, is refused. zlib 1.3.2 (`make cc-project-zlib`) passes on M1 Apple clang 21 and the L4 image (GCC 15, clang 23), 2026-11-18. LLVM 21.1.8 subset (`make cc-project-llvm`: `llvm-tblgen` and `LLVMCore`, CMake + Ninja, kache as compiler launcher, TableGen-generated headers in the build tree) passes on M1 Apple clang 21 and the L4 image (GCC 15, clang 23; the image needs `apk add python3` for LLVM's configure), 2026-11-19: 377 objects byte-identical across raw, cold and warm kache, warm build 377 local hits and no compiles, warm `llvm-tblgen` output identical to raw. cc-crate fixture, Chromium, Meson/Autotools launchers not started. |
| CCP-08 | RUNNING | CCP-01 | Store-fault cases (`cc-rel-store-*`), broker exit race fix (PR #32), C/C++ hits verify blob hashes by default (PR #46), the non-local cache advisory left compiler stderr (PR #44). |
| CCP-09 | TODO | CCP-02..CCP-08 | Performance protocol; estimator locked in lock.toml (seed 20261117). |

## CCP-01 differential runner

Code: `crates/kache-e2e/src/cc_oracle/`, binary `kache-cc-oracle`, cases in
[oracle/](oracle/README.md) (contract, verdict order, negative controls).
ccache is the lock.toml release binary for the host lane, fetched and
digest-checked by `scripts/fetch-ccache.sh`.

Initial cases: blocking `cc-core-c-object`, `cc-core-cxx-object`,
`cc-core-failed-compile`, `cc-core-user-depfile`, `cc-core-warning-replay`;
open `cc-flag-s-named-output` (CCP-03) and `cc-eqp-comment-only` (CCP-04).

Local evidence, 2026-11-18, branch `feat/ccp01-cc-oracle` (reports were
scratch; CI runs below are the durable copies):

| Host | Toolchains | `cc-oracle` | `cc-oracle-full` | negative controls |
|---|---|---|---|---|
| M1 workstation, Apple clang 21.0.0 | cc | 10/10 MATCH | open cases fail as expected | 5/5 detected |
| L4 image (OrbStack, aarch64 musl), GCC 15.2.0 + LLVM clang 23.1.5 | gcc, clang | 20/20 MATCH | open cases fail as expected | 5/5 detected |
| Debian 13 container (aarch64 glibc), GCC 14.2 + clang 19.1.7, musl-built binaries | gcc, clang | 20/20 MATCH | open cases fail as expected | not run |

Open-case results, identical on every host and toolchain:

- `cc-flag-s-named-output`: `KACHE_MISSED_REUSE` in both profiles. ccache
  restores `-S -o asm.s` in direct mode; kache passes `-S` through.
- `cc-eqp-comment-only`: `KACHE_MISSED_REUSE` in the reproducible profile
  only. In the transparent profile kache preprocesses first and hits on
  the preprocessed key. In the reproducible profile it compiles first and
  records `dup`, while ccache reuses the result in preprocessor mode.

Findings for later packages:

- **Diagnostic leak (CCP-08).** With its store on a FUSE filesystem (an
  OrbStack virtiofs bind mount), kache prints `[kache] the cache directory
  is on a FUSE filesystem ...` to the compiler's stderr on every compile.
  The oracle reports it as `KACHE_WRONG_OUTPUT` (stderr differs from raw).
  Reproduce: run `make cc-oracle` in the L4 image with
  `CC_ORACLE_OUT` on the bind-mounted checkout. Fixed in PR #44: C/C++
  compiles only log the advisory.
- On error, clang deletes an existing `-o` object and GCC leaves it in place.
  Kache matches both: `cc-core-failed-compile` passes for both toolchains.

CI: `make cc-oracle` plus negative controls are blocking steps in
`e2e (macOS)` and `test (Linux * musl)`, and run in the
`cc-oracle (Linux * glibc)` jobs. The first push showed reproducible-profile
misses on musl CI that came from editing inside kache's racing-write window.
The runner now waits for edits to settle; see the oracle README.

## CCP-02 input identity

Cases (all in [oracle/](oracle/)): `cc-input-same-size-mtime`,
`-atomic-replace`, `-symlink-retarget`, `-header-shadowing`,
`-has-include`, `-forced-include`, `-include-env`, `-search-order`,
`-option-order`, `-touch`, `-header-deleted`, `-sysroot-symlink`
(transparent only, see below), `-unmodeled-assembler-read`,
`cc-macro-file`, `cc-macro-date-time`, `cc-depfile-escape` (blocking), and
`cc-input-token-paste-probe`, `-clang-config-file`, `-gcc-specs`,
`-sanitizer-ignorelist` (open).

Local results, 2026-11-18, macOS Apple clang 21 and the L4 image (GCC 15,
clang 23): no wrong output anywhere. Every blocking case matches. The open
cases are `KACHE_MISSED_REUSE`: kache passes through clang `--config`
files, GCC `-specs` files, and `-fsanitize` with an explicit ignorelist,
where ccache caches them and invalidates correctly. Kache invalidates
correctly too; it just never caches. The token-pasted
`__has_include` probe misses only in the reproducible (compile-first)
profile; the transparent profile reuses it.

Findings:

- **Whole-second change times block caching for 2 s.** Kache's
  racing-write guard widens its window to 2 s when a stamp sits on a whole
  second, since such a stamp may have been truncated. Linux change times
  come from the coarse clock, so about one edit in 250 (HZ=250) lands on
  a whole second, and a compile within 2 s of that edit runs but is not
  cached. The oracle waits these windows out (see the oracle README). This
  costs reuse, not correctness. Revisit under CCP-04/CCP-09 if real
  edit-compile loops show it.
- **Undocumented SDK-root maps.** With `-isysroot`/`--sysroot`, kache's
  default profile also adds `-ffile-prefix-map=<sysroot>=/kache/sdkroot`
  for both the spelled and the symlink-resolved sysroot. The C/C++ docs
  list only `/kache/cc-root`. The reproducible profile cannot model these
  per step yet, so `cc-input-sysroot-symlink` runs transparent only.
  Owner: CCP-07 (SDK selection).
- Apple clang ignores `--sysroot` for headers and the musl LLVM clang
  ignores `-isysroot`, so the sysroot case passes both.

## CCP-03..CCP-08 progress

**Wrong-output findings and their disposition:**

1. **Stale object after a line-shifting edit (fixed, PR #33).** The key
   probe ran `cc -E -P`, which drops line markers and collapses long blank
   runs. Where the expansion hash alone was the key (no captured read set,
   for example `KACHE_CC_SOURCE_DATE_EPOCH=passthrough`), a comment block
   growing from 20 to 21 lines restored the old object: stale DWARF line
   table and warnings at the old line (`lines.c:24` vs raw `lines.c:25`).
   Reproduced on Apple clang 21, GCC 15 and clang 23. The fix keys
   prefix-mapped line markers apart from the text. Regression:
   `cc-eqp-line-number-significance` (blocking).
2. **Bit-flipped blobs were restored as hits (fixed, PR #46).** Hits
   re-hashed blobs only under `KACHE_VERIFY_RESTORES=sampled|always`, and
   the default was `off`. The owner delegated the decision. C/C++ hits
   now default to `always`; Rust stays `off`, since an rlib restore would
   read every byte. `cc-rel-store-corrupt-blobs` is blocking.
3. **Daemon spawn failed on an instant compiler exit (fixed, PR #32).**
   macOS identity pinning retried without re-checking for exit, failing
   the compile with "cannot read compiler process identity".
4. The first `-S` implementation reported a hit with no `.s` restored. The
   oracle caught it before landing. Entries lacking the invocation's
   named output are now rejected as a miss.

**Coverage gained (blocking, matching raw on macOS and Linux GCC/clang):**
the Chromium flag set (LTO compile modes, `-mllvm` pairs, `-fsanitize=`,
`-mmacos-version-min=`, codegen hardening flags, `-fprofile-use=` and
`--warning-suppression-mappings=` keyed by content, GCC `-fprofile-use=`
refused because it names a profile directory), `-S -o`, Objective-C,
Objective-C++, and recovery from damaged stores.

**Open gaps (`KACHE_MISSED_REUSE`, correct fallback):** `@rsp` response
files (no case yet), clang `--config`, GCC `-specs`, explicit sanitizer
ignorelists, PCH
consumers (conditional), and the token-pasted header probe and
comment-only edits in the default (reproducible) profile.

**Harness changes that judged better:**

- A `raw-passthrough` arm. A reproducible-profile passthrough runs the
  case's own command, so it is judged against that, not the
  prefix-mapped reference.
- Code generation is any execution naming the unit except
  preprocessing, dependency listing, syntax checks or queries (PCH
  builds have no `-c`).
- Per-case `ccache_config` and `kache_env` for conditional profiles.
- The cache-disabled control runs in the transparent profile only.
- Store-fault actions.

GCC's `.gcno` and the object carry a compile-time stamp even under
`-frandom-seed` and `SOURCE_DATE_EPOCH`: the notes' bytes 8..12 and one
4-byte run in the object, nothing else. The oracle hashes a pair whose
object holds its notes' stamp with the stamp zeroed, and a disagreeing
pair as written, so GCC coverage is judged like any other output.

**CCP-05 sidecars.** A `-c` compile with `--coverage`, `-ftest-coverage`,
`-fprofile-arcs`, `-gsplit-dwarf` or clang's `--serialize-diagnostics`
caches the files it writes beside the object (`CcArgs::sidecar_outputs`).
Observed on Apple clang 21, GCC 15.2 and clang 23 (L4 image):

- Both compilers name `.gcno` and `.dwo` after the object with its last
  extension replaced (`-o d/x.c.o` gives `d/x.c.gcno`); without `-o`, after
  the source stem in the working directory.
- `-gsplit-dwarf` without `-g` writes a `.dwo` under GCC and not under
  clang; Apple clang never writes one. The `.dwo` is required only under
  debug info on non-macOS hosts and stored when written otherwise.
- A coverage object names its absolute `.gcda` path and the notes file the
  compilation directory. A split-DWARF object names its `.dwo` as spelled.
  The probe blanks per-TU paths, so the key folds these in a
  `sidecars.v1` field: working directory and output for coverage, output
  only for split DWARF (its compilation directory is prefix-mapped).
  Removing the output fold makes `cc-out-gcno` fail with
  `KACHE_WRONG_OUTPUT` (`b/u.o` reused `a/u.o`'s entry).
- A sidecar is stored only when this compile wrote it: a required one that
  is missing, or a file unchanged (mtime, size) since before the compile,
  stops the store. A hit restores the whole set, and an entry whose
  sidecar set differs from the invocation's misses.
- GCC coverage hits were also checked by hand in the L4 image: `gcov`
  reports 100% after running the binary linked from a restored pair.

**CCP-05 diagnostic formatting finding.** Before this fix, every
`-fdiagnostics-*` flag and the color switches were left out of the
probe argv (to share the read-set memo) and out of the key. A
`-fdiagnostics-format=msvc` or `-fdiagnostics-color=always` compile then
replayed another formatting's warnings (`cc-out-diagnostics-format`,
`cc-out-diagnostics-format-msvc`: `KACHE_WRONG_OUTPUT` on the old binary).
Issue #1461 requires warning-free units to reuse across formatting, so the
key folds only `-fdiagnostics-format=`, and each entry's stored
diagnostics record the formatting flags they were printed under. An entry
with diagnostics under other flags is rejected and replaced. GCC options
that write diagnostics to files pass through. A colored entry also serves a
colorless invocation with its CSI and OSC escapes removed: for GCC 15,
clang 23 and Apple clang 21 that leaves the plain output byte for byte. A
colorless entry seen by a colored build is replaced, so a tree built both
ways converges on one colored entry (`cc-out-diagnostics-color-reuse`,
blocking). ccache instead compiles with color forced on, which the oracle's
argument contract does not allow.

**CCP-05 GCC diagnostic files.** Observed with GCC 15.2 (L4 image):
`-fdiagnostics-format=json-file` always writes a file, `[]` when there is
nothing to report, into the working directory as the object's name less its
extension, the source's extension, then `.gcc.json`, whatever directory
`-o` names. Two identical compiles write identical bytes, so it is a
required sidecar. The preprocessing pass must not carry the flag: with it,
`-E` wrote `<source>.gcc.json` and clobbered another compile's file
(`cc-out-diagnostics-json-file` failed until it was stripped). Dropping
the sidecar fails `restore` with a missing file. SARIF
(`-fdiagnostics-format=sarif-file`, `-fdiagnostics-add-output=sarif`) is
refused permanently: two identical compiles write different logs, since
each records its start and end times and cc1's temporary `.s` path.

**CCP-06 PCH.** Observed with Apple clang 21: a PCH records each
input's modification time and size, and clang rejects it when either
changes, even with identical bytes ("file has been modified since the
precompiled header was built"). A consumer's `-MD` file names the PCH's
inputs but not the PCH. So both a PCH build (`-x <lang>-header ... -o`,
mode `Precompile`) and a `-include-pch` consumer fold the stamps of every
read file, and the consumer folds the PCH's bytes. Dropping the stamps
fails `cc-pch-clang-use` (`touch-header` restores a PCH clang would
reject); dropping the bytes fails `cc-pch-clang-define-variants` (a
retargeted `p.pch` reuses the other PCH's object). PCH builds are bound to
their checkout, since a PCH records unmapped absolute paths. ccache needs
`pch_defines,time_macros` sloppiness for the same reuse; kache needs none.

**CCP-06 modules.** Observed with clang 23 on Linux: a consumer's `-E`
output keeps `import M;` as `__preprocessed_import M;`, its `-MD` file
names neither the `.pcm` nor the module interface, and clang accepts a
`.pcm` whose interface was edited or removed since it was built. A module
imported through another must itself be named by `-fmodule-file=` or found
on `-fprebuilt-module-path=`. So the key folds the bytes of each
`-fmodule-file=` path (absent when missing, since clang fails only if the
unit imports it) and the name and bytes of every `.pcm` in each prebuilt
directory. Dropping the fold fails all four module cases with
`KACHE_WRONG_OUTPUT`; folding prebuilt entries by name only fails the
swapped-build and template-edit steps. Apple clang 21 disables C++20 named
modules, so the cases run on Linux only. Producing a `.pcm` (`--precompile`)
stays unclassified and passes through; the cases give those compiles
`reuse = "miss"`, which requires code generation in every step.

**CCP-04 finding: expansion-equivalent reuse is not sound as ccache does it.**
ccache's preprocessor mode keys by the `-E` text. That text separates
tokens by one space whatever their columns were, so ccache reuses an
object across an edit that widens an inline comment or the spacing
before code on a line. The object's debug column info and the
diagnostics' columns and source excerpts change with such an edit.
`cc-eqp-column-significance` shows ccache 4.14.3 returning the stale
result in both direct and preprocessor mode (`CCACHE_WRONG_OUTPUT`).
Kache had the same bug wherever the expansion alone was its key; PR #45
fixed it by always keying the read-set digest.

Sound comment-edit reuse needs a key that keeps every token's line and
column while dropping comment text. A replayed diagnostic must also not
quote a changed line. Candidates:

- clang's token dump (`-Xclang -dump-tokens`), which carries spelling
  locations. It is clang-only and costs an extra frontend pass.
- A kache lexer over the read set that blanks comments to the same
  width and strips trailing whitespace. It must handle line splices,
  trigraphs, raw strings and `#embed` data exactly, or it serves wrong
  output.

Neither is enough, because comment text itself is observable. GCC's
`-Wimplicit-fallthrough` honors a `/* fallthrough */` comment: rewording
it to `/* carry on    */` keeps the width, every token's column and the
`-E` text, yet the compile now warns (`cc-eqp-fallthrough-comment`;
ccache 4.14.3 returns the silent result in both modes). `-Wcomment`
(nested `/*`, in `-Wall`) and clang's `-Wdocumentation` read comment
text too, and an entry with empty stderr does not prove the edited
comment is silent. Sound comment-edit reuse would have to model every
comment-reading diagnostic of every compiler version, so kache declines
it: a comment edit recompiles. Decided 2026-11-19 under the campaign's
correctness-first rule; `cc-eqp-comment-only` stays open as the recorded
gap rather than being counted toward parity.

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

## Session 2026-11-18 (unattended run)

Merged to main, in order. Each PR passed `make check`, and `cc-oracle` plus
the negative controls on macOS and the L4 image, before merge. Required CI
was green; the only failures were the advisory glibc e2e lanes on the known
KACH-27 fixture `e2e-rust-transitive-native`.

| PR | What |
|---|---|
| #31 | CCP-01 differential runner (`kache-cc-oracle`, `make cc-oracle*`), CI wiring |
| #32 | daemon: re-check for exit while retrying a compiler identity pin (spawn failed on instant compilers) |
| #33 | **wrong output fixed:** line positions keyed apart from the `-E -P` text (stale objects after line-shifting edits) |
| #34 | CCP-02 input-identity cases and runner features |
| #35 | e2e: shim fixtures ignore an inherited `CC` (KACH-27 finding 3); key tracing records |
| #36 | CCP-03 Chromium official flags, `-S -o` caching; CCP-05..08 cases |
| #37 | CI: upload musl e2e results when the advisory step fails |
| #39 | `-fsanitize-ignorelist` keyed by content |
| #40 | CCP-07 zlib real-project check (`make cc-project-zlib`) |
| #38 | GNU `@file` response files expanded and cached; `e2e-cc-response-file` now asserts caching |
| #41 | this ledger section |

No PRs open at hand-off.

## Session 2026-11-18, second run

The owner delegated the two open decisions ("I take your recommendations
on the rest"). Each PR passed `make check`. A local merge of all of
them passed `make cc-oracle cc-oracle-negative-control` on macOS (Apple
clang 21) and in the L4 image (GCC 15, clang 23), and macOS `make e2e`
(52 executed, all green).

| PR | What |
|---|---|
| #43 | stop tracking fixture build outputs; closes KACH-23, so `make e2e` no longer dirties the tree |
| #44 | the non-local cache advisory leaves C/C++ compiler stderr (logged, and `kache doctor`); rustc unchanged. The oracle now runs with its output on the OrbStack FUSE bind mount |
| #45 | **wrong output fixed:** preprocess-first keys fold the read-set digest; the expansion alone lost column positions (`cc-eqp-column-significance`, blocking) |
| #46 | C/C++ hits verify blob hashes by default (`KACHE_VERIFY_RESTORES` decision); `cc-rel-store-corrupt-blobs` blocking |
| #47 | **wrong output fixed:** a restored dep-info file named the producing object (`a/x.o:` in `b/x.d`); object-only targets are now stored as a placeholder and restored as this invocation's object. Also fixes KACH-27 finding 2 (CMake TryCompile `-MT`) |
| #48 | e2e harness drains a build-spawned daemon before each report; fixes the intermittent musl clang-cl fixtures |
| #49 | build-script results stop binding to their checkout on Linux: GNU `ar` member headers and keyed tool paths (`CC = Some(/opt/...)`) read as unmapped absolute paths. Fixes KACH-27 finding 1 (`e2e-rust-transitive-native`) |
| #50 | the Linux e2e gates (glibc jobs, musl in-image step) are required; closes KACH-21 and KACH-27 |

With #44–#49 merged locally, the L4 image's `make e2e` reported 52
executed, 2 skipped and no failures. `make e2e-negative-control` and
`make cc-oracle` also passed. This was the first all-green Linux e2e
run. Before these PRs, L4 failed eight fixtures. PR #50's CI then passed
every job with the Linux gates required: glibc e2e on x86_64 and ARM64,
the musl in-image e2e on both architectures, and macOS.

## Session 2026-11-19 (third unattended run)

Each kache PR passed `make check`, `make cc-oracle
cc-oracle-negative-control` on macOS and in the L4 image (GCC 15,
clang 23), macOS `make e2e`, and CI with every gate required.

| PR | What |
|---|---|
| #52 | CCP-05: `.gcno`, `.dwo` and `.dia` sidecars cached and restored as a set; `cc-out-gcno`, `cc-out-split-dwarf`, `cc-out-serialized-diagnostics` blocking |
| #53 | the oracle compares GCC coverage pairs with their shared stamp zeroed; `cc-out-gcno` runs for GCC |
| #54 | **wrong output fixed:** hits replayed diagnostics printed under other formatting flags (`-fdiagnostics-format=msvc`, `-fdiagnostics-color=always`); `cc-out-diagnostics-format*` blocking, `cc-out-diagnostics-color-reuse` open |

**L4 oracle output must not sit on the OrbStack bind mount.** With the
oracle's work tree on the FUSE mount, about one run in thirteen showed
`KACHE_MISSED_REUSE`: the daemon accepted a store handoff, then failed to
stat its own hard-linked snapshot (`stating compiler output for
__kache_cc_object.o`). The same three cases ran 30 times with the work tree
on the container's overlay filesystem and never failed, and CI (real
filesystems) has not shown it. Kache already reports that mount as not
host-local. Run L4 oracle passes with `CC_ORACLE_OUT=/tmp/cc-oracle`.

## Resume instructions

1. Read this ledger, [lock.toml](lock.toml), [cases.toml](cases.toml) and
   the plan. Run `git fetch` and check the open PRs.
2. Verify every kache change with all of:
   - `make check`;
   - `make cc-oracle cc-oracle-negative-control` on macOS;
   - the same in the L4 image, with `TOOLCHAINS="gcc clang"
     CC_ORACLE_OUT=/tmp/cc-oracle` (see the session 2026-11-19 note);
     after syncing a scratch copy with `rsync -a`, touch its `.rs` files:
     rsync keeps the source's older mtimes, so a copy that last built a
     hand-applied mutation keeps the mutated binary (2026-11-19: a stale
     binary failed `cc-module-prebuilt-path` until rebuilt);
   - `PATH=/opt/homebrew/bin:$PATH make e2e`.
3. Next work, in priority order:
   - The LLVM subset, the Chromium
     target, and CCP-09 performance.
4. All campaign work runs in isolated worktrees under `.work/wt/`
   (gitignored via `.git/info/exclude`). Evidence needed afterwards lives in
   CI runs (90-day retention) or in committed cases and scripts.
