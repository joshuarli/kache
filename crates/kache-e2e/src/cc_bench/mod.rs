//! Raw, ccache and kache C/C++ build timing under the protocol locked in
//! `campaigns/cpp/lock.toml` (`[perf_estimator]`).
//!
//! For each workload every arm first replays the whole scenario sequence
//! untimed through compiler-logging shims: that establishes how many units
//! each scenario compiles, the cache counters it moves, and that every arm's
//! objects equal the raw compiler's. Timed pairs then replay the same
//! sequence on the real compiler, in a seeded order per pair. A timed replay
//! whose counters or objects differ from the validated ones is not admitted.
//! Preparation (unpacking, configure, CMake) happens outside the timer.

pub mod activity;
pub mod stats;
pub mod workload;

use anyhow::{Context, Result, bail};
use deser::Serialize;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use workload::{Archives, CompilerCmd, Workload};

/// Past the racing-write window a cache applies to fresh inputs: 2 s for a
/// whole-second stamp, plus margin.
const SETTLE: Duration = Duration::from_millis(2500);
/// make compares whole-second stamps (GNU make 3.81, macOS), so an edit has
/// to land in a later second than the outputs it should invalidate.
const MAKE_TICK: Duration = Duration::from_millis(1100);
/// How long a timed replay waits for the events its validation recorded,
/// outside the timer: on a loaded host the log trails the build by seconds.
const EVENT_DEADLINE: Duration = Duration::from_secs(60);
/// The whole ccache configuration each ccache arm reads; kache's is empty.
const CCACHE_CONF: &str = "max_size = 20G\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[deser(rename_all = "snake_case")]
pub enum Arm {
    Raw,
    Ccache,
    Kache,
}

impl Arm {
    pub const ALL: [Arm; 3] = [Arm::Raw, Arm::Ccache, Arm::Kache];
    pub fn as_str(self) -> &'static str {
        match self {
            Arm::Raw => "raw",
            Arm::Ccache => "ccache",
            Arm::Kache => "kache",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub kache: PathBuf,
    pub ccache: PathBuf,
    pub toolchain: String,
    pub out: PathBuf,
    pub workloads: Vec<String>,
    pub pairs: usize,
    pub seed: u64,
    pub validate_only: bool,
    /// Acceptance: at least five complete admitted pairs on an eligible host,
    /// more only while a target's interval crosses it.
    pub acceptance: bool,
    pub archives: Archives,
    pub jobs: usize,
    /// This binary, re-run as the per-build resource meter.
    pub meter: PathBuf,
    /// `PATH` for every arm, from [`declared_path`].
    pub path: String,
}

impl Config {
    /// Whether a process command line belongs to the benchmark: its output
    /// tree, the launchers, or the kache daemon beside the kache binary.
    pub fn is_ours(&self, command: &str) -> bool {
        let kache_dir = self.kache.parent().map(|d| d.display().to_string());
        [Some(self.out.display().to_string()), Some(self.ccache.display().to_string()), kache_dir]
            .into_iter()
            .flatten()
            .any(|prefix| !prefix.is_empty() && command.contains(&prefix))
    }
}

/// One timed (or validating) scenario of one replay.
#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub scenario: String,
    pub wall_ms: f64,
    pub cpu_ms: f64,
    pub max_rss_kib: u64,
    /// Compiler code-generation executions, from the shims (validation only).
    pub codegen: Option<u64>,
    pub counters: Counters,
    /// blake3 over every object's path and bytes.
    pub objects: String,
    pub object_count: usize,
    /// Host load across the timed build.
    pub load: crate::bench_host::PhaseLoad,
    /// The build command, as run.
    pub command: String,
    /// What unrelated processes used around the build; `None` when `ps`
    /// could not be read.
    pub activity: Option<activity::Activity>,
}

/// Cache results a scenario recorded: kache's event log, ccache's stats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counters {
    pub hits: u64,
    pub misses: u64,
    pub other: u64,
}

impl Counters {
    fn total(&self) -> u64 {
        self.hits + self.misses + self.other
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Replay {
    pub arm: Arm,
    pub pair: Option<usize>,
    pub admitted: bool,
    pub rejection: Option<String>,
    pub store_bytes: u64,
    pub observations: Vec<Observation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkloadReport {
    pub workload: String,
    pub units: usize,
    /// blake3 of the pinned archive, or of the generated corpus sources.
    pub input_digest: Option<String>,
    /// The whole environment each arm's commands ran with.
    pub environments: BTreeMap<String, Vec<(String, String)>>,
    pub acceptance: Option<Acceptance>,
    pub validation: Vec<Replay>,
    pub validation_failures: Vec<String>,
    pub timed: Vec<Replay>,
    pub ratios: Vec<RatioRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RatioRow {
    pub scenario: String,
    pub against: Arm,
    pub summary: stats::RatioSummary,
    pub target: Option<f64>,
    pub verdict: Option<stats::Verdict>,
    pub kache_median_ms: Option<f64>,
    pub other_median_ms: Option<f64>,
}

/// A workload's acceptance result.
#[derive(Debug, Clone, Serialize)]
pub struct Acceptance {
    pub complete_pairs: usize,
    /// Across every timed replay's builds, unrelated processes stayed at or
    /// below [`activity::MAX_UNRELATED_CORES`].
    pub eligible: bool,
    pub verdict: AcceptanceVerdict,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[deser(rename_all = "snake_case")]
pub enum AcceptanceVerdict {
    Pass,
    Fail,
    Inconclusive,
    Undersampled,
    Ineligible,
}

impl AcceptanceVerdict {
    pub fn label(self) -> &'static str {
        match self {
            AcceptanceVerdict::Pass => "PASS",
            AcceptanceVerdict::Fail => "FAIL",
            AcceptanceVerdict::Inconclusive => "INCONCLUSIVE",
            AcceptanceVerdict::Undersampled => "UNDERSAMPLED",
            AcceptanceVerdict::Ineligible => "INELIGIBLE",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    pub source_revision: Option<String>,
    pub source_dirty: Option<bool>,
    /// blake3 of each binary the arms ran.
    pub binaries: BTreeMap<String, String>,
    /// The configuration files the arms read, by name, with their contents.
    pub configuration: BTreeMap<String, String>,
}

/// `cpu_ms` and `max_rss_kib` come from `getrusage(RUSAGE_CHILDREN)` in the
/// meter: the build's own process tree. The kache daemon was already running
/// and is not in it.
pub const RESOURCE_ACCOUNTING: &str = "child-process accounting: cpu_ms and max_rss_kib cover the build's process tree only and omit the already-running kache daemon";

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub mode: &'static str,
    pub resource_accounting: &'static str,
    pub provenance: Provenance,
    /// Unrelated activity before any workload ran.
    pub preflight: Option<activity::Activity>,
    pub toolchain: String,
    pub compiler: String,
    pub kache: String,
    pub ccache: String,
    pub host: crate::bench_host::HostInfo,
    pub seed: u64,
    pub jobs: usize,
    pub workloads: Vec<WorkloadReport>,
}

struct ArmSetup {
    compiler: CompilerCmd,
    env: Vec<(String, String)>,
    store: PathBuf,
    runtime: PathBuf,
}

struct ArmPaths<'a> {
    root: &'a Path,
    runtime: &'a Path,
}

fn arm_runtimes() -> Result<BTreeMap<Arm, tempfile::TempDir>> {
    Arm::ALL.into_iter().map(|arm| {
        let runtime = tempfile::Builder::new().prefix("kache-cc-")
            .permissions(std::fs::Permissions::from_mode(0o700)).tempdir_in("/tmp")
            .context("creating a short private benchmark runtime")?;
        Ok((arm, runtime))
    }).collect()
}

fn daemon_command(kache: &Path, setup: &ArmSetup, args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new(kache).args(args).env_clear()
        .envs(setup.env.iter().map(|(k, v)| (k, v))).output()
        .with_context(|| format!("launching kache {}", args.join(" ")))?;
    anyhow::ensure!(output.status.success(),
        "kache {} failed ({}) with runtime {}\nstdout: {}\nstderr: {}",
        args.join(" "), output.status, setup.runtime.display(),
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    Ok(output)
}

fn require_daemon_ready(kache: &Path, setup: &ArmSetup) -> Result<()> {
    let output = daemon_command(kache, setup, &["--json", "daemon", "status"])?;
    let data: deser_value::Value = deser_json::from_slice(&output.stdout)
        .context("reading benchmark daemon readiness")?;
    anyhow::ensure!(data["success"].as_bool() == Some(true)
        && data["daemon_running"].as_bool() == Some(true)
        && data["daemon_version"].as_str().is_some()
        && data["daemon_version"] == data["version"]
        && data["daemon_epoch"].as_u64().is_some()
        && data["daemon_epoch"] == data["epoch"],
        "benchmark daemon is not ready with the current build at {}: {}",
        setup.runtime.display(), String::from_utf8_lossy(&output.stdout));
    Ok(())
}

struct ReplayDaemon<'a> {
    kache: &'a Path,
    setup: &'a ArmSetup,
    armed: bool,
}

impl ReplayDaemon<'_> {
    fn start(&self) -> Result<()> {
        daemon_command(self.kache, self.setup, &["daemon", "start"])?;
        require_daemon_ready(self.kache, self.setup)
    }

    fn stop(&self) -> Result<()> {
        daemon_command(self.kache, self.setup, &["daemon", "stop"])?;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.stop()?;
        self.armed = false;
        std::fs::copy(self.setup.runtime.join("events.jsonl"), self.setup.store.join("events.jsonl"))
            .context("preserving benchmark events after daemon shutdown")?;
        Ok(())
    }
}

impl Drop for ReplayDaemon<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.stop();
        }
    }
}

