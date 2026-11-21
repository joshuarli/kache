# Deslop: src/cache_key submodules

Component 11 of the deslop campaign (DSLP-15, DSLP-42, DSLP-69). Scope:
`src/cache_key/apple_install_name.rs` (286 lines: 166 production, 120 tests) and
`src/cache_key/tests.rs` (18,292 lines, 466 `#[test]` functions). `src/cache_key.rs`
is component 12 and has [its own record](deslop-cache-key.md); the two share the
`mod cache_key;` declaration. Base `11cbdba8`, branch `deslop/k3`, macOS arm64,
`nightly-2026-11-09`.

The install-name reader feeds the link key for Apple dylibs and text stubs
(`apple_dylib_and_framework_install_names_bind_link_keys`), so it is key-adjacent:
a wrongly accepted name is a wrong cache hit. No key, parser behavior or accepted
input changed; the commits below add tests and a lint guard only.

Test runs used the system `TMPDIR` (a checkout-local one fails 4 tests in this
module tree, DSLP-88) and `RUSTC_WRAPPER=`.

## DSLP-15: Types

Commands and candidate list as in [deslop-support-crates.md](deslop-support-crates.md).
Counts per file:

| Candidate | `apple_install_name.rs` prod / all | `tests.rs` all | Disposition |
| --- | ---: | ---: | --- |
| `unsafe_code` | 0 / 0 | 22 | Adopted for production via `#[cfg_attr(not(test), forbid(unsafe_code))]` on `mod cache_key;` (below). The 22 in `tests.rs` are the `std::env::set_var` and `remove_var` wrappers, which run under `key_test_lock`. |
| `unwrap_used` | 2 / 11 | 1,675 | Rejected. `tests.rs` is assertion code. The 2 production hits are `bytes.try_into().unwrap()` in the `layout` word readers; every caller passes a 4-byte slice cut from a buffer already length-checked (`macho` rejects `bytes.len() < header`, `read` fixes the first 32 bytes). Replacing them adds an error arm for an impossible state. |
| `expect_used` | 0 / 0 | 46 | Rejected: test messages. |
| `indexing_slicing` | 11 / 22 | 108 | Rejected: fixed-offset header fields (`bytes[12..16]`, `bytes[20..24]`) after explicit bound checks; `macho_refuses_invalid_load_commands` truncates and corrupts each offset. |
| `as_conversions`, `cast_possible_truncation` | 6, 0 / 8, 2 | 12, 4 | Rejected: `u32` header words to `usize` (`word(..) as usize`), bounded by `MAX_HEADER` before use, and `usize` to `u32` when building fixtures. |
| `redundant_clone` | 0 / 0 | 23 | Rejected: nursery lint, all in test setup. |
| `wildcard_enum_match_arm`, `match_wildcard_for_single_variants`, `needless_pass_by_value`, `cast_sign_loss`, `cast_possible_wrap`, `cast_lossless`, `fn_params_excessive_bools`, `struct_excessive_bools`, `if_not_else`, `unreachable_pub` | 0 | 0 | Nothing to adopt. |

Escape hatches, `git grep` at base and now: `apple_install_name.rs` 12 to 14 and
`tests.rs` 1,753 to 1,755 lines matching `unsafe|#[allow(|.unwrap()|.expect(`; the
growth is `.unwrap()` and `.unwrap_err()` in the new tests. Production hatches:
unchanged (2 `unwrap`, 0 `unsafe`, 0 `allow`).

Change: `d511f37e build(cache-key)` adds `#[cfg_attr(not(test), forbid(unsafe_code))]`
to `mod cache_key;` in `src/main.rs`, covering `src/cache_key.rs` and this
directory. Witness: an `unsafe { std::mem::zeroed() }` function added to
`apple_install_name.rs` failed `cargo check -p kache --lib` (exit 101, `usage of an
unsafe block`); the same in `cache_key.rs` failed identically; both restored with
`cmp` exit 0. Clippy with `-D warnings` over all targets stays clean.

## DSLP-42: Tests

Tiers and cost. `tests.rs` is one in-process tier: temporary directories, fake
compiler scripts and `PathNormalizer` fixtures, no network. `cargo test -p kache
--lib -- cache_key::` runs 472 tests at base plus 2 ignored fixtures (each re-executed by the
test that names it with `--exact`) in 47 to 56s of test time on this loaded host,
of which most is serialization: 126 tests take the process-wide `key_test_lock`
because they mutate environment variables, so the suite is effectively serial
and the per-test times from `--report-time` mostly measure lock waits. The
apple module's 8 tests run in 0.05s. Outside this tier the key is exercised by
`tests/cache_key_underkeying_test.rs`, `proc_macro_env_key_test.rs`,
`input_prediction_test.rs` and `macos_native_install_name_test.rs`; they were not
run in this lane's isolated runs (the final `make test` covers them).

