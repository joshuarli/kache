//! On-disk content-addressed store for compiler probe results.
//!
//! Each [`ResolvedConfig`] is one small JSON file under
//! `<cache_dir>/probes/<key>.json`. The key is the compiler binary's
//! identity (see [`probe_key_for`]); writes are atomic (temp file +
//! rename) so concurrent `make -j` processes never observe a partial
//! file — a late writer simply wins.

use super::{PROBE_SCHEMA_VERSION, ProbeRequest, ResolvedConfig};
use crate::invocation::InvocationEnv;
use std::fs::Metadata;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Subdirectory of the kache cache dir that holds probe records.
const PROBE_SUBDIR: &str = "probes";

/// Content-addressed key for a probe, or `None` if the compiler cannot
/// be resolved / stat'd (the caller then probes uncached).
///
/// The key binds the record to the exact compiler binary: its resolved
/// absolute path plus a `stat` fingerprint. `ctime` sits alongside
/// `mtime` on purpose — a `cp -p` or restore-from-backup swap can
/// preserve `mtime` but always bumps `ctime`, so the pair catches a
/// compiler change `mtime` alone would miss. Deriving the key is a
/// `stat`, not a process fork.
#[cfg(test)]
pub fn probe_key(prober_id: &str, req: &ProbeRequest<'_>) -> Option<String> {
    probe_key_for(prober_id, req, &InvocationEnv::capture())
}

/// A probe belongs to the client's compiler selection, environment and cwd.
/// Keep the cwd even for absolute compiler paths: a driver wrapper can select
/// tools or read configuration relative to its current directory. Under a
/// declared base dir the cwd, and `PWD`, are keyed by their path below it:
/// declaring one asserts that the tree reads the same files wherever it is.
pub(crate) fn probe_key_for(
    prober_id: &str,
    req: &ProbeRequest<'_>,
    env: &InvocationEnv,
) -> Option<String> {
    probe_key_for_mode(prober_id, req, env, false)
}

pub(crate) fn native_probe_key_for(
    prober_id: &str,
    req: &ProbeRequest<'_>,
    env: &InvocationEnv,
) -> Option<String> {
    if prober_id != "cc" || !native_driver_file(req.compiler, env) {
        return None;
    }
    probe_key_for_mode(prober_id, req, env, true)
}

fn native_driver_file(program: &str, env: &InvocationEnv) -> bool {
    let Some(resolved) = env
        .resolve_program(Path::new(program))
        .and_then(|p| p.canonicalize().ok())
    else {
        return false;
    };
    for path in [Path::new(program), resolved.as_path()] {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        if ["ccache", "sccache", "distcc", "icecc", "kache"]
            .iter()
            .any(|launcher| name == *launcher || name.starts_with(&format!("{launcher}-")))
        {
            return false;
        }
    }
    let mut magic = [0u8; 4];
    if std::fs::File::open(resolved)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_err()
    {
        return false;
    }
    matches!(
        magic,
        [0x7f, b'E', b'L', b'F']
            | [0xfe, 0xed, 0xfa, 0xce]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0xca, 0xfe, 0xba, 0xbf]
            | [0xbf, 0xba, 0xfe, 0xca]
    )
}

fn probe_key_for_mode(
    prober_id: &str,
    req: &ProbeRequest<'_>,
    env: &InvocationEnv,
    native: bool,
) -> Option<String> {
    let resolved = env
        .resolve_program(Path::new(req.compiler))?
        .canonicalize()
        .ok()?;
    let cwd = under_base_dirs(
        fingerprint_text(env.cwd()?.as_os_str().to_owned()),
        req.base_dirs,
    );
    probe_key_from_parts_for_mode(
        prober_id,
        req,
        &resolved,
        env.vars_os()
            .map(fingerprint_env_pair)
            .map(|(name, value)| {
                let value = if name == "PWD" {
                    under_base_dirs(value, req.base_dirs)
                } else {
                    value
                };
                (name, value)
            }),
        Some(&cwd),
        native,
    )
}

/// `path` with the longest declared base dir containing it replaced by that
/// base's target, or `path` itself when none does.
fn under_base_dirs(path: String, base_dirs: &[(String, String)]) -> String {
    let below = |root: &str| {
        let root = root.trim_end_matches(['/', '\\']);
        let rest = path.strip_prefix(root)?;
        (!root.is_empty() && (rest.is_empty() || rest.starts_with(['/', '\\']))).then_some(rest)
    };
    base_dirs
        .iter()
        .filter_map(|(root, target)| Some((root.len(), target, below(root)?)))
        .max_by_key(|(len, _, _)| *len)
        .map(|(_, target, rest)| format!("{target}{rest}"))
        .unwrap_or(path)
}

