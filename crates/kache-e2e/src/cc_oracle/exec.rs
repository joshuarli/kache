//! Runs one arm of a case through its steps and records what it observed.

use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use super::case::{Action, Case, Profile, Reuse};
use super::judge::{Entry, EntryKind, Executions, Inventory, Observation};

/// The deterministic modification time given to fixture files, so tools that
/// refuse to trust files modified moments before a compile behave the same
/// in every arm. Later writes advance from here a minute per step.
const FIXTURE_EPOCH: u64 = 1_767_225_600;

/// A step's edits must be settled before it compiles: an edit inside the
/// stamp window both caches use to detect a racing write is not the
/// quiescent state the step describes. The window follows kache's
/// (`kache_store::file_hash::stamp_window_ns`): 1 ms for fine stamps, plus
/// 10 ms when a stamp sits on a centisecond and 2 s when it sits on a whole
/// second, since such a stamp may have been truncated. Linux's coarse file
/// clock trails the wall clock by up to a tick, so add 10 ms more.
pub fn settled_after_ns(stamps: impl IntoIterator<Item = i64>) -> i64 {
    stamps
        .into_iter()
        .map(|stamp| {
            let grain = if stamp.rem_euclid(1_000_000_000) == 0 {
                2_000_000_000
            } else if stamp.rem_euclid(10_000_000) == 0 {
                10_000_000
            } else {
                0
            };
            stamp + grain + 1_000_000 + 10_000_000
        })
        .max()
        .unwrap_or(0)
}

fn wait_until_settled(root: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mut stamps = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let meta = std::fs::symlink_metadata(&path)?;
        stamps.push(meta.mtime() * 1_000_000_000 + meta.mtime_nsec());
        stamps.push(meta.ctime() * 1_000_000_000 + meta.ctime_nsec());
        if meta.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        }
    }
    let deadline = settled_after_ns(stamps);
    loop {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos() as i64;
        if now > deadline {
            return Ok(());
        }
        std::thread::sleep(Duration::from_nanos((deadline - now) as u64 + 1));
    }
}

/// The prefix map kache documents adding to GCC/Clang misses.
pub const KACHE_CC_ROOT: &str = "/kache/cc-root";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Arm {
    Raw,
    RawRepeat,
    /// Reproducible profile only: the case's own command, which is what a
    /// kache passthrough runs and so what it must match.
    RawPassthrough,
    Ccache,
    CcachePreprocessor,
    Kache,
}

impl Arm {
    pub const ALL: [Arm; 5] = [Arm::Raw, Arm::RawRepeat, Arm::Ccache, Arm::CcachePreprocessor, Arm::Kache];

    pub fn for_profile(profile: Profile) -> Vec<Arm> {
        let mut arms = Arm::ALL.to_vec();
        if profile == Profile::Reproducible {
            arms.insert(2, Arm::RawPassthrough);
        }
        arms
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Arm::Raw => "raw",
            Arm::RawRepeat => "raw-repeat",
            Arm::RawPassthrough => "raw-passthrough",
            Arm::Ccache => "ccache",
            Arm::CcachePreprocessor => "ccache-preprocessor",
            Arm::Kache => "kache",
        }
    }
}

/// A fault the negative-control mode injects into the kache arm (or, for
/// `ReferenceFailure`, the raw arms) to prove the oracle detects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// After the first recompilation, put the populate step's outputs back.
    StaleOutput,
    /// After the first required restore, delete one declared output.
    OmitSidecar,
    /// Hide the first recompilation's code generation, as if kache reused a
    /// stale entry whose bytes happened to match.
    DisabledInvalidation,
    /// Run the kache arm with `KACHE_DISABLED=1`.
    CacheDisabled,
    /// Point the raw arms at a compiler that always fails.
    ReferenceFailure,
}