False-pass audit of `tests.rs`: 25 tests start with `if !rustc_available() {
return; }` and 2 return after `skipping: <dir> is not writable`; in a
compiler-less environment they pass without asserting. That is a finding, not a
change: `cargo test` always has `rustc` on `PATH`. Tests that assert only
`is_err()` (the apple reject tests did) pass through any rejecting branch; the
new tests assert the message.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Mach-O reader accepts exactly dylibs, both widths and byte orders | `macho_reads_both_widths_and_byte_orders` (every truncation length), `macho_refuses_invalid_load_commands` | Guarded. |
| Text-stub reader accepts only plain, simple names | `stub_reads_plain_and_quoted_names`, `stub_refuses_ambiguous_and_complex_names` | Guarded. |
| File-level bounds (stub size cap, sparse 256 MiB dylib reads only the header) | `file_reader_bounds_stubs_and_reads_only_dylib_headers` | Guarded. |
| Oversized or incomplete load commands, an install-name inside another stub field, a short or repeated `LC_ID_DYLIB` | none checked the cause before this lane | Gaps, closed below. |
| Install names bind link keys | `apple_dylib_and_framework_install_names_bind_link_keys`, `apple_verbatim_extensionless_dylib_install_name_is_keyed`, `native_dylib_content_does_not_change_key` | Guarded. |

Coverage (toolchain `llvm-cov` with `-C instrument-coverage`, `cargo test -p kache
--lib`; recipe in [deslop-support-crates.md](deslop-support-crates.md)):
`apple_install_name.rs` production lines 125 of 129 (96.9%). The four misses were
the continuation lines of four `ensure!` messages (lines 27, 32, 58, 146): the
four branches in the table above. `tests.rs` is test code and was not scored.

Gap closure, `9d1c1630 test(cache-key)`: three tests,
`file_reader_refuses_load_commands_it_cannot_bound`,
`macho_refuses_a_short_or_repeated_dylib_id_command` and
`stub_refuses_an_install_name_spelled_inside_another_field`, assert the exact
rejection message. Each was proved with one injected fault against
`cargo test -p kache --lib -- cache_key::apple_install_name` (exit 101 each time,
file restored, `cmp` exit 0):

| Fault in `apple_install_name.rs` | Failing test |
| --- | --- |
| `length >= bytes.len()` to `length + 4 >= bytes.len()` | `file_reader_refuses_load_commands_it_cannot_bound` |
| `name.is_none() && length >= 24` to `name.is_none()` | `macho_refuses_a_short_or_repeated_dylib_id_command` |
| `name.is_none() && length >= 24` to `length >= 24` | `macho_refuses_a_short_or_repeated_dylib_id_command` |
| the stub spelling check replaced by `true` | `stub_refuses_an_install_name_spelled_inside_another_field` |

Not faulted: `commands <= MAX_HEADER - header` (the u32::MAX header test is
the witness by construction; deleting the guard would allocate 4 GiB, so no run).
A critical decision elsewhere in this module tree: `if workspace_unit && !covered`
in `same_tree_guard` flipped to `||` failed
`a_workspace_row_keeps_its_guard_only_while_the_guard_covers_the_closure` and
`this_checkouts_rows_keep_the_guard_while_it_covers_the_closure` (470 passed,
2 failed). The Mach-O `== 6` (dylib file type) check changed to `== 7` failed 5
tests: `macho_reads_both_widths_and_byte_orders`,
`file_reader_bounds_stubs_and_reads_only_dylib_headers`,
`apple_dylib_and_framework_install_names_bind_link_keys`,
`apple_verbatim_extensionless_dylib_install_name_is_keyed`,
`native_dylib_content_does_not_change_key`.

Needs environment: `cargo-mutants` and `cargo-llvm-cov` are not installed
(DSLP-87), so there is no mutant survivor list and the scoped lane is
unqualified; line coverage above was measured by hand and branch coverage was not
enabled. Cost budget: the three new tests add under 0.01s.

## DSLP-69: Dead code

| Tool | Result |
| --- | --- |
| rustc `dead_code` through `cargo clippy --workspace --all-targets -- -D warnings` | Clean at base and after, including `tests.rs` helpers (`pub(crate) fn dylib` is used from `tests.rs`). |
| Whole-word search of the module's items (`read`, `dylib`, every `fn` in `tests.rs` that is not a `#[test]`) across `.rs`, `.md`, `.toml`, `.sh`, `.py`, `.yml` | No item with a single reference. The two `#[ignore]` fixtures are each named once more by the test that re-executes them (`--exact`), so they are live. |
| `cargo` `unused_dependencies` lint over `cargo check --workspace --all-targets` | No warnings, negative control fired (see the support-crates record). |
| `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat` | `Needs environment`: not installed. |

Removed: nothing. No walk-backs, no allowlist.

## Findings for the parent

- P4, `src/cache_key/tests.rs`: 25 tests return silently when `rustc` is missing and 2 when the cache directory is unwritable; they pass without asserting. Turn the first into an assertion, since `cargo test` guarantees `rustc`.
- P4, `src/cache_key/tests.rs`: 126 tests share one process lock to mutate the environment, which makes the module serial (47 to 56s). Injecting the environment through `KeyEnv` and `InvocationEnv` (already used by the newer tests) would let them run in parallel.
