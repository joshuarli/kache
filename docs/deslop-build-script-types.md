# Deslop: build-script cache types

DSLP-14, Types component 10 of 28. Independent base:
`3f0218aba1b34ea7c809c807db0719ab79af9295`. Scope is the single
`src/build_script.rs` file: 6,814 lines including comments and tests (2,424
before the test module's cfg attribute, 4,390 from that attribute onward).
Related child modules, launcher, wrapper, store and fixtures were inspected
as consumers/boundaries, not added to this ticket's diagnostic totals.

Platform: Linux x86_64. rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`;
Clippy `0.1.101 (5b7b37aefc 2026-11-08)`. No additional tools installed.
The named deslop Types and typing-audit guidance were used. No source change
met the adoption bar; this is an audit/backlog-only change, not a claim that
the entire strict candidate profile is clean.

## Measurement

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn -F unsafe_code \
  -W clippy::wildcard_enum_match_arm \
  -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used \
  -W clippy::redundant_clone -W clippy::needless_pass_by_value
```

The kache package has `default = []` and no optional features. Its library
target is `src/main.rs`; the build-script module is included there. Cargo
must compile the package graph, but only diagnostics with a primary span in
`src/build_script.rs` are counted here. Deduplicate by lint code, file, line
and column across lib/test targets. `--cap-lints warn` makes the forbid probe
measurable without stopping at its intentional unsafe operations; a zero
exit is not a global forbid pass.

| Candidate | Production | Tests | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 3 | 45 | Retain owned-descriptor Linux lease operations and test FFI/environment setup. |
| `wildcard_enum_match_arm` | 1 | 0 | Retain `link_stays_inside`'s rejecting default for unsupported path components. |
| `match_wildcard_for_single_variants` | 0 | 0 | No adoption witness. |
| `cast_possible_truncation` | 8 | 0 | All are monotonic elapsed milliseconds narrowed to existing u64 telemetry fields. |
| `cast_sign_loss` | 0 | 0 | No adoption witness. |
| `unwrap_used` | 0 | 676 | Test setup, assertions and fixture teardown; no production trust deletion available. |
| `expect_used` | 0 | 13 | Test readiness assertions and explicit expected files/metadata. |
| `redundant_clone` | 1 | 6 | Ownership/style suggestions, not type defects. |
| `needless_pass_by_value` | 0 | 0 | No adoption witness. |

Counts remain unchanged after the audit: 13 production and 740 test
diagnostics. No lint setting was adopted. Zero-hit settings do not reject a
demonstrated defect or delete an escape hatch. Broad settings with required
FFI or hundreds of intentional test assertions do not satisfy the density
and witness conditions. These retained diagnostics are not all defects and
are not automatically classified as false positives.

Specific dispositions:

- `wait_for_no_writers_with` owns the file/descriptor and makes three libc
  `fcntl` calls: configure the descriptor's lease-break signal, acquire a read
  lease, release it. Errors remain a best-effort false result; the retry path
  distinguishes EAGAIN and observes its deadline. `forbid(unsafe_code)` would
  forbid required kernel operations, not find a new defect. Test unsafe sites
  include independent lease probes and environment restoration under the
  existing process-state lock. No new environment mutation or broad allow.
- The eight elapsed-time casts are at lines 553, 556, 561, 643, 2004, 2047,
  2080 and 2103. Their producers are `Instant::elapsed().as_millis()` and
  consumers are build-script event/store duration fields. A u64 millisecond
  overflow would require roughly 584 million years of elapsed execution.
  Replacing a cast with an arbitrary saturating/default policy would change
  its extreme-value contract without a reachable defect witness. None was
  adopted merely to lower the count.
- The path wildcard at line 2179 rejects every component not explicitly
  allowed by the internal-relative-link grammar. It fails closed for new or
  unsupported component variants; changing it to an accepting default would
  be a defect, not a Types improvement.
- `run_cached` clones StoreOptions at line 536 before passing ownership into
  `Store::open_with_options`; Clippy can remove that copy. Test clones at
  3365, 4580, 4581, 5724, 5889 and 5927 are similar style findings. No clone
  suppresses a type check or admits invalid state. Removing copies is not an
  adoption witness for this Types ticket.

## Escape inventory and type boundaries

The campaign text counter is:

```sh
rg -n '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\(' src/build_script.rs
```

Matching lines: production **5 -> 5**, tests **711 -> 711**, total
**716 -> 716**. This line count is not the Clippy count: it includes the
diagnostic text "unsafe name", the event-logging `too_many_arguments` allow,
and comments, and excludes `unwrap_err` and `cfg_attr` allowances. Two
`cfg_attr(not(unix), allow(dead_code))` attributes on launcher constants are
separately retained for platforms where the native launcher is not installed.
There are no production unwraps/expects in this file. No suppression, cast,
assertion or panic was introduced, and no checker/runtime witness is claimed
for an unmade source change.

Boundaries traced:

1. **Dispatch and Cargo environment.** `src/main.rs` selects `run_shim` from
   `KACHE_BUILD_SCRIPT_PATH`. `src/wrapper.rs` recognizes build-script output
   and installs the launcher after compilation. `Environment::capture`
   requires absolute OUT_DIR and manifest roots, retaining optional TARGET,
   CARGO_HOME/HOME and base-dir values. Missing capture data triggers the
   existing uncached path; an absent value is not an empty fabricated path.
   The preserved script's exit code and stdout/stderr replay are unchanged.
2. **Prediction and cache keys.** `stored_prediction` returns None for absent
   or incompatible schema/version data, but propagates malformed JSON errors.
   `Prediction.embedded_roots` deliberately distinguishes old `None` from
   present-but-empty roots. `rewrites_text` and `portable_out_dir` preserve old
   readers' restrictions. Action-v3 keys fold tool bytes/fingerprints,
   environment names and presence, declared inputs, target ownership and
   embedded roots; nonportable outputs remain bound to the appropriate root.
   The `fold` usize-to-u64 lengths are non-narrowing on supported 32/64-bit
   targets, not among the eight truncation diagnostics. No recipe changed.
3. **Persistence and restore.** Private `Manifest`, `EmptyFile`, `Symlink`
   and child `RootedSymlink` use serialized strings rather than claiming raw
   stored paths are validated. Reads accept manifest versions 1 through 5,
   validate named/rooted paths before restore, and use `checked_relative`
   before joining stored names under OUT_DIR. The latter rejects empty,
   absolute and non-Normal components. `restore_symlink` verifies its target
   and real parent directories. Replacing these strings with trusted PathBuf
   fields at deserialization would not by itself establish those invariants.
4. **Failure and partial state.** `finish_restore` cleans rejected outputs;
   a cleanup failure gets `RestoreCleanupFailed` context and cannot silently
   fall back to the real script. `InputsChanged` identifies the limited
   settling-rerun case, distinct from other recording refusals. `Result`,
   `Option`, defaulted legacy lists and explicit booleans already preserve
   these partial/legacy states; no new enum removes a demonstrated escape.
5. **Consumer fixtures.** Existing tests cover replay across checkouts,
   portable/remapped roots, missing or changed inputs, legacy predictions,
   rooted/escaping links, named external libraries, failed cleanup, shared
   binary installation and launch records in legacy and per-unit Cargo
   layouts. Relevant integration consumers include
   `tests/build_script_declared_inputs_test.rs`,
   `tests/build_script_inputs_test.rs`, `tests/build_script_launcher_test.rs`
   and the generated-OUT_DIR include case in `tests/integration_test.rs`.

The child modules `declarations`, `inputs`, `outside`, `restore` and
`rooted_links` were read at these interfaces; this is not a separate full
strict-profile audit of their source. Linux compilation does not validate
macOS-only ZERO_AR_DATE or non-Unix branches. Existing safety assumptions in
kernel APIs, filesystem races and lock-held environment helpers remain;
compiler-clean counts are not proof of those runtime assumptions.

## Verification and disposition

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache --all-features build_script::tests::
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

The scoped filter executed 96 module tests. Other binaries ran zero tests
under this filter; do not count their empty passes as integration coverage.
The required workspace gate supplies unfiltered consumer verification.

```text
CLIPPY_MEASURE_EXIT=0
test result: ok. 96 passed; 0 failed; 0 ignored; 0 measured; 3108 filtered out; finished in 0.49s
BUILD_SCRIPT_TEST_EXIT=0
test-resource guard: all checks passed
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.70s
MAKE_CHECK_EXIT=0
```

No source/API, CLI, exit-code, wire, key, manifest or launcher change. Runtime
and performance are unaffected because the source diff is empty; no
performance gate or oracle/project/scenario campaign was run. Cross-platform
execution was not performed locally. No new defect or owner decision was
proved, so no finding was filed and `Next: 103` stays unchanged. The absent
`todo` CLI was handled by the established direct backlog-edit convention.