impl Control {
    pub const ALL: [Control; 5] = [
        Control::StaleOutput,
        Control::OmitSidecar,
        Control::DisabledInvalidation,
        Control::CacheDisabled,
        Control::ReferenceFailure,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Control::StaleOutput => "stale-output",
            Control::OmitSidecar => "omit-sidecar",
            Control::DisabledInvalidation => "disabled-invalidation",
            Control::CacheDisabled => "cache-disabled",
            Control::ReferenceFailure => "reference-failure",
        }
    }

    pub fn parse(s: &str) -> Option<Control> {
        Control::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

#[derive(Debug, Clone)]
pub struct Toolchain {
    pub name: String,
    /// From `cc --version`; `None` when the banner names neither family.
    pub family: Option<super::case::Family>,
    pub cc_name: String,
    pub cxx_name: String,
    pub cc: PathBuf,
    pub cxx: PathBuf,
}

/// `gcc-15` drives C++ as `g++-15`, `clang` as `clang++`, `cc` as `c++`.
pub fn cxx_driver_name(cc: &str) -> String {
    if let Some(rest) = cc.strip_prefix("gcc") {
        format!("g++{rest}")
    } else if let Some(rest) = cc.strip_prefix("clang") {
        format!("clang++{rest}")
    } else if let Some(rest) = cc.strip_suffix("cc") {
        format!("{rest}c++")
    } else {
        format!("{cc}++")
    }
}

/// `PATH` without directories that would put another compiler cache in
/// front of the real compiler.
pub fn clean_path(path: &std::ffi::OsStr) -> OsString {
    let kept: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|dir| !dir.join(".kache-shims").exists())
        .filter(|dir| !dir.components().any(|c| c.as_os_str() == "ccache"))
        .collect();
    std::env::join_paths(kept).unwrap_or_default()
}

fn is_cache_wrapper(path: &Path) -> bool {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    matches!(target.file_name().and_then(|n| n.to_str()), Some("kache" | "ccache" | "sccache"))
}

fn find_compiler(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file() && !is_cache_wrapper(candidate))
}

pub fn resolve_toolchain(name: &str, path: &std::ffi::OsStr) -> Result<Toolchain, String> {
    let cxx_name = cxx_driver_name(name);
    let cc = find_compiler(name, path).ok_or_else(|| format!("`{name}` not found on PATH"))?;
    let cxx = find_compiler(&cxx_name, path).ok_or_else(|| format!("`{cxx_name}` not found on PATH"))?;
    let banner = Command::new(&cc)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    Ok(Toolchain {
        name: name.to_string(),
        family: super::case::Family::from_version(&banner),
        cc_name: name.to_string(),
        cxx_name,
        cc,
        cxx,
    })
}

/// Write the observation shims: one script per driver name that logs its
/// working directory and arguments, then execs the real compiler. Every arm
/// runs the same shim bytes, so compiler identity is equal across arms.
pub fn write_shims(dir: &Path, toolchain: &Toolchain, real_override: Option<&Path>) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, real) in [(&toolchain.cc_name, &toolchain.cc), (&toolchain.cxx_name, &toolchain.cxx)] {
        let real = real_override.unwrap_or(real);
        let script = format!(
            "#!/bin/sh\n{{ printf '%s\\037' \"$PWD\"; for a in \"$@\"; do printf '%s\\037' \"$a\"; done; printf '\\n'; }} >> \"$CC_ORACLE_LOG\"\nexec '{}' \"$@\"\n",
            real.display().to_string().replace('\'', "'\\''")
        );
        let path = dir.join(name);
        std::fs::write(&path, script)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Classify the shim log records one invocation produced.
pub fn classify(log: &str, fixture: &Path, source: &str) -> Executions {
    let mut out = Executions::default();
    let fixture = fixture.to_string_lossy();
    for line in log.lines().filter(|l| !l.is_empty()) {
        let mut fields: Vec<&str> = line.split('\u{1f}').collect();
        if fields.last() == Some(&"") {
            fields.pop();
        }
        let Some((cwd, args)) = fields.split_first() else { continue };
        let names_unit = *cwd == fixture && args.contains(&source);
        let has = |flag: &str| args.contains(&flag);
        if !names_unit || has("-###") || has("--version") {
            out.probe += 1;
        } else if has("-E") {
            out.preprocess += 1;
        } else if has("-c") || has("-S") {
            out.codegen += 1;
            out.codegen_argv.push(args.iter().map(|a| a.to_string()).collect());
        } else {
            out.probe += 1;
        }
    }
    out
}

pub fn inventory(root: &Path) -> Result<Inventory> {
    let mut out = Inventory::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Inventory) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let rel = path.strip_prefix(root)?.to_string_lossy().into_owned();
        let meta = std::fs::symlink_metadata(&path)?;
        let mode = meta.permissions().mode() & 0o7777;
        let entry = if meta.file_type().is_symlink() {
            Entry {
                kind: EntryKind::Symlink,
                mode,
                digest: std::fs::read_link(&path)?.to_string_lossy().into_owned(),
                size: 0,
            }
        } else if meta.is_dir() {
            walk(root, &path, out)?;
            Entry {
                kind: EntryKind::Dir,
                mode,
                digest: String::new(),
                size: 0,
            }
        } else {
            let bytes = std::fs::read(&path)?;
            Entry {
                kind: EntryKind::File,
                mode,
                digest: blake3::hash(&bytes).to_hex().to_string(),
                size: bytes.len() as u64,
            }
        };
        out.insert(rel, entry);
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            symlink(std::fs::read_link(&src)?, &dst)?;
        } else if kind.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
            set_mtime(&dst, FIXTURE_EPOCH)?;
        }
    }
    Ok(())
}

