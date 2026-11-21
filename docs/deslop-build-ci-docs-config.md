# Deslop: build, CI, docs and configuration (component 28 of 28)

Tickets DSLP-32 (types), DSLP-59 (tests), DSLP-86 (dead code). Base `11cbdba8`, branch `deslop/k7`, macOS arm64,
`nightly-2026-11-09`. Scope: `build.rs` (92 lines), `Cargo.toml` (130), `Cargo.lock` (167 packages), `.cargo/`
(`config.toml`, `mutants.toml`), `.github/workflows/` (4 files, 656 lines), `Makefile` (362), `Dockerfile` (195),
`Cross.toml`, `rust-toolchain.toml`, `docs/` (18 MDX pages, 4 JSON files), `campaigns/` (72 oracle cases, 165
inventory entries), `README.md` and `C_CPP_PARITY_CAMPAIGN.md`. `.github/` and `Cargo.lock` are review-only here: no
workflow or dependency was changed. `launcher/` is compiled by `build.rs` but is not in this ticket's Where (filed in
component 27).

## DSLP-32: Types

Same single clippy run as component 25 (`docs/deslop-tests-adaptive-diagnostic.md`).

| Candidate | Hits in scope | Disposition |
| --- | ---: | --- |
| `clippy::expect_used` in `build.rs` | 4 | Reject. `build.rs:16,19,24,48` (`OUT_DIR`, `TARGET`, the empty-launcher write, the `rustc` spawn) are the way a build script reports failure. |
| every other candidate in `build.rs` | 0 | No witness (`unsafe_code`, `unwrap_used`, casts, wildcards). |
| `unsafe_code = "forbid"` in `[lints.rust]` | n/a | Reject for the package: `libc` FFI in `src/` needs `unsafe`; `build.rs` itself has none. |
| `clippy::too_many_arguments`, `clippy::permissions_set_readonly_false` (the two manifest allows) | 13 and 9 when enabled | Keep. Both fire in `src/` and `crates/kache-store`, so the allows are live; they also made the local attributes in components 25 and 26 redundant. |
| `unused_crate_dependencies` on the `kache` package | 0 | Clean; every `[dependencies]` and `[dev-dependencies]` entry names a crate the package uses. |

Typing escape hatches in configuration are the mutation exclusions. `.cargo/mutants.toml` held 16 `exclude_globs` and
131 `exclude_re` entries (147); after DSLP-86 below it holds 4 and 91 (95), and each remaining entry still names an
identifier that exists. No other scope file carries an escape hatch.

`Cargo.lock` (review only): 167 packages; `cargo update --workspace --locked --offline` changes nothing; duplicates in
normal and build edges are `foldhash` 0.1.5 and 0.2.0 and `hashbrown` 0.15.5 and 0.17.1, both reached through
transitive dependencies.

`.github/` (review only): every `uses:` is pinned to a 40-character commit, every workflow starts at `permissions: {}`,
every job has a `timeout-minutes`, and `concurrency` is set. Gaps filed below: no workflow runs the other legs of
`make check` (`client-crates-check`, `test-resource-guard-check`), none runs `make pr` or `mutants-diff`, and none runs
`scripts/check-docs.sh`.

## DSLP-59: Tests

Nothing here has line coverage, so the contracts below are traced to the check that would fail.

