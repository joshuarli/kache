# Deslop: crates/kache-store tests

Types, Tests and Dead code for DSLP-10, DSLP-37 and DSLP-64, scope
`crates/kache-store/src/store/tests.rs` (14,735 lines, 399 functions: 366
`#[test]`, of which 2 are `#[ignore]` subprocess fixtures, and 33 helpers).
Base `11cbdba8` plus the core component's commits (branch `deslop/k2`), macOS
arm64, nightly-2026-11-09. This is the on-disk store contract suite named in
AGENTS.md; no store format changes here.

Commands run from the worktree root with
`export TMPDIR=$PWD/.work/tmp CARGO_BUILD_JOBS=4`.

## DSLP-10 Types

Measurement: the same `cargo clippy --no-deps -p kache-store --all-targets
--features test-support -- --cap-lints warn -W ...` run as in
[deslop-kache-store-core.md](deslop-kache-store-core.md), filtered to this
file.

| Candidate | Hits | Decision |
| --- | --- | --- |
| `unwrap_used` | 2,439 | Rejected: tests assert by unwrapping. |
| `expect_used` | 37 | Rejected, same reason. |
| `redundant_clone` | 14 | **Adopted and fixed** (commit `refactor(store): remove redundant clones in store tests`). Each clone was the last use of its value; one fix left a self-rebinding (`let config = config;`) that was dropped. Lint is enabled crate-wide in the remaining-modules commit, once `store.rs` is also clean. |
| `cast_possible_truncation`, `cast_sign_loss` | 10 and 3 | Rejected: fixture sizes and counters bounded by the test (`vec![b'x'; last.size as usize]`, `group as u8 + 1`, `refcount as usize` against a loop count `N`). |
| `wildcard_enum_match_arm` | 1 | Rejected (`_ => panic!(..)` on a `FileHashCache` variant match in a test). |
| `unsafe_code` | 5 | Rejected as a `forbid`: 4 are `std::env::set_var`/`remove_var`, which edition 2024 makes `unsafe`, and 1 is `libc::geteuid`. The 4 env calls are now guarded by a type, below. |
| `#[allow(..)]` | 4 | **Removed**: all four were `clippy::permissions_set_readonly_false`, which `crates/kache-store/Cargo.toml` already allows for the whole crate. Escape-hatch count in the file: 4 allows removed (`rg -c '#\[allow'` 4 to 0). |

Typing fix with a witness. `EnvVarGuard` wrapped `set_var`/`remove_var` and
relied on each caller holding `process_state_test_lock`
(`src/test_support.rs`), the lock that serialises tests which touch
process-global state. The guard now takes the lock guard as a borrowed witness
(`EnvVarGuard<'lock>`, constructors `set(&lock, key, value)` and
`remove(&lock, key)`), so the lock must exist and outlive the guard. All six
call sites already held it, so no test changed behaviour. Witness (negative
control): deleting the `&_env_lock` argument at the first call site makes
`cargo check -p kache-store --tests` fail with `E0061: this function takes 2
arguments but 1 argument was supplied`; the file was restored from a saved
copy and `cmp` exited 0.

## DSLP-37 Tests

Run (post-change): `cargo test -p kache-store --features test-support`, exit 0,
46 s wall with warm dependencies at load 25 to 50: 624 passed, 2 ignored,
18.98 s of test time (the pre-change run was 624 passed, 19.50 s), plus
`byte_counters` 5, `staged_publication` 1, `thread_store_counters` 1. The
suite is one tier (library tests over real SQLite and a temp directory); the
only subprocess tests are the two `#[ignore]` fixtures, launched by
`process_exit_releases_advisory_key_lock` and
`wait_for_committed_returns_false_without_an_owner` through
`store::tests::<fixture>`, so neither is dead.

False-pass scan (a body scan for tests with no assertion form) found two:
`opening_a_current_index_does_not_need_the_write_lock` (`tests.rs:546`, the
assertion is that `open_index_db(..).unwrap()` returns while another handle
holds `BEGIN IMMEDIATE`) and `advisory_key_lock_child_fixture` (`:6691`, a
fixture asserted by its parent). Neither is vacuous.

