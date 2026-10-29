[![CI](https://github.com/kunobi-ninja/kache/actions/workflows/ci.yml/badge.svg)](https://github.com/kunobi-ninja/kache/actions/workflows/ci.yml)
[![Bench](https://github.com/kunobi-ninja/kache/actions/workflows/bench.yml/badge.svg)](https://github.com/kunobi-ninja/kache/actions/workflows/bench.yml)
[![Crates.io](https://img.shields.io/crates/v/kache.svg)](https://crates.io/crates/kache)
[![Documentation](https://img.shields.io/badge/docs-kunobi.ninja-blue)][docs-badge]

# Kache

**Build once. Reuse the work.**

Kache is a compiler cache for Rust and C/C++ on Linux and macOS.
It restores matching compiler outputs across worktrees and CI runs, so you spend
less time rebuilding and less disk space storing the same bytes.

Built by [Kunobi][kunobi-brand].

[Get started][first-reuse] · [Benchmarks][nav-benchmarks] · [Kache vs sccache][nav-comparison]

## Install

```bash
cargo install kache --locked
kache init
```

`kache init` shows the changes before applying them. It configures Cargo's compiler
wrapper and offers a background service. On Unix it also sets up C/C++ compiler
shims. Run `kache init --check` for a preview.

Then use Cargo as usual in your project. Kache caches eligible compiler work
automatically.

Cargo installation needs Rust 1.95 or newer. Prefer a prebuilt package?
[Homebrew, APT, AUR, mise, and Nix →](https://kunobi.ninja/docs/kache/getting-started/installation)

## See it reuse a build

Build the same revision in two worktrees with separate target directories.
The second can restore compatible outputs from the first.
[Try the walkthrough][first-reuse].

A new cache needs a build to fill it. Reuse requires matching inputs, toolchain,
and build settings. Unsupported compiler invocations run normally.
[What can be cached →](https://kunobi.ninja/docs/kache#current-support)

## Why use Kache?

- Reuse eligible Rust libraries, executables, and build scripts across worktrees.
  It also caches GCC, Clang, and clang-cl.
- Keep identical output bytes once in a cache with a disk budget. On filesystems
  with copy-on-write cloning, restored outputs share disk blocks with the cache.
- Run builds together. Compiles with the same cache key share one in-flight
  build, while the scheduler paces compiler processes.
- Work fully offline. The cache lives on the local machine; nothing is uploaded.
- See what happened. Inspect hits and misses, then ask `kache explain` what changed.

Rust executables and build-script runs are cached by default.
See [platform support and caching limits](https://kunobi.ninja/docs/kache#current-support)
for workload details.

## Less disk per worktree

In a Firefox 151 benchmark with Kache 0.7.0 on macOS/APFS, the second worktree
added about 3 GB of new data. [Read the measurements and methodology →][storage-report]

Nightly workflows measure cold and warm builds of real projects, including
Firefox, LLVM, and SurrealDB. [Browse the benchmark reports][benchmark-guide]
to see the workload, platform, and validation behind each result.

## Watch a build and explain a miss

```bash
kache stats --last-build  # latest recorded build session
kache explain             # why the latest build missed
kache explain --all       # all recorded skips and passthroughs, by reason
kache doctor              # check your setup
```

Reports describe recorded builds. `Details missing` means those builds did
not record enough information to explain a miss. Enable detailed recording
before the next build with `kache config set cache.explain_miss true`;
this cannot reconstruct details from earlier builds.

## Add it to CI

Install your Rust toolchain, then add the official action before your build:

```yaml
- uses: kunobi-ninja/kache-action@v1
- run: cargo build --locked
```

The action can persist the local store through GitHub's cache service.

## Star History

<a href="https://www.star-history.com/?repos=kunobi-ninja%2Fkache&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=kunobi-ninja/kache&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=kunobi-ninja/kache&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=kunobi-ninja/kache&type=date&legend=top-left" />
 </picture>
</a>


## Find your next step

| I want to… | Guide |
| --- | --- |
| Confirm my first cache hits | [Quick start][first-reuse] |
| Cache Make or CMake builds | [C/C++](https://kunobi.ninja/docs/kache/getting-started/c-cpp) |
| Change storage limits or build policy | [Configuration](https://kunobi.ninja/docs/kache/getting-started/configuration) |
| Investigate unexpected misses | [Troubleshooting](https://kunobi.ninja/docs/kache/getting-started/troubleshooting) |
| Understand cache keys and correctness | [How it works](https://kunobi.ninja/docs/kache/how-it-works/architecture) |
| Look up a command or flag | [Command reference](https://kunobi.ninja/docs/kache/commands/reference) |

## Help improve Kache

[Report a bug](https://github.com/kunobi-ninja/kache/issues/new?template=bug_report.md),
[request a feature](https://github.com/kunobi-ninja/kache/issues/new?template=feature_request.md),
or read [CONTRIBUTING.md](.github/CONTRIBUTING.md) to contribute.

Licensed under [Apache 2.0](LICENSE).

[docs-badge]: https://kunobi.ninja/docs/kache?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=docs_badge
[kunobi-brand]: https://kunobi.ninja/?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=brand
[nav-benchmarks]: https://kunobi.ninja/docs/kache/benchmarks?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=nav_benchmarks
[nav-comparison]: https://kunobi.ninja/docs/kache/getting-started/comparison?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=nav_comparison
[hero-image]: https://kunobi.ninja/product/kache?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=hero_image
[storage-chart]: https://kunobi.ninja/blog/kache-storage-worktrees?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=storage_chart
[storage-report]: https://kunobi.ninja/blog/kache-storage-worktrees?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=storage_report
[benchmark-guide]: https://kunobi.ninja/docs/kache/benchmarks?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=benchmark_guide
[first-reuse]: https://kunobi.ninja/docs/kache/getting-started/quick-start?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=first_reuse
[kunobi-desktop]: https://kunobi.ninja/product/desktop?utm_source=github&utm_medium=readme&utm_campaign=kache&utm_content=desktop