/// The scenario names in replay order, for a workload.
pub fn scenarios(workload: &Workload) -> Vec<&'static str> {
    let mut names = vec![
        "cold",
        "warm",
        "edit-source-settled",
        "revert-source-settled",
        "edit-header-settled",
        "revert-header-settled",
        "edit-source-immediate",
        "revert-source-immediate",
    ];
    if workload.relocatable {
        names.push("relocated");
    }
    if workload.kind == workload::Kind::CorpusC {
        names.extend(["unit-daemon-start", "unit-steady"]);
    }
    names
}

fn arm_setup(cfg: &Config, arm: Arm, paths: &ArmPaths<'_>, tree: &Path, compiler_dir: Option<&Path>, cc: &Path, cxx: &Path) -> ArmSetup {
    let state = paths.root.join("state").join(arm.as_str());
    let store = state.join("store");
    let cc = compiler_dir.map_or_else(|| cc.to_path_buf(), |dir| dir.join(cc.file_name().unwrap()));
    let cxx = compiler_dir.map_or_else(|| cxx.to_path_buf(), |dir| dir.join(cxx.file_name().unwrap()));
    // kache's default profile maps KACHE_BASE_DIR and pins the epoch; the
    // other arms get the same effective flags and environment.
    let aligned = vec![format!("-ffile-prefix-map={}=/kache/base-dir", tree.display())];
    let base_dir = tree.parent().unwrap().display().to_string();
    // The whole environment: every spawn clears its inherited one, so no
    // CCACHE_*, KACHE_*, CFLAGS or verification toggle leaks into an arm.
    let mut env = vec![
        ("PATH".to_string(), cfg.path.clone()),
        ("LC_ALL".to_string(), "C".to_string()),
        ("HOME".to_string(), state.join("home").display().to_string()),
        ("TMPDIR".to_string(), state.join("tmp").display().to_string()),
    ];
    let (launcher, flags) = match arm {
        Arm::Raw => (None, aligned.clone()),
        Arm::Ccache => {
            env.push(("CCACHE_DIR".into(), store.display().to_string()));
            env.push(("CCACHE_CONFIGPATH".into(), state.join("ccache.conf").display().to_string()));
            env.push(("CCACHE_BASEDIR".into(), base_dir));
            env.push(("CCACHE_NOHASHDIR".into(), "1".into()));
            (Some(cfg.ccache.clone()), aligned.clone())
        }
        Arm::Kache => {
            env.push(("KACHE_CACHE_DIR".into(), store.display().to_string()));
            env.push(("KACHE_RUNTIME_DIR".into(), paths.runtime.display().to_string()));
            env.push(("KACHE_CONFIG".into(), state.join("kache.toml").display().to_string()));
            env.push(("KACHE_BASE_DIR".into(), tree.display().to_string()));
            env.push(("KACHE_HOST_CONFIG".into(), String::new()));
            (Some(cfg.kache.clone()), Vec::new())
        }
    };
    if arm != Arm::Kache {
        env.push(("SOURCE_DATE_EPOCH".into(), "0".into()));
    }
    let join = |driver: &Path| {
        let mut words: Vec<String> = launcher.iter().map(|l| l.display().to_string()).collect();
        words.push(driver.display().to_string());
        words.extend(flags.iter().cloned());
        words.join(" ")
    };
    ArmSetup {
        compiler: CompilerCmd {
            cc: join(&cc),
            cxx: join(&cxx),
            launcher,
            cc_path: cc.clone(),
            cxx_path: cxx.clone(),
            aligned_flags: flags,
        },
        env,
        store,
        runtime: paths.runtime.to_path_buf(),
    }
}

/// The relocated tree's setup differs only in the mapped root.
fn retarget(setup: &ArmSetup, from: &Path, to: &Path) -> ArmSetup {
    let swap = |s: &str| s.replace(&from.display().to_string(), &to.display().to_string());
    ArmSetup {
        compiler: CompilerCmd {
            cc: swap(&setup.compiler.cc),
            cxx: swap(&setup.compiler.cxx),
            launcher: setup.compiler.launcher.clone(),
            cc_path: setup.compiler.cc_path.clone(),
            cxx_path: setup.compiler.cxx_path.clone(),
            aligned_flags: setup.compiler.aligned_flags.iter().map(|f| swap(f)).collect(),
        },
        env: setup
            .env
            .iter()
            .map(|(k, v)| (k.clone(), if k == "KACHE_BASE_DIR" { swap(v) } else { v.clone() }))
            .collect(),
        store: setup.store.clone(),
        runtime: setup.runtime.clone(),
    }
}

