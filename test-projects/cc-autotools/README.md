# Autotools/libtool launcher fixture (CCP-07)

This bounded C project builds a shared and static libtool library and a linked
test program. The library accesses an exported variable so PIC and non-PIC
compilations exercise different relocation models. Automake generates depcomp
rules with `-MD -MP -MF`, `.lo` descriptors, and `.deps` manifests.

From the repository root:

```sh
env -u TMPDIR -u KACHE_CACHE_DIR make cc-project-autotools TOOLCHAINS="gcc clang cc"
```

Requires Python 3, Make, a C compiler/linker, Autoconf, Automake, and Libtool
(`libtoolize` on PATH). On Ubuntu:

```sh
sudo apt-get install --no-install-recommends autoconf automake libtool
```

For each compiler, the runner separately exercises `CC="kache gcc"` (or the
selected compiler, including the documented `CC="kache cc"`) and compiler-name
PATH symlinks to kache. Configure probes must use that CC. Builds enable both
shared and static libraries with `-O0 -g0`; kache keeps default path mapping,
with epoch injection disabled. No source uses path or time macros. Raw builds
use the same source/build paths and CFLAGS, and every cached phase must match
their object, `.lo`, and dependency-file bytes.

Phases: raw baseline, raw header edit, cold kache, output-cleaned warm kache,
incremental header edit, revert, and relocated source/build trees. Each phase
runs the project's tests and checks the linked program's value. Cold and edited
builds must record exactly three misses (main plus PIC/non-PIC library units).
Warm, reverted, and relocated builds must record three local hits and zero code
generation. Source edits settle outside the compiler-cache racing-write window.
Each run has an isolated store; logs and per-phase digest/event JSON remain in
`.work/tmp/cc-project-autotools/run-*/`.

Negative control (expected nonzero: disables kache only for the warm build):

```sh
env -u TMPDIR -u KACHE_CACHE_DIR python3 scripts/cc-project-autotools.py \
  target/release/kache .work/tmp/cc-project-autotools gcc --negative-control
```

This is an opt-in project lane, not part of `make check` or CI. Adding it to CI
needs an explicit provisioning decision for Autoconf/Automake/Libtool and Python
in the chosen cells. macOS and musl have not yet been verified for this fixture.
