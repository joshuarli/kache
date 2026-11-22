# Deslop: crates/kache-store remaining modules

Types, Tests and Dead code for DSLP-11, DSLP-38 and DSLP-65. Scope:
`crates/kache-store/src/store.rs` (6,708 lines at base), `test_support.rs`,
`workspace.rs`, and the integration tests `tests/byte_counters.rs`,
`tests/staged_publication.rs`, `tests/thread_store_counters.rs`. Base
`11cbdba8`, branch `deslop/k2`, macOS arm64, nightly-2026-11-09. `store.rs`
owns the on-disk store format; no format, schema or hash input changed.

Commands run from the worktree root with
`export TMPDIR=$PWD/.work/tmp CARGO_BUILD_JOBS=4`.

## DSLP-11 Types

Candidate run (same flags as [deslop-kache-store-core.md](deslop-kache-store-core.md)):
`cargo clippy --no-deps -p kache-store --lib --features test-support
--message-format=json -- --cap-lints warn -W clippy::... -W unsafe_code -W
unreachable_pub`, then the same with `--all-targets`; both exit 0. The
`test-support` feature compiles three `#[doc(hidden)]` helpers
(`insert_entry_row_for_test`, `set_last_accessed_for_test`,
`remove_clone_for_test`) into the library build, so the lib run counts their
unwraps; they are test code.

| Candidate | `store.rs` / `workspace.rs` / `test_support.rs` / `tests/` | Decision |
| --- | --- | --- |
| `unwrap_used` | prod 3 (`blob.parent()` at 3 sites), test-support 3; integration tests 17, `test_support.rs` 5 | The 3 production sites are **fixed**, see below. Test code rejected. |
| `expect_used` | prod 3 (`blob.parent()` x2, `last_error` in `try_open_index_db`), test-support 1 | Two `parent()` sites fixed. `try_open_index_db` rejected: a loop invariant on `STORE_OPEN_MAX_ATTEMPTS = 6`; restructuring it removes the hatch but no test observes the retry count, so there is no witness. A rewrite was tried and reverted. |
| `redundant_clone` | 1 | **Adopted**: `Store::open` cloned a config it never used again. With the core and tests commits all three components are clean, so `redundant_clone = "warn"` is now in `Cargo.toml` and `make lint` enforces it. |
| `match_wildcard_for_single_variants` / `wildcard_enum_match_arm` | 1 prod in `workspace.rs` (`publish`), 1 in `store.rs:1700`, 2 in `workspace.rs` tests | `workspace.rs` **adopted**: `_ =>` over `Outcome` now names `Outcome::Prepared { .. }`. `store.rs:1700` rejected (`rusqlite::Error` is `#[non_exhaustive]`; the wildcard is required). |
| `cast_sign_loss`, `cast_possible_truncation` | 20 and 8 | Rejected as lint: most are behind `.max(0)` (`store.rs` 5266, 5752, 5842, 5875 to 5878, 6336, 6338). The unguarded ones are filed below as a finding rather than changed without a witness. |
| `unsafe_code = forbid` | `store.rs` 1 (macOS `getxattr` probe of the backup-exclusion attribute), `test_support.rs` 1 (`getuid`) | Rejected: neither has a std equivalent. |
| `#[allow(..)]` | 1 (`too_many_arguments`) | **Removed**: `Cargo.toml` already allows it crate-wide. |
| `unreachable_pub` | 0 | Clean (adopted in the core commit). |

Fixes with witnesses:

1. `blob_path(hash).parent().unwrap()/.expect()` at three hash-based sites
   (`move_verified_blob`, `verify_pending_blob`, legacy migration). `blob_path`
   always joins a shard directory and a file name, so the sites now ask for
   the shard directory (`Store::blob_shard_dir`, built before the hash is
   joined). Witness: `blob_shard_dir_is_the_directory_holding_the_blob`
   (`store/tests.rs`, added here) asserts `blob_path(h).parent()` equals
   `blob_shard_dir(h)` for a full hash and for 3-, 2- and 1-character hashes.
   Only `hash == ""` differs (never valid; the remote trust boundary rejects
   it).
2. `materialize_blob_with_durability` and `publish_staged_blob_with` unwrapped
   `blob.parent()` of a caller-supplied path, so a destination such as `/`
   panicked the wrapper process instead of failing the operation. Both return
   `Result`, so the condition is now an error. This is a runtime change in its
   own commit (`fix(store): ...`) with
   `materialize_blob_reports_a_destination_without_a_parent_directory`, which
   panicked at `store.rs:409` before the fix and passes after.

Escape hatches in scope before and after (`rg -c
'\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\('` over production code):
`store.rs` 7 to 2 lines (the 5 `parent()` unwrap/expect sites and the `#[allow]` removed; what remains is the `getxattr` block and `try_open_index_db`),
`workspace.rs` 0 to 0. None added.

