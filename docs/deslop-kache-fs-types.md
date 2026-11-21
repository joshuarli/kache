# kache-fs Types audit

DSLP-8, component 4 of 28. Base: `d89e7e2cc3093a6a8b5a1a901fc0d3c5d5465913`.
Independent branch from latest origin/main at creation; no unmerged deslop
branch is included. Scope is all 16 Rust files in crates/kache-fs: 4,927 lines
before, consisting of 2,544 production, 2,129 tests and 254 optional test-helper
lines. Helpers are lib.rs's cfg(any(test, feature = "testing")) testutil module
and process_state_test.rs, which is gated by feature "testing". They are not
ordinary runtime code even when compiled by --all-features.

## One contract tightened

Class: **not a runtime bug; an unchecked conversion with an existing guard**.
Unix home_dir consults HOME, then passwd_home when HOME is missing or empty.
passwd_home obtains its getpwuid_r buffer capacity from sysconf. Previously a
negative value selected 512, and a nonnegative libc::c_long was cast to usize.
The guard already prevented sign loss. Replace the guard plus cast with
usize::try_from(...).unwrap_or(512), and deny clippy::cast_sign_loss in home.rs.
One numeric cast and its manual sign proof are removed. No new panic, allocation,
public signature or fallback is introduced.

On the supported 64-bit Linux and macOS ABIs, every nonnegative c_long fits
usize. Zero still selects zero, positive values retain their magnitude, and
all negative values still select 512. The existing getpwuid_r error/null checks,
buffer ownership and empty-directory handling are untouched. Windows does not
compile this Unix implementation. This is not a fix for a demonstrated bad
home-directory result, and no such defect is claimed.

The module-scoped lint has zero remaining false positives. A crate-wide setting
is not adopted: macOS allocation parsing has other guarded signed conversions,
and its Clippy results were not measured on this Linux host.

### Rejected-program witness

With the deny retained, temporarily restored the original guarded match and
ran:

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-fs --lib --all-features --no-deps
```

```text
error: casting `i64` to `usize` may lose the sign of the value
  --> crates/kache-fs/src/home.rs:64:26
64 |                 sized => sized as usize,
   |                          ^^^^^^^^^^^^^^
note: the lint level is defined here
  --> crates/kache-fs/src/home.rs:9:9
9 | #![deny(clippy::cast_sign_loss)]
TYPE_WITNESS_EXIT=101
```

Restored the checked conversion before all positive checks. This is a Clippy
policy witness, not a claim that rustc alone rejects the old cast. No compile_fail
doctest is added for a Clippy-only diagnostic. The persistent module deny guards
the production call site. No new runtime parser or predicate needs boundary tests;
the existing empty-HOME test exercises the passwd fallback. No tests of the
standard library's TryFrom implementation are added.

## Strict-profile measurements

```sh
taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-fs --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn \
  -W clippy::wildcard_enum_match_arm -W clippy::match_wildcard_for_single_variants \
  -W clippy::cast_possible_truncation -W clippy::cast_sign_loss \
  -W clippy::unwrap_used -W clippy::expect_used -W clippy::redundant_clone \
  -W clippy::needless_pass_by_value

taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 \
  cargo clippy -p kache-fs --all-targets --all-features --no-deps \
  --message-format=json -- --cap-lints warn -F unsafe_code
```

Discovery cap is local to these measurement commands; no checker setting was
weakened in the repository. Deduplicate primary diagnostic spans across library
and library-test targets. Separate calls on one line count separately. The first
command ran before and after; both exited 0. The unsafe probe exited 0 and is a
baseline measurement; the changed code retains the same sysconf unsafe operation.

Counts below are **before -> after**, on x86_64-unknown-linux-gnu with features
deser, staging and testing. macOS and Windows gated bodies are not compiled here.

| Candidate | Production | Test helpers | Tests | Disposition |
| --- | ---: | ---: | ---: | --- |
| unsafe_code = forbid | 11 -> 11 | 1 -> 1 | 4 -> 4 | Reject. Native statvfs/statfs, ioctl and passwd FFI are not replaceable by safe std equivalents with these contracts. Helper atexit cleanup and existing environment/permission tests remain. |
| wildcard_enum_match_arm | 0 -> 0 | 0 -> 0 | 0 -> 0 | Reject: no witness. |
| match_wildcard_for_single_variants | 0 -> 0 | 0 -> 0 | 0 -> 0 | Reject: no witness. |
| cast_possible_truncation | 2 -> 1 | 0 -> 0 | 0 -> 0 | Do not adopt broadly. The home cast is removed. Remaining Linux cast converts constant EXTENTS_PER_PAGE = 128 into the FIEMAP u32 header field. |
| cast_sign_loss | 1 -> 0 | 0 -> 0 | 0 -> 0 | Adopt in home.rs only; one trust removal, zero local false positives. |
| unwrap_used | 0 -> 0 | 5 -> 5 | 187 -> 187 | Reject: ordinary production is already clean. Test setup, expected results and test-helper invariants are intentional. |
| expect_used | 0 -> 0 | 10 -> 10 | 4 -> 4 | Reject: ordinary production is already clean. Helpers fail test setup explicitly; tests assert expected measurements. |
| redundant_clone | 0 -> 0 | 0 -> 0 | 0 -> 0 | Reject: no witness. |
| needless_pass_by_value | 0 -> 0 | 0 -> 0 | 0 -> 0 | Reject: no witness. |

The remaining FIEMAP cast is bounded by its constant, not a runtime extent count.
The ioctl reply count is separately checked against the fixed page array before
slicing, and extents::walk rejects excess entries, overflow, overlaps, unsupported
flags and malformed LAST termination. No new defect is established there.

## Escape-hatch accounting and retained trust

Matching-line pattern: `\bunsafe\b|#!?\[allow\(|\.unwrap\(\)|\.expect\(` over
crates/kache-fs/**/*.rs. This includes inactive platform code and comment matches,
but not numeric casts. The cast removal is therefore reported separately, not
misrepresented as a reduction in this counter.

