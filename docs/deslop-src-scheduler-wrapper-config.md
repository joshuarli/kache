# Deslop: `src` scheduler through wrapper_config, and test-projects (component 27 of 28)

Tickets DSLP-31 (types), DSLP-58 (tests), DSLP-85 (dead code). Base `11cbdba8`, branch `deslop/k7`, macOS arm64,
`nightly-2026-11-09`. Scope: `src/scheduler.rs`, `service.rs`, `since.rs`, `store_view.rs` and `store_view/tests.rs`,
`term.rs`, `test_support.rs`, `toolchain_dylib.rs`, `tree_stamp.rs`, `verify_compare.rs`, `version.rs`, `volume_gc.rs`,
`wrapper_config.rs` (8,284 lines) and the four fixtures in `test-projects/` (472 lines), 205 unit tests. `src/scheduler/`
(`permit_wait.rs`, `pressure.rs`) is a directory the ticket's Where does not list and no other ticket lists; it was left
alone and is filed below.

## DSLP-31: Types

Same single clippy run as component 25 (`docs/deslop-tests-adaptive-diagnostic.md`), deduplicated over lib, bin and
test targets.

| Candidate | Hits in scope | Disposition |
| --- | ---: | --- |
| `unsafe_code` (forbid) | 12 | Reject for the crate: `getrusage` and `sysctlbyname` (`scheduler.rs`), `ioctl(TIOCGWINSZ)` (`term.rs`), `pre_exec`/`setsid` (`test_support.rs`) need libc, and `set_var`/`remove_var` in `verify_compare.rs:511-519` are edition-2024 unsafe under a lock. Two blocks had a safe form and were removed (below). |
| `clippy::unwrap_used` | 420 | Reject. 0 in non-test code: every site is after the file's `#[cfg(test)]` line (`scheduler.rs` 161, `tree_stamp.rs` 37, `verify_compare.rs` 37, the rest fewer). |
| `clippy::expect_used` | 12 | Reject, 0 in non-test code. |
| `unreachable_pub` | 23 | Reject. `scheduler.rs` 15, `service.rs` 6, `version.rs` 1: `pub` items inside private modules. Narrowing changes no behavior and rejects no defect. |
| `clippy::cast_possible_truncation` | 5 | Reject. `scheduler.rs:123` CPU count `usize` to `u32`; `:911` already `.min(u64::from(u32::MAX))`; `:1859` a test; `term.rs:446` spinner elapsed milliseconds; `store_view/tests.rs:21` a fixture byte. |
| `clippy::cast_sign_loss` | 4 | Reject. `scheduler.rs:781` `ru_maxrss` after the `<= 0` guard in `rss_from_getrusage`; `tree_stamp.rs:375,376` ctime and nanoseconds folded into a hash, where a wrapped negative value hashes to a different but stable number; `since.rs:251` a test. |
| `clippy::redundant_clone` | 2 | Reject. `store_view/tests.rs:76,134`, config clones in fixtures. |
| `clippy::needless_pass_by_value` | 1 | Reject. `wrapper_config.rs:75` takes `Option<PathBuf>` it clones from. |
| `clippy::wildcard_enum_match_arm` | 1 | Adopt. `service.rs:233`, `_ => {}` on `kache_shims::Stability`. |
| `clippy::match_wildcard_for_single_variants` | 1 | Adopt. `service.rs:486`, `other =>` on `StartupLog`. |
| `dead_code`, `unused_crate_dependencies` | 0 | Clean. |

Escape-hatch count (token matches in scope files): `unsafe` 12 to 10, `allow` attributes 0 to 0, non-test `unwrap` and
`expect` 0 to 0.

Adopted fixes, each with a witness and `cargo clippy -p kache --all-targets -- -D warnings` exit 0:

- `version.rs`: `VERSION` removed a leading `v` with `core::str::from_utf8_unchecked`. `str::split_at` is `const` on this
  toolchain, so `strip_leading_v` does the same cut safely. Witness: the new unit test
  `version::tests::only_one_leading_v_is_stripped_and_the_rest_flows_through` (`src/version.rs:23`), and end to end
  `KACHE_VERSION=v9.9.9-rc.1 cargo build --bin kache --bin kache-engine` prints `kache 9.9.9-rc.1` while an unset
  variable prints `kache 2.0.0-rc.2`. Fault control: `split_at(1)` changed to `split_at(2)` fails that test with
  `left: ".5.0-rc.4"`, `right: "0.5.0-rc.4"` (exit 101); restored, `cmp` exit 0.
- `term.rs`: `terminal_columns` built its `winsize` with `unsafe { mem::zeroed() }`; four integer fields make a literal
  the same value. Witness: `terminal_color_policy_and_wrapping_match_the_actual_command` drives the real ioctl through a
  pty and passes.
- `service.rs`: the two wildcard arms now list `InstallerManaged | UserManaged` and `Journal`, so a new variant is a
  compile error instead of silence. No runtime change.

## DSLP-58: Tests