/// Run `command` under the meter: wall time here, CPU and peak RSS of its
/// process tree from the meter's own child accounting.
fn measure(meter: &Path, command: Command, env: &[(String, String)], log: Option<&Path>) -> Result<(f64, f64, u64)> {
    let report = tempfile::NamedTempFile::new()?;
    let mut metered = Command::new(meter);
    metered.env_clear();
    metered.arg("--meter").arg(report.path()).arg("--").arg(command.get_program()).args(command.get_args());
    if let Some(dir) = command.get_current_dir() {
        metered.current_dir(dir);
    }
    metered.envs(env.iter().map(|(k, v)| (k, v)));
    if let Some(log) = log {
        metered.env("CC_ORACLE_LOG", log);
    }
    let start = Instant::now();
    let output = metered.output().with_context(|| format!("running {command:?}"))?;
    let wall = start.elapsed().as_secs_f64() * 1000.0;
    if !output.status.success() {
        bail!("{command:?} failed: {}\n{}", output.status, String::from_utf8_lossy(&output.stderr));
    }
    let text = std::fs::read_to_string(report.path())?;
    let mut fields = text.split_whitespace().map(|f| f.parse::<u64>());
    let (Some(Ok(user_us)), Some(Ok(sys_us)), Some(Ok(rss))) = (fields.next(), fields.next(), fields.next()) else {
        bail!("meter report `{text}` is malformed");
    };
    Ok((wall, (user_us + sys_us) as f64 / 1000.0, rss))
}

/// The meter: run the command, then report its children's CPU and peak RSS.
pub fn meter(report: &Path, argv: &[std::ffi::OsString]) -> Result<i32> {
    let (program, args) = argv.split_first().context("--meter needs a command")?;
    let status = Command::new(program).args(args).status()?;
    // SAFETY: getrusage writes into the zeroed struct it is given.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_CHILDREN, &mut usage);
        usage
    };
    let micros = |t: libc::timeval| t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64;
    // ru_maxrss is in bytes on macOS and KiB on Linux.
    let rss_kib = if cfg!(target_os = "macos") { usage.ru_maxrss as u64 / 1024 } else { usage.ru_maxrss as u64 };
    std::fs::write(report, format!("{} {} {}\n", micros(usage.ru_utime), micros(usage.ru_stime), rss_kib))?;
    Ok(status.code().unwrap_or(1))
}

fn counters(arm: Arm, cfg: &Config, setup: &ArmSetup) -> Result<Counters> {
    match arm {
        Arm::Raw => Ok(Counters::default()),
        Arm::Kache => {
            let mut c = Counters::default();
            let Ok(text) = std::fs::read_to_string(setup.runtime.join("events.jsonl")) else {
                return Ok(c);
            };
            for line in text.lines().filter(|l| !l.contains("CMakeScratch") && !l.contains("\"crate_name\":\"ztest")) {
                let Some(at) = line.find("\"result\":\"") else { continue };
                let result = &line[at + 10..];
                let result = &result[..result.find('"').unwrap_or(0)];
                match result {
                    "local_hit" | "remote_hit" | "hit" => c.hits += 1,
                    // `dup`: an entry miss whose compile wrote blobs the
                    // store already held.
                    "miss" | "dup" => c.misses += 1,
                    _ => c.other += 1,
                }
            }
            Ok(c)
        }
        Arm::Ccache => {
            let output = Command::new(&cfg.ccache)
                .arg("--print-stats")
                .env_clear()
                .envs(setup.env.iter().map(|(k, v)| (k, v)))
                .output()?;
            let text = String::from_utf8_lossy(&output.stdout);
            let get = |key: &str| {
                text.lines()
                    .find_map(|l| l.strip_prefix(key).and_then(|v| v.trim().parse::<u64>().ok()))
                    .unwrap_or(0)
            };
            Ok(Counters {
                hits: get("direct_cache_hit\t") + get("preprocessed_cache_hit\t"),
                misses: get("cache_miss\t"),
                other: get("uncacheable_call\t") + get("compile_failed\t") + get("preprocessor_error\t"),
            })
        }
    }
}

/// Some of a build's events land after its last compile exits, so the log
/// is read once it has stopped growing for a while (outside the timer).
fn await_quiet_event_log(log: &Path) {
    let quiet = Duration::from_millis(300);
    let deadline = Instant::now() + Duration::from_secs(10);
    let size = || std::fs::metadata(log).map_or(0, |m| m.len());
    let mut last = size();
    let mut since = Instant::now();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        let now = size();
        if now != last {
            last = now;
            since = Instant::now();
        } else if since.elapsed() >= quiet {
            return;
        }
    }
}

fn delta(after: &Counters, before: &Counters) -> Counters {
    Counters {
        hits: after.hits - before.hits,
        misses: after.misses - before.misses,
        other: after.other - before.other,
    }
}

