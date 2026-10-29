//! Unified scenario runner CLI.

use anyhow::{Result, bail};
use lexopt::prelude::*;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::bench_runner::{self, BenchRunConfig, CacheBackend};
use crate::fixture_runner::{self, FixtureRunConfig};
use crate::scenario::{ScenarioMetadata, Selectors, SourceKind, discover_metadata};

const SCENARIOS_DIR: &str = "./scenarios";

const HELP: &str = "\
Run kache scenarios selected by tags/name/source kind.

usage: kache-scenario [options]
  --kache <path>          kache binary under test [./target/release/kache]
  --sccache <path>        sccache binary [sccache]
  --mbx <path>            mbx binary [mbx]
  --cache-backend <name>  kache|sccache|mbx [kache]
  --scenarios <dir>       scenario directory [./scenarios]
  --select <selector>     repeatable; selectors are ANDed
  --profile <name>        shorthand for --select name:<name>
  --list                  list selected scenarios instead of running them
  --out <path>            e2e result JSON path [./tmp/e2e/results.json]
  --only <substr>         legacy fixture-only name substring filter
  --negative-control      fixture falsifiability mode (fixtures only)
  --deny-missing-tools    fail if a supported fixture misses a tool
  --ref <git-ref>         override a clone scenario's pinned source.ref
  --work-dir <dir>        benchmark work dir for a clone scenario
  --skip-clone            reuse clone worktrees already present
  --force-setup           re-run clone scenario setup
  --retry                 re-measure warm phase from a prior cold snapshot
  --trace-keys            cache-key trace logging (kache backend only)
  --warm-same-tree        add a same-worktree warm rebuild phase
  -h, --help              print this help and exit";

#[derive(Debug)]
struct Args {
    kache: PathBuf,
    sccache: PathBuf,
    mbx: PathBuf,
    cache_backend: CacheBackend,
    scenarios: PathBuf,
    select: Vec<String>,
    profile: Option<String>,
    list: bool,
    out: PathBuf,
    only: Option<String>,
    negative_control: bool,
    deny_missing_tools: bool,
    git_ref: Option<String>,
    work_dir: Option<PathBuf>,
    skip_clone: bool,
    force_setup: bool,
    retry: bool,
    trace_keys: bool,
    warm_same_tree: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            kache: PathBuf::from("./target/release/kache"),
            sccache: PathBuf::from("sccache"),
            mbx: PathBuf::from("mbx"),
            cache_backend: CacheBackend::Kache,
            scenarios: PathBuf::from(SCENARIOS_DIR),
            select: Vec::new(),
            profile: None,
            list: false,
            out: PathBuf::from("./tmp/e2e/results.json"),
            only: None,
            negative_control: false,
            deny_missing_tools: false,
            git_ref: None,
            work_dir: None,
            skip_clone: false,
            force_setup: false,
            retry: false,
            trace_keys: false,
            warm_same_tree: false,
        }
    }
}

