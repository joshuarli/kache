# C/C++ differential oracle cases

Each directory is one case from [cases.toml](../cases.toml) with
`status = "oracle"`. `make cc-oracle` runs the blocking cohort;
`make cc-oracle-full` adds the open cases; `make cc-oracle CASE=<id>` replays
one. Reports land in `.work/tmp/cc-oracle/<run-id>/report.{json,md}`.

## What a run does

For every selected case, profile, and toolchain the runner executes five
arms one after another. Each arm starts from a fresh copy of `source/` at the
same absolute path, with its own cache, `HOME`, `TMPDIR`, and an empty
configuration file:

| Arm | Command |
|---|---|
| `raw`, `raw-repeat` | the compiler, twice, to prove the outputs are stable |
| `raw-passthrough` | reproducible profile only: the case's own arguments and environment, which is what a kache passthrough runs |
| `ccache` | pinned ccache, normal configuration (direct, then preprocessor) |
| `ccache-preprocessor` | pinned ccache with `CCACHE_NODIRECT=1` |
| `kache` | `kache <compiler> ...` with its own daemon and store |

The compiler each arm runs is an observation shim: a shell script that logs
its working directory and arguments, then execs the real driver. All arms run
identical shim bytes. A logged execution in the fixture that names the
case's `source` is preprocessing with `-E`, `-M` or `-MM`, a probe with
`-###`, `--version` or `-fsyntax-only`, and code generation otherwise (`-c`,
`-S`, a precompiled header). Every execution elsewhere is a probe.

After every compile the runner records exit status, stdout, stderr, and the
whole fixture tree (path, kind, mode, blake3). Kache must match `raw` exactly.
Timestamps are not compared. Source files get a fixed modification time, and
each written file gets a later fixed time, so neither cache sees a file
modified moments before the compile. Change times cannot be set, so after
preparing the tree and after any step that edits it, the runner waits until
every change time is outside kache's racing-write window. The window is 1 ms
plus a coarse-clock tick, widened when a stamp looks truncated: 10 ms on a
centisecond, 2 s on a whole second. Each step then compiles a quiescent tree.

## Profiles

- `transparent`: kache runs with `KACHE_CC_PATH_NORMALIZE=0` and
  `KACHE_CC_SOURCE_DATE_EPOCH=passthrough`. Every arm compiles the case's
  arguments unchanged.
- `reproducible`: kache runs with its defaults. `raw` and both ccache arms get
  `-ffile-prefix-map=<fixture>=/kache/cc-root` appended and
  `SOURCE_DATE_EPOCH=0`, which is what kache documents adding on a miss.

In both profiles every code generation kache runs must use exactly the `raw`
arguments, apart from its `-MD -MF <tmp> -MT __kache_direct_capture`
dependency capture, or else the case's own arguments unchanged (a
passthrough). Anything else is a harness error: the profile no longer
describes what kache does.

## Verdicts

Each compile in each step gets one verdict. Checks run in this order:

1. `HARNESS_ERROR`: `raw` exited differently from the step's `status`.
2. `COMPILER_NONDETERMINISM`: `raw` and `raw-repeat` differ.
3. `KACHE_WRONG_OUTPUT`: kache's status, streams, or tree differ from `raw`,
   or from `raw-passthrough` when every code generation kache ran used the
   case's own arguments.
4. Reuse, from the step's `reuse`:
   - `miss`: kache ran no code generation, so it served a stale entry
     (`KACHE_WRONG_OUTPUT`).
   - `hit`: kache ran code generation (`KACHE_MISSED_REUSE`), or kache's own
     event log did not record a hit (`HARNESS_ERROR`).
   - `like-ccache`: ccache's normal arm matched `raw` without generating
     code, but kache generated code (`KACHE_MISSED_REUSE`).
   - `any`: no requirement.
5. `HARNESS_ERROR`: kache's code generation ran different arguments.

A case limited to some `families` reports `UNSUPPORTED_BY_COMPILER` for a
toolchain whose `--version` banner names another family. That verdict does
not fail the gate, but a run in which every case was unsupported does.

`CCACHE_WRONG_OUTPUT` records where a ccache arm differs from `raw`. It does
not fail the gate. A run fails if any compile gets a failing verdict, a toolchain
is missing (`BLOCKED`), or the run records fewer compiles than the case
declares.

## Negative controls

`make cc-oracle-negative-control` runs the blocking cohort once per control
and passes only if every run the control was injected into reports the
expected verdict:

| Control | Fault | Expected |
|---|---|---|
| `stale-output` | after kache's first required recompilation, put back the populate outputs (only where the bytes differ) | `KACHE_WRONG_OUTPUT` |
| `omit-sidecar` | after kache's first required restore, delete the last declared output | `KACHE_WRONG_OUTPUT` |
| `disabled-invalidation` | hide kache's first required recompilation from the observer | `KACHE_WRONG_OUTPUT` |
| `cache-disabled` | `KACHE_DISABLED=1` in the kache arm, transparent profile only (a disabled kache runs the case's own arguments, which the reproducible reference does not) | `KACHE_MISSED_REUSE` |
| `reference-failure` | the raw arms run a compiler that always fails | `HARNESS_ERROR` |

## case.toml

```toml
summary = "what the case proves"
cohort = "blocking"            # or "open": a known gap, run by cc-oracle-full
os = ["macos"]                 # optional; empty means every host
families = ["clang"]           # optional; empty means every compiler family
profiles = ["transparent", "reproducible"]
outputs = ["unit.o", "unit.d"] # removed by delete-outputs; checked by omit-sidecar
env = { CPATH = "{fixture}/inc" }  # every arm; may not set harness, KACHE_* or CCACHE_* variables
ccache_config = { sloppiness = "pch_defines,time_macros" }  # ccache arms only: a conditional profile

[[compile]]
argv = ["{cc}", "-c", "unit.c", "-o", "unit.o"]   # {cc} or {cxx}
source = "unit.c"

[[step]]
name = "populate"
reuse = "miss"                 # the first step must be "miss"

[[step]]
name = "restore"
reuse = "hit"
status = 0                     # the raw exit status this step expects
action = [
  { op = "delete-outputs" },
  { op = "write", path = "unit.h", content = "...", keep_mtime = false },
  { op = "replace", path = "unit.h", content = "..." },  # write a sibling, rename over
  { op = "symlink", path = "cfg.h", target = "b.h" },   # create or retarget
  { op = "touch", path = "unit.h" },                    # new mtime, same content
  { op = "remove", path = "old.h" },
  { op = "restart-daemon" },   # kache arm only
]
```

`{fixture}` in `argv`, `source`, and `env` values expands to the fixture's
absolute path. Writes, replacements, and touches set a fixed modification
time that advances one minute per step; `keep_mtime = true` on a write keeps
the file's previous time instead.

`TOOLCHAINS` names C drivers on `PATH` (`cc`, `gcc`, `clang`, `gcc-15`); the
C++ driver follows from the name (`c++`, `g++`, `clang++`, `g++-15`).
Directories holding kache shims (`.kache-shims`) or named `ccache` are
dropped from `PATH`, so no other cache sits in front of the compiler.
