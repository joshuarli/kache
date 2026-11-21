# Deslop: src/compiler/cc/tests.rs

Scope: `src/compiler/cc/tests.rs` (10,613 lines, 374 `#[test]` functions, 13
helpers), base `9dddb9f7`, macOS arm64, nightly-2026-11-09. One test change
(`093b3af3`): a test that always passed on macOS without asserting. No
production change from this component. Host note and the `RUSTC_WRAPPER=`
bypass are in [deslop-cli-submodules.md](deslop-cli-submodules.md).

## DSLP-19 Types

Commands: the two `cargo clippy -p kache --lib [--tests] --message-format=json
-- -W warnings -W clippy::... -W unreachable_pub -W unsafe_code` runs from the
submodule doc, both exit 0. The file is all test code (reported only by the
test target):

| Candidate | Count | Verdict |
| --- | --- | --- |
| `unwrap_used` | 927 | Rejected. A test that cannot build its fixture should panic at the call. |
| `expect_used` | 25 | Rejected, same. |
| `redundant_clone` | 13 | Rejected: fixture copies, no defect (the lint is nursery). |
| `unsafe_code` | 15 | Rejected as a standing lint: 12 are `set_var`/`remove_var` (Rust 2024 makes them `unsafe`) and 3 are `geteuid` root guards. Checked for soundness instead, below. |
| every other candidate | 0 | Nothing to adopt. |

Adopted profile: none. Escape hatches
(`rg -c '\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('`): 938 lines, no
`#[allow(`; unchanged.

The 12 environment edits were checked for the data race that makes
`set_var` unsound under parallel tests. Every one runs under
`process_state_test_lock()` (`cc/tests.rs:6328`, `:8310`, `:9161`, `:9528`) except
the PATH and `KACHE_ACTIVE` edits in `recognizes_unknown_wrapper_via_probe`,
which take `config::config_path_lock()`: that is a re-export of the same
`kache_fs::process_state_test::process_state_test_lock`
(`crates/kache-config/src/testing.rs:3`), so all writers share one mutex. No
defect. Readers that do not take the lock are not excluded by either path; that
is the existing design of the lock, not a change in this scope.

## DSLP-46 Tests

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed
(DSLP-87): no coverage and no mutation survivors. By-hand audit:

Tiers: all 374 tests are unit tests of `compiler::cc` in the library test
binary; most are pure (parse, key, map, gate) and a handful run a real C
compiler or `/bin/sh` shim. Cost: the `cli::` plus `compiler::` subset is 746
tests in 47.66s (`cargo test -p kache --lib -- cli:: compiler::`, exit 0).

Silent-pass audit (`rg -n '^\s+return;' src/compiler/cc/tests.rs`, 7 sites):
five are root-user and `/var` symlink guards that skip by design
(`:8381`, `:8441`, `:8702`, `:10083`, `:10089`; the first three print a
`skipped:` note). Two were a problem: `recognizes_unknown_wrapper_via_probe`
returned unconditionally on macOS (a pass that executes nothing) and still
returns at `:830` when no C compiler is installed. The first is fixed; the second is a conditional skip kept as is,
filed below.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| GNU vs clang-cl argument parsing; unsupported flags refuse rather than pass | `:335` `parser_skips_gnu_only_rows_under_cl`, `:9946` `gnu_double_dash_does_not_skip_unsupported_flags`, `:1522` to `:1656` refusal tests | Asserted. |
| A raw checkout root never reaches a portable key's object | `:5431` `store_gate_keeps_raw_roots_out_of_portable_keys`, `:5712` `object_scan_finds_raw_roots_the_token_mapper_skips` | Asserted (see fault). |
| Include-dir digest fails closed on unreadable directories | `:8437` `include_dir_digest_unreadable_dir_is_fail_closed` | Asserted (root skip is by design). |
| Prefix maps follow base dirs and byte skipping misses no prefix | `:9527`, `:4917` | Asserted. |
| Dependency capture flags per dialect | `:4582` `preprocess_dep_capture_is_complete_for_both_dialects` | Asserted. |
| Direct-capture probe rejects a driver that writes no object | `:8792` | Asserted. |
| Response files expand for GNU drivers only and refuse unsafe ones | `:10215` | Asserted. |
| `SOURCE_DATE_EPOCH` support needs matching expansions | `:8850` | Asserted. |
| Wrapper recognition via the `-E` family probe | `recognizes_unknown_wrapper_via_probe` (Linux only after the fix; no assertion ran on macOS before) | Fixed false pass. |

False-pass witness: in `cc.rs` `recognizes_for`, `if is_unresolvable_bare_program(arg0, env)`
changed to `if true || ...`, which makes every unknown program unrecognised.
`cargo test -p kache --lib -- compiler::cc::tests::recognizes_unknown_wrapper_via_probe`
exit 0, `1 passed ... finished in 0.00s`: the test passed under the fault.
After gating the test with `#[cfg(not(target_os = "macos"))]`, the same
command lists 0 tests. Restored `cc.rs` with `cmp` exit 0. The Linux
behaviour is unchanged (the macOS carve-out was the only difference); I could
not run Linux here.

Injected fault in the store gate, `cc_unsafe_to_store`: `no_artifacts ||
key_path_bound` changed to `&&`. `cargo test -p kache --lib --
compiler::cc::tests::store_gate` exit 101,
`compiler::cc::tests::store_gate_keeps_raw_roots_out_of_portable_keys` FAILED
(`cc/tests.rs:5432`, the scan was invoked for a bound key). Restored; `cmp` exit 0.

## DSLP-73 Dead code

- rustc `dead_code` clean at `-D warnings` for `--lib --tests`.
- The 13 non-`#[test]` functions in the file each have at least one other use
  in it (`rg -w`); none unreferenced. `#[ignore]` and `#[allow(` in the file: 0.
- One gated-out test (above) is a false pass removed, not dead code.
- Needs environment: `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat`.

Disposition: nothing removed.