| Contract (line = test fn in `tests.rs`) | Guarding tests | Disposition |
| --- | --- | --- |
| Index schema generations upgrade once, never empty the file-hash memo | `index_ddl_runs_once_per_schema_generation` `:87`, `index_from_generation_three_gains_the_crate_name_index` `:322`, `an_older_release_reopening_the_index_does_not_empty_the_file_hash_memo` `:389` | Asserted |
| Content hash serialization is pinned | `content_hash_golden_pins_serialization` `:12449` | Asserted (golden) |
| Artifact names cannot escape the entry directory; symlinks are never stored as blobs | `put_rejects_an_artifact_name_that_escapes_the_entry_dir` `:3968`, `put_never_stores_a_symlink_as_a_blob` `:1061`, `import_rejects_unsafe_artifact_name` `:8725` | Asserted. The one gap here, import of a symlinked artifact, is already DSLP-90 |
| Import verifies hashes and sizes, batch import is atomic | `import_rejects_content_hash_mismatch` `:8687`, `verified_batch_import_is_atomic_and_registers_every_entry` `:12640` | Asserted |
| Zero-byte outputs are refused except metadata | `test_store_put_rejects_zero_byte_artifact` `:8452` | Asserted |
| Corrupt or lost index is quarantined, rebuilt, never loses servable entries | `open_index_db_self_heals_a_corrupt_index` `:4927`, `quarantine_corrupt_index_moves_wal_and_shm_sidecars` `:4958`, `rebuild_index_from_store_recovers_entries_after_the_index_is_lost` `:4157`, `stranded_entry_sweep_readopts_servable_entries_and_removes_the_rest` `:4734` | Asserted |
| Blob refcounts: dedup, release on remove, no double decrement, no leak on corrupt meta | `test_remove_entry_decrements_refcount` `:9249`, `two_removers_on_one_entry_never_double_decrement_shared_blob` `:9918`, `remove_entry_refuses_on_corrupt_meta_no_refcount_leak` `:6851`, `reput_with_different_outputs_releases_only_the_dropped_blob` `:7146` | Asserted; fault below |
| Concurrent writers and removers never dangle | `test_concurrent_puts_sharing_blob_are_consistent` `:9325`, `test_concurrent_put_remove_never_dangles` `:9425`, `publish_racing_an_unlink_never_errors` `:9504`, `cas_publication_holds_the_writer_lock_through_rename` `:1865` | Asserted |
| Advisory key lock and GC lock exclusivity, released at process exit | `concurrent_advisory_lock_acquisition_has_one_winner` `:6587`, `process_exit_releases_advisory_key_lock` `:6652`, `gc_lock_is_mutually_exclusive` `:11027` | Asserted |
| Eviction hysteresis, shared and hardlinked blobs retained, upload intents pinned, fresh imports kept | `evict_fires_at_the_trigger_and_stops_at_the_target` `:10172`, `evict_noop_within_the_hysteresis_band` `:10128`, `evict_leaves_an_entry_whose_blob_is_still_hardlinked_outside` `:5278`, `durable_upload_intent_pins_payload_across_every_eviction_policy_until_retired` `:5405`, `an_automatic_sweep_keeps_what_the_remote_just_delivered` `:6226` | Asserted |
| Restore verification evicts a corrupted or unreadable blob | `verify_restores_evicts_a_corrupted_blob` `:11701`, `get_evicts_when_verified_blob_is_unreadable` `:8407` | Asserted |
| Legacy entry migration keeps refcounts after a late failure | `test_migration_of_legacy_entry` `:11998`, `legacy_migration_keeps_completed_refcounts_after_a_later_io_failure` `:12177` | Asserted |
| Key-lock sweep never follows symlinks or removes held locks | `key_lock_sweep_never_follows_a_symlink_named_like_a_key_lock` `:11361` | Asserted |
| Read-only store serves hits, writes nothing, creates nothing | `read_only_store_serves_hits_and_writes_nothing` `:14082`, `read_only_store_refuses_without_a_live_index_and_creates_nothing` `:14278` | Asserted |
| Deferred durability verifies pending hits until the flush | `deferred_durability_verifies_pending_hits_until_the_flush` `:2246` | Asserted |
| Quarantine cleanup leaves live index and other paths | `full_clear_removes_quarantined_files_and_preserves_live_index_and_other_paths` `:14537` | Asserted |

No format boundary is `none`; no test added.

Injected fault (critical decision: releasing blob references on removal).
In `crates/kache-store/src/store.rs:6215` change `refcount = refcount - ?1` to
`refcount = refcount + ?1`; `cargo test -p kache-store --lib --features
test-support remove_entry_decrements_refcount` exits 101,
`store::tests::test_remove_entry_decrements_refcount` fails at its
post-removal refcount assertion with `left: 3, right: 1`. Restored from a saved copy;
`cmp` exit 0 and `git diff --stat -- crates/kache-store/src/store.rs` empty.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` 27.1.0 are not
installed, so there is no line or branch coverage and no survivor list for the
`store.rs` decisions this file guards. Test cost: this component adds no tests
and warm test time is unchanged within noise.

## DSLP-64 Dead code

rustc `dead_code` and `unused_crate_dependencies` are clean over this file
(`cargo clippy --no-deps -p kache-store --all-targets --features test-support
-- -D warnings` fails only on `store.rs:2329`, which belongs to the
remaining-modules component and is not in this file). Findings:

- Duplicate function names: `rg -o '^\s*(pub )?fn (\w+)' -r '$2' tests.rs |
  sort | uniq -d` returns `drop` (the `Drop` impl of `EnvVarGuard`, not a
  test). No duplicated test names.
- Commented-out tests (`// #[test]`, `// fn`): none.
- Helpers: all 33 are referenced (compiler `dead_code` is a hard error here).
- `#[ignore]` tests: 2, both launched by a parent test through
  `store::tests::<fixture>`, so live.
- Removed: the four redundant `#[allow]` lines (see Types).

Needs environment: `cargo-machete`, `cargo-udeps` (the file has no
dependencies of its own; the crate's dependencies are covered in the core
document). Nothing removed beyond the allows; no consumers outside the crate
(the file is `#[cfg(test)]`).
