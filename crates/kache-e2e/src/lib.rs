//! End-to-end test harness for kache.
//!
//! Drives example projects through a fixed lifecycle (cold → warm → noop) and
//! asserts per-fixture contracts against kache's own JSON report. The harness
//! is the single source of truth for "kache works end-to-end" coverage; the
//! per-language bash scripts it replaces shipped per-language assertions in
//! shell, which made the contract opaque and impossible to extend without
//! growing more scripts.
//!
//! Each fixture scenario is self-contained: `scenario.toml` declares source,
//! env, commands, verification, tags, and per-phase assertions. The harness
//! owns the lifecycle; the TOML owns what the fixture is and what it expects.

pub mod assertions;
mod bench_host;
mod bench_otlp;
pub mod bench_profile;
pub mod bench_runner;
pub mod daemon;
mod disk_usage;
pub mod fixture;
pub mod fixture_runner;
pub mod phase;
pub mod report;
pub mod result;
pub mod runner;
pub mod scenario;
pub mod scenario_runner;
pub mod source;

use std::path::{Path, PathBuf};

/// Make a canonicalized path safe to embed in the shell commands the harness
/// drives (fixture `make` recipes via `/usr/bin/sh`) and to invoke directly
/// as a program.
pub fn portable_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// Re-exec the sibling `kache` binary as `<kache> <family> <forwarded-args>`.
/// Never returns — exits with the child's status.
///
/// This backs the `kache-cc` / `kache-cxx` shim binaries. A fixture whose
/// `build.rs` uses the `cc` crate (rust-c-ffi) routes its C/C++ compile
/// through `$KACHE cc`.
///
/// `kache` is located as a sibling of this binary (cargo builds the harness
/// bins and `kache` into the same `target/<profile>/` dir), so no env or
/// path plumbing is needed and it survives a fixture relocation.
pub fn run_kache_compiler_shim(family: &str) -> ! {
    let kache = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.to_path_buf()))
        .map(|dir| dir.join(format!("kache{}", std::env::consts::EXE_SUFFIX)));
    let Some(kache) = kache.filter(|p| p.exists()) else {
        eprintln!("kache-{family} shim: could not locate sibling `kache` binary");
        std::process::exit(2);
    };
    let forwarded: Vec<String> = std::env::args().skip(1).collect();
    let status = std::process::Command::new(&kache)
        .arg(family)
        .args(&forwarded)
        .status();
    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!(
                "kache-{family} shim: failed to run `{} {family} …`: {e}",
                kache.display()
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::portable_path;
    use std::path::Path;

    #[test]
    fn leaves_unix_style_paths_untouched() {
        assert_eq!(
            portable_path(Path::new("/usr/local/bin/kache")),
            Path::new("/usr/local/bin/kache")
        );
    }
}