## DSLP-38 Tests

Crate run before this component's changes: 624 passed, 2 ignored, 19.0 s of
test time, plus the three integration binaries (5 + 1 + 1). This component
adds two tests (both in `store/tests.rs`, each milliseconds) and nothing else.

The three integration binaries are deliberately one-test-owner-per-process
(`tests/staged_publication.rs` states why): each owns its process's
thread-local and static byte counters so exact deltas cannot race.

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Publication counts only committed bytes; a prepared target is invisible until `publish` | `tests/staged_publication.rs:12` | Asserted |
| Restore and store byte counters accumulate per materialization method and copy reason | `tests/byte_counters.rs:6`, `:27`, `:54`, `:84`, `:111` | Asserted |
| Publication bytes exclude other threads' work | `tests/thread_store_counters.rs:26` | Asserted |
| Prepared workspace files are discarded only when unpublished; absent targets clear, other client errors propagate; denied operations do not retry and scopes restore | `workspace.rs:158`, `:202`, `:228` (`prepared_files_discard_only_unpublished_stages`, `clearing_an_absent_target_succeeds_but_other_client_errors_propagate`, `denied_client_operations_never_retry_in_the_daemon_and_scopes_restore`) | Asserted; fault below |
| Process-state lock restores the working directory on drop | `test_support.rs:58` (`dropping_the_guard_restores_the_current_directory`) | Asserted |
| Everything `store.rs` does to the on-disk store (schema, blobs, refcounts, eviction, locks, recovery, read-only mode) | `store/tests.rs`, mapped in [deslop-kache-store-tests.md](deslop-kache-store-tests.md) | Asserted there; fault run in that document |
| `blob_path` shard layout | `test_blob_path_sharding`, `blob_path_is_panic_safe_for_short_hash`, `blob_shard_dir_is_the_directory_holding_the_blob` | Asserted; the last one added here |
| A blob destination with no parent fails the operation | `materialize_blob_reports_a_destination_without_a_parent_directory` | Added here; failed before the fix |

Injected fault (critical decision: a prepared workspace file is discarded
only if it was not published). In `crates/kache-store/src/workspace.rs:113`
change `if !self.published` to `if self.published`;
`cargo test -p kache-store --lib --features test-support workspace::tests`
exits 101 with `workspace::tests::prepared_files_discard_only_unpublished_stages`
panicking at `workspace.rs:186`; the other two workspace tests still pass.
Restored from a saved copy; `cmp` exit 0 and the worktree was clean.

Needs environment: coverage (`cargo-llvm-cov`) and mutation (`cargo-mutants`
27.1.0) are not installed; there is no survivor list for `store.rs`.

## DSLP-65 Dead code

Tools: rustc `dead_code` and `unused_crate_dependencies` clean
(`cargo clippy --no-deps -p kache-store --all-targets --features test-support
-- -D warnings`, exit 0 after the changes). Needs environment:
`cargo-machete`, `cargo-udeps`, `cargo-bloat`.

Public-item scan over `store.rs` and `workspace.rs` (`.work/tmp/pubscan.sh`,
`rg -w` outside the crate, in other crate files and in the item's own file):
141 `pub` items, 38 with no reference outside `crates/kache-store`. Of those,
11 are only types and constants used inside `store.rs`
(`CrateHistoryEntry`, `EntryInfo`, `GcLock`, `MigrationStats`, ...: return
types and implementation details exposed by `pub use store::*`). Five public
methods have **no production caller anywhere in the workspace** (callers are
tests only):

| Item | Where | Evidence | Disposition |
| --- | --- | --- | --- |
| `blobs_at_least` | `store.rs` `pub fn` | Callers: 2 in `store/tests.rs`; 0 in `src/`, docs or other crates | Consumer question |
| `keys_for_crates` | `store.rs` `pub fn` | Callers: 6 in `store/tests.rs` | Consumer question |
| `wait_for_committed_with_timeout` | `store.rs` `pub fn` | Callers: 2 in `store/tests.rs` | Consumer question |
| `reconcile_blob_index` | `store.rs` `pub fn` | Callers: 8 in `store/tests.rs` | Consumer question |
| `import_verified_restored_entries` | `store.rs` `pub fn` | Callers: 16 in `store/tests.rs`; the remote-storage import path AGENTS.md defers, and the subject of DSLP-90 | Consumer question; do not remove with remote storage deferred |

All five belong to a published crate's public API, so none was removed; the
question for the owner is whether any out-of-tree consumer uses them. No
platform-specific code was proven dead (the macOS `getxattr` probe is
reachable on macOS; this host built only macOS).
