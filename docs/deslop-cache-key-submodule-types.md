# Deslop: cache-key submodule types

DSLP-15, Types component 11 of 28. Independent base:
`3f0218aba1b34ea7c809c807db0719ab79af9295`. The ticket's directory contains
two files, 18,578 lines before this change (the ticket estimated 18,491):
`tests.rs` (18,292) and `apple_install_name.rs` (286). Only the latter has
production code: 166 lines before its test-module cfg attribute. The parent
`src/cache_key.rs` is DSLP-16, inspected here only at consumer boundaries.

Platform: Linux x86_64; rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`;
Clippy `0.1.101 (5b7b37aefc 2026-11-08)`. No additional tools installed.
Applied the deslop Types and typing-audit guidance. No forbidden C/C++
campaign paths, shared configuration, public APIs or dependencies changed.

## Profile and dispositions

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

Count unique primary spans in `src/cache_key/`, deduplicated by code, file,
line and column across lib/test targets. kache has no optional features;
`default = []`. Its library target is `src/main.rs`. `--cap-lints warn`
permits measurement of required unsafe/test operations; its zero exit is
not a claim that the entire candidate profile passes as errors.

| Candidate | Production before -> after | Tests before -> after | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 0 -> 0 | 22 -> 22 | Test environment mutation/restoration, retained. |
| `wildcard_enum_match_arm` | 0 -> 0 | 0 -> 0 | No demonstrated adoption witness. |
| `match_wildcard_for_single_variants` | 0 -> 0 | 0 -> 0 | No demonstrated adoption witness. |
| `cast_possible_truncation` | 0 -> 0 | 6 -> 6 | Bounded fixture sizes and test-clock assumptions, retained. |
| `cast_sign_loss` | 0 -> 0 | 0 -> 0 | No demonstrated adoption witness. |
| `unwrap_used` | 2 -> 0 | 1698 -> 1699 | Adopt only on `layout`; fixture assertions remain outside its scope. |
| `expect_used` | 0 -> 0 | 46 -> 46 | Explicit test setup/readiness assertions, retained. |
| `redundant_clone` | 0 -> 0 | 23 -> 23 | Ownership/style findings, not a typing witness. |
| `needless_pass_by_value` | 0 -> 0 | 0 -> 0 | No demonstrated adoption witness. |

Adopted: `#[deny(clippy::unwrap_used)]` on `layout`, removing both production
escapes with **zero false positives**. The scope is intentional: it guards
the byte-reader producer, not thousands of fixture assertions. No blanket
allow was added. Other zero-hit settings reject nothing demonstrated here;
test-only settings would mostly reject intentional setup/assertions rather
than remove unchecked production trust. Those diagnostics are retained with
reasons, not reclassified wholesale as bugs or false positives.

This supersedes the `unwrap_used` rejection in
`docs/deslop-cache-key-submodules.md`, which kept both unwraps because every
caller passes a length-checked 4-byte slice. The readers now return that
invariant as an error instead of relying on it.

Specific retained trust:

- `tests.rs` owns process-state fixtures including `ScopedEnv`, whose callers
  must hold `key_test_lock`/the process-state test lock. The unsafe operations
  measured here are environment changes and restoration. This audit does not
  prove every environment reader in the process honors that lock. New tests
  do not read or mutate the environment.
- Two truncation findings are the dylib fixture's `usize` command length
  encoded as u32; fixture names are small literals. Three are SystemTime
  nanoseconds encoded as i64 in file-race tests, retaining their current-clock
  assumption rather than inventing an overflow fallback. The sixth is the
  positive `MIN_PERSISTED_HASH_BYTES` threshold cast for test-string size.
  These are not production input parsing. Production u32-to-usize reads and
  the 1 MiB usize-to-u64 limit are non-narrowing on supported 32/64-bit targets.
- Test expects name absent prerequisites or intended record/output states;
  unwraps include parser setup, file operations, expected cache entries and
  successful probe fixtures. Converting them to Result would move the same
  failure to the test harness, not establish a new production invariant.
- The 23 clone suggestions concern owned fixture values that could be moved
  or borrowed. No clone suppresses a type check; no ownership-style edits.

Campaign text counter:

```sh
rg -n '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\(' src/cache_key
```

Matching lines: production **2 -> 0**, tests **1763 -> 1764**, total
**1765 -> 1764**. The new fixture-layout unwrap is test-only; the production
panicking conversions are gone. `tests.rs` stays at 1,753 matching lines;
`apple_install_name.rs` goes from 12 to 11. Text counts include comments,
generated Rust source and strings containing "unsafe", unlike lint counts.

## One contract, producer through consumers

Classification: **not a reachable malformed-file bug at existing callers;
unchecked conversion trust removed**. Previously private `Word` accepted
`&[u8]` and returned u32 by unwrapping conversion to a four-byte array. Every
existing call passes exactly four bytes after header/command bounds checks.
The signature did not express the conversion's possible failure for other
slice widths; a future misuse could panic or tempt a fabricated zero value.

