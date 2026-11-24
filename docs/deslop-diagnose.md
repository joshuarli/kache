# Deslop diagnose

Bounded review of kache's riskiest boundaries at base `aa8df06` (branch
`deslop/ground`), run on a macOS arm64 host with nightly-2026-11-09. Every
finding below was reproduced against the built `kache` binary or the library
before it was filed. Reproductions use an isolated cache directory under
`.work/tmp`; none touched a real kache cache or `~/.config`.

## Method

The review read the code behind each boundary, then ran experiments that vary
one input and compare kache's artifact with the one plain `rustc` produces for
the same arguments. A hit that differs from the direct compile is a wrong hit.

Experiment harness (not committed; `.work/` is ignored): a wrapper script that
writes a `kache.toml` with `local_only = true`, `local_store`/`runtime_dir`
under the case directory, clears `RUSTC_WRAPPER` and `KACHE_SOCKET_PATH`, sets
`KACHE_HOST_CONFIG=` and runs `target/debug/kache rustc ...`. This is the shape
`tests/cache_key_underkeying_test.rs` uses.

## Scope reviewed

| Boundary | What was exercised | Result |
| --- | --- | --- |
| Cache-key soundness, rustc link arguments | `-C link-arg=-Wl,-sectcreate,...,FILE` and `-Wl,-exported_symbols_list,FILE` on a binary, FILE content changed between builds | **Wrong hit**, DSLP-89 |
| Cache-key soundness, compile-time env | `option_env!` unset vs empty value | Correct: distinct keys, miss |
| Cache-key soundness, native libraries | `-l dylib=foo` with changed `libfoo.dylib` install name and compatibility version | Correct: miss |
| Cache-key soundness, linker side files | Read `native_linker_side_files_are_unmodeled` and `link_token_names_input_file` in `src/cache_key.rs` | Source of DSLP-89 |
| Store integrity, concurrent writers | 32 parallel `kache rustc` invocations, 3 distinct keys, 2 rounds, one cache | No defect: 3 misses, 29 hits, no errors or store failures, one `rlib` digest per key |
| Store integrity, existing guards | `test_concurrent_put_remove_never_dangles`, `import_rejects_content_hash_mismatch`, `declared_inputs_of_a_packages_build_script_key_its_units`, `rustc_response_file_content_changes_cache_key` | Pass at base |
| Remote or shared store trust | `ArtifactStore::import_downloaded_entry` with a symlinked artifact in the entry directory | Defect in the library, no production caller, DSLP-90 |
| Path handling, debug bundle unpack | Read `unpack_debug_bundle_with_cap` (`src/compiler/mod.rs`): rejects rooted and `..` paths, links, and a 2 GiB cap, then unpacks into a private temp directory | No defect found |
| Path handling, restored artifact names | Read the rustc restore loop in `src/wrapper.rs` and `kache_format::is_safe_artifact_name` | No defect found |
| Cache directory privacy | Created cache and runtime directories are mode 0700, sockets 0600 | No defect found |

## Findings

### DSLP-89 (P1): linker input files named by `-C link-arg` are not keyed

A binary linked with `-C link-arg=-Wl,-sectcreate,__DATA,__note,<file>` or
`-C link-arg=-Wl,-exported_symbols_list,<file>` gets a cache key that contains
the argument string but not the file's content. Editing the file and rebuilding
restores the artifact built from the old content.

Cause: `unix_link_arg_inputs` hashes a linker operand only when its extension is
one of `.lib .a .obj .o .res .def .exp .manifest`
(`link_token_names_input_file`, `src/cache_key.rs`). The fail-closed list in
`linker_value_has_unmodeled_file` covers map, order and `@response` files only.
Options whose operand is a file read by the linker (`-sectcreate`,
`-exported_symbols_list`, `-unexported_symbols_list`, `-T`, `--version-script`,
`--dynamic-list`, `-filelist`) fall through both. The key logs a
`residual absolute path` warning for the argument and proceeds.

Reproduction (macOS, from the checkout root, `KC` is a wrapper that runs
`target/debug/kache` with an isolated `KACHE_CACHE_DIR`, `KACHE_RUNTIME_DIR`
and a `local_only` config):

```sh
printf '_alpha\n' > syms.txt
# main.rs exports `alpha` and `beta` with #[unsafe(no_mangle)] extern "C" fns
KC rustc --crate-name l2 --crate-type bin --edition 2021 --emit=link -C link-dead-code \
   --out-dir a main.rs -C link-arg=-Wl,-exported_symbols_list,$PWD/syms.txt
nm -gU a/l2 | grep -E ' _(alpha|beta)$'      # _alpha only
printf '_beta\n' > syms.txt
rustc ... --out-dir d ...                      # direct: _beta only
KC rustc ... --out-dir b ...                   # kache:  _alpha only (stale hit)
```

The same stale hit shows for `-sectcreate` with a data file: after the file
changed from `NOTE_v1_AAAAAAAA` to `NOTE_v2_BBBBBBBB`, direct `rustc` embeds v2,
kache restores v1 and `kache report` counts a local hit. Linux linker scripts
(`-Wl,-T,file`, `-Tlink.x` found through `-L`) and version scripts use the same
path; they were not run on this host.

### DSLP-90 (P4): `import_downloaded_entry` follows symlinks in a downloaded entry

`ArtifactStore::import_downloaded_entry` validates `files[]` names and hashes,
but checks each artifact with `is_file()` and `fs::metadata`, which follow
symlinks, and then `move_verified_blob` renames the entry's path into the blob
store and calls `set_blob_readonly` (`fs::set_permissions`, which follows
links). An entry directory containing `artifact -> /path/outside` whose target
bytes match the claimed hash is accepted: the blob becomes a symlink and the
outside file is made read-only (observed: `blob_is_symlink=true`,
`outside_mode=444` from a 0644 file). A blob that is a symlink also has no
stable content: whatever the link target holds later is what a restore reads.

No production code calls it: `import_restored_entry` and
`import_downloaded_entry` are referenced only by tests, and `AGENTS.md` defers
remote storage. The finding is latent. It is filed under owner decision
because the options are to delete the unused import path or to harden it
(`symlink_metadata`, reject non-regular files) before a remote layer returns.

### Already filed during Ground

DSLP-88 (P2): eleven library tests fail when `TMPDIR` is inside the checkout;
they pass with the system temporary directory. DSLP-87 (P3): `cargo-llvm-cov`
and `cargo-mutants` are not installed on the host.

## Not reviewed

- The C/C++ wrapper, oracle and benchmarks (`src/compiler/cc*`, `kache-cc*`):
  owned by `docs/roadmap.md`.
- Linux and Windows behavior, including Linux linker scripts (DSLP-89 predicts
  the same defect), musl, and cgroup scheduling.
- The daemon wire protocol and `kache-protocol` fuzzing beyond the guard tests.
- Build-script replay write-back (`OUT_DIR` links and recorded target-directory
  paths): read the documented checks, no experiment run.
- Eviction racing a restore, and hardlinked restored outputs modified in place
  by later build steps.
- A cache directory falling back to `/tmp/kache` when neither `HOME` nor a
  passwd entry exists (`default_cache_dir`). Directories are created 0700, but
  a pre-existing directory owned by another user was not tested because it
  needs a second account; treat it as an unproven hypothesis.
- Proc macros and build scripts reading undeclared inputs: documented as
  outside the key (`docs/how-it-works/cache-key.mdx`).
- Fuzzing, mutation (`cargo-mutants`) and coverage (`cargo-llvm-cov`): tools
  unavailable (DSLP-87).