fn set_mtime(path: &Path, unix_secs: u64) -> Result<()> {
    set_mtime_to(path, SystemTime::UNIX_EPOCH + Duration::from_secs(unix_secs))
}

fn set_mtime_to(path: &Path, time: SystemTime) -> Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(time))
        .with_context(|| format!("setting the mtime of {}", path.display()))
}

/// Everything an arm needs from the run besides the case itself.
pub struct ArmContext<'a> {
    pub kache: &'a Path,
    pub ccache: &'a Path,
    pub toolchain: &'a Toolchain,
    pub shims: &'a Path,
    pub failing_shims: &'a Path,
    pub fixture: &'a Path,
    pub state: &'a Path,
    pub system_path: &'a std::ffi::OsStr,
    pub control: Option<Control>,
}

pub struct ArmRun {
    /// `[step][compile]`.
    pub observations: Vec<Vec<Observation>>,
    pub durations_ms: Vec<Vec<u64>>,
    /// Whether the selected control found a step to fault in this arm.
    pub injected: bool,
}

/// A disabled kache runs the case's own arguments, so it equals the raw
/// reference only in the transparent profile; in the reproducible profile
/// the reference carries kache's prefix map and the control would report
/// wrong output instead of the missed reuse it exists to prove.
fn cache_disabled_applies(control: Option<Control>, profile: Profile) -> bool {
    control == Some(Control::CacheDisabled) && profile == Profile::Transparent
}

/// Expand `{fixture}` in a case-supplied argument or environment value.
pub fn expand(value: &str, fixture: &Path) -> String {
    value.replace("{fixture}", &fixture.to_string_lossy())
}

pub fn effective_args(case_argv: &[String], profile: Profile, fixture: &Path) -> Vec<String> {
    let mut args: Vec<String> = case_argv[1..].iter().map(|a| expand(a, fixture)).collect();
    if profile == Profile::Reproducible {
        args.push(format!("-ffile-prefix-map={}={KACHE_CC_ROOT}", fixture.display()));
    }
    args
}

