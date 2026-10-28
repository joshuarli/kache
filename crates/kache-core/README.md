# kache-core

[![crates.io](https://img.shields.io/crates/v/kache-core.svg)](https://crates.io/crates/kache-core)
[![docs.rs](https://img.shields.io/docsrs/kache-core)](https://docs.rs/kache-core)

Shared data types for [Kache](https://github.com/kunobi-ninja/kache), a compiler cache for Rust and C/C++.

## What it provides

- `store_config`: local storage options (`cache_dir`, size limit, shared-entry eviction, deferred durability). Compiler settings stay with the caller.
- `timeline`: one build session, in order, with every time in Unix epoch milliseconds. Records carry no filesystem paths and no environment values outside the allowlisted run context.

## Versioning

Released with Kache under the same version number. The minimum Rust version is 1.93.

## License

Apache-2.0