fn objects_digest(workload: &Workload, tree: &Path) -> Result<(String, usize)> {
    let mut hasher = blake3::Hasher::new();
    let outputs = workload.outputs(tree)?;
    for path in &outputs {
        hasher.update(path.strip_prefix(tree)?.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        hasher.update(blake3::hash(&std::fs::read(path)?).as_bytes());
    }
    Ok((hasher.finalize().to_hex().to_string(), outputs.len()))
}

fn codegen_count(log: &Path) -> u64 {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.lines()
        .filter(|line| {
            let args: Vec<&str> = line.split('\u{1f}').skip(1).collect();
            let has = |f: &str| args.contains(&f);
            has("-c") && !has("-E") && !has("-###") && !has("--version") && !has("-M") && !has("-MM") && !has("-fsyntax-only")
                && !args.iter().any(|a| a.contains("CMakeScratch") || a.contains("kache-cc-direct-probe") || a.starts_with("ztest"))
        })
        .count() as u64
}

/// Compiler shims that log one line per execution in a single write, so
/// parallel compiles cannot interleave their records.
fn write_shims(dir: &Path, tc: &crate::cc_oracle::exec::Toolchain) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    for real in [&tc.cc, &tc.cxx] {
        let script = format!(
            "#!/bin/sh\nline=$(printf '%s\\037' \"$PWD\" \"$@\")\nprintf '%s\\n' \"$line\" >> \"$CC_ORACLE_LOG\"\nexec '{}' \"$@\"\n",
            real.display().to_string().replace('\'', "'\\''")
        );
        let path = dir.join(real.file_name().context("compiler path has no name")?);
        std::fs::write(&path, script)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Replay every scenario for one arm. `shims` set means validation.
fn replay(
    cfg: &Config,
    workload: &Workload,
    arm: Arm,
    paths: &ArmPaths<'_>,
    shims: Option<&Path>,
    tc: &crate::cc_oracle::exec::Toolchain,
    validated: Option<&Replay>,
) -> Result<Replay> {
    let root = paths.root;
    let runtime = paths.runtime;
    let tree = root.join("tree");
    let moved = root.join("moved");
    let setup = arm_setup(cfg, arm, paths, &tree, shims, &tc.cc, &tc.cxx);
    for entry in std::fs::read_dir(runtime)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    let state = setup.store.parent().unwrap().to_path_buf();
    if state.exists() {
        std::fs::remove_dir_all(&state)?;
    }
    for sub in ["store", "home", "tmp"] {
        std::fs::create_dir_all(state.join(sub))?;
    }
    std::fs::write(state.join("ccache.conf"), CCACHE_CONF)?;
    std::fs::write(state.join("kache.toml"), "")?;
    let log = shims.map(|_| state.join("exec.log"));
    let daemon = match arm {
        Arm::Kache => Some(ReplayDaemon { kache: &cfg.kache, setup: &setup, armed: true }),
        _ => None,
    };
    if let Some(daemon) = &daemon {
        daemon.start().context("benchmark daemon preflight failed")?;
    }
    let mut env = setup.env.clone();
    if let Some(log) = &log {
        env.push(("CC_ORACLE_LOG".into(), log.display().to_string()));
    }
    workload.prepare(&tree, &setup.compiler, &cfg.archives, &env, cfg.jobs)?;
    let (source, header) = workload.edit_targets();
    let original_source = std::fs::read(tree.join(source))?;
    let original_header = std::fs::read(tree.join(header))?;
    let mut observations = Vec::new();
    for name in scenarios(workload) {
        let mut current = &setup;
        let relocated;
        let mut tree_now = tree.clone();
        let command = match name {
            "cold" => workload.build(&tree, &setup.compiler, cfg.jobs),
            "relocated" => {
                relocated = retarget(&setup, &tree, &moved);
                let mut env = relocated.env.clone();
                if let Some(log) = &log {
                    env.push(("CC_ORACLE_LOG".into(), log.display().to_string()));
                }
                workload.prepare(&moved, &relocated.compiler, &cfg.archives, &env, cfg.jobs)?;
                current = &relocated;
                tree_now = moved.clone();
                workload.build(&moved, &relocated.compiler, cfg.jobs)
            }
            "unit-daemon-start" | "unit-steady" => {
                if name == "unit-daemon-start" && let Some(daemon) = &daemon {
                    daemon.stop()?;
                }
                let (command, object) = workload.single_unit(&tree, &setup.compiler).unwrap();
                std::fs::remove_file(object)?;
                command
            }
            _ => {
                let (path, original) = if name.contains("source") { (source, &original_source) } else { (header, &original_header) };
                std::thread::sleep(MAKE_TICK);
                if name.starts_with("edit") {
                    std::fs::write(tree.join(path), workload::edited(original, if name.ends_with("immediate") { "immediate" } else { "settled" }))?;
                } else if name.starts_with("revert") {
                    std::fs::write(tree.join(path), original)?;
                }
                if name.ends_with("settled") {
                    std::thread::sleep(SETTLE);
                }
                workload.build(&tree, &setup.compiler, cfg.jobs)
            }
        };
        if name == "warm" {
            workload.clean(&tree, &env)?;
        }
        if let Some(log) = &log {
            std::fs::write(log, "")?;
        }
        let before = counters(arm, cfg, &setup)?;
        let command_line = format!("{command:?}");
        let activity_start = activity::Snapshot::take();
        let load_start = crate::bench_host::LoadSample::take();
        let (wall_ms, cpu_ms, max_rss_kib) = measure(&cfg.meter, command, &current.env, log.as_deref())?;
        let load = crate::bench_host::PhaseLoad::between(&load_start, &crate::bench_host::LoadSample::take());
        let activity = activity_start
            .zip(activity::Snapshot::take())
            .map(|(start, end)| start.until(&end, |c| cfg.is_ours(c), cpu_ms.round() as u64));
        if arm == Arm::Kache {
            require_daemon_ready(&cfg.kache, &setup)?;
            await_quiet_event_log(&setup.runtime.join("events.jsonl"));
        }
        let mut after = counters(arm, cfg, &setup)?;
        if let Some(expected) = validated.and_then(|v| v.observations.iter().find(|o| o.scenario == name)) {
            let deadline = Instant::now() + EVENT_DEADLINE;
            while delta(&after, &before).total() < expected.counters.total() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
                after = counters(arm, cfg, &setup)?;
            }
        }
        let (objects, object_count) = objects_digest(workload, &tree_now)?;
        observations.push(Observation {
            scenario: name.to_string(),
            wall_ms,
            cpu_ms,
            max_rss_kib,
            codegen: log.as_deref().map(codegen_count),
            counters: delta(&after, &before),
            objects,
            object_count,
            load,
            command: command_line,
            activity,
        });
    }
    if let Some(daemon) = daemon {
        daemon.finish()?;
    }
    let store_bytes = crate::disk_usage::measure(&setup.store, &[]).cache_bytes;
    Ok(Replay { arm, pair: None, admitted: true, rejection: None, store_bytes, observations })
}

/// What a validation replay must show for each arm.
fn validate(replays: &[Replay]) -> Vec<String> {
    let mut failures = Vec::new();
    let raw = replays.iter().find(|r| r.arm == Arm::Raw).unwrap();
    for replay in replays {
        for (obs, reference) in replay.observations.iter().zip(&raw.observations) {
            let at = format!("{} {}", replay.arm.as_str(), obs.scenario);
            if obs.objects != reference.objects {
                failures.push(format!("{at}: objects differ from raw"));
            }
            if obs.object_count == 0 {
                failures.push(format!("{at}: the build made no objects"));
            }
            let codegen = obs.codegen.unwrap_or(0);
            match (replay.arm, obs.scenario.as_str()) {
                (_, "cold") if codegen == 0 => failures.push(format!("{at}: nothing compiled")),
                (Arm::Ccache | Arm::Kache, "warm" | "revert-source-settled" | "revert-header-settled" | "relocated" | "unit-steady") if codegen != 0 => {
                    failures.push(format!("{at}: {codegen} compiles where a restore was expected"))
                }
                (Arm::Ccache | Arm::Kache, s) if s.starts_with("edit") && codegen == 0 => {
                    failures.push(format!("{at}: an edit compiled nothing"))
                }
                _ => {}
            }
        }
    }
    failures
}

pub fn run(cfg: &Config) -> Result<Report> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let tc = crate::cc_oracle::exec::resolve_toolchain(&cfg.toolchain, &path).map_err(anyhow::Error::msg)?;
    let version = |p: &Path| {
        Command::new(p)
            .arg("--version")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string())
            .unwrap_or_default()
    };
    std::fs::create_dir_all(&cfg.out)?;
    let out = std::fs::canonicalize(&cfg.out)?;
    let cfg = &Config { out: out.clone(), ..cfg.clone() };
    let preflight = preflight(cfg);
    let mut report = Report {
        mode: if cfg.acceptance { "acceptance" } else { "diagnostic" },
        resource_accounting: RESOURCE_ACCOUNTING,
        provenance: provenance(cfg, &tc),
        preflight: preflight.clone(),
        toolchain: cfg.toolchain.clone(),
        compiler: version(&tc.cc),
        kache: version(&cfg.kache),
        ccache: version(&cfg.ccache),
        host: crate::bench_host::HostInfo::collect(&out),
        seed: cfg.seed,
        jobs: cfg.jobs,
        workloads: Vec::new(),
    };
    let mut rng = stats::Rng::new(cfg.seed);
    for workload in workload::all().into_iter().filter(|w| cfg.workloads.is_empty() || cfg.workloads.iter().any(|n| n == w.name)) {
        let root = out.join(workload.name);
        let runtimes = arm_runtimes()?;
        let shims = root.join("shims");
        write_shims(&shims, &tc)?;
        eprintln!("=> {} validation", workload.name);
        let validation = Arm::ALL
            .iter()
            .map(|&arm| replay(cfg, &workload, arm, &ArmPaths { root: &root, runtime: runtimes[&arm].path() }, Some(&shims), &tc, None))
            .collect::<Result<Vec<_>>>()?;
        let validation_failures = validate(&validation);
        let mut timed = Vec::new();
        let host_quiet = !cfg.acceptance || preflight.as_ref().is_some_and(|a| a.eligible);
        if validation_failures.is_empty() && !cfg.validate_only && host_quiet {
            let limit = if cfg.acceptance { stats::MAX_PAIRS } else { cfg.pairs };
            for pair in 0..limit {
                // A busy host cannot reach acceptance, so stop timing on it.
                if cfg.acceptance && (!needs_another_pair(&workload, &timed) || !replays_eligible(&timed)) {
                    break;
                }
                let mut order = Arm::ALL;
                rng.shuffle(&mut order);
                eprintln!("=> {} pair {} ({})", workload.name, pair + 1, order.map(Arm::as_str).join(", "));
                for arm in order {
                    let validated = validation.iter().find(|v| v.arm == arm);
                    let mut r = replay(cfg, &workload, arm, &ArmPaths { root: &root, runtime: runtimes[&arm].path() }, None, &tc, validated)?;
                    r.pair = Some(pair);
                    admit(&mut r, &validation);
                    timed.push(r);
                }
            }
        }
        let ratios = ratios(&workload, &timed);
        let acceptance = cfg.acceptance.then(|| accept(&workload, &validation_failures, &timed, &ratios, preflight.as_ref()));
        let environments = Arm::ALL
            .iter()
            .map(|&arm| {
                let setup = arm_setup(cfg, arm, &ArmPaths { root: &root, runtime: runtimes[&arm].path() }, &root.join("tree"), None, &tc.cc, &tc.cxx);
                (arm.as_str().to_string(), setup.env)
            })
            .collect();
        report.workloads.push(WorkloadReport {
            workload: workload.name.to_string(),
            units: workload.units,
            input_digest: input_digest(&workload, &cfg.archives, &root.join("tree")),
            environments,
            acceptance,
            validation,
            validation_failures,
            timed,
            ratios,
        });
    }
    Ok(report)
}