| Contract | Guard (file:line) | Disposition |
| --- | --- | --- |
| `build.rs` embeds a working build-script launcher on Unix | `tests/build_script_declared_inputs_test.rs:256` `declared_inputs_of_a_packages_build_script_key_its_units`; `tests/launcher_personality_test.rs:14` compiles `launcher/compile.rs` itself | Guarded, see the fault control |
| `build.rs` `link_flags` keeps only link-affecting `-C` options | none | Gap: build scripts are not test targets, so no test can reach it. Filed. |
| PR gate: `make test`, `make lint` | `.github/workflows/ci.yml:42`, `lint.yml:38` | Guarded |
| Musl workspace tests in the builder image | `ci.yml:114-139` builds `Dockerfile` and runs `make test TARGET=...` | Guarded |
| `make check` legs `client-crates-check`, `test-resource-guard-check` | none in CI | Gap. Filed. |
| Changed-line mutation (`make pr`, `mutants-diff`) | none in CI | Gap. Filed. |
| Documentation structure (front matter, JSON, `meta.json` entries, links) | `scripts/check-docs.sh` (exit 0, 18 pages) | Passes, but nothing runs it. Filed. |
| CLI reference matches the binary | none | Gap with a witnessed defect, below. |
| `docs/commands/cli.schema.json`, `report.schema.json` match emitted JSON | `tests/cli_commands_test.rs`, `src/report/tests.rs`, `crates/kache-e2e/src/report.rs` | Guarded |
| C/C++ oracle inventory and blocking cohort | `make cc-oracle` and its negative control, `ci.yml:72,74,139,198,200`; `campaigns/cpp/STATUS.md` counts (165 entries, 72 `oracle`) agree with `cases.toml` and the 72 `oracle/*/case.toml` | Guarded |

Witnessed defect, fixed: `docs/commands/reference.mdx`, `installation.mdx` and `quick-start.mdx` described a
`kache completions <shell>` command and said `kache init` installs shell completions. The shipped binary says so
otherwise:

```sh
target/debug/kache completions zsh      # error: unrecognized subcommand 'completions'   (exit 2)
rg -i completion src crates             # no hit outside unrelated test names
git log -S completions -- src/cli_args.rs   # 3a5f0f40 "delete tui, clap, tracing, etc"
```

The three pages now omit it (`6177b432`). Comparing the `Usage: kache X` pages of `kache --help` with the `## kache X`
headings of the reference, the help-only names are `gc`, `purge` and `install-shims` (hidden, documented in a table),
`report` (documented under `stats`) and `help`; `completions` was the only heading with no command.

Hand-injected fault (critical decision: build the launcher on Unix). `build.rs`, `CARGO_CFG_UNIX` guard flipped from
`.is_none()` to `.is_some()`, so a Unix build embeds an empty launcher:

```sh
cargo test --test build_script_inputs_test --test cached_units_regression_test \
           --test build_script_declared_inputs_test -- build_script      # exit 101
  test declared_inputs_of_a_packages_build_script_key_its_units ... FAILED
  panicked at tests/build_script_declared_inputs_test.rs:302:5   left: ["v2:unset", "v2:unset"], right: ["v2:x", "v2:x"]
cp .work/tmp/build.rs.orig build.rs; cmp .work/tmp/build.rs.orig build.rs   # cmp exit 0
git diff --exit-code build.rs                                              # exit 0
```

The same fault survived `cargo test --test build_script_launcher_test` (exit 0, both tests pass with a 0-byte embedded
launcher: `target/kache-test-bootstrap/debug/build/kache/*/out/build-script-launcher` was 0 bytes). Those two tests
check the install and its failure record, not that the installed file is the real launcher. Another test catches the
fault, so the decision is guarded, but the launcher's own test file has a survivor. Filed.

Needs environment: `cargo-llvm-cov` and `cargo-mutants` are not installed (`which` exits 1 for both), so no coverage
or mutation score exists for any file here; `make mutants-diff BASE=origin/main` is the lane once installed.

## DSLP-86: Dead code

Tools: rustc through the clippy run (0 `dead_code`, 0 `unused_crate_dependencies`); `cargo-machete`, `cargo-udeps` and
`cargo-bloat` are not installed (Needs environment), so unused dependencies were checked by `rg -w` per crate name over
`src/`, `build.rs` and `tests/` (every `[dependencies]` entry has at least one use) and by the compiler lint above.

Removed, with the search that proves each:

- `.cargo/mutants.toml`, 12 of 16 `exclude_globs`: `crates/kache-client/src/{stdio,frontend}_windows.rs`,
  `crates/kache-ipc/src/{windows_handles,windows_process,windows_msvc}.rs`, `execution/{windows,windows_client}.rs`,
  `execution/workspace/windows.rs`, `src/invocation/{windows_broker,process/windows}.rs`,
  `src/daemon_execution_windows.rs` and `tests/windows_execution_test.rs` do not exist (`[ -e path ]` fails for each);
  Windows support was dropped in `d09be683`.