pub fn run_arm(case: &Case, profile: Profile, arm: Arm, ctx: &ArmContext<'_>) -> Result<ArmRun> {
    let state = ctx.state.join(arm.as_str());
    if state.exists() {
        std::fs::remove_dir_all(&state)?;
    }
    for sub in ["store", "home", "tmp"] {
        std::fs::create_dir_all(state.join(sub))?;
    }
    let config = state.join("config");
    std::fs::write(&config, "")?;
    let log = state.join("exec.log");
    if ctx.fixture.exists() {
        std::fs::remove_dir_all(ctx.fixture)?;
    }
    copy_tree(&case.source_dir, ctx.fixture).with_context(|| format!("preparing {}", case.id))?;
    wait_until_settled(ctx.fixture)?;

    let raw_like = matches!(arm, Arm::Raw | Arm::RawRepeat | Arm::RawPassthrough);
    let shims = if raw_like && ctx.control == Some(Control::ReferenceFailure) {
        ctx.failing_shims
    } else {
        ctx.shims
    };
    let mut path = OsString::from(shims);
    path.push(":");
    path.push(ctx.system_path);

    let mut env: Vec<(&str, OsString)> = vec![
        ("PATH", path),
        ("HOME", state.join("home").into()),
        ("TMPDIR", state.join("tmp").into()),
        ("LC_ALL", "C".into()),
        ("LANG", "C".into()),
        ("CC_ORACLE_LOG", log.clone().into()),
    ];
    let store = state.join("store");
    match arm {
        Arm::Raw | Arm::RawRepeat | Arm::RawPassthrough => {}
        Arm::Ccache | Arm::CcachePreprocessor => {
            env.push(("CCACHE_DIR", store.clone().into()));
            env.push(("CCACHE_CONFIGPATH", config.clone().into()));
            if arm == Arm::CcachePreprocessor {
                env.push(("CCACHE_NODIRECT", "1".into()));
            }
        }
        Arm::Kache => {
            env.push(("KACHE_CACHE_DIR", store.clone().into()));
            env.push(("KACHE_CONFIG", config.clone().into()));
            if profile == Profile::Transparent {
                env.push(("KACHE_CC_PATH_NORMALIZE", "0".into()));
                env.push(("KACHE_CC_SOURCE_DATE_EPOCH", "passthrough".into()));
            }
            if cache_disabled_applies(ctx.control, profile) {
                env.push(("KACHE_DISABLED", "1".into()));
            }
        }
    }
    let case_env: Vec<(String, String)> = case.env.iter().map(|(k, v)| (k.clone(), expand(v, ctx.fixture))).collect();
    for (k, v) in &case_env {
        env.push((k.as_str(), v.into()));
    }
    if profile == Profile::Reproducible
        && !matches!(arm, Arm::Kache | Arm::RawPassthrough)
        && !case.env.contains_key("SOURCE_DATE_EPOCH")
    {
        env.push(("SOURCE_DATE_EPOCH", "0".into()));
    }

    let arm_env = ArmEnv {
        shims,
        vars: &env,
        log: &log,
        store: &store,
    };
    let result = run_steps(case, profile, arm, ctx, &arm_env);
    if arm == Arm::Kache {
        crate::daemon::stop_and_wait(ctx.kache, &store, Duration::from_secs(60));
    }
    let mut run = result?;
    // Only a required hit can reveal a disabled cache.
    if cache_disabled_applies(ctx.control, profile)
        && arm == Arm::Kache
        && case.steps.iter().any(|step| step.reuse == Reuse::Hit)
    {
        run.injected = true;
    }
    if ctx.control == Some(Control::ReferenceFailure) && raw_like {
        run.injected = true;
    }
    Ok(run)
}

struct ArmEnv<'a> {
    shims: &'a Path,
    vars: &'a [(&'a str, OsString)],
    log: &'a Path,
    store: &'a Path,
}

fn run_steps(case: &Case, profile: Profile, arm: Arm, ctx: &ArmContext<'_>, arm_env: &ArmEnv<'_>) -> Result<ArmRun> {
    let ArmEnv {
        shims,
        vars: env,
        log,
        store,
    } = *arm_env;
    let mut run = ArmRun {
        observations: Vec::new(),
        durations_ms: Vec::new(),
        injected: false,
    };
    let mut populated: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut kache_events = 0usize;
    for (index, step) in case.steps.iter().enumerate() {
        for action in &step.action {
            apply(action, case, arm, ctx, store, index)?;
        }
        if step.action.iter().any(Action::edits_files) {
            wait_until_settled(ctx.fixture)?;
        }
        let fault_here = arm == Arm::Kache && !run.injected && index > 0;
        let mut step_obs = Vec::new();
        let mut step_ms = Vec::new();
        for compile in &case.compiles {
            std::fs::write(log, "")?;
            let driver = match compile.argv[0].as_str() {
                "{cc}" => shims.join(&ctx.toolchain.cc_name),
                _ => shims.join(&ctx.toolchain.cxx_name),
            };
            let mut command = match arm {
                Arm::Raw | Arm::RawRepeat | Arm::RawPassthrough => Command::new(&driver),
                Arm::Ccache | Arm::CcachePreprocessor => {
                    let mut c = Command::new(ctx.ccache);
                    c.arg(&driver);
                    c
                }
                Arm::Kache => {
                    let mut c = Command::new(ctx.kache);
                    c.arg(&driver);
                    c
                }
            };
            let args = match arm {
                Arm::Kache => compile.argv[1..].iter().map(|a| expand(a, ctx.fixture)).collect(),
                Arm::RawPassthrough => effective_args(&compile.argv, Profile::Transparent, ctx.fixture),
                _ => effective_args(&compile.argv, profile, ctx.fixture),
            };
            command.args(&args).current_dir(ctx.fixture).env_clear();
            for (k, v) in env {
                command.env(k, v);
            }
            let started = Instant::now();
            let output = command
                .output()
                .with_context(|| format!("{} arm: spawning {}", arm.as_str(), driver.display()))?;
            step_ms.push(started.elapsed().as_millis() as u64);
            let log_text = std::fs::read_to_string(log).unwrap_or_default();
            let mut executions = classify(&log_text, ctx.fixture, &expand(&compile.source, ctx.fixture));
            let kache_result = if arm == Arm::Kache {
                let events = kache_events_since(ctx.kache, store, &mut kache_events)?;
                events.last().cloned()
            } else {
                None
            };

            if fault_here {
                match ctx.control {
                    Some(Control::StaleOutput) if step.reuse == Reuse::Miss && !populated.is_empty() => {
                        for (path, bytes) in &populated {
                            if std::fs::read(path).ok().as_ref() != Some(bytes) {
                                std::fs::write(path, bytes)?;
                                run.injected = true;
                            }
                        }
                    }
                    Some(Control::DisabledInvalidation) if step.reuse == Reuse::Miss => {
                        executions.codegen = 0;
                        executions.codegen_argv.clear();
                        run.injected = true;
                    }
                    Some(Control::OmitSidecar) if step.reuse == Reuse::Hit => {
                        if let Some(path) = case
                            .outputs
                            .iter()
                            .rev()
                            .map(|o| ctx.fixture.join(o))
                            .find(|p| p.exists())
                        {
                            std::fs::remove_file(path)?;
                            run.injected = true;
                        }
                    }
                    _ => {}
                }
            }

            step_obs.push(Observation {
                status: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
                tree: inventory(ctx.fixture)?,
                executions,
                kache_result,
            });
        }
        if index == 0 && arm == Arm::Kache {
            populated = case
                .outputs
                .iter()
                .map(|o| ctx.fixture.join(o))
                .filter(|p| p.is_file())
                .map(|p| std::fs::read(&p).map(|b| (p, b)))
                .collect::<std::io::Result<_>>()?;
        }
        run.observations.push(step_obs);
        run.durations_ms.push(step_ms);
    }
    Ok(run)
}