/// Returns `None` for `--help`.
fn parse_cli(argv: &[OsString]) -> Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut parser = lexopt::Parser::from_args(argv.iter().skip(1));
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Long("kache") => args.kache = value_path(&mut parser, "--kache")?,
            Long("sccache") => args.sccache = value_path(&mut parser, "--sccache")?,
            Long("mbx") => args.mbx = value_path(&mut parser, "--mbx")?,
            Long("cache-backend") => {
                let raw = value_string(&mut parser, "--cache-backend")?;
                args.cache_backend = raw
                    .parse()
                    .map_err(|_| format!("invalid --cache-backend '{raw}': expected kache, sccache, or mbx"))?;
            }
            Long("scenarios") => args.scenarios = value_path(&mut parser, "--scenarios")?,
            Long("select") => args.select.push(value_string(&mut parser, "--select")?),
            Long("profile") => args.profile = Some(value_string(&mut parser, "--profile")?),
            Long("list") => args.list = true,
            Long("out") => args.out = value_path(&mut parser, "--out")?,
            Long("only") => args.only = Some(value_string(&mut parser, "--only")?),
            Long("negative-control") => args.negative_control = true,
            Long("deny-missing-tools") => args.deny_missing_tools = true,
            Long("ref") => args.git_ref = Some(value_string(&mut parser, "--ref")?),
            Long("work-dir") => args.work_dir = Some(value_path(&mut parser, "--work-dir")?),
            Long("skip-clone") => args.skip_clone = true,
            Long("force-setup") => args.force_setup = true,
            Long("retry") => args.retry = true,
            Long("trace-keys") => args.trace_keys = true,
            Long("warm-same-tree") => args.warm_same_tree = true,
            Long("help") => return Ok(None),
            Short('h') => return Ok(None),
            Long(name) => return Err(format!("unexpected argument '--{name}'")),
            Short(ch) => return Err(format!("unexpected argument '-{ch}'")),
            Value(value) => {
                return Err(format!(
                    "unexpected argument '{}'",
                    value.to_string_lossy()
                ));
            }
        }
    }
    Ok(Some(args))
}

fn value_string(parser: &mut lexopt::Parser, flag: &str) -> Result<String, String> {
    parser.value().map(|v| v.to_string_lossy().into_owned()).map_err(|_| format!("{flag} requires a value"))
}

fn value_path(parser: &mut lexopt::Parser, flag: &str) -> Result<PathBuf, String> {
    parser.value().map(PathBuf::from).map_err(|_| format!("{flag} requires a value"))
}

impl Args {
    /// Was any clone-benchmark-only option supplied?
    ///
    /// These options mean nothing to a fixture scenario, so passing one without
    /// selecting a clone scenario is a mistake worth reporting rather than
    /// silently ignoring — the caller would otherwise believe a flag took
    /// effect when it did not. Every benchmark option belongs on this list;
    /// forgetting to add a new one is how a flag becomes silently inert.
    fn has_bench_options(&self) -> bool {
        self.git_ref.is_some()
            || self.work_dir.is_some()
            || self.skip_clone
            || self.force_setup
            || self.retry
            || self.trace_keys
            || self.warm_same_tree
            || self.cache_backend != CacheBackend::Kache
    }
}

/// Scenarios run on CI hosts that can carry a real `/etc/kache/config.toml`
/// (the self-hosted Macs set keys there for every build), and an arm must
/// measure what its scenario configures, not what the host does. Every kache
/// this process spawns inherits its environment, so turning the host layer
/// off once here covers all of them. A caller who wants the host layer
/// exports `KACHE_HOST_CONFIG` explicitly, which is left alone.
fn turn_off_host_config() {
    if std::env::var_os("KACHE_HOST_CONFIG").is_none() {
        // SAFETY: called first thing in `main`, before this process starts
        // any thread that could read the environment concurrently.
        unsafe { std::env::set_var("KACHE_HOST_CONFIG", "") };
    }
}

