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

pub mod stats;
pub mod workload;

use anyhow::{Context, Result, bail};
use deser::Serialize;
use std::collections::BTreeMap;
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
    pub archives: Archives,
    pub jobs: usize,
    /// This binary, re-run as the per-build resource meter.
    pub meter: PathBuf,
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
}

/// Cache results a scenario recorded: kache's event log, ccache's stats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counters {
    pub hits: u64,
    pub misses: u64,
    pub other: u64,
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

#[derive(Debug, Clone, Serialize)]
pub struct Report {
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

fn arm_setup(cfg: &Config, arm: Arm, root: &Path, tree: &Path, compiler_dir: Option<&Path>, cc: &Path, cxx: &Path) -> ArmSetup {
    let state = root.join("state").join(arm.as_str());
    let store = state.join("store");
    let cc = compiler_dir.map_or_else(|| cc.to_path_buf(), |dir| dir.join(cc.file_name().unwrap()));
    let cxx = compiler_dir.map_or_else(|| cxx.to_path_buf(), |dir| dir.join(cxx.file_name().unwrap()));
    // kache's default profile maps KACHE_BASE_DIR and pins the epoch; the
    // other arms get the same effective flags and environment.
    let aligned = vec![format!("-ffile-prefix-map={}=/kache/base-dir", tree.display())];
    let base_dir = tree.parent().unwrap().display().to_string();
    let mut env = vec![
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
            env.push(("KACHE_CONFIG".into(), state.join("kache.toml").display().to_string()));
            env.push(("KACHE_BASE_DIR".into(), tree.display().to_string()));
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
    }
}

/// Run `command` under the meter: wall time here, CPU and peak RSS of its
/// process tree from the meter's own child accounting.
fn measure(meter: &Path, command: Command, env: &[(String, String)], log: Option<&Path>) -> Result<(f64, f64, u64)> {
    let report = tempfile::NamedTempFile::new()?;
    let mut metered = Command::new(meter);
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
            let Ok(text) = std::fs::read_to_string(setup.store.join("events.jsonl")) else {
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
fn replay(cfg: &Config, workload: &Workload, arm: Arm, root: &Path, shims: Option<&Path>, tc: &crate::cc_oracle::exec::Toolchain) -> Result<Replay> {
    let tree = root.join("tree");
    let moved = root.join("moved");
    let setup = arm_setup(cfg, arm, root, &tree, shims, &tc.cc, &tc.cxx);
    let state = setup.store.parent().unwrap().to_path_buf();
    if state.exists() {
        std::fs::remove_dir_all(&state)?;
    }
    for sub in ["store", "home", "tmp"] {
        std::fs::create_dir_all(state.join(sub))?;
    }
    std::fs::write(state.join("ccache.conf"), "max_size = 20G\n")?;
    std::fs::write(state.join("kache.toml"), "")?;
    let log = shims.map(|_| state.join("exec.log"));
    let kache_daemon = |verb: &str| -> Result<()> {
        if arm == Arm::Kache {
            Command::new(&cfg.kache).args(["daemon", verb]).envs(setup.env.iter().map(|(k, v)| (k, v))).output()?;
        }
        Ok(())
    };
    let mut env = setup.env.clone();
    if let Some(log) = &log {
        env.push(("CC_ORACLE_LOG".into(), log.display().to_string()));
    }
    workload.prepare(&tree, &setup.compiler, &cfg.archives, &env, cfg.jobs)?;
    kache_daemon("start")?;
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
                if name == "unit-daemon-start" {
                    kache_daemon("stop")?;
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
        let (wall_ms, cpu_ms, max_rss_kib) = measure(&cfg.meter, command, &current.env, log.as_deref())?;
        if arm == Arm::Kache {
            await_quiet_event_log(&setup.store.join("events.jsonl"));
        }
        let after = counters(arm, cfg, &setup)?;
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
        });
    }
    kache_daemon("stop")?;
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
    let mut report = Report {
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
        let shims = root.join("shims");
        write_shims(&shims, &tc)?;
        eprintln!("=> {} validation", workload.name);
        let validation = Arm::ALL
            .iter()
            .map(|&arm| replay(cfg, &workload, arm, &root, Some(&shims), &tc))
            .collect::<Result<Vec<_>>>()?;
        let validation_failures = validate(&validation);
        let mut timed = Vec::new();
        if validation_failures.is_empty() && !cfg.validate_only {
            for pair in 0..cfg.pairs {
                let mut order = Arm::ALL;
                rng.shuffle(&mut order);
                eprintln!("=> {} pair {} ({})", workload.name, pair + 1, order.map(Arm::as_str).join(", "));
                for arm in order {
                    let mut r = replay(cfg, &workload, arm, &root, None, &tc)?;
                    r.pair = Some(pair);
                    admit(&mut r, &validation);
                    timed.push(r);
                }
            }
        }
        let ratios = ratios(&workload, &timed);
        report.workloads.push(WorkloadReport {
            workload: workload.name.to_string(),
            units: workload.units,
            validation,
            validation_failures,
            timed,
            ratios,
        });
    }
    Ok(report)
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
    let pairs: BTreeMap<usize, BTreeMap<Arm, &Replay>> = timed.iter().filter(|r| r.admitted).fold(BTreeMap::new(), |mut m, r| {
        m.entry(r.pair.unwrap()).or_default().insert(r.arm, r);
        m
    });
    let complete: Vec<&BTreeMap<Arm, &Replay>> = pairs.values().filter(|p| p.len() == 3).collect();
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
        "# cc-bench {}\n\n- compiler: {}\n- kache: {}\n- ccache: {}\n- host: {} {} CPUs, {}\n- jobs: {}, seed: {}\n\n",
        report.toolchain,
        report.compiler,
        report.kache,
        report.ccache,
        report.host.cpu_model.as_deref().unwrap_or("unknown CPU"),
        report.host.logical_cpus.map_or("?".into(), |n| n.to_string()),
        report.host.os,
        report.jobs,
        report.seed
    );
    for w in &report.workloads {
        s.push_str(&format!("## {}\n\n", w.workload));
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
