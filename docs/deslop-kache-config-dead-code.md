# kache-config dead-code audit (DSLP-5)

## Scope and outcome

Base: `bcb257a6f8901a2bade7804121f562acae98de8b` (`origin/main` when the
independent `deslop/dslp-5` worktree was created). This is not based on DSLP-4
and does not include its test changes. Reviewed the crate's configuration,
invocation, policy and optional testing modules, README, manifests, AGENTS.md,
open tickets, recent crate history and workspace consumers.

**No source or dependency removal is proven safe.** Rust's dead-code check and
cargo-machete are clean. Two manual public-API retirement candidates remain in
the reasoned allowlist below and are deferred to owner decision in DSLP-102.
Closed DSLP-5 by removing its ticket, matching earlier deslop commits; `todo`
is unavailable. No formatter ran.

Contracts are **unaffected**: this diff changes only this audit and the backlog.
Rust APIs, CLI flags/exit codes, TOML/environment semantics, cache-key inputs,
store formats and wrapper protocol are unchanged. No performance-path change
requires a campaign. No edits to `src/compiler/cc*`, `src/probe/*` or
`campaigns/cpp/*` were made.

## Tools and commands

Linux x86_64, four of eight logical CPUs, system temporary directory, no ambient
cache override. Versions: rustc `1.102.0-nightly (5b7b37aef 2026-11-08)`, Cargo
`1.102.0-nightly (c07ac2c4d 2026-11-06)`, cargo-machete `0.10.1`.

Environment diagnosis: Cargo was on the expected user PATH, but neither machete
nor udeps was installed. Installed machete into user Cargo with
`taskset -c 0-3 env -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 cargo install --locked cargo-machete`,
using the repository's approved replacement registry. No project dependency,
toolchain, shared configuration or CI lane changed. The environment blueprint's
rust-src/clippy/rustfmt maintenance was unnecessary; compilation and Clippy work.

Each rustc/test command below used the prefix
`taskset -c 0-3 env -u RUSTC_WRAPPER -u TMPDIR -u KACHE_CACHE_DIR CARGO_BUILD_JOBS=4`:

| Command | Result |
| --- | --- |
| `cargo rustc -p kache-config --lib -- -D dead_code` | Exit 0; no findings |
| `cargo rustc -p kache-config --lib --features file-write -- -D dead_code` | Exit 0; no findings |
| `cargo rustc -p kache-config --lib --all-features -- -D dead_code` | Exit 0; no findings |
| `taskset -c 0-3 cargo machete --with-metadata crates/kache-config` | Exit 0; no unused dependencies |
| `cargo test -p kache-config --all-features` | Exit 0; 112 passed, 0 failed; 0 doc-tests |

`cargo metadata --no-deps --format-version 1` identifies one library target and
no binaries or build-script target in kache-config. `unreachable_pub` in binary
crates is not applicable to this slice. The three rustc commands cover every
declared feature combination: default, file-write and test-support (which enables
file-write and kache-fs/testing). There is no scoped `allow(dead_code)` or lint
suppression. The all-feature test run exercises test-only helpers. macOS paths
were read, not executed; no cross-target build or CI for this branch is claimed.

```text
cargo-machete didn't find any unused dependencies in crates/kache-config. Good job!
MACHETE_EXIT=0
```

Manual dependency review agrees: anyhow supplies error contexts/results;
kache-core supplies size/store configuration; kache-fs supplies platform paths,
normalization and test locking; kache-shims resolves programs; blake3 fingerprints
provenance; glob matches exclusions; deser derives schema codecs; deser-toml and
deser-value implement parsing/merging/editing; log supplies diagnostics. Dev
dependencies supply fixture files, logging assertions and test collections.
No disputed dependency remains, so the tools reference's conditional
`cargo +nightly udeps` step was not needed and was not run.

## Entry points, registrations and consumers

- `lib.rs` publicly exports config, invocation, policy, `Config` and
  `InvocationEnv`; testing is enabled only for tests/test-support. The manifest
  is publishable, so repository searches cannot establish external API non-use.
  No dynamic Rust plugin/entry registry was found in the crate.
- `kache-client/src/frontend.rs::run` captures `InvocationEnv`, calls
  `Config::load_for`, and uses socket/trust settings before daemon execution.
  `kache-client/src/execution.rs::Execution::prepare` validates and serializes
  the snapshot into protocol inputs: reachable product paths, not test references.
- `src/config.rs` wildcard-reexports config; `src/invocation.rs` reexports
  `InvocationEnv`; `src/cache_key.rs` reexports `is_manifest_dir_var`.
  `src/wrapper.rs` uses invocation-relative routing and probe memo paths;
  `src/cli.rs` uses provenance and routed stores; `src/volume_gc.rs`,
  `src/store_view.rs` and `src/daemon.rs` use shard budgets. Searching only for
  `kache_config::` would miss these consumers.
