# Deslop: src/compiler

Scope: `src/compiler/cc.rs`, `flags.rs`, `mod.rs`, `platform.rs`, `rustc.rs`
(about 17,000 lines, with the inline test modules), base `4460882f`, macOS
arm64, nightly-2026-11-09. Four commits: `642bfed9` (cc depfile expects),
`15dde4bd` (exhaustive `link_strategy`), `aea5d0d9` (test for it), `b68562fb`
(redundant allow). No argument parsing, cache key, flag or output behaviour
changed. Host note and the `RUSTC_WRAPPER=` bypass are in
[deslop-cli-submodules.md](deslop-cli-submodules.md).

## DSLP-20 Types

Commands: the two `cargo clippy -p kache --lib [--tests] --message-format=json
-- -W warnings -W clippy::... -W unreachable_pub -W unsafe_code` runs from the
submodule doc, both exit 0, before and after. Production (lib target) counts,
before then after; test-only counts after:

| Candidate | Before | After | Test-only | Verdict |
| --- | --- | --- | --- | --- |
| `expect_used` | 2 (`cc.rs`) | 0 | 17 | Fixed by type: `capture_depfile` returns a UTF-8-checked `(PathBuf, String)` so the two `to_str().expect(..)` calls that followed a `filter(to_str().is_some())` are gone. Test-only: rejected. |
| `wildcard_enum_match_arm` | 3 (`mod.rs:252`, `rustc.rs:356`, `cc.rs:10153`) | 2 | 0 | `ArtifactKind::link_strategy` now names every immutable kind; a new variant is a compile error instead of silently sharing the cache blob's inode. Kept: `rustc.rs:356` (`kind => kind` pass-through of an already classified kind) and `cc.rs:10153` (`_` for every non-`-c` mode, classified by filename). |
| `unwrap_used` | 0 | 0 | 286 | Rejected: test code. |
| `unsafe_code` | 4 | 4 | 2 | Rejected: `geteuid` (`cc.rs:1584`) and `uname` (`platform.rs:622-628`) have no std equivalent and no direct `rustix`; each has a SAFETY comment. Test sites are `set_var` in `shim_tests`. |
| `cast_possible_truncation` | 3 | 3 | 0 | Rejected: `value as u8` in the C escape decoder (`cc.rs:2375-2387`): a hex or octal escape wraps modulo 256 as the C compiler does, so the truncation is the specified behaviour. |
| `needless_pass_by_value` | 3 | 3 | 0 | Rejected: owned arguments such as the `Vec<String>` in `add_preprocess_dep_capture`; changing them is signature churn, not a typing hatch. |
| `redundant_clone` | 0 | 0 | 3 | Rejected: test fixtures. |
| `unreachable_pub` | 81 | 81 | 13 | Rejected: style; 81 per 17k lines. |
| `cast_sign_loss`, `match_wildcard_for_single_variants` | 0 | 0 | 0 | Nothing. |

Adopted profile: none as a standing lint. Escape hatches
(`rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('`, lines): `cc.rs` 8 to
6, `rustc.rs` 77 to 76 (the redundant `#[allow(clippy::too_many_arguments)]`;
`Cargo.toml` already allows that lint for the crate, so clippy `-D warnings`
stays clean without it); `mod.rs` 104 and `platform.rs` 114 unchanged. Not
increased.

Witnesses:

- `capture_depfile` call sites: `preprocess_dep_capture_is_complete_for_both_dialects`
  (`cc/tests.rs:4582`) passes the argument directly; the preprocess-memo and
  capture paths run in the cc key tests.
- Exhaustive `link_strategy`: compile-time. Behaviour pinned by the new
  `every_kind_restores_with_its_mutation_profile` (`mod.rs:1895`).

Proof: `cargo clippy -p kache --lib --tests -- -D warnings` exit 0;
`cargo test -p kache --lib -- cli:: compiler::` exit 0, 746 passed, 2 ignored,
25.14s run.

