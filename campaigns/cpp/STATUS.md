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

## Owner ruling, 2026-11-23 (later): CI runs in the background

Performance gates and macOS CI are waived; `KACHE_MACOS_CI=off` skips the
macOS jobs. Linux CI does not block merges: PRs merge after their local
verification on L3, and Linux CI results are reconciled afterwards.

## Owner ruling, 2026-11-23: performance gates waived

The owner waived the performance acceptance gates. Feature completion now
comes first: turn `planned` inventory cases into blocking oracle cases and
fix what they expose. The steps 2-3 measurement work in the focused cycle
below is suspended, not failed. Scope also now includes co-developing
gluon (`~/d/gluon`, Meson front end and Ninja executor) through the CCP-07
Meson launcher cell.

Native L3 host for this work: Alpine 3.25 (edge) x86_64 musl, GCC 15.2.0,
Alpine Clang 23.1.4 (`cc` is Clang), pinned ccache 4.14.3 musl-static, eight
CPUs, shared with unrelated workloads (correctness cells only).

**CCP-03 response files, 2026-11-23 (L3):** `cc-rsp-escaped`, `-empty`,
`-cycle`, `-depth`, `-option-value`, `-relative-paths`, `-encoding`,
`-content-change` and the new Clang-only `cc-rsp-encoding-bom-clang` are
blocking oracle cases. All match under GCC 15 and Clang 23 in both profiles,
and none needed a kache change. Driver difference found: GCC rejects a
UTF-8 byte-order mark as an argument while Clang accepts it, so the BOM step
is Clang-only.

**ccache defect (reference, not kache):** ccache 4.14.3 never returns on a
cyclic response file (`a.rsp` -> `@b.rsp` -> `@a.rsp`), while GCC (`too many
@-files encountered`) and Clang (`recursive expansion`) fail at once.
`cc-rsp-cycle` runs its ccache arm with `disable = true`, so kache is held to
the raw compiler. Reproduce: `CCACHE_DIR=$(mktemp -d) timeout 10 ccache gcc
@a.rsp` with those two files exits 143. Not reported upstream (third-party;
needs owner consent).

## Current state (post-#141 at `ee2402bf`; formal L1 rerun 2026-11-23)

Latest [formal L1 acceptance rerun](reports/2026-11-23-l1-rerun/README.md):
GCC 11 and Clang 14, all six workloads eligible, ten complete pairs each;
**all six FAIL overall performance acceptance**. Clang tiny-C cold vs ccache
now passes, **0.716 [0.619, 0.827]**, but this does not close its other targets.
Main's exact-source post-merge checks are **11/11 green**. Acceptance rules
are unchanged; M1, newer-L1 compilers, native ARM64 L2 and LLVM timing gaps
remain open.