pub fn main() -> Result<()> {
    turn_off_host_config();
    let argv: Vec<OsString> = std::env::args_os().collect();
    let args = match parse_cli(&argv) {
        Ok(Some(args)) => args,
        Ok(None) => {
            println!("{HELP}");
            return Ok(());
        }
        Err(message) => {
            eprintln!("kache-scenario: error: {message}");
            eprintln!("usage: kache-scenario [options]");
            std::process::exit(2);
        }
    };
    let mut select = args.select.clone();
    if let Some(profile) = &args.profile {
        select.push(format!("name:{profile}"));
    }
    if select.is_empty() && !args.list {
        bail!(
            "no scenario selectors provided; pass --select suite:e2e, --select suite:bench, or --profile <name>"
        );
    }

    let selectors = Selectors::parse_many(&select)?;
    let metadata = discover_metadata(&args.scenarios)?;
    let selected = metadata
        .iter()
        .filter(|scenario| selectors.matches_metadata(scenario))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        bail!(
            "no scenarios selected under {} (after --select {})",
            args.scenarios.display(),
            selectors.describe()
        );
    }
    if args.list {
        print_scenario_list(&selected, args.cache_backend);
        return Ok(());
    }

    let fixture_selected = selected
        .iter()
        .any(|scenario| scenario.source_kind == SourceKind::Fixture);
    let clone_selected = selected
        .iter()
        .any(|scenario| scenario.source_kind == SourceKind::Clone);

    if args.negative_control && clone_selected {
        bail!("--negative-control is only valid for fixture scenarios");
    }
    if args.deny_missing_tools && clone_selected {
        bail!("--deny-missing-tools is only valid for fixture scenarios");
    }
    let bench_flags = args.has_bench_options();
    if bench_flags && !clone_selected {
        bail!("benchmark options were provided, but no clone scenario was selected");
    }

    if fixture_selected {
        fixture_runner::run(FixtureRunConfig {
            kache: args.kache.clone(),
            scenarios: args.scenarios.clone(),
            out: args.out.clone(),
            only: args.only.clone(),
            select: select.clone(),
            negative_control: args.negative_control,
            deny_missing_tools: args.deny_missing_tools,
        })?;
    }

    if clone_selected {
        bench_runner::run_bench(BenchRunConfig {
            kache: args.kache,
            sccache: args.sccache,
            mbx: args.mbx,
            cache_backend: args.cache_backend,
            scenarios: args.scenarios,
            select,
            git_ref: args.git_ref,
            work_dir: args.work_dir,
            skip_clone: args.skip_clone,
            force_setup: args.force_setup,
            retry: args.retry,
            trace_keys: args.trace_keys,
            warm_same_tree: args.warm_same_tree,
        })?;
    }

    Ok(())
}

fn print_scenario_list(selected: &[&ScenarioMetadata], cache_backend: CacheBackend) {
    let benchmark_only = selected
        .iter()
        .all(|scenario| scenario.tags.iter().any(|tag| tag == "suite:bench"));
    if benchmark_only {
        println!("{} benchmark profiles:", cache_backend_label(cache_backend));
    } else {
        println!("matching profiles:");
    }
    for scenario in selected {
        println!("  {}", profile_hint(&scenario.name, cache_backend));
    }
    if benchmark_only {
        println!("use: {}", benchmark_recipe_hint(cache_backend));
    }
}

fn cache_backend_label(cache_backend: CacheBackend) -> &'static str {
    match cache_backend {
        CacheBackend::Kache => "kache",
        CacheBackend::Sccache => "sccache",
        CacheBackend::Mbx => "mbx",
    }
}

fn benchmark_recipe_hint(cache_backend: CacheBackend) -> &'static str {
    match cache_backend {
        CacheBackend::Kache => "make bench PROFILE=<profile>",
        CacheBackend::Sccache => "make bench-sccache PROFILE=<profile>",
        CacheBackend::Mbx => "make bench-mbx PROFILE=<profile>",
    }
}

/// The `-<backend>` suffix a comparison scenario carries in its name; the
/// recipe argument is the bare subject.
fn backend_suffix(cache_backend: CacheBackend) -> Option<&'static str> {
    match cache_backend {
        CacheBackend::Kache => None,
        CacheBackend::Sccache => Some("-sccache"),
        CacheBackend::Mbx => Some("-mbx"),
    }
}