## DSLP-47 Tests

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed
(DSLP-87): no coverage figures, no mutation survivor list. By-hand audit:

Tiers: unit tests colocated per file (`flags.rs` 10, `mod.rs` 72, `platform.rs`
24, `rustc.rs` 37, plus 374 in `cc/tests.rs` covered by DSLP-19); the real
`rustc` and shim behaviour is exercised by `mod.rs` `shim_tests` (two tests run
in a child process) and by `tests/` at binary level.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Flag classification is anchored and per dialect | `flags.rs:388` `regex_matcher_is_anchored_and_excludes_by_pattern`, `:490` `anchoring_survives_top_level_alternation_in_pattern`, `:441` | Asserted. |
| Artifact kind by filename; extensionless vs unknown; `.dsym.tar` wins over `tar` | `mod.rs:1826`, `:1913` | Asserted. |
| Restore link strategy per kind (a shared inode must never reach a mutable output) | `mod.rs:1895` `every_kind_restores_with_its_mutation_profile` (new), `rustc.rs:1256` for crate types, `wrapper/tests.rs:4509` | Survivor found and killed (see fault). |
| Workspace-wrapper chain: only Clippy is cacheable; a Kache command is never a compiler | `mod.rs:1311`, `:1291` | Asserted. |
| Debug bundle unpack cap is exact and rejects unsafe members | `mod.rs:1747` `unpack_debug_bundle_cap_boundary_is_exact` | Asserted. |
| Platform: memo kept per kernel release on arm64 macOS only; debug-bundle commands per Mach-O format | `platform.rs:803`, `:917` | Asserted (the Linux and macOS branches are cfg-gated; only the host's runs here). |
| rustc refuse reasons for queries and print requests | `rustc.rs:819`, `:827` | Asserted. |

Fault and survivor: in `ArtifactKind::link_strategy` `DebugSidecar` was moved
from the shared-inode arm into the `Copy` arm. Before the new test,
`cargo test -p kache --lib -- compiler::tests::` passed (the wildcard arm made
the strategy of Library, Object, DepInfo and DebugSidecar unasserted anywhere;
only Metadata, DebugBundle, executables and wasm were). With the new
table test: exit 101, `compiler::tests::every_kind_restores_with_its_mutation_profile`
FAILED (`DebugSidecar`, left `Copy`, right `Hardlink`), 42 other tests in the
module passed. Restored; `cmp` exit 0. The fault is a run against the tree with
the new test, which is the proof that the test, not the pre-existing ones, kills it.

## DSLP-74 Dead code

- rustc `dead_code` clean at `-D warnings` for `--lib` and `--lib --tests`.
- Workspace-wide caller search (`rg -w NAME src crates tests`) of the 137
  visibility-qualified production items across the five files: the least used
  have one reference outside their definition and all are live:
  `COMPILER_ADAPTERS` (`mod.rs:792`), `execute_streaming_until_dep_info`
  (`wrapper.rs:5599`), `execute_preserving_incremental`
  (`wrapper.rs:7147`), `execute_preserving_incremental_until_dep_info`
  (`wrapper.rs:5460`), `execute_passthrough_preserving_incremental`
  (`wrapper.rs:7145`).
- Test-only helpers behind `#[cfg(test)]` with a single consumer, kept because
  they have live consumers: `is_passthrough_compiler_invocation`
  (`mod.rs:856`, one test), `is_cacheable_workspace_wrapper_chain` (`mod.rs:886`,
  one test), `platform.rs` `verifying_into` (`wrapper/tests.rs:4979`),
  `cc_is_internal_key_probe` and `embeds_codeview_debug` (`cc/tests.rs`). They
  are test seams over `InvocationEnv::capture()`; folding each into its test is
  possible but not a deletion.
- Removed: the `#[allow(clippy::too_many_arguments)]` above (a suppression of
  an already-allowed lint). `#[ignore]`: two in `mod.rs` (`:2933`, `:2992`), each
  run in a child process by a live test.
- Needs environment: `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat`.
