# Deslop: crates/kache-fs

Types, Tests and Dead code passes (DSLP-8, DSLP-35, DSLP-62) over `crates/kache-fs` (4,927 lines:
`lib.rs` 1,306, `apfs.rs` 834, `volume.rs` 625, `staging.rs` 497, `linux.rs` 242, `home.rs` 233,
and ten smaller files). Same base, host and environment as the other component documents. `kache-fs`
is published and `docs/compatibility.mdx` names it as following the major-version compatibility
rule, so unused public items are consumer questions here, not cuts. Features: `deser`, `staging`,
`testing` (the commands below enable all three).

## DSLP-8 Types

Escape hatches in compiled production code, excluding `#[cfg(test)]` modules:
19 `unsafe` sites compiled on this host (24 textual occurrences, counting the five in the
`cfg(windows)` module of `home.rs`, and tests), 3 `#[allow]` (`volume.rs:207,313,315`, each explaining a libc type that differs between
gnu and musl and which kache ships both), and 14 `unwrap()` and `expect()` calls (clippy's count), all inside the
feature-gated `testutil` module and `process_state_test.rs` (compiled only with
`feature = "testing"`). Strictly production code has 0 `unwrap` and 0 `expect`. 14 of the
production `unsafe` sites have no `SAFETY:` comment within four lines (`copy.rs:34,62,67,89`,
`home.rs:66,68,80`, `staging.rs:41,46,48`, `volume.rs:329,330`, `capacity.rs:65`, `apfs.rs:147`);
filed as a finding rather than adding comments in this pass.

Candidates, measured with `.work/scripts/strict.sh kache-fs prod|all --features staging,testing,deser`
(`cargo clippy --no-deps -p kache-fs ... -- --cap-lints warn -W <lint>`; unique lint and line):

| Candidate | Production | With tests | Disposition |
| --- | --- | --- | --- |
| `unwrap_used`, `expect_used` | 5, 9 | 227, 14 | Not adopted: every production hit is in `testutil` or `process_state_test.rs`, which are not `cfg(test)`, so `allow-unwrap-in-tests` does not cover them and denying the lint needs two `#[allow]` attributes on test-support modules. That would raise the hatch count by two to forbid nothing that exists today |
| `panic` | 1 | 6 | Rejected: `testutil`'s overlapping-fixture assertion (`lib.rs:542`) |
| `cast_possible_truncation`, `cast_sign_loss`, `cast_possible_wrap` | 1, 4, 2 | 6, 4, 2 | Rejected, each inspected: `home.rs:62` (`sysconf` result checked `< 0` first), `apfs.rs:203,210,447,456` (`attrreference_t` offsets, validated through `checked_add_signed`/`get`), `volume.rs:188` (a C `char` to `u8`) |
| `indexing_slicing` | 7 | 12 | Rejected: `lib.rs:201..222` is `CloneSketch`, whose length is clamped and reset when out of range (`insert` resets an oversized deserialized `len` before indexing), `apfs.rs:416` reads after `get` |
| `as_conversions` | 15 | 29 | Rejected: widenings and FFI field conversions |
| `unsafe_code` | 19 | 23 | Rejected: FFI (`statvfs`, `getpwuid_r`, `clonefile`, `ioctl`, `getattrlistbulk`, ACL calls) has no safe equivalent |
| `unnecessary_wraps` | 1 | 1 | **Fixed**: `identity.rs:35` (see below) |
| `option_option` | 1 | 1 | Rejected: `testutil::mock_volume_usage` distinguishes "no mock" from "mock says unavailable" |
| `map_err_ignore` | 2 | 2 | Rejected: `apfs.rs:327,367` map a `TryFrom` failure to a fixed `InvalidData` error |
| `unreachable_pub` | 24 | 25 | Rejected: nominal `pub` inside private modules (`ledger::PinnedOutside` is the one that matters, see below) |
| `missing_docs`, `unused_results`, `missing_const_for_fn`, `let_underscore_must_use` | 40, 13, 9, 3 | | Rejected: noise |

Adopted: none. One escape hatch removed (commit `14802a1e`): `identity::metadata_identity(path,
metadata, follow)` silenced two unused arguments with `let _ = (path, follow);` and returned a
`Result` that was always `Ok`. Now `metadata_identity(&Metadata) -> (InodeId, u64)`; the three callers
(`file_identity`, `directory_identity`, `FallbackProbe::measure_file`) drop their `?` and
`.map`. `pub(crate)`, so no public item changed; 88 tests pass before and after
(`cargo test -p kache-fs --features staging,testing,deser`), and `cargo clippy -p kache-fs
--all-targets --all-features -- -D warnings` exits 0. Witness: `clippy::unnecessary_wraps`
no longer fires on `identity.rs`.

Typing findings, not changed:

- `ledger::InodeLedger::pinned_outside` returns `PinnedOutside`, which `lib.rs` does not re-export
  (`pub use ledger::{InodeId, InodeLedger}`), so callers can read the fields but cannot name the
  type (P4; re-exporting is a public API addition).
- `extents.rs` writes `ACCOUNTABLE_FLAGS` as `0x3801` with a comment naming four flags, two of which
  (`UNWRITTEN`, `MERGED`) have no constant (P4).
- `staging.rs` declares a private `struct Clone`, shadowing the `Clone` trait name in that module
  (P4).
- `paths::strip_verbatim_prefix` is an identity function (`Cow::Borrowed`), a Windows remnant; it is
  public and called by the root crate.

## DSLP-35 Tests

Tiers: 88 unit tests, all in-file (2.1 s warm for the crate, 0.5 s on an idle run). No integration
tier; consumers (`kache-store`, `kache-config`, the root crate) exercise it through their own suites.
The platform-specific probes are tested only on their own host: `apfs.rs` (5) here, `linux.rs` (2)
and `extents.rs` (5, portable) in Linux CI; `try_clone_extents_into` has no assertion on this host.

Contracts and guards (`crates/kache-fs/src/`):

| Contract | Guarding tests | Disposition |
| --- | --- | --- |
| Hard links counted once; links outside the set reported as pinned | `lib.rs` `hard_links_are_counted_once`, `links_reaching_outside_the_measured_set_are_reported`; `ledger.rs` four tests | asserted; F5, F7 caught; F4 survivor now caught |
| Symlinks never followed | `lib.rs` `symlinks_are_never_followed`; `apfs.rs` `bulk_directory_read_matches_per_file_probing` | F6 caught by the APFS test only: the portable reader is not used on macOS |
| Unreadable directories reported, not counted as empty | `an_unreadable_directory_is_reported_...`, `an_unreadable_subtree_...` | asserted; F19 caught |
| Unknown measurements stay unknown (`Confidence`, `shared_ratio`, fallback probe) | `shared_ratio_is_withheld_...`, `directory_totals_keep_private_shared_and_unknown_...`, `fallback.rs` three tests | asserted; F8, F20 caught |
| Snapshot-held and worthless bytes | `file_sizing_reports_shared_bytes_and_worthlessness`, `cloned_files_do_not_double_count_...`, `apfs.rs` clone tests | asserted; F1 to F3 caught |
| Clone sketch is bounded, device-scoped, rejects legacy samples | `clone_samples_are_bounded_...`, `legacy_clone_samples_...`, `empty_clone_samples_...` | asserted; F12 caught |
| Volume counters: units, identity, overflow rejected | `capacity.rs` four tests, `lib.rs` `volume_bytes_*`, `volume_usage_*` | asserted; F9, F15 to F18, F14 caught |
| Filesystem classification (local, network, container, unknown stays silent) | `volume.rs` 17 tests | asserted |
| Clone and copy never replace; writable modes | `copy.rs` six tests | asserted; F10 caught |
| Staged publication: create-only, replace, cleanup, umask, ACL | `staging.rs` 12 tests | asserted; S1 caught |
| File and directory identity, handle identity | `identity.rs` three tests | asserted; F13 caught |
| Home and cache directory resolution | `home.rs` two tests (`$HOME` set and empty) | F11 caught; the passwd fallback and `XDG_CACHE_HOME` are not tested (gap, below) |
| FIEMAP and `getattrlistbulk` decoding | `extents.rs` five, `apfs_values.rs` three, `apfs.rs` `single_reply_decodes_...` | asserted |
| Test fixtures (volume mocks, `write_executable`) | `lib.rs:686..801` | asserted |

Hand-injected faults (`cargo-mutants` is not installed). `.work/scripts/mut.sh` substitutes one
expression, runs `cargo test -p kache-fs --features testing,staging,deser --lib -q`, restores the file
and checks `cmp`; exit 101 means a test failed:

| Mutant | Before | After |
| --- | --- | --- |
| F1 snapshot refcount `0 \| 1` to `0` | caught | |
| F2 drop the partial-sharing arm | caught | |
| F3 `nlink > 1` to `> 2` in `frees_nothing` | caught | |
| F4 `nlink <= 1` to `< 1` in `InodeLedger::admit` | **survived** (exit 0): the single-link case still returned `true` | caught (`unlinked_files_always_admitted`, exit 101) |
| F5 pinned when `links_seen <= nlink` | caught | |
| F6 follow symlinks in the portable reader | caught (APFS test) | |
| F7 do not subtract pinned bytes from `unique` | caught | |
| F8 ratio reported for an estimate | caught | |
| F9 drop the `checked_mul` overflow guard | caught | |
| F10 swap the 0644 and 0755 modes | caught | |
| F11 accept an empty `$HOME` | caught | |
| F12 drop the duplicate check in `CloneSketch::insert` | caught | |
| F13 follow a symlink in `directory_identity` | caught | |
| F14 `used_ratio` zero-total guard | caught | |
| F15 to F18 `statvfs` id, inode and fragment-unit decisions, `VolumeId` display | caught | |
| F19 `is_partial` threshold | caught | |
| F20 `shared_ratio` zero-allocation guard | caught | |
| S1 create-only publish becomes `persist` | caught (`create_only_publication_preserves_a_race_winner`) | |

Survivors: 1 found (F4), 1 killed; 20 of the 21 sampled decisions were already asserted.

Gaps recorded, not closed: the passwd fallback in `home::passwd_home` (needs an unset `HOME` in a
process of its own), `XDG_CACHE_HOME` handling, and `try_clone_extents_into` on a non-reflink
filesystem are only reachable on specific hosts.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` (DSLP-87); Linux-only code (`linux.rs`,
`extents`-driven probing, the Linux arms of `volume.rs`) is not exercised on this macOS host.

## DSLP-62 Dead code

Commands: `cargo clippy --no-deps -p kache-fs --all-targets -- -W dead_code -W unused
-W unreachable_pub` (features `staging`, `testing`, `deser`) and `cargo clippy -p kache-fs
--all-targets --all-features -- -D warnings`: no `dead_code` or `unused` warning; the manifest sets
`warnings = "deny"`. `.work/scripts/pubrefs.py crates/kache-fs` found no `pub` item without a
reference. Six public methods are referenced only from tests, workspace-wide (`rg -w`, excluding
`.work` and `target`):

| Item | Production consumers | Disposition |
| --- | --- | --- |
| `DirSizing::is_partial`, `DirSizing::shared_ratio`, `FileSizing::frees_nothing`, `VolumeUsage::used_ratio`, `CloneSketch::shares_with`, `InodeLedger::distinct_inodes` | none in the workspace | Consumer question: part of a published crate documented as semver-stable; no cut |

Windows: `home.rs` carries a 130-line `#[cfg(windows)] mod imp` (five `unsafe` blocks), but the
crate takes `std::os::unix` unconditionally (`identity.rs`, `fallback.rs`, `copy.rs`, `capacity.rs`,
`staging.rs`), CI has no Windows job, and no `cfg(windows)` appears in the rest of the crate, so the
module cannot be reached by any build the repository runs. Not removed: it is a platform-support
decision (`Needs owner`: delete the Windows branch, or restore Windows support elsewhere in the
crate).

Dependencies: `libc`, `rustc-hash` are used in production code; `deser` and `tempfile` are
optional and used behind their features; `deser-json` and `deser-value` are dev-only and used in
`capacity.rs` and `lib.rs` tests.

Needs environment: `cargo-machete`, `cargo +nightly udeps`, `cargo-bloat`.

Removed: only the vestigial `metadata_identity` arguments in DSLP-8.

## Findings filed by this component

- P3 (Needs owner, platform): delete or restore the Windows branch in `home.rs`.
- P4: 14 production `unsafe` sites without a `SAFETY:` comment (list above).
- P4: re-export `PinnedOutside`; name the `extents.rs` flags; rename the private `struct Clone` in
  `staging.rs`.
- P4: test the `$HOME`-unset passwd fallback and `XDG_CACHE_HOME`.