`Word` now returns `Result<u32>`. Both endian readers use `try_into()?`, and
all seven production consumers propagate the result. The compiler rejects
using that Result directly as a numeric command size/tag/offset. There is no
default integer, weakening annotation, unsafe read, new allocation on the
success path or change to borrowed input ownership. New direct-reader tests
exercise all four layout/endian selections, zero-valued four-byte input,
empty input and widths 1, 2, 3 and 5.

Boundaries checked:

1. `read` opens the library, caps text stubs at 1 MiB, and for Mach-O reads a
   fixed header followed by bounded load commands, not the whole file. `stub`
   keeps its conservative TAPI spelling/quoting/duplicate-name rejection.
2. `layout` recognizes both Mach-O widths and byte orders and rejects unknown
   or universal-binary magic. `macho` bounds header, command count, total
   command bytes, per-command length, LC_ID_DYLIB offset, terminator and UTF-8.
   The seven current word slices are all exactly four bytes; therefore the
   new conversion errors cannot replace an existing file-input success or
   error. Existing truncated-input tests cover every prefix of every layout.
   This is not a claim that all unrelated indexing or parser assumptions were
   statically eliminated.
3. Parent `fold_native_link_inputs` selects install-name scanning only when
   `links && context.target.contains("-apple-")`, including appropriate native
   directories/frameworks and resolved verbatim libraries. Its
   `fold_apple_install_name` reads the same String and folds the same
   `apple_install_name.v1:` field and `index/file=name` bytes. System-library
   and nonlinking exclusions, relative/absolute install names and refusal on
   unmodeled libraries remain unchanged. No key-version bump is needed.
4. `compute_key_into` propagates the native-input result through
   `compute_cache_key_with_outputs`, preserving its paired key Result and
   partial KeyOutputs. `src/compiler/rustc.rs::cache_key_in` passes that
   Result through key finishing without changing error states. The
   wrapper/store/prediction users see no API, key,
   serialized metadata, wire, artifact, CLI or exit-code change. No store
   format or compatibility reader was edited.
5. `tests.rs` consumes parent internals and the test dylib builder, not a
   released pin. Its native-link fixtures simulate Darwin targets on Linux:
   `apple_dylib_and_framework_install_names_bind_link_keys` and
   `apple_verbatim_extensionless_dylib_install_name_is_keyed` exercise this
   parser through key folding. Existing tests also cover TAPI names, relative
   name sharing, system exclusions and malformed-library refusal. Other test
   groups cover source identity, predictions, toolchain/environment snapshots,
   file-hash memoization, racy inputs, depfiles and key normalization; their
   fixture types and supported None/absent/error states were not changed.

The Linux tests execute the parser and simulated Apple-key path; they do not
execute a real macOS SDK/linker. The full parent implementation is a separate
ticket, not implicitly certified by this directory audit.

## Witnesses and verification

Reintroducing a reader unwrap (inside Ok, to retain the new return type)
under the adopted guard produced exactly the intended diagnostic:

```text
error: used `unwrap()` on a `Result` value
   --> src/cache_key/apple_install_name.rs:107:55
LINT_WITNESS_EXIT=101
```

Injected fault: replace the little-endian checked conversion with
`bytes.try_into().unwrap_or([0; 4])`. The width test rejects the fabricated
zero for an empty slice:

```text
test cache_key::apple_install_name::tests::word_readers_reject_wrong_widths ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 3204 filtered out; finished in 0.00s
INJECTED_FAULT_EXIT=101
```

Both injections were reverted before final verification. An initial partial
return-type migration also failed type checking until all consumers used
`?`; it is not counted as an additional independent witness.

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache --all-features --lib cache_key::
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

```text
CLIPPY_MEASURE_EXIT=0
CLIPPY_FINAL_EXIT=0
test result: ok. 476 passed; 0 failed; 2 ignored; 0 measured; 2727 filtered out; finished in 3.99s
CACHE_KEY_TEST_EXIT=0
MAKE_CHECK_EXIT=0
```

The two ignored tests are subprocess helpers explicitly spawned by their
toolchain regression parents, not additional standalone coverage claims.
Both test commands ran in `/home/ubuntu/repos/kache/.work/wt/dslp-15`, using
the candidate's source, not a released dependency. The required gate includes
lint, the client-crate graph, resource-guard self-tests and workspace tests.

No additional backlog finding/owner decision was demonstrated;
`Next: 103` remains unchanged.
DSLP-15 is closed by the established direct backlog-edit convention (the
`todo` CLI is not available in this environment).

Limits: no real macOS/Windows/32-bit execution, full oracle/project/scenario
campaign or performance workload was run. No C/C++ compiler keying was edited.
The declared performance gate is `make perf-gate-local`; a Linux workload
would not exercise this Apple reader. There is no performance regression
claim from tests alone. This branch is pushed without a PR or merge; collect
appropriate runtime/performance evidence before merging a runtime-affecting
slice, per the deslop workflow.
