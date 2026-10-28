//! Shared bootstrap for the integration tests that drive the real `kache`
//! binary as a compiler wrapper.
//!
//! These tests can't use `CARGO_BIN_EXE_kache`: the binary under test has to be
//! built with no configured `rustc-wrapper`, so a developer dogfooding kache
//! (or a broken wrapper being debugged) can't influence it. That means a second
//! cargo build into its own target dir.
//!
//! That target dir is ~2 GB, so it is **one stable directory shared by every
//! test binary**, not one per process (#599). Keying it on the PID meant every
//! `cargo test` run left another 2 GB behind per suite, which filled a 926 GB
//! disk twice; the resulting ENOSPC then surfaced as `kache build failed` and
//! `Once instance has previously been poisoned`, reading like a cache-key
//! regression rather than an infrastructure problem.
//!
//! It lives under the workspace target dir so `cargo clean` reclaims it, it is
//! already gitignored, and concurrent suites reuse the same build instead of
//! each doing their own (cargo's build-dir lock serializes them).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// `<target>/kache-test-bootstrap`, derived from the running test binary.
///
/// Cargo 1.99 and earlier place it at `<target>/<profile>/deps/<test-bin>`.
/// Cargo 1.100 places it at `<target>/<profile>/build/<pkg>/<hash>/out/<test-bin>`.
/// The profile directory is the nearest ancestor that holds `deps` or `build`.
fn bootstrap_target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let profile = exe
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("deps").is_dir() || dir.join("build").is_dir())
        .expect("test binary is not under a Cargo profile directory");
    profile
        .parent()
        .expect("Cargo profile directory has a parent")
        .join("kache-test-bootstrap")
}

#[allow(dead_code)]
/// Files and `.dSYM` bundles Cargo wrote for units in `profile`
/// (`<target>/debug`): legacy `deps` and `examples`, each per-unit `out`,
/// and bundles sitting next to an uplifted binary.
pub fn profile_artifacts(profile: &Path) -> Vec<PathBuf> {
    let mut artifacts = Vec::new();
    push_artifacts(profile, &mut artifacts);
    for name in ["deps", "examples"] {
        push_artifacts(&profile.join(name), &mut artifacts);
    }
    let Ok(packages) = std::fs::read_dir(profile.join("build")) else {
        return artifacts;
    };
    for package in packages.flatten() {
        let path = package.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };
        for child in children.flatten() {
            let name = child.file_name();
            let name = name.to_string_lossy();
            if is_cargo_unit_hash(&name) {
                push_artifacts(&child.path().join("out"), &mut artifacts);
            }
        }
    }
    artifacts
}

#[allow(dead_code)]
fn is_cargo_unit_hash(hash: &str) -> bool {
    hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether `profile` holds Cargo fingerprints in either layout.
#[allow(dead_code)]
pub fn profile_has_fingerprints(profile: &Path) -> bool {
    !fingerprint_dirs(profile).is_empty()
}

/// Drop `package`'s fingerprints so the next build cannot be Fresh.
#[allow(dead_code)]
pub fn clear_package_fingerprints(profile: &Path, package: &str) {
    let legacy = profile.join(".fingerprint");
    if let Ok(entries) = std::fs::read_dir(&legacy) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{package}-"))
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let package_dir = profile.join("build").join(package);
    let Ok(units) = std::fs::read_dir(&package_dir) else {
        return;
    };
    for unit in units.flatten() {
        let _ = std::fs::remove_dir_all(unit.path().join("fingerprint"));
    }
}

/// Every file inside a fingerprint directory under `profile`.
#[allow(dead_code)]
pub fn fingerprint_files(profile: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in fingerprint_dirs(profile) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_file() {
                files.push(entry.path());
            }
        }
    }
    files
}

#[allow(dead_code)]
fn fingerprint_dirs(profile: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(profile.join(".fingerprint")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                dirs.push(entry.path());
            }
        }
    }
    let Ok(packages) = std::fs::read_dir(profile.join("build")) else {
        return dirs;
    };
    for package in packages.flatten() {
        if !package.path().is_dir() {
            continue;
        }
        let Ok(units) = std::fs::read_dir(package.path()) else {
            continue;
        };
        for unit in units.flatten() {
            let fingerprint = unit.path().join("fingerprint");
            if fingerprint.is_dir() {
                dirs.push(fingerprint);
            }
        }
    }
    dirs
}