fn profile_hint(name: &str, cache_backend: CacheBackend) -> String {
    let mut profile = name.strip_prefix("bench-").unwrap_or(name).to_string();
    if let Some(suffix) = backend_suffix(cache_backend)
        && let Some(stripped) = profile.strip_suffix(suffix)
    {
        profile = stripped.to_string();
    }
    profile
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kache the runner spawns inherits its environment, so the runner
    /// turns the host layer off, and leaves a value the caller set alone.
    #[test]
    fn the_runner_turns_the_host_layer_off_unless_the_caller_set_it() {
        let previous = std::env::var_os("KACHE_HOST_CONFIG");
        // SAFETY: no other test in this crate reads or writes this variable,
        // and it is restored before any assertion can panic.
        unsafe { std::env::remove_var("KACHE_HOST_CONFIG") };
        turn_off_host_config();
        let unset = std::env::var_os("KACHE_HOST_CONFIG");
        unsafe { std::env::set_var("KACHE_HOST_CONFIG", "/srv/kache.toml") };
        turn_off_host_config();
        let set = std::env::var_os("KACHE_HOST_CONFIG");
        unsafe {
            match previous {
                Some(value) => std::env::set_var("KACHE_HOST_CONFIG", value),
                None => std::env::remove_var("KACHE_HOST_CONFIG"),
            }
        }

        assert_eq!(unset.as_deref(), Some(std::ffi::OsStr::new("")));
        assert_eq!(
            set.as_deref(),
            Some(std::ffi::OsStr::new("/srv/kache.toml"))
        );
    }

    /// Every clone-benchmark option, on its own, must be recognised as one.
    /// The check is a long disjunction, and a term that stops contributing
    /// makes its flag silently inert against a fixture selection instead of
    /// producing the "no clone scenario was selected" error.
    fn parse_argv(argv: &[&str]) -> Args {
        let owned: Vec<OsString> = argv.iter().map(OsString::from).collect();
        match parse_cli(&owned).expect("test argv must parse") {
            Some(args) => args,
            None => panic!("test argv must not request help"),
        }
    }

    #[test]
    fn every_benchmark_option_is_recognised_on_its_own() {
        let plain = parse_argv(&["kache-scenario", "--select", "suite:e2e"]);
        assert!(
            !plain.has_bench_options(),
            "a selection with no benchmark option must not look like one"
        );

        for option in [
            vec!["--ref", "797e8a9"],
            vec!["--work-dir", "tmp/bench/x"],
            vec!["--skip-clone"],
            vec!["--force-setup"],
            vec!["--retry"],
            vec!["--trace-keys"],
            vec!["--warm-same-tree"],
            vec!["--cache-backend", "sccache"],
        ] {
            let mut argv = vec!["kache-scenario", "--select", "suite:e2e"];
            argv.extend(option.iter().copied());
            let args = parse_argv(&argv);
            assert!(
                args.has_bench_options(),
                "{option:?} must count as a benchmark option"
            );
        }
    }

    #[test]
    fn profile_hint_matches_bench_recipe_arguments() {
        assert_eq!(
            profile_hint("bench-firefox", CacheBackend::Kache),
            "firefox"
        );
        assert_eq!(profile_hint("bench-eza", CacheBackend::Kache), "eza");
        assert_eq!(
            profile_hint("bench-firefox-sccache", CacheBackend::Sccache),
            "firefox"
        );
        assert_eq!(profile_hint("bench-hk-mbx", CacheBackend::Mbx), "hk");
        // The label and the recipe are what a `--list` reader types next;
        // both name the backend they were asked about.
        for (backend, label, recipe, suffix) in [
            (CacheBackend::Kache, "kache", "make bench PROFILE=<profile>", None),
            (
                CacheBackend::Sccache,
                "sccache",
                "make bench-sccache PROFILE=<profile>",
                Some("-sccache"),
            ),
            (
                CacheBackend::Mbx,
                "mbx",
                "make bench-mbx PROFILE=<profile>",
                Some("-mbx"),
            ),
        ] {
            assert_eq!(cache_backend_label(backend), label);
            assert_eq!(benchmark_recipe_hint(backend), recipe);
            assert_eq!(backend_suffix(backend), suffix);
        }
        assert_eq!(
            profile_hint("bench-hk-mbx", CacheBackend::Kache),
            "hk-mbx",
            "only the selected backend's suffix is stripped"
        );
    }
}