/// A `vars_os` pair as fingerprint text. Converted here rather than via
/// `std::env::vars()`, which panics if *any* environment variable holds
/// non-UTF-8 — even one the fingerprint would ignore.
fn fingerprint_env_pair(
    (name, value): (std::ffi::OsString, std::ffi::OsString),
) -> (String, String) {
    (fingerprint_text(name), fingerprint_text(value))
}

/// Valid UTF-8 (the only kind the fingerprinted build tools produce)
/// passes through verbatim, so probe keys are unchanged. Non-UTF-8 text
/// becomes a U+FFFD-tagged hex dump of its lossless encoded bytes: unlike
/// `to_string_lossy`, distinct invalid sequences stay distinct and sort
/// deterministically. (A valid value that textually equals such a dump
/// could alias one — accepted: this is a probe memo key, and the worst
/// case of the old behavior was aborting the compile.)
fn fingerprint_text(text: std::ffi::OsString) -> String {
    match text.into_string() {
        Ok(utf8) => utf8,
        Err(os) => {
            use std::fmt::Write as _;
            let mut out = String::from('\u{fffd}');
            for byte in os.as_encoded_bytes() {
                let _ = write!(out, "{byte:02x}");
            }
            out
        }
    }
}

/// Content-addressed key for a probe evaluated against an explicit
/// environment. Split from [`probe_key`] so key stability and variance
/// are testable without racing against concurrent environment mutations.
#[cfg(test)]
pub(crate) fn probe_key_with_env(
    prober_id: &str,
    req: &ProbeRequest<'_>,
    env_vars: impl Iterator<Item = (String, String)>,
) -> Option<String> {
    let resolved = resolve_program(req.compiler)?;
    probe_key_from_parts(prober_id, req, &resolved, env_vars, None)
}

#[cfg(test)]
fn probe_key_from_parts(
    prober_id: &str,
    req: &ProbeRequest<'_>,
    resolved: &Path,
    env_vars: impl Iterator<Item = (String, String)>,
    cwd: Option<&str>,
) -> Option<String> {
    probe_key_from_parts_for_mode(prober_id, req, resolved, env_vars, cwd, false)
}

fn probe_key_from_parts_for_mode(
    prober_id: &str,
    req: &ProbeRequest<'_>,
    resolved: &Path,
    env_vars: impl Iterator<Item = (String, String)>,
    cwd: Option<&str>,
    native: bool,
) -> Option<String> {
    let meta = std::fs::metadata(resolved).ok()?;
    let fingerprint = compiler_fingerprint(&meta);
    let env_fp = fingerprint_env_for(env_vars, native, req.key_env_vars);

    let mut h = blake3::Hasher::new();
    h.update(b"probe_schema:");
    h.update(PROBE_SCHEMA_VERSION.to_string().as_bytes());
    h.update(if native {
        b"\nnative_env:"
    } else {
        b"\nopaque_env:"
    });
    for pattern in req.key_env_vars {
        h.update(&(pattern.len() as u64).to_le_bytes());
        h.update(pattern.as_bytes());
    }
    h.update(b"\nprober:");
    h.update(prober_id.as_bytes());
    h.update(b"\ncompiler_path:");
    h.update(resolved.to_string_lossy().as_bytes());
    h.update(b"\ncompiler_stat:");
    h.update(fingerprint.as_bytes());
    // The resolved-invocation probe (`cc -###`) depends on the compile
    // flags and the environment, so both join the key. Without this a
    // record memoized under one flag set or one `SDKROOT` could be
    // served to a build that used a different one.
    h.update(b"\nkey_args:");
    for arg in req.key_args {
        h.update(arg.as_bytes());
        h.update(b"\x1f");
    }
    h.update(b"\nenv:");
    h.update(env_fp.as_bytes());
    if let Some(cwd) = cwd {
        h.update(b"\ncwd:\n");
        h.update(cwd.as_bytes());
    }
    Some(h.finalize().to_hex().to_string())
}