- `Config::load_with_provenance_for` reaches file/host merging, environment
  normalization and CI recovery policy. The policy export has an internal
  caller despite no direct external reference. Schema structs/fields are read
  by deser derives and TOML, not only explicit Rust constructors; `workspace`
  preserves wrapper declarations. `ENV_FILE_KEYS` feeds override diagnostics
  and CLI reference rendering in `src/cli_args.rs`. None is dead registration.
- Root Cargo.toml enables file-write normally and test-support for tests;
  kache-client uses defaults. Public file editing remains an explicit feature/API
  contract even where only tests call its current implementation.
  `scripts/check-docs.sh` reads config.rs to validate schema/default docs;
  `scripts/crates-io.py` discovers publishable workspace packages. No custom
  package include/exclude, scoped build script or generated source entry point
  overrides the library manifest.
- Reviewed `git log -4 --oneline -- crates/kache-config`, plus
  `git log -S 'EnvOverrides::detect' -- src crates` and
  `git log -S 'cache_dir_for_path(' -- src crates`. History is supplementary,
  not proof that published exports can be removed.

Repository-wide searches covered Rust, manifests, docs, scripts and workflows,
with per-symbol searches within the crate. Representative commands:

```sh
rg -n 'kache-config|kache_config' --glob 'Cargo.toml' --glob '*.rs' --glob '*.sh' --glob '*.yml'
rg -n '\b(EnvOverrides|cache_dir_for_path|save_file_config_to)\b' --glob '*.rs' --glob '*.md' --glob '*.py' --glob '*.sh' --glob '*.toml'
rg -n 'allow\(|dead_code|unreachable_pub' crates/kache-config
cargo metadata --no-deps --format-version 1
```

## Reasoned allowlist and debloat dispositions

These are manual findings, not suppressed compiler/dependency findings.
There are **two** retained candidates (below the five-entry cap):

| Candidate | Evidence and disposition |
| --- | --- |
| `EnvOverrides` and `detect` | Searches find only the declaration/implementation and `config::tests::test_env_overrides_detect`. No production caller found. Public through `kache_config::config`; external consumer reachability is unknown. Deletion is an API walk-back, not a proven cut. Retain; DSLP-102 requests owner policy/approval. |
| `Config::cache_dir_for_path` | No repository call sites found; delegates to `volume_store_for`, which has local test callers. The wrapper uses `routed_for_path_for`, but that does not revoke this public API. Retain pending DSLP-102; external consumers were not checked. |

Debloat followed the tools. Its structural lenses produced no further safe cut:

- **Layers:** live-env wrappers and explicit `*_for` snapshots serve different
  process boundaries. Engine reexports are module boundaries, not a second
  resolver. Deleting public wrappers changes API.
- **Duplicated facts:** ENV_FILE_KEYS, size helpers and invocation normalization
  already have common owners. Test expected values are independent assertions.
- **Speculation/features:** file-write and test-support have declared consumers;
  trust domains, volume routing, bypass/exclusion and key-env settings have live
  loaders and tests. Removing opt-in behavior is a walk-back.
- **Machinery:** snapshots avoid daemon/global-env races; provenance/fingerprints
  drive reloads; layering preserves host defaults and atomic tables. Removing
  these loses current behavior.
- **Dependencies:** direct uses match machete; no dependency replacement is justified.
- **Tests:** snapshot/relative-path, parser, volume, host-layer and policy cases
  protect distinct behavior. Test-only path/host helpers are gated and called;
  process-state guards restore environment under a shared lock. Only owner-approved
  API retirement should delete associated tests. DSLP-4 owns test-quality changes.
- **Tooling/CI:** manifest features and client-graph, resource guard, lint and
  workspace gates have live entry points; none was removed or changed.
- **Comments/docs:** README describes the current resolver; no independently safe
  obsolete-documentation cut was established. Broad prose cleanup is out of scope.

No code was deleted/quarantined or temporarily removed; no failed removal was
reverted. Per-removal entry-point/platform/consumer evidence is consequently not
applicable. The only deferred decisions are the two exports in DSLP-102.

## Required consumer gate

Run from this independent worktree:

```sh
taskset -c 0-3 env -u TMPDIR -u KACHE_CACHE_DIR -u RUSTC_WRAPPER CARGO_BUILD_JOBS=4 make check
```

Includes `make lint`, client-crates-check (client/config/protocol/IPC plus IPC
tests), test-resource-guard-check, and workspace library/integration tests.
Cargo metadata resolves config to this worktree's manifest, not a released pin.
The scoped all-feature pass supplements the workspace feature graph. Gate exit 0;
the final workspace test tail was:

```text
test result: ok. 620 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 4.00s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
MAKE_CHECK_EXIT=0
```

The complete raw gate log is retained locally at
`/home/ubuntu/repos/kache/.work/dslp-5-make-check.log`.