| Scope | Before | After |
| --- | ---: | ---: |
| Production | 27 | 27 |
| Optional test helpers | 16 | 16 |
| Tests | 236 | 236 |
| Total | 279 | 279 |

- Native calls: copy.rs's Linux FICLONE and macOS clonefile; linux.rs's FIEMAP;
  capacity.rs's statvfs; volume.rs's statfs; home.rs's Unix passwd and Windows
  known-folder APIs; apfs.rs's getattrlist/getattrlistbulk and directory descriptors;
  staging.rs's macOS ACL query/free. These expose information or clone semantics
  that std::fs metadata/copy does not provide. Unsafe is retained, not certified
  by this type audit as a complete memory-safety review.
- volume.rs's unnecessary_cast and two useless_conversion allowances bridge
  glibc versus musl field types. Removing them would either break the warning-deny
  build or weaken negative-size rejection on glibc. Its cfg_attr dead-code
  allowance for Linux magic constants is outside this text pattern but retained
  for cross-platform compilation and classifier tests.
- testutil's mutex unwraps expose poisoned test fixtures; its expects fail file
  creation, writing, synchronization and executable setup. process_state_test
  retains atexit directory lifetime management and its setup unwraps. Tests retain
  existing environment guards and umask/permission setup. No new environment test
  or live-environment dependency was added.
- Static review of inactive branches: APFS signed allocation counts are clamped
  before unsigned conversion; parser references use checked offsets and bounded
  slices. Windows GUID casts split defined bit fields, and native known-folder
  buffers are copied then freed. These are not host Clippy measurements or a
  Windows build claim. No broader refactor or unsupported platform promise follows.

## Boundaries and consumers

Public FileSizing, DirSizing, Sharing/Confidence and ProbeCaps retain their
unknown-versus-known semantics. Missing private allocation is None, not zero.
VolumeStats preserves optional identity/inode observations and checked byte
products. InodeLedger preserves hardlink identity/deduplication and pinned-outside
accounting. Non-exhaustive enums and optional deser representations are unchanged.
No CLI, report JSON, cache key, store format or wrapper wire format changes.

Inspected producers and consumers: probe_for/measure_file and SizeProbe directory
aggregation; fallback and APFS/FIEMAP accounting; copy/reflink permission and
existing-destination behavior; StagedFile's optional clone ownership, writer and
publication cleanup; home/cache lookup; inode/path identity; lexical paths;
sample_write_clock's closed-writer timestamp. None needs a new domain type for
the selected conversion.

Workspace Cargo path dependencies resolve this worktree's kache-fs. Consumers
include kache-config (home/cache and path handling), kache-store (measurements,
staging and clone fallback), kache-ipc (workspace staging/publication),
kache-shims (file identity), kache-e2e (test helpers), and the root CLI/service and
build-script paths. Reviewed call sites include config.rs home fallback,
src/service.rs, ipc's execution/workspace.rs and store's link.rs/sharing.rs.
Only home.rs changes, with no public API or observable runtime change on supported
ABIs. Consumer workspace tests run through make check against this path dependency.
No performance workload, full oracle or project campaign was run: filesystem
measurement, restore, allocation and syscall counts are unchanged.

## Verification

Tools: rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`, Cargo
`1.102.0-nightly (c07ac2c4d 2026-11-06)`, Clippy
`0.1.101 (5b7b37aefc 2026-11-08)`. Python 3.15.2 parsed JSON/counts. Heavy
commands use four of eight logical CPUs and clear RUSTC_WRAPPER. The loaded
environment has all required Rust components; no maintenance/install was needed.
The todo CLI is unavailable, so closure uses the established direct backlog-edit
convention. DSLP-8 is removed, Next stays 103, and existing owner tickets remain.
No new demonstrated defect needs a ticket; no forbidden path was edited.

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 cargo test -p kache-fs --all-features
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER \
  CARGO_BUILD_JOBS=4 make check
```

Scoped result:

```text
test result: ok. 83 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
FS_TEST_EXIT=0
```

macOS/Windows execution and cross-target Clippy are not claimed. No dependency,
toolchain, build/CI configuration, blanket allowance or formatter churn is added.

Required gate output:

```text
test-resource guard: all checks passed
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.68s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
MAKE_CHECK_EXIT=0
```