Since that run, #143 (leased store-connection pool) merged at `c56bfcb5`;
merged-main `ci` and `lint` are green. Its A/B is diagnostic only, and **no
formal acceptance result exists for a revision containing the pool**. The
current work order is
[Focused cycle after #143](#focused-cycle-after-143-2026-11-23).

Counts are taken from the files, not carried forward:
`grep '^status = ' campaigns/cpp/cases.toml | sort | uniq -c` and
`grep -l '^cohort = ' campaigns/cpp/oracle/*/case.toml`.

- Inventory (`cases.toml`): 188 entries, and every oracle directory has
  one: 103 `oracle`, 75 `planned`, 6 `conditional`, 4 `out_of_scope`.
- Oracle cases (`oracle/`): 103, of which 99 `blocking` and 4 `open`
  (`cc-eqp-comment-only`, `cc-input-token-paste-probe` and the CCP-04
  equivalent-expansion gaps).
- Gates: every Linux and macOS CI job is required (PR #50);
  KACH-21 and KACH-27 are closed. Main had a one-off musl
  `cc-out-diagnostics-color-reuse` failure after #136; it passed on rerun.
  This is a correctness-CI flake note, not acceptance-timing evidence.
  `make e2e` on the M1 host needs
  `PATH=/opt/homebrew/bin:$PATH`, because the gluon `ninja` at
  `~/.local/bin` stripped trailing spaces from variable values
  (laputa-systems/gluon GLU-29, fixed in its PR #34, merged 2026-11-19).

Engineering work still open (not provisioning):

- CCP-03 flags, 2026-11-23 (L3, GCC 15, Clang 23): `-x` operands with any
  extension, `-Wp,`/`-Xpreprocessor` `-D`/`-U` forwarding, debug-detail
  flags (`-g[no-]column-info`, `-gline-tables-only`, `-gmlt`), wchar and
  charset flags, preprocessed `.i`/`.ii`/`-x *-cpp-output` input (keyed by
  its bytes; previously refused, and compile-first capture leaked Clang
  "argument unused" warnings), and GCC `-g3` (the time-macro check compared
  the `#define` lines it injected) are cached and blocking oracle cases.
  Distribution hardening flags (`-f[no-]plt`, `-fcf-protection`,
  `-fzero-call-used-regs`, `-m[no-]red-zone`, GCC `-fharden-*`) are probe-keyed.
  Unknown flags pass through with raw side files (`cc-unknown-flag-passthrough`).
  Clang `-fprofile-instr-use=` and `-fprofile-sample-use=` are keyed by the
  profile's content like `-fprofile-use=` (`cc-flag-profile-use`). GCC
  `-fprofile-use[=dir]` and Clang's bare `-fprofile-use` are keyed too: every file
  name and content under a GCC profile directory, the `.gcda` beside the object,
  or `default.profdata`, each including absence; `-fprofile-dir` with profile use
  still refuses. Verified on L3: objects equal raw with and without the profile,
  and removing it misses.
- CCP-02 VFS overlays, 2026-11-24 (L3, Clang 23): `-ivfsoverlay` compiles cache, keyed by
  the overlay's bytes; mapped headers reach the read set under their external names.
  Overlays with `use-external-names: false` pass through (`cc-input-vfs-overlay`).
- CCP-03: the `@file` variants are oracle cases (see the owner ruling
  above). Clang `--config=` and GCC `-specs=` files named by a path are keyed
  by content (`cc-input-clang-config-file`, `cc-input-gcc-specs`, blocking);
  bare names (searched in the driver's directories), specs `%include` and
  nested config `@` files still pass through.
- CCP-04: comment-only reuse is declined by decision (LIMIT, below); the
  token-pasted header probe stays an open case.
- CCP-05: sidecars under `-S` pass through.
- CCP-06: implicit modules and module production pass through.
- CCP-07: the cc-crate fixture, Chromium and Meson launchers; the Autotools
  fixture has local Linux glibc coverage, with macOS/musl cells still unverified.
- CCP-08: compiler replacement, C/C++ driver selection and profile
  switches against a populated cache now have oracle cases, and concurrent
  same-key and different-key compiles an integration test
  (`test_cc_concurrent_compiles_each_get_the_compilers_object`). Wrapper
  selection: a `cc` shim and `kache cc` share one entry
  (`test_cc_a_compiler_shim_and_the_launcher_share_one_entry`). Two defects
  were behind that miss: the preprocess memo keyed the compiler's spelling
  (`cc` vs `/usr/bin/cc`), and `kache cc` behind shims resolved `cc` to the
  shim, so its probe and memo keyed kache's own binary and would not follow
  a replaced compiler. Store races, covered by kache-only tests rather than
  ccache-compared oracle cases, so most `cc-rel-*` rows stay `planned`.
  Partial damage is now oracle-covered on L3 (2026-11-23, GCC 15 and Clang 23):
  `cc-out-missing-blob` and `cc-out-corrupt-blob` damage one blob of an
  object-plus-depfile entry, `cc-rel-truncated-entry` cuts metadata in half and
  `cc-rel-missing-entry` removes it; each recovers by compiling and then hits.
  Remaining CCP-08 rows, 2026-11-24 (L3): `cc-rel-output-materialization-failure`
  is covered by the `materialize_*` store and wrapper tests (a missing blob is a
  miss; failed copies and renames leave no temp file or read-only source);
  `cc-rel-stale-locks` by `sweep_stale_staging_respects_min_age`;
  `cc-rel-cancellation-signals` by `a_compiler_killed_by_a_signal_fails_and_is_never_cached`
  (no success, object or entry, and kache now dies of the compiler's signal as raw
  does; pinned ccache exits 255);
  `cc-rel-size-limit-eviction` cannot be an oracle case because the eviction grace
  period protects recently accessed entries; `cc-rel-readonly-disabled` has no
  read-only mode to exercise (KACH-35; the disabled half is the `cache-disabled`
  negative control); `cc-rel-stats-reset` does not apply, because kache's statistics
  are computed from the event log over a time window rather than kept as counters.
  CCP-04/05 rows answered by existing cases, 2026-11-24: `cc-out-depinfo-full` by
  `cc-flag-deps`, `cc-core-user-depfile` and `cc-depfile-escape` (restored depfiles
  equal raw); `cc-out-full-inventory-validation` by `cc-out-missing-blob` and
  `cc-out-corrupt-blob` (a partial or damaged entry never reports a hit);
  `cc-eqp-compile-first-miss` by `probe_keyed_flags_cache_and_key_on_value_live`
  (a new spelling compiles first and later hits); `cc-eqp-dep-refresh` and
  `cc-eqp-memo-fastpath` wait on equivalent reuse, which CCP-04 declines.
  Kache-only race coverage:
  - Eviction: a blob removed between lookup and restore leaves every output
    as it was and recompiles; nothing is published before every artifact is
    staged (`a_cc_hit_whose_blob_goes_before_restore_leaves_every_output_and_recompiles`).
    Compiles racing `kache clean --cache --all` each get the compiler's
    object (`test_cc_compiles_racing_a_store_clear_get_the_compilers_object`);
    that test does not reliably reach the lookup-to-restore window, the unit
    test does.
  - Killed compile: after `kache cc` is killed mid-compile, the next
    compile neither waits nor misbuilds
    (`test_cc_a_killed_compile_does_not_hold_up_the_next`); Linux delegation
    depends on host isolation, including seccomp and no_new_privs. A delegated
    compile whose orphaned descendant keeps output pipes open no longer holds
    the discovery lock past cancellation.
  - Stale locks: every store lock is `flock`, released by the kernel when
    its holder dies; old per-key lock files are swept for disk only.
  - Interrupted publication: a put stages and renames blobs before one
    transaction commits the row, so a kill leaves no visible partial entry.
    A wrapper killed before its hand-off left loose snapshots in
    `staging/handoff/`; the handoff sweep now reclaims them.
  A `meta.json` stranded by a kill between its rename and the row's commit
  is a miss until GC housekeeping finds it an hour later. GC gives it its
  rows back when every blob is present at its recorded size, and removes it
  otherwise (`stranded_entry_sweep_readopts_servable_entries_and_removes_the_rest`).
- CCP-09: `kache-cc-bench --acceptance` (#80, #81) applies the locked
  rules. It requires five complete admitted pairs, adds up to ten only while
  an interval crosses its target, and turns an ineligible host away. It also
  records provenance and each arm's declared environment. The first
  acceptance attempt at `aa8df06a` (M1, Apple clang 21; tiny C, C++
  headers, zlib) was **INELIGIBLE** for every workload, with 1 complete pair
  each: [report.json](reports/2026-11-20-m1-aa8df06a/report.json),
  [report.md](reports/2026-11-20-m1-aa8df06a/report.md). Unrelated
  processes used 0.37 to 6.9 cores per replay, chiefly macOS
  `softwareupdated`, an OrbStack VM and another session's benchmark. No
  eligible M1 acceptance verdict exists. Its single-pair ratios are
  diagnostics only. The loaded-host tables under "CCP-09 baseline" predate
  #74/#75 and the acceptance mode.
- Previous CCP-09 L1 rerun, 2026-11-21, clean `c4d2b947` (includes #101, #105,
  #109), same three workloads and `--jobs 4` as #99:
  [GCC 11 report](reports/2026-11-21-l1-c4d2b947/gcc/report.json),
  [Clang 14 report](reports/2026-11-21-l1-c4d2b947/clang/report.json),
  [comparison, phase evidence and LLVM estimate](reports/2026-11-21-l1-c4d2b947/README.md).
  Both standalone five-second preflights passed on their first attempt
  (GCC 0.021952 unrelated cores, Clang 0.041824); both runner preflights
  passed too. Threshold unchanged at 0.5 cores. **All 6/6 workloads are
  eligible, 60/60 complete pairs and 180/180 admitted/eligible arm replays;
  all six FAIL performance acceptance.** Every correctness/reuse validation
  passed. The unchanged rule uses time-weighted whole-replay activity:
  1573/1740 individual short build windows passed, while every whole replay
  remained under the limit (maximum 0.366677 cores); none was removed.
  Compared with #99's 2/6 eligible workloads, activity accounting no longer
  blocks these cells, but this is still not passing L1 acceptance.
  Cold/warm paired kache/ccache geomeans:

  | Compiler | Tiny C | C++ headers | zlib |
  |---|---|---|---|
  | GCC 11 | 1.158 / 2.947 | 0.974 / 3.507 | 1.029 / 1.456 |
  | Clang 14 | 1.293 / 3.284 | 1.046 / 2.825 | 1.453 / 1.278 |

  Top mean paired wall losses are now eligible: **Clang tiny-C cold
  +724.8 ms**, then **GCC C++ settled header edit +675.8 ms**. Clang's
  epoch-support record remains `no`, requiring preprocessing fallback;
  retained cold events now show 48 preprocessors for 48 compiles, versus
  96 in #99, with 1061 ms summed key time. GCC's invalidated existing
  read-set memo takes preprocessing before recompilation: 12 preprocessors,
  12 compiles, 1429 ms summed key time. Phase sums locate the paths, not an
  exact allocation of all excess wall time. Clang tiny-C cold excess was
  +1708.6 ms in #99 (ineligible/diagnostic); Clang C++ cold improved from
  +872.8 to +330.1 ms. Conversely, eligible C++ warm ratios regressed from
  1.657 to 3.507 (GCC) and 1.560 to 2.825 (Clang), with zero compiler,
  preprocessor or probe runs. This twelve-TU/four-job hit-path regression
  was unprofiled in that report. The diagnostic checkpoint below now locates
  the large regression in standalone timestamp scanning, not native daemon
  serialization. LLVM performance acceptance is still **not run**:
  reserve roughly 6–18 hours for both compilers (high uncertainty, all
  phases included), verify scratch ceilings before launch, and profile
  the demonstrated smaller-workload losses first.
- Historical CCP-09 L1 attempt, 2026-11-21, clean `da350481` (#93, after #92):
  this Ubuntu 22.04 x86_64 glibc VM's standalone five-second preflight
  passed (0.04185 unrelated cores). Both toolchain preflights passed too.
  [GCC 11 report](reports/2026-11-21-l1-da350481/gcc/report.json),
  [Clang 14 report](reports/2026-11-21-l1-da350481/clang/report.json),
  [analysis, event evidence and host details](reports/2026-11-21-l1-da350481/README.md).
  All validation passed. C++ headers reached 10 eligible pairs per compiler
  and **FAIL**ed performance targets; tiny C and zlib were **INELIGIBLE**
  after 1 pair each on GCC and 5 each on Clang. This is not passing L1
  acceptance, nor coverage of the newest primary-lane compiler majors.
  Largest mean paired wall losses are Clang tiny-C cold (+1.709 s) and
  header edit (+1.633 s), diagnostic only because ineligible. The largest
  eligible losses are Clang C++ cold (+0.873 s) and header edit (+0.436 s).
  Events identify Clang 14's preprocessing fallback and second time-macro
  safety expansion (two preprocessors per miss); hits and relocation
  have zero compiler/preprocessor/probe runs. An activity-sampler fixture
  also exposes false cumulative CPU charges on renamed kernel workers;
  resolve accounting before another acceptance attempt, without relaxing
  thresholds. LLVM was deferred: estimated multi-hour timing and several
  GiB of scratch are unjustified while eligibility measurement is unresolved.
- Real workloads: zlib and the LLVM subset cache fully (CCP-07). cc-rs build
  scripts also do. A probe on 2026-11-20 used `CC="kache cc"`, no Rust
  wrapper, and `zstd-sys`, bundled `libsqlite3-sys` and `blake3`. Its first
  build missed 38 C compiles, and a second build in a fresh target dir, with
  different `OUT_DIR` paths, took 38 local hits and linked. It was an
  informal probe, not a fixture.

- Autotools/libtool: `make cc-project-autotools TOOLCHAINS="gcc clang cc"`
  passes on Ubuntu 22.04 x86_64 (GCC 11.4, clang 14), 2026-11-20. The bounded
  fixture covers prefix launchers and PATH shims, PIC/non-PIC objects and
  depfiles, cold/warm, incremental header edit/revert and relocation. See CCP-07
  below; CI provisioning and other host cells remain open.

Provisioning, not engineering:

- A quiet benchmark host throughout timing. The M1 remains shared with other
  sessions' work and is retried only after a quiet preflight. A separate
  native L1 cloud VM is now available (rerun above); being virtualized
  does not itself disqualify it. Linux activity accounting was fixed in
  #101; the rerun has eligible data but fails performance targets. L1 newer-compiler
  cells, native ARM64 L2 acceptance and eligible M1 acceptance remain missing;
  shared CI correctness
  runners are not acceptance-timing evidence.
- M1 standalone LLVM clang: not installed on this host, and no authorized
  setup installs it (`brew install llvm` would change the host). Missing
  cell: M1 × standalone LLVM, every oracle and bench run. Apple clang 21
  covers the rest of M1.
- Chromium: the ~1.6 GB archive digest is `RECORD-AT-FETCH` in lock.toml.
- The earlier history of this ledger (groundwork baseline, Linux gate
  assignment, bootstrap PR notes) is in git: `git log -p --
  campaigns/cpp/STATUS.md` before 2026-11-19.

## Focused cycle after #143, 2026-11-23

This replaces the 2026-11-22 ranking below as the work order. Review baseline
is `c56bfcb5` (includes #143). Refresh main and read what merged after it
before acting; do not reopen issues it already resolved. Work the steps in
order. Do not widen scope into another architecture exercise.

**Retired criticisms.** Token-pasted timestamp keying (#133), compiled-wrapper
driver identity (#135), private response-file observation (#134), mapped
assembler listings (#132) and response-file control characters (#136) are
resolved. Keep their regressions. Do not reopen them without new evidence.
The historical warm regression is already bisected to `438c5f04`; **do not
repeat that bisect.** Keep the timestamp correctness repair and profile only
losses that current code reproduces.

### 1. Quoted-macro capture risk (correctness, first)

**Result, 2026-11-23: reproduced and fixed** (Linux x86_64, Alpine Clang
23.1.4). A shared cache served checkout A's object to checkout B on B's cold
compile under default compile-first capture: `sizeof(ROOT_TEXT)` returned
A's length (74 vs 100). After the fix, all three variants pass: default
capture, capture with `SOURCE_DATE_EPOCH=0`, and deferred discovery off.
The resolved `-cc1` tokens are keyed through the prefix maps, so the two `-D`
operands agreed, and the capture guard read only file contents. Repair:
`cc_macro_operands_spell_roots` routes a capture to the existing expansion
when an explicit or driver-resolved `-D`/`-U` operand spells a mapped root.
The expansion then binds the key to its checkout if the text survives, and
an unused rooted macro still hits across checkouts. Preprocessing memos are
retired to v16. Regressions:
`quoted_root_macros_compile_each_checkouts_text_from_a_shared_cache`
(`tests/cc_passthrough_args_test.rs`; compared with raw Clang; checks cold,
warm and relocated results for both bodies) and
`macro_operands_spelling_mapped_roots_require_expansion` (driver-injected
defines). Disabling the guard fails both. Not covered here: GCC compile-first
(it needs a pinned epoch, same code path) and a real compiler config file
(clang `--config` is still refused).

Original hypothesis, kept for context: Identical source in two
checkouts whose roots differ in length:

    int path_size(void) { return sizeof(ROOT_TEXT); }

Each checkout passes its own absolute path as `-DROOT_TEXT='"<root>/asset"'`,
with its declared base directory and the normal path-mapping policy. The
hypothesis has three parts. Root normalization can make the two `-D` operands
agree. The capture guard (`f109aad6`) inspects only captured file contents,
which contain no root. Constant folding removes the root string from the
object, so the object scan cannot see it. A local GCC check already shows the
two raw objects differ (47 vs 78) and neither contains a root.

- Add a bounded end-to-end regression: one native Clang, one shared cache,
  the public kache invocation, the compile-first capture path. Populate and
  warm checkout A, then compile in checkout B. Compare B's object and its
  observable result with raw Clang under the same effective policy. Also
  check warm reuse within each checkout.
- Cover the epoch-capability variants where available. Cover definitions
  injected through compiler configuration as well as explicit `-D`.
- The existing preprocessing-path regression for literal `-D` roots
  (`src/compiler/cc/tests.rs`) does not exercise the capture bypass. It does
  not settle this question.
- If it reproduces: make the smallest repair. Either key the semantic macro
  operand raw, or route these captures through the existing expansion and
  path-binding mechanism. Retire affected memo versions. Keep portable
  `__FILE__` behavior and warm reuse.
- If an existing guard prevents it: name the guard, keep the regression, and
  make no speculative production change.

### 2. Measure the integrated pool revision

After step 1 and its gates, run scoped locked acceptance on one exact
revision containing #143 (and step 1, if it changed code). Scope: tiny C and
C++ headers, GCC and Clang, on the same native L1 host.

- Keep every scenario and its parallelism. Formal tiny C stays **serial**;
  C++ headers keep their defined job count. Cover cold, warm, relocation,
  edit and revert.
- Thresholds, sample limits, estimator and quiet-host policy are locked.
- Retain raw observations, exact source and executable identities, compiler
  versions, and correctness/reuse evidence.
- Verify daemon readiness and executable identity before replays. Record any
  standalone fallback separately. Establish compiler/preprocessor activity
  from execution counters or separate untimed traces.
- #143's numbers (about 11% single warm hit, 17% on a 48-TU `-j4` build)
  come from a parallel fixture. Do not multiply them into the serial tiny-C
  ratios; only this rerun establishes the integrated effect.
- Commit the report through the existing reporting machinery and link it
  from Current state.

### 3. One measured loss, one focused change

From step 2's results, pick **one** bottleneck. Weigh absolute time and
distance from its target. Baseline gaps from `ee2402bf`: tiny-C warm 2.468×
(GCC) and 2.448× (Clang) against a 1.15× target; C++ headers warm 1.307× and
1.339×; Clang tiny-C cold is 0.716× ccache but 1.343× raw, and GCC tiny-C
cold is 1.994× raw, against a 1.15× cold-vs-raw target.

- If tiny warm hits are still far behind, profile the whole request path and
  name its largest remaining fixed cost. If miss or edit work dominates the
  chosen workload, trace that path before choosing an optimization.
- Make one change. Run a bounded parent-versus-change comparison, then rerun
  only the affected acceptance workload.
- Keep timestamp sensitivity, opaque-wrapper environment keying, artifact
  verification and the pool's conservative invalidation. A faster result
  must keep correct outputs and required reuse.

**Milestone:** one named compiler × workload × host cell passing every
required scenario. If it stays blocked, report the specific failing
criterion and its evidence rather than expanding scope.

### 4. Host-evidence gaps

Keep the macOS and Linux correctness CI gates as they are. Get a quiet
native macOS acceptance result, and advance newer-compiler and native ARM64
performance evidence as suitable hosts appear. Independent hosts may run
concurrently; a timed host must stay uncontended. The LLVM workload stays in
the campaign; check its time and scratch limits before launch. It comes
after this cycle, not before.

### Cycle report

End the cycle with focused code/test commits where justified, committed
benchmark evidence, and a short report stating:

- whether the quoted-macro collision reproduced, and the fix or the guard
  that prevents it;
- the exact revision tested and its completed CI status;
- current PASS, FAIL, INCONCLUSIVE and UNMEASURED results;
- before/after ratios with confidence intervals and useful absolute times;
- the next single blocking issue.

## Formal L1 acceptance rerun, 2026-11-23

Clean main **`ee2402bfb20c2d018c4036bec5e53a1084f79a9a`** (#141), same native
Ubuntu 22.04 x86_64 host, GCC 11.4, Clang 14.0, pinned ccache 4.14.3,
same three workloads, jobs 4, seed and input digests as #99/#114.
[Report, intervals, raw observations, analysis and native-daemon evidence](reports/2026-11-23-l1-rerun/README.md).
The unchanged locked runner uses #140's short private runtimes and verifies
daemon readiness/version/epoch before every replay. Passive records sampled
62 native daemon PIDs (both socket paths <=32 bytes); separate **untimed**
strace checks confirm each compiler's twelve warm C++ requests are twelve hits
with zero compiler/preprocessor or standalone-engine executions.

The first standalone GCC preflight refused a busy host (0.704695 unrelated
cores), before any benchmark. The next attempt and Clang's first attempt pass
(0.031940 / 0.025950 cores); runner preflights also pass (0.013966 / 0.033933).
No timed sample was discarded or repeated. **6/6 workloads eligible, 60/60
complete pairs, 180/180 admitted/eligible replays**; all correctness/reuse
validations and complete-pair raw-object comparisons pass. All six reach the
existing ten-pair cap. Maximum whole-replay unrelated activity is 0.381263
cores; all 1740 build windows remain, including 172 individually over the
short-window threshold, under the unchanged time-weighted replay rule.

Ratios below are kache/ccache geomeans. Cold target <=1.10; warm/relocated
<=1.15. `I` is INCONCLUSIVE, not PASS. Full CIs and every edit/revert target
are in the report. **All six overall verdicts remain FAIL.**

| Compiler / workload | Cold | Warm | Relocated | Overall |
|---|---|---|---|---|
| GCC 11 tiny C | 1.239 FAIL | 2.468 FAIL | 2.356 FAIL | FAIL |
| GCC 11 C++ headers | 1.086 I | 1.307 FAIL | 1.441 I | FAIL |
| GCC 11 zlib | 1.192 I | 1.304 FAIL | 1.112 I | FAIL |
| Clang 14 tiny C | **0.716 PASS** | 2.448 FAIL | 2.287 FAIL | FAIL |
| Clang 14 C++ headers | 1.058 I | 1.339 FAIL | 1.973 FAIL | FAIL |
| Clang 14 zlib | 1.332 FAIL | 1.213 I | 1.305 FAIL | FAIL |

Cold-vs-raw <=1.15 is also required: FAIL for both tiny-C and zlib cells,
INCONCLUSIVE for both C++ header cells. Clang tiny-C cold vs ccache is a real
passing subcriterion, **0.716 [0.619, 0.827]**, not a passing workload. Relative
to #114's standalone fallback, C++ warm ratios improve 3.507 ->1.307 (GCC)
and 2.825 ->1.339 (Clang), but both still fail. These integrated measurements
are not isolated attribution of #138, #139 or #141. Largest current mean paired
wall losses include Clang C++ immediate source edit +534.0 ms, GCC C++ cold
+504.9 ms and Clang tiny-C header edit +389.0 ms; these absolute-time rankings
do not replace the locked interval-based acceptance verdicts.

No eligible M1, newest-L1-compiler, native ARM64 L2 or LLVM subset performance
acceptance is added here. Those cells remain missing, as does M1 standalone
LLVM Clang provisioning. The campaign acceptance rules remain unchanged.

## Diagnostic checkpoint and next actions, 2026-11-22

[Warm checkpoint report](reports/2026-11-22-warm-checkpoint/README.md) preserves
Steps 1–3, compact observations and portable analysis. **Diagnostic only, not
formal acceptance. The acceptance rules are unchanged.** The formal rerun
above now supplies native-daemon GCC 11/Clang 14 verdicts, still **FAIL** overall.
Eligible M1, newer-L1-compiler and native ARM64 L2 acceptance evidence remains
missing. Green CI correctness does not close those timing gaps.

Both historical endpoints' default sockets exceed Linux's limit. The large
twelve-TU/four-job warm regression reproduces in standalone fallback, not at
comparable magnitude with verified private daemons. The bounded bisect identifies
`438c5f04` (timestamp read-set scanning/allocation). Measured baseline `9553a432` retains
a standalone penalty through full header-analysis loading/deserialization.
Native Clang tiny-C cold has +332.5 ms median paired excess [62.9, 430.1].
GCC settled-header-edit is inconclusive: +138.6 ms [−632.2, 904.1]. Its historical
+675.8 ms excess remains **unassigned**, not disproved or explained by phase sums.

The diagnostic fixes below merged through `c5c3d206`; the latest formal run
also includes #140 (short native-daemon runtimes) and #141 (safe Clang capture):

- Correctness: #132 passes through prefix-mapped assembler listings; #133
  repairs token-pasted timestamp keying; #134 fixes observation of private
  response-file compiles; #135 requires self-identifying native Clang and keeps
  GCC/opaque drivers whole-environment-keyed; #136 passes through ambiguous
  form-feed/vertical-tab response files. These safeguards remain in force.
- #138 replaces directive candidate enumeration with `memchr_iter`. On a real
  2.0 MB preprocessed header-fixture TU, 21 alternating microbenchmark pairs
  reduce directive-text scanning **8.098 → 1.521 ms (81.2%)** and the complete
  raw/decoded hidden-input scan **14.325 → 7.859 ms (45.1%)**. These are scanner
  measurements, not whole-build gains or attribution of the GCC excess.
- #139 reads a validated timestamp-only projection of persisted header analyses,
  upgrading to full analysis for probes. The v6 format and content-bound seal
  are unchanged. Twenty alternating standalone pairs give **45.6 → 37.2 ms**,
  ratio **0.816 [0.712, 0.875]**; thirty daemon pairs give **20.30 → 20.23 ms**,
  ratio **0.997 [0.968, 1.051]**. These are diagnostic 95% bootstrap CIs against
  the pre-fix baseline, not acceptance or a combined post-merge benchmark.
  Standalone's median gain is 18.4%; its interval is not entirely above 15%,
  and the daemon interval does not exclude a small regression.

Ranked next actions, with correctness safeguards retained. **Superseded as
the work order by
[Focused cycle after #143](#focused-cycle-after-143-2026-11-23);** kept as
the diagnostic record:

1. Profile the **remaining current L1 misses** in the 2026-11-23 formal report:
   warm/revert/relocation overhead, GCC tiny-C and C++ cold, Clang zlib cold
   and C++ immediate source edits. #141 implements guarded Clang compile-first;
   Clang 14 tiny-C cold vs ccache now passes, but its other targets do not.
   Preserve full read-set/time-macro proofs, expansion fallback, races, mapped
   path and assembler checks; do not generalize to unmeasured compiler majors.
2. Reprofile the **remaining standalone header-analysis cost after #139** before
   selecting another optimization. Full persisted JSON is still loaded; do not
   assume the old full-deserialization hotspot survives unchanged. Preserve
   content-bound verdicts, full probe upgrades and all keying safeguards.
3. Keep the historical **unassigned GCC settled-header-edit excess** explicit.
   The current C++ header-edit mean is -38.5 ms vs ccache; this does not explain
   the historical +675.8 ms or identify its cause. Use controlled compiler/
   process evidence before claiming attribution to a particular optimization.
4. Rerun only affected locked **L1 acceptance** after a relevant change or a
   resolved measurement problem. The verified-daemon rerun is complete and
   all six workloads still FAIL overall; do not rerun unchanged cells to fish
   for passing intervals or relax admission/targets.
5. Obtain eligible **M1**, **newer-L1-compiler** and native **ARM64 L2** acceptance
   evidence separately; provision M1 standalone LLVM Clang before claiming its
   missing compiler cell. Shared CI correctness does not fill these timing gaps.
6. Then run locked **LLVM subset performance acceptance** on a quiet host with
   verified daemons and checked scratch ceilings. The 6–18 hour two-compiler
   estimate remains uncertain planning evidence, not a measured acceptance result.

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
| M1 | xcode-27 CI runner / Apple Silicon Mac | RUNNING (partial) | Apple clang 21.0.0 / Xcode 27.0 covered. Standalone LLVM clang not provisioned (missing cell, see Current state). |
| L1 | ubuntu-26.04 | RUNNING (required, green) | e2e-linux-glibc and cc-oracle (apt GCC + clang) jobs |
| L2 | ubuntu-26.04-arm | RUNNING (required, green) | Same jobs on ARM64 |
| L3 | Alpine 3.24.2 musl x86_64 (container on ubuntu-26.04) | RUNNING (required, green) | linux-musl job: image LLVM 23.1.5 (prerelease b29e4d0), gluon ninja, apk GCC 15.2.0; `make test`, in-image e2e and cc-oracle; native L3 host 2026-11-23 (Alpine 3.25 edge x86_64, GCC 15.2.0, Clang 23.1.4, ccache 4.14.3 musl-static, kache `a828da56`): `make cc-project-zlib` 17/17 objects identical raw/cold/warm for both compilers; `make cc-project-autotools` passes for both compilers in prefix and shim launcher styles; `make cc-project-llvm` built with gluon `ninja` (`5616e911`) passes for both compilers (381 objects identical, warm 377/377 local hits); libdrm 2.4.134 through gluon `meson`/`ninja` with kache as launcher: transparent cold and warm objects byte-identical to raw for both compilers, and with default path mapping warm and relocated builds hit and equal cold (exploratory, no committed script) |
| L4 | Alpine 3.24.2 musl aarch64 (container on ubuntu-26.04-arm) | RUNNING (required, green) | Same image on ARM64; also this host's local Linux lane under OrbStack |

## Package ledger

| Package | State | Depends on | Evidence / next |
|---|---|---|---|
| CCP-00 | DONE (bootstrap) | — | Bootstrap exit met: pins, initial inventory, Linux gates assigned (required since PR #50). Remaining inventory enumeration is carried by CCP-01..08 as the plan allows. Go-ahead for CCP-01: user, 2026-11-18 ("complete the campaign unattended"). |
| CCP-01 | DONE | CCP-00 bootstrap | Merged in PR #31 (a2ea56fd). CI evidence: run 44306769765 (logs and on-failure report artifacts retained 90 days, so until about 2027-02-16). `cc-oracle` and negative controls passed in `e2e (macOS)` (Apple clang), `cc-oracle (Linux x86_64/ARM64 glibc)` (apt GCC + clang) and `test (Linux x86_64/ARM64 musl)` (image GCC 15 + clang 23). Exit criteria met: minimal transitions pass on macOS and Linux, each control fails as intended, and the deliberate `-S` gap reports `KACHE_MISSED_REUSE`. `make cc-bench` moves to CCP-09. |
| CCP-02 | RUNNING | CCP-01 | Branch `feat/ccp02-input-identity`: 20 input-identity cases; see "CCP-02 input identity" below. Not yet covered: same-path compiler replacement, wrapper/driver-mode selection, VFS overlays, PGO profiles, -march=native, user key config, diagnostic locale, profile switches against a populated cache, hash/compile/publish race barriers. |
| CCP-03 | RUNNING | CCP-01, CCP-02 | Branch `feat/ccp03-chromium-flags`: Chromium official flags verified and landed from the WIP branch; `-S -o` cached. GNU `@file` response files cached (PR #38; `cc-rsp-basic`, `-quoted`, `-nested`, `-missing` blocking); `-fsanitize-ignorelist` keyed by content (PR #39). Open: further `@file` variants (planned), clang `--config` and GCC `-specs` (open cases), `-x` and `-Wp,`/`-Xpreprocessor` audits. See "CCP-03..CCP-08 progress" below. |
| CCP-04 | LIMIT | CCP-02 | Comment-only reuse is declined: comment text and width are observable (see "CCP-04 finding"). ccache's reuse is wrong output when a comment changes width before code (`cc-eqp-column-significance`) and when a same-width reword changes GCC's fallthrough warning (`cc-eqp-fallthrough-comment`, blocking, ccache wrong in both modes). `cc-eqp-comment-only` stays open as the recorded gap. |
| CCP-05 | RUNNING | CCP-02, CCP-03 | `.gcno`, `.dwo` and `.dia` sidecars cached (PR #52); `cc-out-gcno` (GCC and clang), `cc-out-split-dwarf`, `cc-out-serialized-diagnostics` blocking. See "CCP-05 sidecars" below. Diagnostic formatting keyed per entry (`cc-out-diagnostics-format*`). Diagnostics-bearing entries replay only under their own color setting; empty diagnostics serve every setting (`cc-out-diagnostics-color-reuse`, `-color-cross`, `-source-escapes` blocking). GCC `-fdiagnostics-format=json-file` cached as a sidecar (`cc-out-diagnostics-json-file`, blocking); SARIF outputs refused for good (see "CCP-05 GCC diagnostic files"). Open: sidecars under `-S`. |
| CCP-06 | RUNNING | CCP-02, CCP-03, CCP-05 | clang PCH builds and `-include-pch` consumers cached by default (`cc-pch-clang-use`, `cc-pch-clang-define-variants` blocking); see "CCP-06 PCH" below. Clang explicit module consumers (`-fmodule-file=`, `-fprebuilt-module-path=`) cached; `cc-module-fmodule-file`, `cc-module-body-edit`, `cc-module-prebuilt-path`, `cc-module-add-remove` blocking on Linux clang 23; see "CCP-06 modules" below. Implicit PCH (GCC `<name>.gch` through `#include`/`-include`, clang's driver `-include` → `.pch`) keyed by every candidate; `cc-pch-gcc-include`, `cc-pch-gcc-leading-include`, `cc-pch-gcc-directory`, `cc-pch-gcc-skipped-text`, `cc-pch-clang-implicit` blocking; see "CCP-06 implicit PCH". Open: implicit modules, module production. |
| CCP-07 | RUNNING | CCP-03..CCP-06 | Blocking `cc-host-objc`, `cc-host-objcxx`, `cc-host-universal-objects`, `cc-host-universal-slice-headers` (universal objects keyed slice by slice: the driver refuses `-E` with several `-arch`, and a compile's depfile lists only the last slice's headers). `-Xarch_<arch>` operands keyed per slice (`cc-host-xarch`, blocking): each slice is re-parsed with its own operands, and dropping the other slices' operands is what separates them (keeping all fails `edit-x86_64-header` with `KACHE_WRONG_OUTPUT`). `-Xarch_*` outside a universal compile, or naming no built slice, is refused. zlib 1.3.2 (`make cc-project-zlib`) passes on M1 Apple clang 21 and the L4 image (GCC 15, clang 23), 2026-11-18. LLVM 21.1.8 subset (`make cc-project-llvm`: `llvm-tblgen` and `LLVMCore`, CMake + Ninja, kache as compiler launcher, TableGen-generated headers in the build tree) passes on M1 Apple clang 21 and the L4 image (GCC 15, clang 23; the image needs `apk add python3` for LLVM's configure), 2026-11-19: 377 objects byte-identical across raw, cold and warm kache, warm build 377 local hits and no compiles, warm `llvm-tblgen` output identical to raw. Autotools/libtool, 2026-11-20, informal probe on expat 2.7.1 (M1 Apple clang 21, `CC="kache cc"`, transparent profile). All 20 objects were byte-identical across raw, cold and warm kache. libtool's Darwin shared-object compiles (`-fno-common -DPIC`) passed through as an unsupported flag until `-f[no-]common` was keyed (`cc-flag-fcommon`). After that, a warm rebuild took 20/20 local hits, and only link steps passed through. Meson launcher, 2026-11-23, L3 (Alpine x86_64 musl, GCC 15.2.0, Clang 23.1.4): `make cc-project-meson` configures `test-projects/cc-meson` (C and C++ static library, executable, `configure_file` header) with gluon's `meson` and builds with gluon's `ninja` (gluon `5616e911`, `CC="kache gcc"` launcher prefix). Raw, cold, warm, edit, revert and relocation give byte-identical objects and a correct program; warm, revert and relocation run no compiles, and the header edit recompiles exactly its two dependents. cc-crate fixture, 2026-11-23, L3 (GCC 15.2.0, Clang 23.1.4): `make cc-project-cc-crate` builds `test-projects/cc-crate` (cc 1.11.1, one C and one C++ static library, Rust FFI) with kache as the `CC`/`CXX` prefix and `CC_KNOWN_WRAPPER_CUSTOM=kache`. Objects are byte-identical to raw; warm, revert and relocation run no compiles; the header edit recompiles both units; the binary prints the expected value in every phase. Chromium not started; Autotools now has a local glibc fixture (`make cc-project-autotools`, GCC 11/clang 14; see below); other host cells remain unverified. |
| CCP-08 | RUNNING | CCP-01 | Store-fault cases (`cc-rel-store-*`), broker exit race fix (PR #32), C/C++ hits verify blob hashes by default (PR #46), the non-local cache advisory left compiler stderr (PR #44). Identity cases (2026-11-19, blocking): `cc-input-compiler-replaced` (a driver at the same path that changes only code generation), `cc-input-driver-mode` (C vs C++ driver), `cc-policy-profile-switch` (transparent ↔ reproducible against a populated cache, every arm following the step's profile). Wrapper selection (shim vs `kache cc`) shares one entry. Open: user key config, controlled hash/compile/publication races, concurrency and eviction. |
| CCP-09 | RUNNING | CCP-02..CCP-08 | `make cc-bench` (`kache-cc-bench`): raw/ccache/kache over a tiny-C corpus (48 units, `-j1`), a header-heavy C++ corpus (12 units), pinned zlib and the pinned LLVM subset; scenarios cold, output-cleaned warm, settled and immediate source/header edits and reverts, relocated tree, and a single-unit compile after the kache daemon stops and at steady state. Untimed shim validation first; timed pairs admitted only when counters and objects match it. Estimator corrected to a log-space t-interval (lock.toml `uncertainty_correction`). See "CCP-09 baseline" below. |

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

- **Time macros (CCP-02).** Clang <16 ignores `SOURCE_DATE_EPOCH`; kache
  probes epoch support and passes time-dependent translation units through
  on those drivers. `cc-macro-date-time` probes the raw compiler's pinned
  date and time before running: unsupported drivers report
  `UNSUPPORTED_BY_COMPILER`; capable drivers still must match and restore.

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

Time macros: clang <16 ignores `SOURCE_DATE_EPOCH`. Kache verifies driver
support before trusting an epoch pin and passes time-dependent translation
units through when it is ignored (PR #88); ordinary inputs still cache.
The `cc-macro-date-time` capability probe gates only unsupported drivers,
leaving the blocking miss/restore requirements intact on GCC and newer clang.

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

**CCP-07 Autotools/libtool fixture (2026-11-20).**
`make cc-project-autotools` runs the checked-in
[`test-projects/cc-autotools`](../../test-projects/cc-autotools/README.md)
through raw and kache builds. Local Ubuntu 22.04 x86_64 evidence: GCC 11.4,
clang 14 and system `cc`, each with a prefix launcher and compiler-name PATH
shim; Autoconf 2.71, Automake 1.16.5 and Libtool 2.4.6. Configure probes use
the selected CC; libtool builds a shared/static library and linked test program.
Three objects (main plus PIC/non-PIC library units), one `.lo` descriptor and
two depfiles match raw byte for byte. Automake's depcomp rules use
`-MD -MP -MF`; deleting outputs/depfiles still restores them. Incremental header
edits compile current values; reverts and relocation restore. Each of six
compiler/launcher combinations records 3 cold misses, 3 edit misses and 3 hits
each for output-cleaned warm, revert and relocation, with no warm codegen.
Default path mapping is enabled; the transparent escape hatch disables
cross-root sharing by design. No production defect found. Disabling kache only
for the warm phase makes the lane fail its 3-hit requirement. This is opt-in:
CI integration needs Autoconf/Automake/Libtool and Python provisioning in the
chosen cells; no workflow/image changed. macOS, musl and ARM64 remain unverified.

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

**Open gaps at the time (2026-11-18; see Current state for today's):**
`@rsp` response files, clang `--config`, GCC `-specs`, explicit sanitizer
ignorelists, PCH consumers, the token-pasted header probe and comment-only
edits. Response files (PR #38), ignorelists (PR #39) and PCH consumers
(PR #58 and the implicit PCH work) have since landed.

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
`-frandom-seed` and `SOURCE_DATE_EPOCH`: the notes' bytes 8..12 and the
`stamp` field of the object's `gcov_info` record (`.data.rel+0x10` in a
GCC 15 aarch64 object), nothing else. clang's stamp is deterministic. Until
2026-11-19 the oracle zeroed every occurrence of the stamp's bytes anywhere
in the object, which could hide an unrelated difference in bytes that
happened to match, and looped forever on a zero stamp. It now locates the
one field structurally (`cc_oracle/gcov.rs`) and reports any pair it cannot
locate as `coverage stamp unresolved` instead of normalizing it. Rerun
2026-11-19: `cc-out-gcno` MATCHes for GCC 15 and clang 23 in the L4 image,
and the stale-output, omit-sidecar and disabled-invalidation controls still
detect their faults. `make cc-gcov-roundtrip` (link, run and `gcov` the
restored objects and notes, against a raw build) passes for GCC 15 (L4)
and Apple clang 21 (`GCOV="xcrun llvm-cov gcov"`); the L4 clang has no
profile runtime or `llvm-cov`.

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
- Coverage keys also bind every mapped checkout root on the preprocessing
  fallback. Clang 14 ignores `SOURCE_DATE_EPOCH` and takes that path; its
  deterministic object still embeds the absolute `.gcda` path, so the
  portable-key output guard previously declined to store it. Both discovery
  modes now restore the object/notes pair without relaxing that guard or
  gating `cc-out-gcno` on compiler version.

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
that write diagnostics to files pass through. A colored entry with
diagnostics serves only a colored invocation, and a colorless entry only a
colorless one; the other setting recompiles and replaces the entry, so a
tree built both ways alternates. Empty diagnostics serve every setting
(`cc-out-diagnostics-color-reuse`, blocking). From PR #56 until 2026-11-19 a
colored entry served a colorless invocation with every CSI and OSC sequence
removed. That is wrong whenever the message text holds escapes of its own:
GCC 15 prints a `#pragma GCC warning "\033[31mUSER\033[0m"` string's escape
bytes as they are under `-fdiagnostics-color=never`, so stripping printed
`warning: USER` where GCC prints the escapes (`cc-out-diagnostics-color-cross`
fails with `KACHE_WRONG_OUTPUT` on the old code; clang prints such bytes as
`<U+001B>`). Compiler decoration cannot be told from source text in the
stored stream, so nothing is stripped. `cc-out-diagnostics-source-escapes`
replays those escapes from a hit under each setting. ccache instead
compiles with color forced on, which the oracle's argument contract does
not allow.

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

**CCP-06 implicit PCH (distinct from the clang `-include-pch` support
above).** Observed with GCC 15.2 (L4 image), 2026-11-19: GCC loads
`<dir>/<name>.gch`, a file or a directory of variants, in place of a header
included before the main file's first token, by `#include` or `-include`,
checking each search directory before the header itself. It accepts a
`.gch` whose flags match, whatever header it was built from and whether that
header changed since. The plain `-E` output expands the textual header, and
once GCC loads a `.gch` its dependency file names neither the `.gch` nor the
header nor anything the header includes (`-fpch-deps` does not change that
in GCC 15). The objects differ with and without the PCH. So kache could
return stale output: before this fix `cc-pch-gcc-include` failed
`gch-appears` with `KACHE_WRONG_OUTPUT` (the object built without the PCH
was restored after a `.gch` appeared), and `cc-pch-gcc-leading-include` and
`cc-pch-gcc-directory` failed the same way. clang's driver likewise turns
`-include X` into `-include-pch X.pch` (or `X.gch`) when that file exists
(`cc-pch-clang-implicit` fails in the reproducible profile without the fix).
The key now folds every candidate that exists (`cc_implicit_pch_key`), by
name and search-directory position: for GCC, `<name>.gch` for each
`-include` operand and each `#include` before the main file's first token,
in the working directory and every user and `-isystem` directory; for clang,
the driver's `X.pch` and `X.gch`. A directory candidate folds each variant's
name and bytes. A leading `#include` spelled by a macro makes every `.gch`
in those directories a candidate. A compile with no candidate keys as
before. Blocking cases (GCC 15, clang 23, L4): `cc-pch-gcc-include`
(`-include`; absent, present, replaced, removed, and an edited header under
an older `.gch` GCC still loads), `cc-pch-gcc-leading-include` (`#include`,
`.gch` in an earlier search directory), `cc-pch-gcc-directory` (rejected
and accepted variants), `cc-pch-clang-implicit`. GCC writes a different
`.gch` on every build (a memory image), so a rebuilt PCH misses every
consumer, as in ccache, and the cases mark such files `unstable`. Limits:
the compiler's built-in system directories are not searched for `.gch`
candidates.

**Skipped text before the leading include (2026-11-20).** The scan for
leading `#include`s stopped at the first non-directive line, even when that
line sat inside `#if 0`. GCC does not count skipped text as the first
token, so for

```c
#if 0
ignored tokens
#endif
#include "h.h"
```

it still loads `h.h.gch`, while kache keyed no candidate. Reproduced on
main `6b592ce1` with `cc-pch-gcc-skipped-text` (GCC 15.2, L4 image): the
`.gch` holds `VALUE 1` and the textual header `VALUE 2`, and the source and
header bytes never change. After the `.gch` appeared, and after it was
replaced, kache restored the object built from the header, in both profiles
and on the warm repeats (`KACHE_WRONG_OUTPUT`). ccache did the same in both
modes (`CCACHE_WRONG_OUTPUT`). The removal steps matched. The scan now
tracks conditional depth. Text inside any conditional means the first
active token is unknown, so the scan treats the leading names as unknown:
every `.gch` in the search directories is a candidate, as for a
macro-spelled include. That path also folds a `leading=unknown` marker. The
marker gives every affected source a new key, so no entry stored under the
old narrower key (possibly an object built against a `.gch`) can be hit
again. Preprocess memo records hold the `-E` result, which the `.gch` does
not change, and need no invalidation. With the fix the case matches in both
profiles, and the other implicit-PCH cases still match.

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

**Extended 2026-11-23 (L3, GCC 15, Clang 23):** unused-macro and
inactive-conditional edits are observable in the same way. Under `-g3` an
added unused macro leaves the `-E` text unchanged but changes the object's
macro debug info. An apostrophe added inside `#if 0` leaves the `-E` text
unchanged, but GCC warns `missing terminating ' character`. kache therefore
recompiles both, where ccache's preprocessor mode reuses.
`cc-eqp-unused-macro` and `cc-eqp-inactive-conditional` are `open` gap cases.
`cc-eqp-inactive-text-warning` (blocking) proves kache replays the new warning,
and `cc-eqp-unused-macro-g3` (blocking) proves the object changes.
`cc-eqp-dep-refresh` and `cc-eqp-memo-fastpath` stay `planned`: they only
apply once equivalent reuse exists.

## CCP-09 baseline

Everything in this section is **older diagnostics**. The runs were made in
diagnostic mode on a loaded host, before the acceptance mode and before
#74/#75. They rank losses historically and are not acceptance evidence.
Current-head results are under "Current state".

**Method.** `make cc-bench TOOLCHAINS=...` fetches and verifies the pinned
archives (untimed), then for each workload replays every arm untimed
through compiler shims that log one line per execution. That replay
fixes, per scenario, the code generations, the cache counters (kache's
event log, ccache's `--print-stats`) and an object digest that must equal
raw's. Timed pairs then run in a seeded order (lock seed 20261117) on the
real compilers and are admitted only when counters and objects match the
validated replay. Preparation (unpack, configure, CMake) and the settle
waits are outside the timer. Kache runs its default profile with
`KACHE_BASE_DIR` at the tree; raw and ccache get the `-ffile-prefix-map`
and `SOURCE_DATE_EPOCH=0` kache adds, and ccache gets `base_dir` and
`hash_dir = false` for the relocated tree. Settled edits wait past the
2 s racing-write window; immediate edits compile at once. Every edit first
waits 1.1 s, because macOS make compares whole-second stamps.

**Validation results, 2026-11-19 (untimed).** Every workload validates on
M1 Apple clang 21 and in the L4 image with GCC 15 and clang 23: all three
arms make raw's objects in every scenario, kache and ccache restore every
unit on warm, revert and relocated builds, and edits recompile exactly the
units raw recompiles (LLVM: 377 units cold, 41 for the `Comdat.h` edit).

**First measured loss (fixed).** The L4 GCC validation showed kache
recompiling every unit of the C++ corpus on every warm build ("the compile
left no usable read set"): the hidden-assembler-input check refused any
expansion containing `\()`, and libstdc++'s `<regex>` spells it in a
string literal. The check now requires a closing `.endm`/`.endr` for that
backstop (`fix(cc): stop refusing C++ units whose strings spell \()`).

**First timed baseline, 2026-11-19 (macOS, provisional).** M1 Max (10
cores), Apple clang 21.0.0, ccache 4.14.3, kache 2.0.0-rc.2 at dd428ad3,
`-j10` (the tiny-C corpus `-j1`), 5 pairs per scenario, every timed replay
admitted. The host was not quiet: an unrelated build put the 1-minute load
at 2 to 46 during the timed builds, against the lock's quiet-host rule. The
pairs are interleaved, so the ratios below are usable for ranking losses,
but none of them is a parity or performance claim. Rerun on a quiet host
before recording verdicts.

kache time over ccache time (geomean, 95% log-space CI):

| scenario | tiny C | C++ headers | zlib | LLVM subset |
|---|---|---|---|---|
| cold | 1.07 (0.97..1.19) | 1.06 (0.91..1.24) | 1.22 (1.17..1.27) | 1.04 (0.93..1.17) |
| warm | 2.22 (1.87..2.63) | 2.22 (2.05..2.41) | 4.49 (3.67..5.49) | 5.98 (5.67..6.31) |
| edit source, settled | 1.08 (0.83..1.39) | 0.95 (0.91..1.00) | 1.24 (1.19..1.29) | 1.13 (1.07..1.20) |
| revert source, settled | 1.11 (0.68..1.83) | 1.83 (1.52..2.20) | 1.88 (1.81..1.95) | 4.11 (3.92..4.30) |
| edit header, settled | 1.48 (1.26..1.73) | 1.09 (1.06..1.12) | 1.47 (1.37..1.57) | 1.06 (1.03..1.09) |
| revert header, settled | 6.65 (5.68..7.80) | 8.88 (7.59..10.38) | 3.03 (2.47..3.72) | 8.69 (7.92..9.54) |
| relocated | 2.57 (1.97..3.35) | 2.99 (2.67..3.34) | 4.74 (3.74..5.99) | not relocatable |

Immediate edits track the settled ones (tiny C source edit 1.13, revert
1.33; LLVM 1.15 and 4.11). Over raw, kache's cold build costs 2.24x on the
`-j1` tiny-C corpus, 1.38x on C++, 1.57x on zlib and 1.30x on LLVM, and its
warm build is 0.33x raw on LLVM but 1.60x raw on zlib.

**Losses ranked by wall time lost to ccache, with causes from the event
logs:**

1. LLVM warm, 11.9 s per build (and revert-source, 0.27 s for one unit):
   387 of 420 hits ran the preprocessor, about 270 ms each. The memo
   matched every input, then failed its include-resolution check: the
   digest folded `__has_include` probes in read-set order, and a memo
   returns its read set in the order its shared rows were first
   recorded. Fixed by folding probes in a canonical order
   (`fix(cc): fold header probes in a canonical order`).
2. Header revert, 1.4 to 1.8 s per build in every workload: the memo kept
   one read set per unit, so a header edit replaced it and the revert
   preprocessed every dependent unit again. Fixed by keeping each unit's
   eight most recent read sets
   (`perf(cc): keep a unit's recent read sets so a reverted header hits`).
3. Header edit, 1.2 s on tiny C and 0.16 s on zlib: kache preprocesses
   and then compiles each dependent unit, as ccache does, but slower.
   Not yet broken down.
4. Warm and relocated hits on the small workloads, 0.3 to 0.4 s per
   build: about 11 ms per hit against ccache's 5 ms on tiny C, and every
   zlib hit preprocessed. Rerun after fixes 1 and 2 before attributing
   further.
5. Cold: within the CI of ccache except zlib (1.22). LLVM pays one
   preprocessor run per unit before compiling, as ccache does, because a
   unit whose working directory is the build tree is not portable enough
   to compile first.

Uncached compiler-seconds by reason: the only refusals in the timed
workloads are zlib's two link steps per build (`cc link mode`, which
ccache does not cache either).

**Rerun after fixes 1 and 2, 2026-11-19 (macOS, provisional).** The same host
and protocol ran kache at ae7ee3e1 (PR #62: the canonical probe order, the
recent read sets, and a per-read-set record of header-probe resolution that
stops each hit parsing megabytes of SDK header scans). It also included the
`KACHE_BASE_DIR` `..` fix (`fix(cc): name memo inputs by their mapped
spelling in the shadowing digest`, KACH-28). The 1-minute load was 1.7 to 13
on the small workloads and 8 to 34 on LLVM. The LLVM run admitted 3 pairs: two
kache replays counted one or two hits short because the event log trailed
the build, and were rejected. `fix(cc-bench): wait for the events validation
recorded before counting` removes that race. No warm, revert or relocated
hit ran the preprocessor.

| scenario | tiny C | C++ headers | zlib | LLVM subset |
|---|---|---|---|---|
| cold | 0.79 (0.77..0.81) | 1.11 (1.08..1.15) | 1.07 (0.84..1.36) | 1.09 (0.97..1.22) |
| warm | 2.12 (1.92..2.35) | 1.09 (1.02..1.18) | 1.87 (1.71..2.04) | 1.15 (1.12..1.18) |
| edit source, settled | 0.91 (0.72..1.14) | 0.97 (0.94..0.99) | 1.11 (0.76..1.63) | 1.13 (0.97..1.31) |
| revert source, settled | 1.56 (1.29..1.88) | 1.36 (1.12..1.64) | 1.17 (0.94..1.48) | 1.20 (1.01..1.44) |
| edit header, settled | 1.18 (1.13..1.23) | 1.07 (1.06..1.08) | 1.24 (1.12..1.36) | 1.05 (1.03..1.07) |
| revert header, settled | 1.93 (1.57..2.37) | 1.33 (1.23..1.43) | 1.55 (1.39..1.72) | 1.41 (1.07..1.87) |
| relocated | 2.22 (1.86..2.64) | 2.48 (2.35..2.61) | 2.02 (1.89..2.17) | not relocatable |

LLVM's warm build went from 5.98x to 1.15x ccache (14.0 s to 2.66 s), and
its header revert from 8.69x to 1.41x. **Losses now, ranked:**

1. Per-hit fixed cost on small units: a tiny-C warm build loses 0.26 s
   over 48 hits, about 5 ms a hit (kache 9.8 ms, ccache 4.4 ms), and
   relocated and revert builds pay the same. A single-unit compile at
   steady state is 1.85x ccache (38.7 ms to 17.2 ms). After a daemon stop it
   is 6.6x (127 ms), so the first compile after a daemon stop pays for
   starting the daemon. A phase trace puts kache's own work for such a hit
   near 3 ms. The rest is outside it (client start, daemon round trip,
   process exit).
   Timestamps across the client and the daemon put 2.6 to 6.6 ms of a
   9 ms hit between the client's connect and the daemon's accept. The
   accept loop slept 5 ms whenever nothing was pending. It now waits in
   `poll()` on the listener (`perf(daemon): accept a connection when it
   arrives, not after a 5 ms sleep`), and a warm single-unit hit went from
   14.2 ms to 7.8 ms (hyperfine, loaded host). The worker's own 2.7 ms and
   about 1 ms of protocol round trips remain. KACH-29 now keys identified
   self-identifying native ELF/Mach-O Clang probes on documented driver inputs plus
   declared `key_env_vars`; the preprocessor memo shares the input set.
   Hyperfine's `HYPERFINE_*` unsetting workaround can go for those drivers.
   GCC (which lacks an OS-derived self path), non-self-identifying drivers,
   forwarding wrappers, Apple's xcrun shim, known launchers and other
   binary formats retain whole-environment keying and still need it.
   Probe schema 8, preprocess memo v11, direct-capability material v4 and epoch probe v2
   keep records from the previous policy separate.
   Local Ubuntu 22.04 verification (GCC 11, Clang 14): 967 focused tests
   passed (2 ignored), `make check` passed, and all five negative controls
   were detected. At that revision the full oracle had 199 MATCH, 44
   compiler-unsupported results and one `cc-out-gcno` reproducible/Clang
   `KACHE_MISSED_REUSE`;
   the pre-change probe-schema-5 release engine reproduced that same miss
   (subsequently fixed under CCP-05 above).
   A provisional 25-run warm GCC microcheck with changing
   `HYPERFINE_ITERATION` went from 6.2 ± 0.2 ms to 3.5 ± 0.2 ms; compiler
   probe records fell from 29 to 1. Other checks ran concurrently, and
   hyperfine warned about sub-5-ms commands: this is not acceptance timing.
2. Header edit, 1.18x on tiny C and 1.24x on zlib: the miss path.
3. Cold C++ and LLVM, 1.09x to 1.11x; tiny-C cold now beats ccache (0.79x).

**Since that rerun, 2026-11-19..20.** Merged against losses 1 and 2:

- #65: the accept-loop wait above.
- #68: an event no longer rewrites the build-session marker within the
  same second. On a warm tiny-C hit, the session-marker work went from a
  median 356 us to 77 us.
- #69: the daemon keeps parsed header probe scans by content hash. On a
  memo miss after a header edit, the include check went from 14.9 ms to
  1.7 ms, and the whole miss from 74 to 60 ms; 52 ms of that is the
  preprocessor and the compile.
- #72: the compiler probe key leaves out per-command shell and
  build-driver variables. GNU make 4.4's per-run jobserver FIFO in
  `MAKEFLAGS` had re-probed once per build.

A warm tiny-C hit's worker time is about 2.4 ms:

| Part | Median |
|---|---|
| key (memo lookup 0.26 ms) | 0.7 ms |
| restore: three broker round trips (prepare, publish, mtime stamp) | 0.67 ms |
| event | 0.25 ms |
| store open (SQLite open and pragmas) | 0.3 to 0.4 ms |

Each part is now below 0.4 ms. Folding the restore's round trips into
one changes the client-daemon protocol. Keeping the store open across
hits must survive an index recovery that replaces `index.db`.

**Rerun at 8430f6b6, 2026-11-20 (macOS, provisional).** Same host and
protocol, run with `--workload corpus-c-tiny --workload corpus-cxx-headers
--workload zlib`. LLVM was not rerun. 5 pairs per scenario, every timed
replay admitted. The 1-minute load was 3.3 to 12.8. kache / ccache geomean,
with the previous rerun's value in brackets:

| scenario | tiny C | C++ headers | zlib |
|---|---|---|---|
| cold | 0.83 (0.79) | 1.11 (1.11) | 0.96 (1.07) |
| warm | 1.89 (2.12) | 1.00 (1.09) | 1.22 (1.87) |
| edit header, settled | 1.34 (1.18) | 1.03 (1.07) | 0.995 (1.24) |
| revert header, settled | 1.63 (1.93) | 1.25 (1.33) | 1.11 (1.55) |
| revert source, settled | 1.00 (1.56) | 1.32 (1.36) | 1.05 (1.17) |
| relocated | 1.96 (2.22) | 2.28 (2.48) | 1.45 (2.02) |

The tiny-C header edit's interval (1.11..1.61) overlaps the previous one.

**Losses now, ranked:**

1. Relocated. Every relocated hit ran the compiler probe again (28 ms for
   a C++ unit): the probe key held the literal cwd and `PWD`, so a tree
   moved with its `KACHE_BASE_DIR` re-probed in every directory. A relocated
   C++ build used 880 ms of CPU against 374 ms warm, for the same 12 hits.
   The fix keys a cwd under a declared base dir by its path below it.
2. Per-hit fixed cost on tiny C: 8.7 ms against 4.5 ms per hit.

   Under hyperfine, a single warm hit costs kache 9.3 ms and ccache 7.2 ms.
   A process that does nothing takes 4.1 ms, so net of that the hit is 5.2 ms
   against 3.1. Uncommitted timing marks put a 7.3 ms hit, from client start
   to done, at:

   | Stretch | Median |
   |---|---|
   | client config load and stdio capture | 0.45 ms |
   | daemon, from request to commit | about 1.2 ms |
   | worker start to compiler dispatch | 0.6 ms |
   | the C/C++ path | 3.5 ms |
   | after the C/C++ path returns, to done | 0.4 ms |

   In the request-to-commit stretch, a per-request descriptor channel (a
   temporary directory in `/tmp` and a datagram socket) takes 0.31 ms, and
   rebuilding the invocation from the received descriptors takes 0.32 ms.
   The worker-start stretch held a PATH scan in `launcher_args_for` (#75
   skips it).

   Candidates, none above 0.5 ms:
   - pass descriptors over the request stream instead of a per-request
     socket (a protocol change);
   - fold the restore's three broker round trips into one;
   - keep the store open across hits;
   - reuse the daemon's configuration.

## Open gaps and blockers

See "Current state" at the top; this list was folded into it on
2026-11-19. The bootstrap items it held are resolved: the Linux gates are
required and green, main is green, and the gluon-ninja shadow is fixed
upstream.

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
2. Verify by the tiers in [AGENTS.md](../../AGENTS.md#verification-tiers):
   the regression while iterating; `make check`, plus `make cc-oracle
   CASE=ID` for the cases a C/C++ keying, restore, output or diagnostic
   change affects, before `gh pr create`; CI on the PR (the oracle and its
   negative controls on macOS and four Linux cells, `e2e`, Linux musl
   workspace tests) before merging. Full runs (`make cc-oracle-full`, the
   project checks, `make cc-bench`) only on request. To reproduce a Linux
   cell locally, use the L4 image with `TOOLCHAINS="gcc clang"`; after an
   `rsync -a` into a scratch copy, touch its `.rs` files, since rsync
   keeps older mtimes and a copy that built a hand-applied mutation keeps
   the mutated binary. `make e2e` on this host needs
   `PATH=/opt/homebrew/bin:$PATH`.
3. Follow
   [Focused cycle after #143](#focused-cycle-after-143-2026-11-23), in
   order: quoted-macro capture regression, integrated pool acceptance, one
   measured loss, then host gaps. Do not repeat the pre-checkpoint bisect or
   treat scanner, standalone or PR A/B diagnostics as passing acceptance.
   Commit every future acceptance report and link Current state.
   Additional engineering coverage remains open:
   - Autotools/libtool launchers (CCP-07) now have a bounded local glibc
      fixture (`make cc-project-autotools`); provision and run other host
      cells before calling the matrix complete. Meson launchers remain open.
      zlib, LLVM and cc-rs build scripts already cache fully.
   - KACH-29 is implemented for self-identifying native Clang: documented
      driver environment inputs and declared patterns key the probe;
      GCC and non-self-identifying drivers retain whole-environment keying. Native hits no longer
      need the hyperfine environment workaround. Cross-host timing remains
      part of the acceptance rerun, not inferred from this key change.
4. All campaign work runs in isolated worktrees under `.work/wt/`
   (gitignored via `.git/info/exclude`). Evidence needed afterwards lives in
   CI runs (90-day retention) or in committed cases and scripts.