fn apply(action: &Action, case: &Case, arm: Arm, ctx: &ArmContext<'_>, store: &Path, step: usize) -> Result<()> {
    let root = ctx.fixture;
    match action {
        Action::Write {
            path,
            content,
            keep_mtime,
        } => {
            let target = root.join(path);
            let previous = std::fs::metadata(&target).and_then(|m| m.modified()).ok();
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, content)?;
            match (keep_mtime, previous) {
                (true, Some(time)) => set_mtime_to(&target, time)?,
                (true, None) => bail!("case {}: keep_mtime on {path}, which did not exist", case.id),
                (false, _) => set_mtime(&target, FIXTURE_EPOCH + 60 * (step as u64 + 1))?,
            }
        }
        Action::Replace { path, content } => {
            let target = root.join(path);
            let staged = target.with_file_name(format!(
                ".{}.oracle-tmp",
                target.file_name().and_then(|n| n.to_str()).unwrap_or("file")
            ));
            std::fs::write(&staged, content)?;
            set_mtime(&staged, FIXTURE_EPOCH + 60 * (step as u64 + 1))?;
            std::fs::rename(&staged, &target)?;
        }
        Action::Symlink { path, target } => {
            let link = root.join(path);
            remove_if_present(&link)?;
            symlink(target, &link)?;
        }
        Action::Touch { path } => set_mtime(&root.join(path), FIXTURE_EPOCH + 60 * (step as u64 + 1))?,
        Action::Remove { path } => remove_if_present(&root.join(path))?,
        Action::DeleteOutputs {} => {
            for output in &case.outputs {
                remove_if_present(&root.join(output))?;
            }
        }
        Action::RestartDaemon {} => {
            if arm == Arm::Kache {
                anyhow::ensure!(
                    crate::daemon::stop_and_wait(ctx.kache, store, Duration::from_secs(60)),
                    "kache daemon did not stop"
                );
            }
        }
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path)?,
        Ok(_) => std::fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// Results of the compile events kache recorded since the last call,