/// Test adapter for the family probe's empty-argument request. The family
/// memo also includes the client's selectors: a driver wrapper can change its
/// reported family without its own executable changing.
#[cfg(test)]
pub fn probe_key_isolated(prober_id: &str, program: &str) -> Option<String> {
    probe_key_for(
        prober_id,
        &ProbeRequest {
            key_env_vars: &[],
            compiler: program,
            args: &[],
            key_args: &[],
            per_tu_paths: &[],
            windows_aware: true,
            base_dirs: &[],
        },
        &InvocationEnv::capture(),
    )
}

/// Hash every `VAR=value` pair in `vars`, sorted for determinism, after
/// dropping the volatile pseudo-variables [`is_volatile_env_name`]
/// rejects.
///
/// Opaque drivers retain the full environment; only verified native drivers
/// use the documented GCC/Clang inputs shared with the preprocessor memo.
#[cfg(test)]
fn fingerprint_env(vars: impl Iterator<Item = (String, String)>) -> String {
    fingerprint_env_for(vars, false, &[])
}

fn fingerprint_env_for(
    vars: impl Iterator<Item = (String, String)>,
    native: bool,
    patterns: &[String],
) -> String {
    let mut vars: Vec<(String, String)> = vars
        .filter(|(name, _)| {
            crate::cache_key::key_env_var_matches(patterns, name)
                || (!is_volatile_env_name(name)
                    && (!native
                        || crate::compiler::cc::cc_memo_env_is_keyed(std::ffi::OsStr::new(name))))
        })
        .collect();
    vars.sort();
    let mut h = blake3::Hasher::new();
    for (k, v) in vars {
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
        h.update(b"\n");
    }
    h.finalize().to_hex().to_string()
}

/// True for environment-variable names that vary between otherwise
/// identical build invocations or are kache-internal/test environment
/// controls and so must stay out of the probe key.
///
/// This excludes kache-internal variables (`KACHE_*`) which configure kache
/// or tests, Windows' hidden cmd.exe bookkeeping variables whose names begin
/// with `=` (`=C:`, `=ExitCode`, …), and the per-command shell and build
/// driver variables of [`crate::invocation::PER_COMMAND_ENV`]. `PWD` stays
/// keyed: clang can take its debug compilation directory from it.
fn is_volatile_env_name(name: &str) -> bool {
    name.starts_with('=')
        || name.starts_with("KACHE_")
        || crate::invocation::PER_COMMAND_ENV.contains(&name)
}

/// Load a probe record by key, or `None` on any miss: file absent,
/// unreadable, corrupt, or written under a different schema version.
/// A schema mismatch is deliberately a miss, never a wrong hit.
pub fn load(cache_dir: &Path, key: &str) -> Option<ResolvedConfig> {
    let bytes = std::fs::read(record_path(cache_dir, key)).ok()?;
    let config: ResolvedConfig = deser_json::from_slice(&bytes).ok()?;
    (config.schema_version == PROBE_SCHEMA_VERSION).then_some(config)
}

/// Write a probe record. Best-effort: any failure is swallowed — a
/// missed write just means the next process re-probes.
pub fn store(cache_dir: &Path, key: &str, config: &ResolvedConfig) {
    let dir = cache_dir.join(PROBE_SUBDIR);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(json) = deser_json::SerializerConfig::builder().pretty(deser_json::Indent::Spaces(2)).build().to_string(config).map(String::into_bytes) else {
        return;
    };
    // Atomic publish: write a temp file in the destination dir, then
    // rename it over the final name. Readers see either the old record
    // or the new one, never a half-written file.
    let Ok(mut tmp) = tempfile::NamedTempFile::new_in(&dir) else {
        return;
    };
    if tmp.write_all(&json).is_err() {
        return;
    }
    let _ = tmp.persist(record_path(cache_dir, key));
}

/// How long a probe record lives after it was written. A hit does not
/// refresh it, so a record still in use is probed again once this passes.
const PROBE_RECORD_TTL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Remove probe records, and temp files a killed writer left, written more
/// than [`PROBE_RECORD_TTL`] before `now`. The probe key covers the
/// environment, so a variable that changes per build writes one record per
/// build and nothing else bounds them. Returns how many files were removed.
pub(crate) fn prune(cache_dir: &Path, now: std::time::SystemTime) -> usize {
    let Ok(records) = std::fs::read_dir(cache_dir.join(PROBE_SUBDIR)) else {
        return 0;
    };
    records
        .flatten()
        .filter(|record| {
            record
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|written| now.duration_since(written).ok())
                .is_some_and(|age| age > PROBE_RECORD_TTL)
        })
        .filter(|record| std::fs::remove_file(record.path()).is_ok())
        .count()
}