Tier and cost: tier 1 unit tests inside the modules, 206 passed and 3 ignored for these modules in 2.0 s of test time
(`cargo test --lib -- scheduler:: service:: since:: store_view:: term:: ...`); the whole lib is 3,135 tests in 71 s. The
`test-projects/` fixtures are exercised by tier-2 tests (`custom_harness_test.rs:56`, `stale_artifact_test.rs:24`,
`integration_test.rs:567,596,732`) and by `scripts/cc-project-autotools.py` for `cc-autotools`. One test was added and
runs in two extra bin targets at 0.00 s each, so the 20% and 30 s budget holds.

| Contract | Guarding tests (file:line) | Disposition |
| --- | --- | --- |
| Tree digests that feed cache keys: a rewrite or a new name changes the stamp, unreadable entries refuse or are stamped, symlinks, exclusions, a cargo build dir is stamped by its tag alone | `tree_stamp.rs:517` `a_stamp_settles_one_window_after_its_newest_write`, `:544`, `:598`, `:625`, `:668`, `:763`, `:788`, `:829` | Guarded |
| `--since` windows: unit parsing, saturation, cutoff before the epoch | `since.rs:144` `parses_every_unit` and 11 more | Guarded; fault control below |
| Verify mode: restored artifacts compared with a recompile, divergences classed content or path-debug | `verify_compare.rs:491-927` (26 tests) | Guarded |
| Scheduler: pool size, RSS-weighted permits, lease reclaim on exit, flights scoped to the cache key, cgroup path parsing | `scheduler.rs:1857`, `:1934`, `:2005`, `:2108`, `:2269`, `:2425` and 68 more | Guarded |
| Daemon service files and exe selection (launchd and systemd) | `service.rs:1204,1230` parse exe from plist and unit, `:1284,1301,1354` exe problems, `:1462,1488` generated content | Guarded |
| Volume shard inventory and GC | `store_view/tests.rs:39-398` (12), `volume_gc.rs:97-160` (5) | Guarded |
| Wrapper detection for doctor and status | `wrapper_config.rs:118,145,178,209` | Guarded |
| Toolchain dylib fallback | `toolchain_dylib.rs:82-161` | Guarded |
| Terminal rendering: colour policy, alignment, widths, durations | `term.rs:475-753` (18) | Guarded; the real ioctl only through the pty test in `cli_commands_test.rs:999` |
| `--version` string | `version.rs:23`, plus end-to-end check above | Was `none` before this change; now guarded |

Hand-injected fault (critical decision: how a `--since` value maps to a window). `src/since.rs`, the `'h'` arm of
`SinceWindow::parse`: `Unit::Hours` changed to `Unit::Mins`.

```sh
cargo test --lib -- since::       # exit 101
  FAILED: since::tests::accepts_one_leading_plus, default_is_twenty_four_hours, label_keeps_the_requested_unit,
          parses_every_unit, saturates_past_the_largest_window     (left: 180, right: 10800)
cp .work/tmp/since.rs.orig src/since.rs; cmp .work/tmp/since.rs.orig src/since.rs   # cmp exit 0
git diff --exit-code src/since.rs                                                   # exit 0
```

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed (`which` exits 1 for both), so there is no
line or branch coverage and no mutation score for this scope; the `.cargo/mutants.toml` exclusions that apply here
(TUI entry points, platform probes) belong to the modules next door, not these.

## DSLP-85: Dead code

Tools: rustc `dead_code` and `unused_crate_dependencies` through the clippy run: 0 diagnostics. `cargo-machete`,
`cargo-udeps` and `cargo-bloat` are not installed (Needs environment). Workspace-wide caller search for every `pub` or
`pub(crate)` item in the 12 files (`rg -w NAME src crates tests build.rs`, items with at most two matches listed): all
six, `weight_from_rss`, `login_service_available`, `NamedSection`, `without_terminal`, `walk_skipping` and
`resolve_wrapper_setting`, have a live non-test caller (`scheduler.rs:425`, `cli.rs:6062`,
`build_script/inputs.rs:447`). No code was removed.

Documentation and pointer defects removed (no runtime effect):

- `test_support.rs`: the two doc lines for `test_config` sat above `without_terminal`, so rustdoc gave that function a
  description of the wrong one and left `test_config` bare. Moved (`0918834a`).
- `test-projects/{custom-harness,hello-world,stale-check}/Cargo.toml`: each said to see
  `test-projects/multi-dep/Cargo.toml` for why it declares an empty `[workspace]`. That project no longer exists
  (`rg multi-dep` finds only these three pointers); the reason is the root `Cargo.toml` comment on the workspace members.
  Consumers run: `custom_harness_test`, `stale_cache_hit_baseline` and `test_wrapper_hello_world` pass (`ab853826`).

Platforms and features covered: macOS arm64, default features. Not covered: Linux-only branches of `service.rs`
(systemd) and `scheduler.rs` (cgroup, `/proc/meminfo`), which this host does not compile.

## Findings filed from this component

- P3: `src/scheduler/` (`permit_wait.rs`, `pressure.rs`) is in no deslop ticket, and neither are `launcher/` (396 lines,
  20 `unsafe` tokens, embedded into every kache build), `scenarios/` and `crates/README.md`.
- P4: an empty `src/lib.rs` is tracked while `Cargo.toml` points `[lib]` at `src/main.rs`; nothing reads it.