/// Tools the workloads run by name besides the compilers.
const TOOLS: [&str; 6] = ["make", "cmake", "ninja", "tar", "xz", "python3"];

/// `PATH` for every arm: the directory where `inherited` first finds each of
/// [`TOOLS`], then the system directories. The arms do not search the caller's
/// whole `PATH`, so a stray tool there cannot change a build, and the report
/// records only the directories the tools live in.
pub fn declared_path(inherited: &std::ffi::OsStr) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mut dirs: Vec<PathBuf> = Vec::new();
    for tool in TOOLS {
        let found = std::env::split_paths(inherited).find(|dir| {
            std::fs::metadata(dir.join(tool)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        });
        if let Some(dir) = found.filter(|dir| !dirs.contains(dir)) {
            dirs.push(dir);
        }
    }
    for system in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        if !dirs.iter().any(|d| d == Path::new(system)) {
            dirs.push(system.into());
        }
    }
    std::env::join_paths(dirs).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Unrelated activity over a few seconds before anything is timed, so a busy
/// host is turned away before a long run rather than after it.
fn preflight(cfg: &Config) -> Option<activity::Activity> {
    let start = activity::Snapshot::take()?;
    std::thread::sleep(Duration::from_secs(5));
    Some(start.until(&activity::Snapshot::take()?, |c| cfg.is_ours(c), 0))
}

/// Pairs whose three arms were all admitted.
fn complete_pairs(timed: &[Replay]) -> BTreeMap<usize, BTreeMap<Arm, &Replay>> {
    let mut pairs: BTreeMap<usize, BTreeMap<Arm, &Replay>> = BTreeMap::new();
    for r in timed.iter().filter(|r| r.admitted) {
        pairs.entry(r.pair.unwrap()).or_default().insert(r.arm, r);
    }
    pairs.retain(|_, p| p.len() == Arm::ALL.len());
    pairs
}

/// Acceptance runs another pair until five are complete, then only while a
/// target's interval still crosses it.
fn needs_another_pair(workload: &Workload, timed: &[Replay]) -> bool {
    complete_pairs(timed).len() < stats::MIN_PAIRS
        || ratios(workload, timed).iter().any(|r| r.verdict == Some(stats::Verdict::Inconclusive))
}

/// Every timed observation's activity was read and was quiet.
fn replays_eligible(timed: &[Replay]) -> bool {
    timed.iter().all(|r| replay_unrelated_cores(r).is_some_and(|cores| cores <= activity::MAX_UNRELATED_CORES))
}

/// Unrelated CPU over a replay's timed builds taken together. A warm build
/// lasts a fraction of a second, where the file-system and log daemons
/// reacting to the build's own writes would read as most of a core, so
/// one build alone is too short a window to judge. `None` when any build's
/// activity could not be read.
fn replay_unrelated_cores(replay: &Replay) -> Option<f64> {
    let (mut cpu_ms, mut wall_ms) = (0u64, 0.0);
    for activity in replay.observations.iter().map(|o| o.activity.as_ref()) {
        let activity = activity?;
        cpu_ms += activity.unrelated_cpu_ms;
        wall_ms += activity.wall_ms;
    }
    (wall_ms > 0.0).then_some(cpu_ms as f64 / wall_ms)
}

fn accept(
    workload: &Workload,
    validation_failures: &[String],
    timed: &[Replay],
    ratios: &[RatioRow],
    preflight: Option<&activity::Activity>,
) -> Acceptance {
    let complete_pairs = complete_pairs(timed).len();
    let eligible = preflight.is_some_and(|a| a.eligible) && !timed.is_empty() && replays_eligible(timed);
    let mut reasons = Vec::new();
    if !validation_failures.is_empty() {
        reasons.push(format!("{} validation failures", validation_failures.len()));
    }
    if !eligible {
        reasons.push(format!(
            "host ineligible: unrelated processes above {} cores, or activity unreadable",
            activity::MAX_UNRELATED_CORES
        ));
    }
    if complete_pairs < stats::MIN_PAIRS {
        reasons.push(format!("{complete_pairs} complete admitted pairs, {} required", stats::MIN_PAIRS));
    }
    let targeted: Vec<&RatioRow> = ratios.iter().filter(|r| r.target.is_some()).collect();
    for row in &targeted {
        if row.verdict != Some(stats::Verdict::Pass) {
            reasons.push(format!(
                "{} vs {}: {:?} (geomean {:.3}, target {})",
                row.scenario,
                row.against.as_str(),
                row.verdict,
                row.summary.geomean,
                row.target.unwrap()
            ));
        }
    }
    let expected = scenarios(workload)
        .iter()
        .flat_map(|s| [Arm::Ccache, Arm::Raw].map(|a| target_for(s, a)))
        .filter(Option::is_some)
        .count();
    let verdict = if !eligible {
        AcceptanceVerdict::Ineligible
    } else if !validation_failures.is_empty() || complete_pairs < stats::MIN_PAIRS || targeted.len() < expected {
        AcceptanceVerdict::Undersampled
    } else if targeted.iter().any(|r| r.verdict == Some(stats::Verdict::Fail)) {
        AcceptanceVerdict::Fail
    } else if targeted.iter().all(|r| r.verdict == Some(stats::Verdict::Pass)) {
        AcceptanceVerdict::Pass
    } else {
        AcceptanceVerdict::Inconclusive
    };
    Acceptance { complete_pairs, eligible, verdict, reasons }
}

fn digest_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}

/// The pinned archive's digest, or for a generated corpus the digest of its
/// sources as prepared.
fn input_digest(workload: &Workload, archives: &Archives, tree: &Path) -> Option<String> {
    match workload.kind {
        workload::Kind::Zlib => archives.zlib.as_deref().and_then(digest_file),
        workload::Kind::Llvm => archives.llvm.as_deref().and_then(digest_file),
        workload::Kind::CorpusC | workload::Kind::CorpusCxx => {
            let mut names: Vec<PathBuf> = std::fs::read_dir(tree)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| ["c", "h", "cpp", "hpp"].iter().any(|x| e == *x)) || p.ends_with("Makefile"))
                .collect();
            names.sort();
            let mut hasher = blake3::Hasher::new();
            for path in names {
                hasher.update(path.file_name()?.as_encoded_bytes());
                hasher.update(b"\0");
                hasher.update(blake3::hash(&std::fs::read(&path).ok()?).as_bytes());
            }
            Some(hasher.finalize().to_hex().to_string())
        }
    }
}