fn record_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(PROBE_SUBDIR).join(format!("{key}.json"))
}

/// A `stat` fingerprint of the compiler binary: size and modification
/// time everywhere, plus change-time and inode on Unix. Any of these
/// shifting means "different binary" → different key → re-probe.
fn compiler_fingerprint(meta: &Metadata) -> String {
    let mut parts = vec![format!("len={}", meta.len())];
    if let Ok(mtime) = meta.modified()
        && let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH)
    {
        parts.push(format!("mtime={}", d.as_nanos()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        parts.push(format!("ctime={}.{}", meta.ctime(), meta.ctime_nsec()));
        parts.push(format!("ino={}", meta.ino()));
    }
    parts.join(",")
}

/// Resolve a program name to an absolute path for stat'ing.
///
/// A name containing a path separator is resolved directly (relative
/// to CWD if relative). A bare name (`cc`, `clang-17`) is searched on
/// `$PATH`, mirroring how the OS resolves it at exec time. Returns
/// `None` if nothing matches.
///
/// On Windows the executable carries a `.exe` suffix the fixture's
/// compiler name (`cc`) omits, so each candidate is tried both as
/// written and with [`EXE_SUFFIX`](std::env::consts::EXE_SUFFIX)
/// appended. Without this the PATH search never matched `cc.exe`, so
/// [`probe_key`] returned `None` and the probe was never memoized —
/// re-running on every build (the #201 warm `probe_runs=1`). No-op on
/// Unix where `EXE_SUFFIX` is empty.
#[cfg(test)]
fn resolve_program(program: &str) -> Option<PathBuf> {
    let p = Path::new(program);
    let has_separator = p.parent().is_some_and(|par| !par.as_os_str().is_empty());
    if has_separator {
        return with_exe_suffix(p.to_path_buf(), std::env::consts::EXE_SUFFIX)
            .into_iter()
            .find_map(|candidate| candidate.canonicalize().ok());
    }
    let path = std::env::var_os("PATH")?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    find_program_in_dirs(program, &dirs, std::env::consts::EXE_SUFFIX)
        .and_then(|candidate| candidate.canonicalize().ok())
}

/// `path` followed by `path` + `exe_suffix` (when the suffix is non-empty
/// and not already present), in priority order. Pure — used by both the
/// PATH search and the explicit-path branch of [`resolve_program`].
#[cfg(test)]
fn with_exe_suffix(path: PathBuf, exe_suffix: &str) -> Vec<PathBuf> {
    let already = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext == exe_suffix.trim_start_matches('.'));
    if exe_suffix.is_empty() || already {
        return vec![path];
    }
    let mut suffixed = path.clone().into_os_string();
    suffixed.push(exe_suffix);
    vec![path, PathBuf::from(suffixed)]
}