/// Every unit `out` directory under `profile`, legacy `<pkg>-<hash>/out` and
/// per-unit `<pkg>/<hash>/out`.
#[allow(dead_code)]
pub fn unit_out_dirs(profile: &Path) -> Vec<PathBuf> {
    let mut outs = Vec::new();
    let Ok(entries) = std::fs::read_dir(profile.join("build")) else {
        return outs;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let direct = path.join("out");
        if direct.is_dir() {
            outs.push(direct);
        }
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };
        for child in children.flatten() {
            if is_cargo_unit_hash(&child.file_name().to_string_lossy()) {
                let nested = child.path().join("out");
                if nested.is_dir() {
                    outs.push(nested);
                }
            }
        }
    }
    outs
}

#[allow(dead_code)]
fn push_artifacts(dir: &Path, artifacts: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let bundle = path.extension().is_some_and(|ext| ext == "dSYM");
        if path.is_file() || bundle {
            artifacts.push(path);
        }
    }
}

/// `KACHE_TEST_USE_CARGO_BIN_EXE=1` skips the bootstrap build and uses the
/// `kache` binary the outer cargo already built for these tests. Only a caller
/// that knows the outer build ran with no `rustc-wrapper` may set it (CI does,
/// with `RUSTC_WRAPPER=""`); the second build is otherwise the guarantee.
fn reuse_outer_build() -> bool {
    std::env::var_os("KACHE_TEST_USE_CARGO_BIN_EXE").is_some_and(|value| value == "1")
}

/// The bootstrap result, shared by every test in this binary.
///
/// Holds `Err(message)` rather than panicking inside the initializer: a panic
/// there leaves the lock poisoned, so every later test reports "previously
/// poisoned" instead of the real cause. Here each test gets the actual cargo
/// failure — including its stderr, which is where `No space left on device`
/// shows up.
static KACHE_BIN: OnceLock<Result<PathBuf, String>> = OnceLock::new();

/// Builds the `kache` binary under test if it isn't built yet.
pub fn build_kache() {
    let _ = kache_binary();
}

/// Path to the `kache` binary under test, building it on first call.
pub fn kache_binary() -> PathBuf {
    match KACHE_BIN.get_or_init(bootstrap_kache) {
        Ok(path) => path.clone(),
        Err(message) => panic!("{message}"),
    }
}

fn bootstrap_kache() -> Result<PathBuf, String> {
    if reuse_outer_build() {
        return Ok(PathBuf::from(env!("CARGO_BIN_EXE_kache")));
    }
    let target_dir = bootstrap_target_dir();
    let output = std::process::Command::new("cargo")
        .args([
            "build",
            "--bin",
            "kache",
            "--bin",
            "kache-engine",
            "--target-dir",
            target_dir.to_str().expect("bootstrap target dir is utf-8"),
            "--config",
            "build.rustc-wrapper=\"\"",
        ])
        .env_remove("RUSTC_WRAPPER")
        .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
        .output()
        .map_err(|e| format!("failed to spawn cargo to build kache: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "kache build failed ({}) in {}\nstdout: {}\nstderr: {}",
            output.status,
            target_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }

    let mut bin = target_dir.join("debug").join("kache");
    if cfg!(windows) {
        bin.set_extension("exe");
    }
    if !bin.is_file() {
        return Err(format!(
            "kache build reported success but {} is missing",
            bin.display()
        ));
    }
    Ok(bin)
}

/// A proc macro that lists `$CARGO_MANIFEST_DIR/assets`, which may be
/// missing, and includes each file it finds, as `sqlx::migrate!` and
/// `include_dir!` do. rustc reports the files it included, never the
/// directory it listed.
#[allow(dead_code)]
pub const SCAN_MACRO: &str = r#"extern crate proc_macro;
#[proc_macro]
pub fn scan(_input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let assets = std::path::Path::new(&manifest_dir).join("assets");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(assets)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    files.sort();
    let mut code = String::from("pub const ASSETS: &[&str] = &[");
    for file in files {
        code.push_str(&format!("include_str!({:?}),", file.display().to_string()));
    }
    code.push_str("];");
    code.parse().unwrap()
}
"#;

/// Keeps tests off the developer's real `~/.config/kache/config.toml`.
pub fn isolated_config_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("config.toml")
}