fn provenance(cfg: &Config, tc: &crate::cc_oracle::exec::Toolchain) -> Provenance {
    let git = |args: &[&str]| {
        Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let engine = cfg.kache.with_file_name("kache-engine");
    let binaries = [
        ("kache", cfg.kache.clone()),
        ("kache-engine", engine),
        ("ccache", cfg.ccache.clone()),
        ("cc", tc.cc.clone()),
        ("cxx", tc.cxx.clone()),
        ("kache-cc-bench", cfg.meter.clone()),
    ]
    .into_iter()
    .filter_map(|(name, path)| {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        digest_file(&path).map(|d| (name.to_string(), format!("{} {d}", path.display())))
    })
    .collect();
    Provenance {
        source_revision: git(&["rev-parse", "HEAD"]),
        source_dirty: git(&["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty()),
        binaries,
        configuration: [("kache.toml", ""), ("ccache.conf", CCACHE_CONF), ("KACHE_HOST_CONFIG", "")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

/// A timed replay is admitted only if its counters and objects are the
/// validated ones, scenario by scenario.
fn admit(replay: &mut Replay, validation: &[Replay]) {
    let reference = validation.iter().find(|v| v.arm == replay.arm).unwrap();
    for (obs, valid) in replay.observations.iter().zip(&reference.observations) {
        let why = if obs.counters != valid.counters {
            Some(format!("{}: counters {:?} where validation recorded {:?}", obs.scenario, obs.counters, valid.counters))
        } else if obs.objects != valid.objects {
            Some(format!("{}: objects differ from the validated build", obs.scenario))
        } else {
            None
        };
        if let Some(why) = why {
            replay.admitted = false;
            replay.rejection = Some(why);
            return;
        }
    }
}

fn target_for(scenario: &str, against: Arm) -> Option<f64> {
    match (scenario, against) {
        ("cold", Arm::Ccache) => Some(1.10),
        ("cold", Arm::Raw) => Some(1.15),
        ("warm", Arm::Ccache) => Some(1.15),
        ("relocated", Arm::Ccache) => Some(1.15),
        (s, Arm::Ccache) if s.starts_with("edit") || s.starts_with("revert") => Some(1.15),
        _ => None,
    }
}

fn ratios(workload: &Workload, timed: &[Replay]) -> Vec<RatioRow> {
    let mut rows = Vec::new();
    let pairs = complete_pairs(timed);
    let complete: Vec<&BTreeMap<Arm, &Replay>> = pairs.values().collect();
    for scenario in scenarios(workload) {
        for against in [Arm::Ccache, Arm::Raw] {
            let wall = |arm: Arm| -> Vec<f64> {
                complete
                    .iter()
                    .filter_map(|p| p[&arm].observations.iter().find(|o| o.scenario == scenario).map(|o| o.wall_ms))
                    .collect()
            };
            let (k, o) = (wall(Arm::Kache), wall(against));
            let Some(summary) = stats::paired_ratio(&k, &o) else { continue };
            let target = target_for(scenario, against);
            rows.push(RatioRow {
                scenario: scenario.to_string(),
                against,
                verdict: target.map(|t| stats::verdict(&summary, t)),
                summary,
                target,
                kache_median_ms: stats::median(&k),
                other_median_ms: stats::median(&o),
            });
        }
    }
    rows
}

/// The human summary written beside `report.json`.
pub fn markdown(report: &Report) -> String {
    let mut s = format!(
        "# cc-bench {} ({})\n\n- source: {}{}\n- compiler: {}\n- kache: {}\n- ccache: {}\n- host: {} {} CPUs, {}\n- jobs: {}, seed: {}\n- CPU and RSS: {}\n{}\n",
        report.toolchain,
        report.mode,
        report.provenance.source_revision.as_deref().unwrap_or("unknown revision"),
        if report.provenance.source_dirty == Some(true) { " (with uncommitted changes)" } else { "" },
        report.compiler,
        report.kache,
        report.ccache,
        report.host.cpu_model.as_deref().unwrap_or("unknown CPU"),
        report.host.logical_cpus.map_or("?".into(), |n| n.to_string()),
        report.host.os,
        report.jobs,
        report.seed,
        RESOURCE_ACCOUNTING,
        report.preflight.as_ref().map_or("- preflight: host activity unreadable\n".to_string(), |a| format!(
            "- preflight: unrelated processes used {:.2} cores over {:.1} s ({})\n",
            a.unrelated_cores,
            a.wall_ms / 1000.0,
            if a.eligible { "eligible" } else { "ineligible" }
        ))
    );
    for w in &report.workloads {
        s.push_str(&format!("## {}\n\n", w.workload));
        if let Some(a) = &w.acceptance {
            s.push_str(&format!("Acceptance: **{}** with {} complete admitted pairs.\n\n", a.verdict.label(), a.complete_pairs));
            for reason in &a.reasons {
                s.push_str(&format!("- {reason}\n"));
            }
            s.push('\n');
        }
        let busiest = w
            .timed
            .iter()
            .flat_map(|r| &r.observations)
            .filter_map(|o| o.activity.as_ref())
            .max_by(|a, b| a.unrelated_cores.total_cmp(&b.unrelated_cores));
        if let Some(a) = busiest {
            s.push_str(&format!(
                "Most unrelated activity around one timed build: {:.2} cores{}.\n\n",
                a.unrelated_cores,
                a.busiest.first().map_or(String::new(), |b| format!(", busiest `{}`", b.name))
            ));
        }
        if !w.validation_failures.is_empty() {
            s.push_str("Validation FAILED; nothing was timed:\n\n");
            for f in &w.validation_failures {
                s.push_str(&format!("- {f}\n"));
            }
            s.push('\n');
        }
        s.push_str("Validation (untimed, through compiler shims): code generations, then cache hits/misses/other per scenario.\n\n| scenario | raw | ccache | kache |\n|---|---|---|---|\n");
        if let Some(first) = w.validation.first() {
            for (i, obs) in first.observations.iter().enumerate() {
                let cell = |arm: Arm| {
                    w.validation.iter().find(|r| r.arm == arm).and_then(|r| r.observations.get(i)).map_or("-".into(), |o| {
                        format!("{} cg, {}h/{}m/{}o", o.codegen.unwrap_or(0), o.counters.hits, o.counters.misses, o.counters.other)
                    })
                };
                s.push_str(&format!("| {} | {} | {} | {} |\n", obs.scenario, cell(Arm::Raw), cell(Arm::Ccache), cell(Arm::Kache)));
            }
        }
        let rejected: Vec<_> = w.timed.iter().filter(|r| !r.admitted).collect();
        s.push_str(&format!("\nTimed replays: {} admitted, {} rejected.\n", w.timed.len() - rejected.len(), rejected.len()));
        let loads: Vec<f64> = w
            .timed
            .iter()
            .flat_map(|r| &r.observations)
            .filter_map(|o| o.load.loadavg_start.map(|l| l[0]))
            .collect();
        if let (Some(low), Some(high)) = (loads.iter().cloned().reduce(f64::min), loads.iter().cloned().reduce(f64::max)) {
            s.push_str(&format!("Host 1-minute load at the start of timed builds: {low:.2} to {high:.2}.\n"));
        }
        for r in rejected {
            s.push_str(&format!("- rejected {} pair {}: {}\n", r.arm.as_str(), r.pair.unwrap_or(0) + 1, r.rejection.as_deref().unwrap_or("")));
        }
        if !w.ratios.is_empty() {
            s.push_str("\n| scenario | kache / | pairs | geomean | 95% CI | target | verdict | kache median ms | other median ms |\n|---|---|---|---|---|---|---|---|---|\n");
            for r in &w.ratios {
                let ci = r.summary.ci95.map_or("-".into(), |[a, b]| format!("{a:.3}..{b:.3}"));
                s.push_str(&format!(
                    "| {} | {} | {} | {:.3} | {} | {} | {} | {:.1} | {:.1} |\n",
                    r.scenario,
                    r.against.as_str(),
                    r.summary.pairs,
                    r.summary.geomean,
                    ci,
                    r.target.map_or("-".into(), |t| format!("{t}")),
                    r.verdict.map_or("-".into(), |v| format!("{v:?}").to_uppercase()),
                    r.kache_median_ms.unwrap_or(f64::NAN),
                    r.other_median_ms.unwrap_or(f64::NAN),
                ));
            }
            if w.units > 0 {
                s.push_str(&format!("\nPer unit ({} units): divide the medians by {}.\n", w.units, w.units));
            }
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon_config(kache: &Path) -> Config {
        Config {
            kache: kache.to_path_buf(),
            ccache: "/unused/ccache".into(),
            toolchain: "cc".into(),
            out: "/unused/out".into(),
            workloads: vec!["corpus-cxx-headers".into()],
            pairs: 5,
            seed: 1,
            validate_only: false,
            acceptance: false,
            archives: Archives::default(),
            jobs: 4,
            meter: "/unused/meter".into(),
            path: "/usr/bin:/bin".into(),
        }
    }

    fn fake_daemon(root: &Path, start: &str, status: &str) -> PathBuf {
        let executable = root.join("kache");
        let script = format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n'daemon start') {start} ;;\n'daemon stop') printf stopped > \"$KACHE_RUNTIME_DIR/stopped\" ;;\n'--json daemon') printf '%s' '{status}' ;;\nesac\n"
        );
        assert!(Command::new("/bin/sh")
            .args(["-c", "printf '%s' \"$1\" > \"$2\"", "sh"])
            .arg(script).arg(&executable).status().unwrap().success());
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        executable
    }

    #[test]
    fn runtimes_are_short_private_and_isolated() {
        let runtimes = arm_runtimes().unwrap();
        let mut paths = std::collections::BTreeSet::new();
        for runtime in runtimes.values() {
            assert!(paths.insert(runtime.path().to_path_buf()));
            assert!(runtime.path().join("daemon.sock").as_os_str().len() < 80);
            assert_eq!(std::fs::metadata(runtime.path()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert_eq!(paths.len(), Arm::ALL.len());
    }

    #[test]
    fn failed_start_or_readiness_aborts_replay_before_preparation() {
        let cases = [
            ("printf 'socket path is too long' >&2; exit 42", "", "42", "socket path is too long"),
            (":", "", "reading benchmark daemon readiness", ""),
            (":", "not-json", "reading benchmark daemon readiness", ""),
            (":", r#"{"success":true,"daemon_running":false}"#, "not ready", ""),
        ];
        for (start, status, expected, stderr) in cases {
            let dir = tempfile::tempdir_in("/tmp").unwrap();
            let kache = fake_daemon(dir.path(), start, status);
            let cfg = daemon_config(&kache);
            let runtime = tempfile::tempdir_in("/tmp").unwrap();
            let root = dir.path().join("workload");
            let tc = crate::cc_oracle::exec::Toolchain {
                name: "unused".into(), family: None, cc_name: "cc".into(), cxx_name: "c++".into(),
                cc: "/unused/cc".into(), cxx: "/unused/c++".into(),
            };
            let workload = workload::all().into_iter().find(|w| w.name == "corpus-cxx-headers").unwrap();
            let error = replay(&cfg, &workload, Arm::Kache,
                &ArmPaths { root: &root, runtime: runtime.path() }, None, &tc, None).unwrap_err();
            let error = format!("{error:#}");
            assert!(error.contains("benchmark daemon preflight failed"), "{error}");
            assert!(error.contains(expected) && error.contains(stderr), "{error}");
            assert!(!root.join("tree/common.hpp").exists());
            assert_eq!(std::fs::read_to_string(runtime.path().join("stopped")).unwrap(), "stopped");
        }
    }

    #[test]
    fn readiness_requires_a_reply_from_the_current_daemon_build() {
        for (status, ready) in [
            (r#"{"success":true,"daemon_running":true,"version":"v","daemon_version":"v","epoch":7,"daemon_epoch":7}"#, true),
            (r#"{"success":true,"daemon_running":true}"#, false),
            (r#"{"success":true,"daemon_running":true,"version":"v","daemon_version":"old","epoch":7,"daemon_epoch":7}"#, false),
            (r#"{"success":true,"daemon_running":true,"version":"v","daemon_version":"v","epoch":7,"daemon_epoch":6}"#, false),
        ] {
            let dir = tempfile::tempdir_in("/tmp").unwrap();
            let kache = fake_daemon(dir.path(), ":", status);
            let cfg = daemon_config(&kache);
            let setup = arm_setup(&cfg, Arm::Kache,
                &ArmPaths { root: dir.path(), runtime: dir.path() }, dir.path(), None, Path::new("/unused/cc"), Path::new("/unused/c++"));
            assert_eq!(require_daemon_ready(&kache, &setup).is_ok(), ready, "{status}");
        }
    }

    #[test]
    fn relocated_replays_count_runtime_events_and_preserve_them_after_shutdown() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let runtime = tempfile::tempdir_in("/tmp").unwrap();
        let kache = fake_daemon(dir.path(), ":", "");
        let cfg = daemon_config(&kache);
        let tree = dir.path().join("tree");
        let setup = arm_setup(&cfg, Arm::Kache,
            &ArmPaths { root: dir.path(), runtime: runtime.path() }, &tree, None, Path::new("/unused/cc"), Path::new("/unused/c++"));
        std::fs::create_dir_all(&setup.store).unwrap();
        assert_eq!(counters(Arm::Kache, &cfg, &setup).unwrap().total(), 0);
        let events = "{\"result\":\"local_hit\"}\n{\"result\":\"miss\"}\n";
        std::fs::write(runtime.path().join("events.jsonl"), events).unwrap();
        let moved = retarget(&setup, &tree, &dir.path().join("moved"));
        assert_eq!(moved.runtime, setup.runtime);
        assert_eq!(moved.store, setup.store);
        assert_eq!(moved.env.iter().find(|(k, _)| k == "KACHE_RUNTIME_DIR"), setup.env.iter().find(|(k, _)| k == "KACHE_RUNTIME_DIR"));
        let counts = counters(Arm::Kache, &cfg, &moved).unwrap();
        assert_eq!((counts.hits, counts.misses, counts.other), (1, 1, 0));
        ReplayDaemon { kache: &kache, setup: &setup, armed: true }.finish().unwrap();
        assert_eq!(std::fs::read_to_string(setup.store.join("events.jsonl")).unwrap(), events);
    }

    fn workload() -> Workload {
        workload::all().into_iter().find(|w| w.name == "zlib").unwrap()
    }

    fn quiet() -> activity::Activity {
        activity::Activity {
            wall_ms: 5000.0,
            unrelated_cpu_ms: 0,
            unrelated_system_cpu_ms: Some(0),
            unrelated_process_cpu_ms: 0,
            unrelated_cores: 0.0,
            busiest: Vec::new(),
            eligible: true,
        }
    }

    /// Timed replays where kache takes `kache_ms` of each pair's per-scenario
    /// times and the other arms 100 ms.
    fn timed(kache_ms: &[f64], activity: Option<activity::Activity>) -> Vec<Replay> {
        let mut replays = Vec::new();
        for (pair, &k) in kache_ms.iter().enumerate() {
            for arm in Arm::ALL {
                let observations = scenarios(&workload())
                    .into_iter()
                    .map(|scenario| Observation {
                        scenario: scenario.to_string(),
                        wall_ms: if arm == Arm::Kache { k } else { 100.0 },
                        cpu_ms: 0.0,
                        max_rss_kib: 0,
                        codegen: None,
                        counters: Counters::default(),
                        objects: String::new(),
                        object_count: 1,
                        load: Default::default(),
                        command: String::new(),
                        activity: activity.clone(),
                    })
                    .collect();
                replays.push(Replay { arm, pair: Some(pair), admitted: true, rejection: None, store_bytes: 0, observations });
            }
        }
        replays
    }

    fn verdict(kache_ms: &[f64], activity: Option<activity::Activity>) -> AcceptanceVerdict {
        let t = timed(kache_ms, activity);
        accept(&workload(), &[], &t, &ratios(&workload(), &t), Some(&quiet())).verdict
    }

    #[test]
    fn five_quiet_pairs_under_target_pass() {
        assert_eq!(verdict(&[100.0, 101.0, 99.0, 100.5, 100.2], Some(quiet())), AcceptanceVerdict::Pass);
    }

    #[test]
    fn four_pairs_are_undersampled_even_when_fast() {
        assert_eq!(verdict(&[90.0, 91.0, 90.5, 90.2], Some(quiet())), AcceptanceVerdict::Undersampled);
    }

    #[test]
    fn a_rejected_arm_leaves_its_pair_incomplete() {
        let mut t = timed(&[100.0, 101.0, 99.0, 100.5, 100.2], Some(quiet()));
        t[0].admitted = false;
        let a = accept(&workload(), &[], &t, &ratios(&workload(), &t), Some(&quiet()));
        assert_eq!((a.verdict, a.complete_pairs), (AcceptanceVerdict::Undersampled, 4));
    }

    #[test]
    fn a_clear_miss_fails_and_a_crossing_interval_is_inconclusive() {
        assert_eq!(verdict(&[150.0, 151.0, 149.0, 150.5, 150.2], Some(quiet())), AcceptanceVerdict::Fail);
        assert_eq!(verdict(&[100.0, 130.0, 105.0, 125.0, 110.0], Some(quiet())), AcceptanceVerdict::Inconclusive);
    }

    #[test]
    fn busy_or_unreadable_activity_is_ineligible_whatever_the_timings() {
        let busy = activity::Activity { unrelated_cpu_ms: 10_000, unrelated_cores: 2.0, eligible: false, ..quiet() };
        assert_eq!(verdict(&[100.0, 101.0, 99.0, 100.5, 100.2], Some(busy)), AcceptanceVerdict::Ineligible);
        assert_eq!(verdict(&[100.0, 101.0, 99.0, 100.5, 100.2], None), AcceptanceVerdict::Ineligible);
        let t = timed(&[100.0, 101.0, 99.0, 100.5, 100.2], Some(quiet()));
        let loud = activity::Activity { eligible: false, ..quiet() };
        assert_eq!(accept(&workload(), &[], &t, &ratios(&workload(), &t), Some(&loud)).verdict, AcceptanceVerdict::Ineligible);
    }

    #[test]
    fn eligibility_is_judged_over_a_replays_builds_together() {
        let mut t = timed(&[100.0, 101.0, 99.0, 100.5, 100.2], Some(quiet()));
        // One short build at 0.8 cores among quiet ones is not heavy work.
        t[0].observations[1].activity = Some(activity::Activity { wall_ms: 300.0, unrelated_cpu_ms: 240, unrelated_cores: 0.8, eligible: false, ..quiet() });
        assert!(replays_eligible(&t));
        for o in &mut t[0].observations {
            o.activity = Some(activity::Activity { unrelated_cpu_ms: 4000, unrelated_cores: 0.8, eligible: false, ..quiet() });
        }
        assert!(!replays_eligible(&t));
        t[0].observations[0].activity = None;
        assert!(replay_unrelated_cores(&t[0]).is_none());
    }

    #[test]
    fn more_pairs_only_while_short_of_five_or_a_target_is_crossed() {
        assert!(needs_another_pair(&workload(), &timed(&[100.0; 4], Some(quiet()))));
        assert!(!needs_another_pair(&workload(), &timed(&[100.0, 101.0, 99.0, 100.5, 100.2], Some(quiet()))));
        assert!(needs_another_pair(&workload(), &timed(&[100.0, 130.0, 105.0, 125.0, 110.0], Some(quiet()))));
    }

    #[test]
    fn the_declared_path_holds_only_tool_and_system_directories() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let (tools, other) = (temp.path().join("tools"), temp.path().join("private-stuff"));
        for dir in [&tools, &other] {
            std::fs::create_dir(dir).unwrap();
        }
        for tool in ["ninja", "cmake"] {
            std::fs::write(tools.join(tool), "").unwrap();
            std::fs::set_permissions(tools.join(tool), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(other.join("make"), "").unwrap();
        let inherited = std::env::join_paths([&other, &tools, &PathBuf::from("/usr/bin")]).unwrap();
        let path = declared_path(&inherited);
        assert!(path.split(':').any(|d| d == tools.display().to_string()), "{path}");
        assert!(!path.contains("private-stuff"), "a non-executable `make` is not a tool: {path}");
        assert_eq!(path.matches("/usr/bin").count(), 1, "{path}");
    }

    #[test]
    fn every_arm_environment_is_declared_whole() {
        let cfg = Config {
            kache: "/opt/k/kache".into(),
            ccache: "/opt/c/ccache".into(),
            toolchain: "cc".into(),
            out: "/out".into(),
            workloads: Vec::new(),
            pairs: 5,
            seed: 1,
            validate_only: false,
            acceptance: true,
            archives: Archives::default(),
            jobs: 1,
            meter: "/opt/k/kache-cc-bench".into(),
            path: "/usr/bin:/bin".into(),
        };
        for arm in Arm::ALL {
            let setup = arm_setup(&cfg, arm, &ArmPaths { root: Path::new("/out/w"), runtime: Path::new("/tmp/runtime") }, Path::new("/out/w/tree"), None, Path::new("/usr/bin/cc"), Path::new("/usr/bin/c++"));
            let names: Vec<&str> = setup.env.iter().map(|(k, _)| k.as_str()).collect();
            assert!(names.contains(&"PATH") && names.contains(&"HOME") && names.contains(&"TMPDIR"), "{arm:?} {names:?}");
            assert_eq!(setup.env.iter().find(|(k, _)| k == "PATH").unwrap().1, "/usr/bin:/bin");
        }
        let kache = arm_setup(&cfg, Arm::Kache, &ArmPaths { root: Path::new("/out/w"), runtime: Path::new("/tmp/runtime") }, Path::new("/out/w/tree"), None, Path::new("/usr/bin/cc"), Path::new("/usr/bin/c++"));
        assert!(kache.env.iter().any(|(k, v)| k == "KACHE_HOST_CONFIG" && v.is_empty()));
        assert!(cfg.is_ours("/opt/k/kache-engine daemon run"));
        assert!(cfg.is_ours("make -C /out/w/tree"));
        assert!(!cfg.is_ours("/usr/bin/indexer"));
    }
}