- `.cargo/mutants.toml`, 40 of 131 `exclude_re` entries, with the comment blocks that described only them. Each one
  names a file that is gone (`windows_security.rs`, `build_intent.rs`, `remote_pack.rs`, `native_link_key.rs`) or
  identifiers found nowhere in `src/`, `crates/` or `tests/` (`run_monitor`, `run_config_editor`, `connect_tokio`,
  `find_daemon_pids`, `join_inflight_download`, `fsync_published_blob`, `send_handoff_windows`, `observe_storage_windows`,
  `NonInheritableStdio` and others). A regex that no mutant name can match excludes nothing. The result parses
  (`tomllib`), is a strict subset of the old lists, and a rerun of the same check flags 0 of the 91 that remain. Consumers:
  `make mutants-diff` is the only reader and `cargo-mutants` is not installed here, so the mutation lane itself was not
  run (Needs environment). Comments above partially removed groups, and the long prose header, still mention Windows
  and the monitor TUI; they need their owner (filed).
- Statements the repository contradicts: `README.md` linked `.github/CONTRIBUTING.md`, deleted in `ab34ef6c`;
  `Makefile` said CI runs the changed-line mutation job (no workflow does); `C_CPP_PARITY_CAMPAIGN.md` said `STATUS.md`
  did not exist and no package had started (`campaigns/cpp/STATUS.md` has a reconciled ledger). All three corrected
  (`d4440416`).
- `docs/`: the `completions` command and the shell-completion setup, above.

Checked and kept:

- `Makefile`: 41 targets; each is in `make help`, a prerequisite, a workflow step or a `$(MAKE)` call, except `help` itself
  and three (`test-bench-short`, `test-bench-edit-loop`, `test-bench-contention`) run unit tests for `scripts/bench/` and
  `scripts/test-bench-contention.py` and are in no workflow, help line or doc. They belong to the scripts component.
  Make goals named in `README.md`, `docs/`, `AGENTS.md` and the workflows all exist.
- `Dockerfile`: every `ARG` is used (`TARGETARCH` 8 times, the others once); both workflows build it.
- `Cross.toml` (two lines, `KACHE_VERSION` passthrough for the `cross` tool): no `cross` invocation exists in the
  Makefile, workflows, scripts or docs, and the file dates from the Windows builds. Consumer question for the owner,
  not deleted.
- `Cargo.toml` `[features] default = []`: no code reads the feature, but `kache` is published, so `cargo install kache
  --no-default-features` would break on removal. Consumer question, not deleted.
- Documentation links: all relative and site-absolute links resolve (`/docs/...` maps to `docs/*.mdx`) except the
  README link fixed above.
- `campaigns/`: 72 oracle directories each with `case.toml` and `source`, matching `cases.toml`; no orphan case.

Platforms: macOS arm64 only; the Linux musl legs of `Makefile` and `Dockerfile` were read, not run.

## Findings filed from this component

- P3: `build.rs` `link_flags` and `target_is_glibc` have no test. Moving them into `launcher/compile.rs`, which
  `tests/launcher_personality_test.rs` already compiles, would make them testable.
- P3: `build_script_launcher_test.rs` survives an empty embedded launcher; add an assertion that the installed pin
  matches the embedded bytes.
- P3: CI runs neither `client-crates-check`, `test-resource-guard-check`, `make pr`, nor `scripts/check-docs.sh`;
  `scripts/check-docs.sh` passes today. Wire the first two into `ci.yml`/`lint.yml` (workflow change, left alone here).
- P3: add a docs test that every visible `kache --help` command has a heading in `docs/commands/reference.mdx` and the
  reverse; it would have caught `completions`.
- P3: `.cargo/mutants.toml` prose (lines 1 to about 250) and the comments over partly pruned groups still describe the
  Windows and TUI exclusions removed here.
- P4: `README.md` links `issues/new?template=bug_report.md` and `feature_request.md`, and `.github/ISSUE_TEMPLATE` does
  not exist.
- P4: `TODO.md:8` says 67 oracle cases (63 blocking); `campaigns/cpp/STATUS.md` and `cases.toml` say 72 (68 blocking).