/// A child `cargo` or `kache` whose Kache state is pinned to `cache_dir`.
///
/// Tests read `events.jsonl` and reach the daemon socket under the cache dir
/// they created, and Kache resolves both under its runtime dir. The parent
/// environment can point that elsewhere: kache-action exports
/// `KACHE_RUNTIME_DIR`, a build that runs the suite through the wrapper leaves
/// `KACHE_ACTIVE`, `KACHE_SOCKET_PATH`, and `KACHE_EVENT_ROOT` behind, and a
/// developer dogfooding Kache has `RUSTC_WRAPPER` configured. All of those
/// are pinned or cleared here, before the caller adds its own wrapper and
/// overrides; a later `env` call on the same key wins.
/// Remote selectors are cleared so a cold daemon cannot inherit runner settings.
///
/// `config` pins `KACHE_CONFIG`. `None` unsets it so the child discovers its
/// configuration the way a real build would.
///
/// The host config layer is turned off: the self-hosted CI Macs carry a real
/// `/etc/kache/config.toml`, and a test must see only the config it writes.
/// A test that wants a host file calls `.env("KACHE_HOST_CONFIG", path)` on
/// the returned command.
///
/// Not every test binary spawns children, so the rest see it as dead.
#[allow(dead_code)]
pub fn hermetic_command(
    program: impl AsRef<OsStr>,
    cache_dir: &Path,
    config: Option<&Path>,
) -> Command {
    let mut command = Command::new(program);
    command
        .env("KACHE_CACHE_DIR", cache_dir)
        .env("KACHE_RUNTIME_DIR", cache_dir)
        .env_remove("KACHE_SOCKET_PATH")
        .env_remove("KACHE_EVENT_ROOT")
        .env_remove("KACHE_ACTIVE")
        .env_remove("KACHE_UNSTABLE")
        .env_remove("KACHE_OUT_DIR_ALIAS_ROOT")
        .env_remove("KACHE_INCREMENTAL_SEED_FROM")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
        // Cargo sets kache's own OUT_DIR on the test process (kache has a
        // build script). A child compile would inherit it and look like a
        // unit with an OUT_DIR.
        .env_remove("OUT_DIR")
        // CI strips debuginfo from kache's own dev profile via
        // CARGO_PROFILE_DEV_DEBUG=0. A fixture build must keep its default
        // debuginfo, or backtraces lose their source locations.
        .env_remove("CARGO_PROFILE_DEV_DEBUG")
        .env_remove("CARGO_PROFILE_DEV_STRIP")
        .env("KACHE_HOST_CONFIG", "");
    match config {
        Some(path) => command.env("KACHE_CONFIG", path),
        None => command.env_remove("KACHE_CONFIG"),
    };
    command
}

/// A scratch directory on the *repository's* filesystem, for tests whose
/// subject is how artifacts are materialized.
///
/// `TempDir::new()` lands in `TMPDIR`, so a cache placed there sits on a
/// different filesystem from the build — or on tmpfs, which supports no
/// reflink. Either way the store ingest and the restore take different paths
/// than they would in a real build, and those paths differ in what file mode
/// they leave behind. Anchoring here keeps such tests on whatever filesystem
/// the developer or CI actually builds on, and `cargo clean` reclaims it.
///
/// `KACHE_TEST_SCRATCH_DIR` overrides it. The developer's filesystem is
/// whatever it is, but CI's is ext4 — where `try_reflink` always fails and the
/// `fs::copy` fallback preserves permissions, hiding every mode-losing
/// reflink path (that is how #822 reverted #648 unnoticed). The
/// `cow-filesystem` job points this at a loopback btrfs mount so those paths
/// actually run.
///
/// Only the tests that materialize artifacts need this, and each integration
/// test binary compiles this module separately, so the rest see it as dead.
#[allow(dead_code)]
pub fn scratch_dir() -> PathBuf {
    let dir = match std::env::var_os("KACHE_TEST_SCRATCH_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => bootstrap_target_dir().with_file_name("kache-test-scratch"),
    };
    std::fs::create_dir_all(&dir).expect("creating scratch dir");
    dir
}

/// Wait until a compile started after this returns counts every file the
/// caller has written as written before it.
///
/// Kache counts an input stamped up to one window before an invocation's
/// start as written during it (`kache_store::file_hash::stamp_written_since`),
/// and a compile that ran before its key does not store then. The window is
/// 1 ms on Linux, macOS and Windows and 20 ms elsewhere where stamps keep a fraction
/// of a second, which this always waits, and about two seconds for a
/// whole-second stamp found under `dirs`. A stamp in the future cannot be
/// waited out and is left to the test.
#[allow(dead_code)]
pub fn settle_writes(dirs: &[&Path]) {
    use kache_store::file_hash::{
        FINE_STAMP_WINDOW_NS, metadata_ctime_ns, metadata_mtime_ns, stamp_clock_ns,
        stamp_window_ns, wall_clock_ns,
    };
    fn collect(path: &Path, stamps: &mut Vec<i64>) {
        let Ok(link) = std::fs::symlink_metadata(path) else {
            return;
        };
        if link.is_dir() {
            for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
                collect(&entry.path(), stamps);
            }
        } else if let Ok(metadata) = std::fs::metadata(path) {
            stamps.extend([metadata_mtime_ns(&metadata), metadata_ctime_ns(&metadata)]);
        }
    }
    let now = wall_clock_ns();
    let mut stamps = Vec::new();
    for dir in dirs {
        collect(dir, &mut stamps);
    }
    let settled_at = stamps
        .into_iter()
        .filter(|&stamp| stamp <= now)
        .map(|stamp| stamp.saturating_add(stamp_window_ns(stamp)))
        .fold(
            stamp_clock_ns().saturating_add(FINE_STAMP_WINDOW_NS),
            i64::max,
        );
    // Kache reads its start from the same clock.
    while stamp_clock_ns() <= settled_at {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The lock a live daemon holds for its whole lifetime, under `runtime_dir`.
/// It is released only after the daemon's shutdown drain, so it is the one
/// signal that works for Unix sockets and Windows named pipes alike.
fn daemon_run_lock_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("daemon.run.lock")
}