/// First existing file matching `program` (then `program+exe_suffix`) in
/// any of `dirs`, in PATH order. Split out so the suffix logic is
/// testable on any host.
#[cfg(test)]
fn find_program_in_dirs(program: &str, dirs: &[PathBuf], exe_suffix: &str) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        with_exe_suffix(dir.join(program), exe_suffix)
            .into_iter()
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile, TempDir};

    fn sample(version: &str) -> ResolvedConfig {
        ResolvedConfig {
            native_driver: false,
            schema_version: PROBE_SCHEMA_VERSION,
            prober: "cc".to_string(),
            compiler_name: "clang".to_string(),
            version_line: version.to_string(),
            resolved_tokens: None,
            host_version_line: None,
        }
    }

    #[test]
    fn native_probe_requires_binary_magic_and_excludes_launchers() {
        let root = TempDir::new().unwrap();
        let driver = root.path().join("driver");
        let env = InvocationEnv::from_parts(
            std::iter::empty::<(&str, &str)>(),
            Some(root.path().into()),
            None,
            root.path().into(),
        );
        for magic in [
            [0x7f, b'E', b'L', b'F'],
            [0xfe, 0xed, 0xfa, 0xce],
            [0xce, 0xfa, 0xed, 0xfe],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xca, 0xfe, 0xba, 0xbe],
            [0xbe, 0xba, 0xfe, 0xca],
            [0xca, 0xfe, 0xba, 0xbf],
            [0xbf, 0xba, 0xfe, 0xca],
        ] {
            std::fs::write(&driver, magic).unwrap();
            assert!(
                native_driver_file(driver.to_str().unwrap(), &env),
                "{magic:?}"
            );
        }
        for body in [
            b"".as_slice(),
            b"ELF",
            b"MZnative",
            b"#!/bin/sh\n",
            b"unknown",
        ] {
            std::fs::write(&driver, body).unwrap();
            assert!(
                !native_driver_file(driver.to_str().unwrap(), &env),
                "{body:?}"
            );
        }
        for name in [
            "ccache",
            "sccache",
            "distcc",
            "icecc",
            "kache",
            "kache-engine",
            "ccache-4",
        ] {
            let launcher = root.path().join(name);
            std::fs::write(&launcher, b"\x7fELFnative").unwrap();
            assert!(
                !native_driver_file(launcher.to_str().unwrap(), &env),
                "{name}"
            );
            #[cfg(unix)]
            {
                let alias = root.path().join(format!("alias-{name}"));
                std::os::unix::fs::symlink(&launcher, &alias).unwrap();
                assert!(
                    !native_driver_file(alias.to_str().unwrap(), &env),
                    "alias {name}"
                );
            }
        }
        std::fs::remove_file(&driver).unwrap();
        assert!(!native_driver_file(driver.to_str().unwrap(), &env));
    }

    /// A `ProbeRequest` for a compiler with no extra arguments.
    fn req(compiler: &str) -> ProbeRequest<'_> {
        ProbeRequest {
            key_env_vars: &[],
            compiler,
            args: &[],
            key_args: &[],
            per_tu_paths: &[],
            windows_aware: true,
            base_dirs: &[],
        }
    }

    #[test]
    fn a_tree_moved_with_its_base_dir_reuses_the_probe() {
        let root = TempDir::new().unwrap();
        let compiler = NamedTempFile::new().unwrap();
        let at = |tree: &str, below: &str| {
            let cwd = root.path().join(tree).join(below);
            let env = InvocationEnv::from_parts(
                [("PWD", cwd.to_str().unwrap())],
                Some(cwd.clone()),
                None,
                root.path().join("tmp"),
            );
            let base = vec![(
                root.path().join(tree).display().to_string(),
                "/kache/base-dir".to_string(),
            )];
            (env, base)
        };
        let key = |(env, base): (InvocationEnv, Vec<(String, String)>)| {
            let request = ProbeRequest {
                key_env_vars: &[],
                base_dirs: &base,
                ..req(compiler.path().to_str().unwrap())
            };
            probe_key_for("cc", &request, &env).unwrap()
        };
        assert_eq!(key(at("first", "src")), key(at("second", "src")));
        assert_ne!(key(at("first", "src")), key(at("first", "lib")));
        let (env, _) = at("first", "src");
        assert_ne!(key((env, Vec::new())), key(at("first", "src")));
    }

    #[test]
    fn a_base_dir_holds_only_paths_below_it() {
        let base = [("/r/a/".to_string(), "/kache/base-dir".to_string())];
        assert_eq!(under_base_dirs("/r/a".into(), &base), "/kache/base-dir");
        assert_eq!(under_base_dirs("/r/a/src".into(), &base), "/kache/base-dir/src");
        assert_eq!(under_base_dirs("/r/ab/src".into(), &base), "/r/ab/src");
        let nested = [
            ("/r/a".to_string(), "/kache/base-dir".to_string()),
            ("/r/a/src".to_string(), "/kache/base-dir-0".to_string()),
        ];
        assert_eq!(under_base_dirs("/r/a/src/x".into(), &nested), "/kache/base-dir-0/x");
    }

    #[test]
    fn client_probe_keys_include_snapshot_env_and_cwd() {
        let root = TempDir::new().unwrap();
        let compiler = NamedTempFile::new().unwrap();
        let request = req(compiler.path().to_str().unwrap());
        let first = InvocationEnv::from_parts(
            [("SDKROOT", "/sdk/first")],
            Some(root.path().join("first-project")),
            None,
            root.path().join("tmp"),
        );
        let other_cwd = InvocationEnv::from_parts(
            [("SDKROOT", "/sdk/first")],
            Some(root.path().join("second-project")),
            None,
            root.path().join("tmp"),
        );
        let mut other_env = first.clone();
        other_env.set("SDKROOT", "/sdk/second");
        let key = probe_key_for("cc", &request, &first).unwrap();
        assert_eq!(probe_key_for("cc", &request, &first), Some(key.clone()));
        assert_ne!(probe_key_for("cc", &request, &other_cwd), Some(key.clone()));
        assert_ne!(probe_key_for("cc", &request, &other_env), Some(key));
    }

    #[test]
    fn fingerprint_env_pair_passes_utf8_verbatim_and_keeps_invalid_distinct() {
        use std::ffi::OsString;
        assert_eq!(
            fingerprint_env_pair((OsString::from("SDKROOT"), OsString::from("/sdk"))),
            ("SDKROOT".to_string(), "/sdk".to_string())
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            // The shape that made `std::env::vars()` panic mid-key. Two
            // different invalid values must stay two different
            // fingerprints (lossy conversion would merge them).
            let a = fingerprint_env_pair((
                OsString::from_vec(vec![b'x', 0xff]),
                OsString::from_vec(vec![b'x', 0xff]),
            ));
            let b = fingerprint_env_pair((
                OsString::from_vec(vec![b'x', 0xfe]),
                OsString::from_vec(vec![b'x', 0xfe]),
            ));
            assert_ne!(a, b);
            assert!(a.0.starts_with('\u{fffd}'));
            assert_eq!(a.1, "\u{fffd}78ff");
        }
    }

    #[test]
    fn fingerprint_env_ignores_windows_hidden_pseudo_vars() {
        // The #201 root cause: on Windows `std::env::vars()` yields
        // cmd.exe's volatile `=`-prefixed vars (per-drive CWD `=C:`, last
        // child's `=ExitCode`). They differ between the cold and warm
        // builds, so including them made the probe key — and thus the
        // memo — unstable. Adding them must not change the fingerprint.
        let real = [
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("SDKROOT".to_string(), "/sdk".to_string()),
        ];
        let with_hidden = [
            ("PATH".to_string(), "/usr/bin".to_string()),
            ("SDKROOT".to_string(), "/sdk".to_string()),
            ("=C:".to_string(), r"C:\work\cold".to_string()),
            ("=ExitCode".to_string(), "00000000".to_string()),
        ];
        assert_eq!(
            fingerprint_env(real.iter().cloned()),
            fingerprint_env(with_hidden.iter().cloned()),
            "`=`-prefixed Windows pseudo-vars must not affect the probe key"
        );
    }

    #[test]
    fn fingerprint_env_still_reflects_real_vars() {
        // The filter must not blunt the key: a genuine env change (a
        // different SDKROOT, which `cc -###` honors) must still move it.
        let a = [("SDKROOT".to_string(), "/sdk/a".to_string())];
        let b = [("SDKROOT".to_string(), "/sdk/b".to_string())];
        assert_ne!(
            fingerprint_env(a.iter().cloned()),
            fingerprint_env(b.iter().cloned()),
            "a real env change must still change the fingerprint"
        );
    }

    #[test]
    fn is_volatile_env_name_flags_equals_and_kache_internal_vars() {
        assert!(is_volatile_env_name("=C:"));
        assert!(is_volatile_env_name("=ExitCode"));
        assert!(is_volatile_env_name("KACHE_ACTIVE"));
        assert!(is_volatile_env_name("KACHE_FAMILY_PROBE_ACTIVE"));
        assert!(is_volatile_env_name("KACHE_CACHE_DIR"));
        assert!(is_volatile_env_name("KACHE_CONFIG"));
        assert!(is_volatile_env_name("MAKEFLAGS"));
        assert!(is_volatile_env_name("SHLVL"));
        assert!(!is_volatile_env_name("PWD"));
        assert!(!is_volatile_env_name("PATH"));
        assert!(!is_volatile_env_name("SDKROOT"));
        assert!(!is_volatile_env_name("A=B"));
    }

    #[test]
    fn a_new_make_run_reuses_the_probe() {
        let compiler = NamedTempFile::new().unwrap();
        let request = req(compiler.path().to_str().unwrap());
        let run = |fifo: &str, level: &str| {
            [
                ("PATH".to_string(), "/bin".to_string()),
                ("MAKEFLAGS".to_string(), format!(" -j8 --jobserver-auth=fifo:{fifo}")),
                ("MAKELEVEL".to_string(), level.to_string()),
                ("SHLVL".to_string(), level.to_string()),
            ]
        };
        let first = probe_key_with_env("cc", &request, run("/tmp/GMfifo1", "1").into_iter());
        let second = probe_key_with_env("cc", &request, run("/tmp/GMfifo2", "2").into_iter());
        assert_eq!(first, second);
        assert!(first.is_some());
    }

    #[test]
    fn probe_key_is_stable_for_the_same_binary() {
        let compiler = NamedTempFile::new().unwrap();
        let request = req(compiler.path().to_str().unwrap());
        let env = [("PATH".to_string(), "/bin".to_string())];
        let k1 = probe_key_with_env("cc", &request, env.iter().cloned()).unwrap();
        let k2 = probe_key_with_env("cc", &request, env.iter().cloned()).unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn probe_key_changes_when_the_binary_changes() {
        let mut compiler = NamedTempFile::new().unwrap();
        let path = compiler.path().to_path_buf();
        let path_str = path.to_str().unwrap();
        let env = [("PATH".to_string(), "/bin".to_string())];

        let k1 = probe_key_with_env("cc", &req(path_str), env.iter().cloned()).unwrap();

        // Rewrite the "binary": size (and mtime/ctime) all move.
        compiler.write_all(b"new compiler bytes").unwrap();
        compiler.flush().unwrap();

        let k2 = probe_key_with_env("cc", &req(path_str), env.iter().cloned()).unwrap();
        assert_ne!(k1, k2, "a changed compiler binary must change the key");
    }

    #[test]
    fn probe_key_is_none_for_a_missing_compiler() {
        assert!(probe_key("cc", &req("/nonexistent/kache-cc-xyz")).is_none());
    }

    #[test]
    fn probe_key_isolated_returns_valid_hash_and_bails_on_missing() {
        let compiler = NamedTempFile::new().unwrap();
        let path_str = compiler.path().to_str().unwrap();

        let k1 = probe_key_isolated("cc-family", path_str).unwrap();
        assert_eq!(k1.len(), 64, "Blake3 hex digest must be 64 characters");

        let k2 = probe_key_isolated("cc-family", path_str).unwrap();
        assert_eq!(k1, k2);

        let k_other = probe_key_isolated("other-id", path_str).unwrap();
        assert_ne!(k1, k_other, "different prober_id must yield different key");

        assert!(probe_key_isolated("cc-family", "/nonexistent/kache-cc-xyz").is_none());
    }

    #[test]
    fn with_exe_suffix_appends_when_set_and_absent() {
        assert_eq!(
            with_exe_suffix(PathBuf::from("/p/cc"), ".exe"),
            vec![PathBuf::from("/p/cc"), PathBuf::from("/p/cc.exe")]
        );
    }

    #[test]
    fn with_exe_suffix_is_single_for_empty_suffix_or_already_present() {
        assert_eq!(
            with_exe_suffix(PathBuf::from("/p/cc"), ""),
            vec![PathBuf::from("/p/cc")]
        );
        assert_eq!(
            with_exe_suffix(PathBuf::from("/p/cc.exe"), ".exe"),
            vec![PathBuf::from("/p/cc.exe")]
        );
    }

    #[test]
    fn find_program_in_dirs_matches_suffixed_executable() {
        // The #201 shape: PATH holds `cc.exe`, fixture names `cc`. With a
        // `.exe` suffix the search must find it; without (Unix), it must not.
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("cc.exe"), b"MZ").unwrap();
        let dirs = [dir.path().to_path_buf()];
        assert_eq!(
            find_program_in_dirs("cc", &dirs, ".exe"),
            Some(dir.path().join("cc.exe"))
        );
        assert!(find_program_in_dirs("cc", &dirs, "").is_none());
    }

    #[test]
    fn find_program_in_dirs_prefers_unsuffixed_when_present() {
        // A bare on-disk `cc` (the Unix shape) is found as-is, before any
        // suffixed candidate.
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("cc"), b"#!/bin/sh\n").unwrap();
        let dirs = [dir.path().to_path_buf()];
        assert_eq!(
            find_program_in_dirs("cc", &dirs, ".exe"),
            Some(dir.path().join("cc"))
        );
    }

    #[test]
    fn probe_key_changes_when_key_args_change() {
        let compiler = NamedTempFile::new().unwrap();
        let path = compiler.path().to_str().unwrap();
        let env = [("PATH".to_string(), "/bin".to_string())];
        let plain = probe_key_with_env("cc", &req(path), env.iter().cloned()).unwrap();
        let flags = ["-O2".to_string()];
        let with_flags = probe_key_with_env(
            "cc",
            &ProbeRequest {
                key_env_vars: &[],
                compiler: path,
                args: &[],
                key_args: &flags,
                per_tu_paths: &[],
                windows_aware: true,
                base_dirs: &[],
            },
            env.iter().cloned(),
        )
        .unwrap();
        assert_ne!(plain, with_flags, "key_args must be part of the probe key");
    }

    #[test]
    fn probe_key_differs_per_prober_id() {
        let compiler = NamedTempFile::new().unwrap();
        let path_str = compiler.path().to_str().unwrap();
        let env = [("PATH".to_string(), "/bin".to_string())];
        let a = probe_key_with_env("cc", &req(path_str), env.iter().cloned()).unwrap();
        let b = probe_key_with_env("rustc", &req(path_str), env.iter().cloned()).unwrap();
        assert_ne!(a, b, "prober id must be part of the key");
    }

    #[test]
    fn probe_key_changes_when_env_changes() {
        let compiler = NamedTempFile::new().unwrap();
        let path_str = compiler.path().to_str().unwrap();
        let a = probe_key_with_env(
            "cc",
            &req(path_str),
            [("SDKROOT".to_string(), "/sdk1".to_string())].into_iter(),
        )
        .unwrap();
        let b = probe_key_with_env(
            "cc",
            &req(path_str),
            [("SDKROOT".to_string(), "/sdk2".to_string())].into_iter(),
        )
        .unwrap();
        assert_ne!(a, b, "a real env change must change the probe key");
    }

    #[test]
    fn store_then_load_roundtrips() {
        let cache = TempDir::new().unwrap();
        let cfg = sample("clang 17.0.0");
        store(cache.path(), "deadbeef", &cfg);
        assert_eq!(load(cache.path(), "deadbeef"), Some(cfg));
    }

    #[test]
    fn store_creates_the_probe_subdir() {
        let cache = TempDir::new().unwrap();
        // The cache dir exists but `probes/` does not yet.
        store(cache.path(), "k", &sample("gcc 13"));
        assert!(cache.path().join(PROBE_SUBDIR).join("k.json").is_file());
    }

    #[test]
    fn prune_removes_only_records_written_before_the_ttl() {
        let cache = tempfile::tempdir().unwrap();
        store(cache.path(), "old", &sample("old"));
        store(cache.path(), "fresh", &sample("fresh"));
        let now = std::time::SystemTime::now();
        let written = |age: u64| {
            filetime::FileTime::from_system_time(now - std::time::Duration::from_secs(age))
        };
        let week: u64 = 604_800;
        filetime::set_file_mtime(record_path(cache.path(), "old"), written(week + 60)).unwrap();
        filetime::set_file_mtime(record_path(cache.path(), "fresh"), written(week - 60)).unwrap();
        let abandoned = cache.path().join(PROBE_SUBDIR).join(".tmpAbCdEf");
        std::fs::write(&abandoned, b"{").unwrap();
        filetime::set_file_mtime(&abandoned, written(week + 60)).unwrap();

        assert_eq!(prune(cache.path(), now), 2);
        assert!(load(cache.path(), "old").is_none());
        assert!(!abandoned.exists());
        assert_eq!(load(cache.path(), "fresh"), Some(sample("fresh")));
        assert_eq!(prune(cache.path(), now), 0);
    }

    #[test]
    fn prune_without_a_probe_dir_removes_nothing() {
        let cache = tempfile::tempdir().unwrap();
        assert_eq!(prune(cache.path(), std::time::SystemTime::now()), 0);
    }

    #[test]
    fn load_is_none_when_record_absent() {
        let cache = TempDir::new().unwrap();
        assert_eq!(load(cache.path(), "missing"), None);
    }

    #[test]
    fn load_is_none_for_a_corrupt_record() {
        let cache = TempDir::new().unwrap();
        let dir = cache.path().join(PROBE_SUBDIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.json"), b"{ not json").unwrap();
        assert_eq!(load(cache.path(), "bad"), None);
    }

    #[test]
    fn load_is_none_on_schema_mismatch() {
        let cache = TempDir::new().unwrap();
        let dir = cache.path().join(PROBE_SUBDIR);
        std::fs::create_dir_all(&dir).unwrap();
        // A record written under a future schema must read as a miss.
        let mut cfg = sample("clang 17");
        cfg.schema_version = PROBE_SCHEMA_VERSION + 1;
        std::fs::write(dir.join("future.json"), deser_json::to_string(&cfg).map(String::into_bytes).unwrap()).unwrap();
        assert_eq!(load(cache.path(), "future"), None);
    }
}