/// skipping probe passthroughs.
fn kache_events_since(kache: &Path, store: &Path, seen: &mut usize) -> Result<Vec<String>> {
    let output = Command::new(kache)
        .args(["report", "--format", "json", "--since", "1h"])
        .env_clear()
        .env("KACHE_CACHE_DIR", store)
        .env("KACHE_CONFIG", store.with_file_name("config"))
        .env("HOME", store.with_file_name("home"))
        .output()
        .context("running kache report")?;
    if !output.status.success() {
        bail!("kache report exited {}: {}", output.status, String::from_utf8_lossy(&output.stderr));
    }
    let (report, _) = crate::report::parse_report_json(&output.stdout)?;
    let events = report.all_events.get(*seen..).unwrap_or_default();
    *seen = report.all_events.len();
    Ok(events
        .iter()
        .filter(|e| !e.is_probe_passthrough())
        .map(|e| e.result.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cxx_drivers_follow_the_c_driver_name() {
        assert_eq!(cxx_driver_name("gcc"), "g++");
        assert_eq!(cxx_driver_name("gcc-15"), "g++-15");
        assert_eq!(cxx_driver_name("clang"), "clang++");
        assert_eq!(cxx_driver_name("clang-21"), "clang++-21");
        assert_eq!(cxx_driver_name("cc"), "c++");
        assert_eq!(cxx_driver_name("x86_64-linux-musl-cc"), "x86_64-linux-musl-c++");
        assert_eq!(cxx_driver_name("icx"), "icx++");
    }

    fn record(cwd: &str, args: &[&str]) -> String {
        let mut s = format!("{cwd}\u{1f}");
        for a in args {
            s.push_str(a);
            s.push('\u{1f}');
        }
        s.push('\n');
        s
    }

    #[test]
    fn classifies_unit_codegen_preprocessing_and_probes() {
        let fx = Path::new("/fx");
        let log = [
            record("/fx", &["--version"]),
            record("/fx", &["-###", "-c", "a.c"]),
            record("/tmp/probe", &["-c", "probe.c", "-o", "probe.o"]),
            record("/other", &["-c", "a.c"]),
            record("/fx", &["-E", "a.c"]),
            record("/fx", &["-c", "a.c", "-o", "a.o"]),
            record("/fx", &["-S", "a.c"]),
            record("/fx", &["-fsyntax-only", "a.c"]),
            String::new(),
        ]
        .concat();
        let e = classify(&log, fx, "a.c");
        assert_eq!((e.codegen, e.preprocess, e.probe), (2, 1, 5));
        assert_eq!(e.codegen_argv[0], ["-c", "a.c", "-o", "a.o"]);
    }

    #[test]
    fn arguments_with_spaces_survive_the_log() {
        let log = record("/f x", &["-c", "my file.c", "-DA=1 2"]);
        let e = classify(&log, Path::new("/f x"), "my file.c");
        assert_eq!(e.codegen_argv, vec![vec!["-c", "my file.c", "-DA=1 2"]]);
    }

    #[test]
    fn reproducible_profile_appends_the_documented_prefix_map() {
        let argv: Vec<String> = ["{cc}", "-c", "a.c"].map(String::from).to_vec();
        let fx = Path::new("/w/fixture");
        assert_eq!(effective_args(&argv, Profile::Transparent, fx), ["-c", "a.c"]);
        assert_eq!(
            effective_args(&argv, Profile::Reproducible, fx),
            ["-c", "a.c", "-ffile-prefix-map=/w/fixture=/kache/cc-root"]
        );
    }

    #[test]
    fn settling_widens_for_truncated_looking_stamps() {
        let ms = 1_000_000;
        assert_eq!(settled_after_ns([5 * 1_000_000_000 + 3 * ms]), 5 * 1_000_000_000 + 14 * ms);
        assert_eq!(settled_after_ns([5 * 1_000_000_000 + 20 * ms]), 5 * 1_000_000_000 + 41 * ms);
        assert_eq!(settled_after_ns([5 * 1_000_000_000]), 7 * 1_000_000_000 + 11 * ms);
        assert_eq!(settled_after_ns([3 * ms, 5 * 1_000_000_000 + 3 * ms]), 5 * 1_000_000_000 + 14 * ms);
        assert_eq!(settled_after_ns([]), 0);
    }

    #[test]
    fn cache_disabled_runs_only_where_disabled_kache_equals_raw() {
        assert!(cache_disabled_applies(Some(Control::CacheDisabled), Profile::Transparent));
        assert!(!cache_disabled_applies(Some(Control::CacheDisabled), Profile::Reproducible));
        assert!(!cache_disabled_applies(Some(Control::StaleOutput), Profile::Transparent));
        assert!(!cache_disabled_applies(None, Profile::Transparent));
    }

    #[test]
    fn fixture_placeholders_expand_in_arguments() {
        let argv: Vec<String> = ["{cc}", "-I{fixture}/inc", "-c", "a.c"].map(String::from).to_vec();
        assert_eq!(
            effective_args(&argv, Profile::Transparent, Path::new("/w/fx")),
            ["-I/w/fx/inc", "-c", "a.c"]
        );
        assert_eq!(expand("{fixture}:{fixture}", Path::new("/r")), "/r:/r");
    }

    #[test]
    fn the_passthrough_reference_runs_only_in_the_reproducible_profile() {
        assert!(!Arm::for_profile(Profile::Transparent).contains(&Arm::RawPassthrough));
        let reproducible = Arm::for_profile(Profile::Reproducible);
        assert!(reproducible.contains(&Arm::RawPassthrough));
        assert_eq!(reproducible.last(), Some(&Arm::Kache));
    }

    #[test]
    fn controls_round_trip_their_names() {
        for c in Control::ALL {
            assert_eq!(Control::parse(c.as_str()), Some(c));
        }
        assert_eq!(Control::parse("nope"), None);
    }

    #[test]
    fn clean_path_drops_cache_shim_directories() {
        let root = tempfile::tempdir().unwrap();
        let shims = root.path().join("shims");
        let plain = root.path().join("bin");
        let ccache = root.path().join("lib/ccache");
        for d in [&shims, &plain, &ccache] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(shims.join(".kache-shims"), "").unwrap();
        let joined = std::env::join_paths([&shims, &plain, &ccache]).unwrap();
        assert_eq!(clean_path(&joined), plain.as_os_str());
    }

    #[test]
    fn resolve_skips_cache_wrappers_named_like_compilers() {
        let root = tempfile::tempdir().unwrap();
        let wrapped = root.path().join("wrapped");
        let real = root.path().join("real");
        std::fs::create_dir_all(&wrapped).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(root.path().join("ccache"), "").unwrap();
        symlink(root.path().join("ccache"), wrapped.join("gcc")).unwrap();
        for name in ["gcc", "g++"] {
            std::fs::write(real.join(name), "").unwrap();
        }
        let path = std::env::join_paths([&wrapped, &real]).unwrap();
        let tc = resolve_toolchain("gcc", &path).unwrap();
        assert_eq!(tc.cc, real.join("gcc"));
        assert_eq!(tc.cxx, real.join("g++"));
        assert!(resolve_toolchain("clang", &path).is_err());
    }

    #[test]
    fn inventory_records_kind_mode_and_content() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("d")).unwrap();
        std::fs::write(root.path().join("d/f"), "abc").unwrap();
        symlink("d/f", root.path().join("l")).unwrap();
        let inv = inventory(root.path()).unwrap();
        assert_eq!(inv.keys().collect::<Vec<_>>(), ["d", "d/f", "l"]);
        assert_eq!(inv["d"].kind, EntryKind::Dir);
        assert_eq!(inv["d/f"].digest, blake3::hash(b"abc").to_hex().to_string());
        assert_eq!(inv["d/f"].size, 3);
        assert_eq!(inv["l"].kind, EntryKind::Symlink);
        assert_eq!(inv["l"].digest, "d/f");
    }

    #[test]
    fn shims_log_and_exec_the_real_compiler() {
        let root = tempfile::tempdir().unwrap();
        let tc = Toolchain {
            name: "cc".into(),
            family: None,
            cc_name: "cc".into(),
            cxx_name: "c++".into(),
            cc: PathBuf::from("/bin/echo"),
            cxx: PathBuf::from("/bin/echo"),
        };
        let dir = root.path().join("shims");
        write_shims(&dir, &tc, None).unwrap();
        let log = root.path().join("log");
        let out = Command::new(dir.join("cc"))
            .args(["-c", "a b.c"])
            .current_dir(root.path())
            .env("CC_ORACLE_LOG", &log)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "-c a b.c\n");
        let cwd = std::fs::canonicalize(root.path()).unwrap();
        let e = classify(&std::fs::read_to_string(&log).unwrap(), &cwd, "a b.c");
        assert_eq!(e.codegen_argv, vec![vec!["-c", "a b.c"]]);

        write_shims(&dir, &tc, Some(Path::new("/usr/bin/false"))).unwrap();
        let status = Command::new(dir.join("c++")).env("CC_ORACLE_LOG", &log).status().unwrap();
        assert!(!status.success());
    }
}