/// Whether a daemon currently holds `lock_path`. The probe
/// does not create the file, so a test that never started a daemon stays
/// untouched. An error other than "missing" counts as held: the caller then
/// asks the daemon to stop, which is harmless if there is none.
fn daemon_run_lock_is_held(lock_path: &Path) -> bool {
    let file = match std::fs::OpenOptions::new().write(true).open(lock_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// Waits until no daemon holds the run lock under `runtime_dir`.
#[allow(dead_code)]
pub fn wait_for_daemon_exit(runtime_dir: &Path, timeout: Duration) -> Result<(), String> {
    wait_for_daemon_lock_release(&daemon_run_lock_path(runtime_dir), timeout)
}

fn wait_for_daemon_lock_release(lock_path: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while daemon_run_lock_is_held(lock_path) {
        if Instant::now() >= deadline {
            return Err(format!(
                "daemon lifetime lock {} remained held after its drain phase",
                lock_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Stops a fixture's daemon before its runtime directory is removed.
/// Declare after the TempDir, or before it in a fixture's field list.
#[allow(dead_code)]
pub struct DaemonGuard {
    program: PathBuf,
    runtime: PathBuf,
}

#[allow(dead_code)]
impl DaemonGuard {
    pub fn new(program: impl AsRef<Path>, runtime: impl AsRef<Path>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            runtime: runtime.as_ref().to_owned(),
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let mut stop = hermetic_command(
            &self.program,
            &self.runtime,
            Some(&self.runtime.join("daemon-guard-no-config.toml")),
        );
        stop_daemon(stop.args(["daemon", "stop"]), &self.runtime);
    }
}

/// Stops the daemon that owns `runtime_dir`, for a test fixture's `Drop`.
///
/// `stop` is a `kache daemon stop` wired to that runtime dir. Each test has its
/// own runtime dir, so tests running in parallel each stop only their own
/// daemon, and `Drop` also runs when a test panics. This keeps a daemon warm
/// for the whole test and leaves nothing behind (kunobi-ninja/kache#704).
///
/// Most tests never start a daemon, and a `daemon stop` with nobody to answer
/// it waits several seconds for a connection, so it only runs while a daemon
/// holds the run lock. Auto-start returns once the daemon is ready, so a
/// daemon a test started holds that lock by the time the test ends.
///
/// Cleanup must never be what wedges or fails a test run: the stop runs with
/// null stdio (no pipe for a daemon to hold open) and a deadline, and every
/// failure is ignored.
#[allow(dead_code)]
pub fn stop_daemon(stop: &mut Command, runtime_dir: &Path) {
    let socket = stop.get_envs().find_map(|(name, value)| {
        (name == "KACHE_SOCKET_PATH")
            .then_some(value)
            .flatten()
            .filter(|value| !value.is_empty())
    });
    let lock_path = socket.map_or_else(
        || daemon_run_lock_path(runtime_dir),
        |socket| {
            let socket = Path::new(socket);
            let socket = if socket.is_absolute() {
                socket.to_owned()
            } else {
                stop.get_current_dir()
                    .unwrap_or(Path::new("."))
                    .join(socket)
            };
            socket.with_extension("run.lock")
        },
    );
    if !daemon_run_lock_is_held(&lock_path) {
        return;
    }
    let spawned = stop
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = spawned {
        // `daemon stop` itself waits up to 35s for the drain.
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            match child.try_wait() {
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(Some(_)) => break,
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }
    // On Windows the daemon can keep handles into the runtime dir during
    // teardown; wait for it before the fixture removes that directory.
    let _ = wait_for_daemon_lock_release(&lock_path, Duration::from_secs(45));
}
