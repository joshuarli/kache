mod args;
mod blob_heal;
mod build_script;
mod build_script_inputs;
use kache_store::atomic;
mod cache_fs;
mod cache_key;
mod cargo_env;
mod cargo_layout;
mod cargo_proxy;
mod checked_regions;
mod cli;
mod compile;
mod compiler;
mod config;
mod config_cli;
mod daemon;
mod daemon_publish;
mod demand;
mod disk_recovery;
mod events;
mod extra_inputs;
mod fallback;
mod heartbeat;
mod index_compact;
#[cfg(unix)]
mod init_shell;
mod invocation;
mod key_env;
mod lifecycle;
use kache_store::link;
mod link_probe;
mod machine;
mod maintenance;
mod miss_chain;
mod native_archive;
mod notice;
mod opcounts;
mod otel;
mod out_dir_alias;
mod path_normalizer;
mod pe_pdb;
mod phase_trace;
mod platform;
mod prestage;
mod probe;
mod probe_memo;
mod report;
mod savings;
mod scheduler;
mod service;
mod since;
mod compiler_store;
use compiler_store as store;
mod store_view;
mod term;
mod test_runner;
#[cfg(test)]
mod test_support;
mod toolchain_dylib;
use kache_ipc as transport;
mod tree_stamp;
mod cli_args;
mod verify_compare;
mod volume_gc;
mod wrapper;
mod wrapper_config;

// macOS local-network privacy attributes a LaunchAgent's network access to
// responsible code. Kache is a standalone executable rather than an app
// bundle, so embed the identity and usage description directly in its Mach-O
// image. The generated LaunchAgent associates itself with the same bundle id
// in `service::launchd_plist_content` (kunobi-ninja/kache#798).
#[cfg(target_os = "macos")]
#[used]
#[unsafe(link_section = "__TEXT,__info_plist")]
static MACOS_INFO_PLIST: [u8; include_bytes!("Info.plist").len()] =
    *include_bytes!("Info.plist");

use anyhow::{Context, Result};
use std::path::PathBuf;

mod version;
pub use version::VERSION;

pub(crate) use cli_args::Cli;

kache_client::define_engine_commands!();

/// `kache list --sort` keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListSort {
    Name,
    Size,
    Hits,
    Age,
}

impl ListSort {
    fn as_str(self) -> &'static str {
        match self {
            ListSort::Name => "name",
            ListSort::Size => "size",
            ListSort::Hits => "hits",
            ListSort::Age => "age",
        }
    }
}

/// The bench phases `kache telemetry write --phase` labels a dump with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchPhase {
    Cold,
    WarmSameTree,
    Warm,
    Pull,
}

impl BenchPhase {
    fn as_str(self) -> &'static str {
        match self {
            BenchPhase::Cold => "cold",
            BenchPhase::WarmSameTree => "warm-same-tree",
            BenchPhase::Warm => "warm",
            BenchPhase::Pull => "pull",
        }
    }
}

pub(crate) enum DaemonCommands {
    /// Show daemon status (also bare `kache daemon`)
    Status,
    /// Run the daemon server in the foreground
    Run,
    /// Start the daemon in the background and wait until it is ready
    Start,
    /// Stop a running daemon
    Stop,
    /// Restart the daemon, through its login service when one is installed
    Restart,
    /// Install the daemon as a per-user login service (launchd, systemd user
    /// unit, Task Scheduler)
    Install,
    /// Remove the daemon service
    Uninstall,
    /// Stream daemon logs
    Log,
}

pub(crate) enum TelemetryCommands {
    /// Snapshot live cache counters into `metrics.otlp.json` + `schema_version`
    Write {
        /// Directory to write. Created if missing.
        dir: PathBuf,
        /// Bench scenario this dump belongs to (same string as `kache.bench.project`).
        scenario: Option<String>,
        /// Bench phase this dump belongs to.
        phase: Option<BenchPhase>,
    },
}

/// Whether `command` has a JSON form. Every command is named, with no
/// catch-all, so a new one does not compile until it says whether `--json`
/// works for it.
fn command_supports_json(command: &Option<Commands>) -> bool {
    let Some(command) = command else {
        return false;
    };
    match command {
        Commands::List { .. }
        | Commands::Gc { .. }
        | Commands::Clean { .. }
        | Commands::Doctor { .. }
        | Commands::Stats { .. }
        | Commands::Explain { .. }
        | Commands::Report { .. } => true,
        Commands::Init { shims, .. } => shims.is_none(),
        Commands::Config { .. } => true,
        Commands::Daemon { command } => match command {
            None | Some(DaemonCommands::Status) => true,
            Some(
                DaemonCommands::Run
                | DaemonCommands::Start
                | DaemonCommands::Stop
                | DaemonCommands::Restart
                | DaemonCommands::Install
                | DaemonCommands::Uninstall
                | DaemonCommands::Log,
            ) => false,
        },
        // Cargo's own output.
        Commands::Cargo { .. } => false,
        // No JSON form yet.
        Commands::Purge { .. }
        | Commands::Telemetry { .. }
        | Commands::InstallShims { .. } => false,
    }
}

/// Why `--json` cannot run `command`, or `None` when it can. The general
/// answer lists only commands `kache --help` shows.
fn json_refusal(command: &Option<Commands>) -> Option<&'static str> {
    if command_supports_json(command) {
        return None;
    }
    Some(match command {
        Some(Commands::Purge { .. }) => {
            "`kache purge` has no JSON form yet; run it without `--json`."
        }
        // The other forms of init have one.
        Some(Commands::Init { .. }) => "`kache init --shims` has no JSON form.",
        _ => {
            "`--json` is supported on init, stats, clean, doctor, explain, list, and daemon status."
        }
    })
}

fn doctor_json_is_read_only(
    fix: bool,
    purge_sccache: bool,
    verify: bool,
    checksums: bool,
    repair: bool,
) -> bool {
    ![fix, purge_sccache, verify, checksums, repair]
        .into_iter()
        .any(|enabled| enabled)
}

/// Diagnostic log file path.
/// macOS: `~/Library/Logs/kache/kache.log` (visible in Console.app).
/// Linux/other: `<cache_dir>/kache.log`.
pub(crate) fn diagnostic_log_path() -> PathBuf {
    diagnostic_log_path_for(&invocation::InvocationEnv::capture())
}

fn diagnostic_log_path_for(env: &invocation::InvocationEnv) -> PathBuf {
    if cfg!(target_os = "macos") {
        env.home_dir()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join("Library/Logs/kache/kache.log")
    } else {
        config::default_cache_dir_for(env).join("kache.log")
    }
}

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024; // 5 MB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogMode {
    Wrapper,
    Cli,
}

fn detect_log_mode(env_args: &[String]) -> LogMode {
    let rustc = std::env::var_os("RUSTC");
    detect_log_mode_with_rustc(env_args, rustc.as_deref())
}

fn detect_log_mode_with_rustc(
    env_args: &[String],
    configured_rustc: Option<&std::ffi::OsStr>,
) -> LogMode {
    if env_args.len() >= 2 {
        let after = &env_args[1..];
        // Real compiler invocation (rustc / cc-family) OR a cc-crate
        // family probe (`kache -E <file>`). Both want wrapper-mode
        // logging (off by default — cargo would otherwise cache the
        // stderr as a stale compiler diagnostic).
        if compiler::is_passthrough_compiler_invocation_with(after, configured_rustc)
            || compiler::is_workspace_wrapper_chain(after)
            || compiler::cc::CcCompiler::recognizes_family_probe(after)
            || compiler::detect_compiler(after).is_some()
        {
            return LogMode::Wrapper;
        }
    }

    LogMode::Cli
}

fn init_logging(mode: LogMode) {
    kache_log::install(logging_logger(mode, &invocation::InvocationEnv::capture()));
}

/// Builds the process-wide logger for `mode` and `env`. Tests call this
/// directly and route records through the returned logger instead of
/// installing anything process-wide, so concurrent tests stay isolated.
fn logging_logger(
    mode: LogMode,
    env: &invocation::InvocationEnv,
) -> kache_log::KacheLogger {
    kache_log::KacheLogger::new(
        stderr_level_for(mode, env),
        || Box::new(invocation::io::stderr()) as Box<dyn std::io::Write + Send>,
        file_sink_for(mode, env),
    )
}

// Wrapper mode: cargo captures RUSTC_WRAPPER stderr and caches it as compiler
// diagnostics, replaying stale warnings on every subsequent build.  Default to
// silent; users can still opt in via KACHE_LOG for one-off debugging.
fn stderr_level_for(mode: LogMode, env: &invocation::InvocationEnv) -> log::LevelFilter {
    let default_filter = if mode == LogMode::Wrapper {
        "off"
    } else {
        "kache=warn"
    };
    env.var("KACHE_LOG")
        .ok()
        .and_then(|filter| kache_log::parse_level_filter(&filter))
        .unwrap_or_else(|| {
            kache_log::parse_level_filter(default_filter).expect("default stderr filter parses")
        })
}

// File layer: persistent log at info level (overridable via KACHE_LOG_FILE).
//
// CLI/daemon mode: always enabled at the default `kache.log` path — these
// run rarely, so 2 syscalls (stat + open) per invocation is fine.
//
// Wrapper mode: cargo can fan out hundreds of `kache rustc …` processes
// per build, so the file layer is OFF by default to avoid the per-crate
// syscalls. Opt in by setting `KACHE_LOG_FILE` explicitly — useful for
// diagnostics when cargo would otherwise eat wrapper stderr. The path
// can be overridden via `KACHE_LOG_FILE_PATH` (e.g. the e2e bench writes
// a per-phase wrapper.log so each cold/warm phase has a clean log).
fn file_sink_for(
    mode: LogMode,
    env: &invocation::InvocationEnv,
) -> Option<kache_log::FileSink> {
    let wrapper_opted_in =
        mode == LogMode::Wrapper && env.var_os("KACHE_LOG_FILE").is_some();
    let enable_file_layer = mode != LogMode::Wrapper || wrapper_opted_in;

    if !enable_file_layer {
        return None;
    }
    (|| -> Option<_> {
        let path = env
            .var_os("KACHE_LOG_FILE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| diagnostic_log_path_for(env));
        let path = env.resolve(&path)?;
        std::fs::create_dir_all(path.parent()?).ok()?;

        // Simple rotation: truncate if file exceeds 5 MB.
        // Skipped when a custom path is provided — the caller owns
        // the file lifecycle (e.g. the bench wipes it per phase).
        if env.var_os("KACHE_LOG_FILE_PATH").is_none()
            && std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES)
        {
            let _ = std::fs::write(&path, b"--- log rotated ---\n");
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;

        let file_level = env
            .var("KACHE_LOG_FILE")
            .ok()
            .and_then(|filter| kache_log::parse_level_filter(&filter))
            .unwrap_or(log::LevelFilter::Info);

        Some(kache_log::FileSink::new(file, file_level))
    })()
}

/// Routes the calling thread's records through `env`'s stderr level and file
/// until the guard drops. Compiler invocations served by the daemon run in a
/// process whose startup environment is the daemon's, so without this the
/// client's `KACHE_LOG*` settings would not reach the records its own
/// compile emits.
fn invocation_log_scope(env: &invocation::InvocationEnv) -> kache_log::InvocationGuard {
    kache_log::enter(&kache_log::InvocationConfig::new(
        stderr_level_for(LogMode::Wrapper, env),
        file_sink_for(LogMode::Wrapper, env),
    ))
}

/// How main reads argv, decided before any of it is parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryRoute {
    /// kache started itself for a subcommand: parse the CLI whatever
    /// argv[0] is. A self-spawn under a shim keeps the shim's name there.
    SelfSpawn,
    /// argv[0] names a compiler: kache runs behind a compiler-name shim.
    Shim,
    /// kache under its own name: argv[1..] picks wrapper or CLI mode.
    Argv,
}

/// Route a process from argv and the [`platform::SELF_SPAWN_ENV`] value.
fn entry_route(argv: &[String], self_spawn: Option<&std::ffi::OsStr>) -> EntryRoute {
    if platform::is_self_spawn(argv, self_spawn) {
        return EntryRoute::SelfSpawn;
    }
    if argv
        .first()
        .is_some_and(|arg0| compiler::shim::invoked_as_compiler(arg0))
    {
        return EntryRoute::Shim;
    }
    EntryRoute::Argv
}

/// The test binary and its arguments when kache runs as a Cargo target
/// runner (`kache test-runner <binary> <args>`). Behind a compiler-name shim
/// argv[1] is a compiler argument, so `cc test-runner` stays a compile.
fn test_runner_args(raw_args: &[std::ffi::OsString]) -> Option<&[std::ffi::OsString]> {
    match raw_args.get(1) {
        Some(arg)
            if arg == "test-runner"
                && !raw_args.first().is_some_and(|arg0| {
                    compiler::shim::invoked_as_compiler(&arg0.to_string_lossy())
                }) =>
        {
            Some(&raw_args[2..])
        }
        _ => None,
    }
}

/// Run the engine CLI, compiler pipeline or daemon process.
pub fn main() -> std::process::ExitCode {
    match run_main() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => std::process::ExitCode::from(report_main_error(&error)),
    }
}

/// Print an error that reached `main` the way every kache command does, and
/// return the exit status. A closed stdout ends quietly with 0.
fn report_main_error(error: &anyhow::Error) -> u8 {
    if machine::failure_exit(error).is_none() {
        return 0;
    }
    eprintln!("{} {error:#}", term::paint("error:", term::Style::Error));
    1
}

fn run_main() -> Result<()> {
    // Recover frontend timing before configuration, logging or worker threads.
    let entry = kache_client::engine_entry(std::env::args_os().collect(), VERSION)?;
    opcounts::mark_process_start_at(entry.started_at);
    if let Some(frontend) = entry.frontend {
        platform::set_frontend_image(frontend)?;
    }
    // SAFETY: before logging, runtimes, or other threads start. DaemonCommand
    // exclusively passes the channel descriptor to this process.
    let mut readiness = unsafe { kunobi_daemon::readiness::channel::Notifier::from_env() }
        .context("opening daemon readiness channel")?;
    if let Some(notifier) = &mut readiness {
        let _ = notifier.progress("starting kache");
    }
    // Cargo target runner. Checked before the probe and shim guards, which
    // exit without running anything: a test must always run.
    let raw_args = entry.args;
    if raw_args.first().is_some_and(|arg| {
        std::path::Path::new(arg)
            .file_name()
            .is_some_and(|name| name == "cargo")
    }) && std::env::var_os(platform::SELF_SPAWN_ENV).is_none()
    {
        init_logging(LogMode::Cli);
        return cargo_proxy::run_shim(raw_args[1..].to_vec());
    }
    if let Some(args) = test_runner_args(&raw_args) {
        init_logging(LogMode::Wrapper);
        std::process::exit(test_runner::run(args));
    }
    if std::env::var_os("KACHE_FAMILY_PROBE_ACTIVE").is_some() {
        // Prevent unbounded recursion when a probed wrapper calls back into kache.
        return Ok(());
    }
    // Cargo running a build script through the launcher the rustc wrapper
    // installed. Decided on the environment alone: argv is the script's own.
    if build_script::is_shim_invocation() {
        init_logging(LogMode::Wrapper);
        std::process::exit(build_script::run_shim());
    }

    // Keep the original argv byte-preserving for `kache cargo`. Compiler
    // adapters still parse UTF-8 option syntax; a wrapper invocation with a
    // non-UTF-8 path is detected here and fails closed below.
    let env_args: Option<Vec<String>> = raw_args
        .iter()
        .cloned()
        .map(|arg| arg.into_string())
        .collect::<Result<_, _>>()
        .ok();
    let lossy_args;
    let detection_args = if let Some(args) = env_args.as_deref() {
        args
    } else {
        lossy_args = raw_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        &lossy_args
    };
    // Compiler-name shim (#310): invoked as `gcc`/`cc`/… through a PATH
    // symlink, so the compiler is our own argv[0] rather than argv[1]. Checked
    // BEFORE `detect_log_mode`, which only ever inspects argv[1..] and would
    // classify `gcc foo.c` as CLI mode and fail parsing `foo.c` as a
    // subcommand. A process kache started for its own subcommand can carry a
    // shim's name in argv[0] too, so its marker is checked first.
    let self_spawn = std::env::var_os(platform::SELF_SPAWN_ENV);
    let shim_args = match entry_route(detection_args, self_spawn.as_deref()) {
        EntryRoute::Shim => compiler::shim::wrapper_args(detection_args),
        EntryRoute::SelfSpawn => {
            // SAFETY: no thread has been spawned yet. Removing the marker
            // keeps it out of every process this one starts.
            unsafe { std::env::remove_var(platform::SELF_SPAWN_ENV) };
            None
        }
        EntryRoute::Argv => None,
    };
    if let Some(shim_args) = shim_args {
        init_logging(LogMode::Wrapper);
        let shim_args = shim_args.map_err(anyhow::Error::msg)?;
        if env_args.is_none() {
            anyhow::bail!(
                "compiler wrapper arguments contain non-UTF-8 bytes and cannot be cached safely"
            );
        }
        return run_wrapper_mode(&shim_args);
    }

    let log_mode = detect_log_mode(detection_args);

    // Detect RUSTC_WRAPPER mode: cargo passes the rustc path as arg[1]
    // In this mode: argv[0]=kache, argv[1]=rustc, argv[2..]=rustc args
    let is_wrapper = log_mode == LogMode::Wrapper;
    init_logging(log_mode);

    if is_wrapper {
        if let Some(env_args) = env_args {
            return run_wrapper_mode(&env_args[1..]);
        }
        anyhow::bail!(
            "compiler wrapper arguments contain non-UTF-8 bytes and cannot be cached safely"
        );
    }

    // CLI mode: parse subcommands. `--json` may appear before or inside
    // the subcommand; help, version and failures already know whether it
    // was seen. Non-UTF-8 arguments fail closed inside the parser; only
    // `cargo` keeps them verbatim.
    let cli = match cli_args::parse_cli(raw_args.get(1..).unwrap_or(&[])) {
        Ok(cli_args::ParseOutcome::Run(cli)) => cli,
        Ok(cli_args::ParseOutcome::Help { text, json }) => {
            if json {
                machine::emit_help()?;
            } else {
                println!("{text}");
            }
            return Ok(());
        }
        Ok(cli_args::ParseOutcome::Version { json }) => {
            if json {
                machine::emit(
                    "version",
                    deser_value::value!({ "version": VERSION }),
                    Vec::new(),
                )?;
            } else {
                println!("kache {VERSION}");
            }
            return Ok(());
        }
        Err(failure) => {
            if failure.json {
                // A closed stdout must not turn a usage error into success.
                let _ = machine::emit_error("cli", "invalid_arguments", &failure.error);
                eprint!("{}", failure.error);
                std::process::exit(2);
            }
            eprintln!("error: {}", failure.error);
            eprintln!("\nFor more information, try '--help'.");
            std::process::exit(2);
        }
    };
    if cli.json && cli.command.is_none() {
        machine::emit_help()?;
        return Ok(());
    }
    let json = cli.json;
    let command = cli.command_path();
    machine::configure(json, command.clone());
    if let Err(error) = run_cli(cli, readiness) {
        let Some((code, exit)) = machine::failure_exit(&error) else {
            return Ok(());
        };
        if json && error.downcast_ref::<machine::Reported>().is_none() {
            // Still exit with the command's status when the document
            // cannot be written.
            let _ = machine::emit_error(&command, code, &error);
        }
        eprintln!("{} {error:#}", term::paint("error:", term::Style::Error));
        std::process::exit(exit);
    }
    Ok(())
}

fn run_cli(cli: Cli, readiness: Option<kunobi_daemon::readiness::channel::Notifier>) -> Result<()> {
    let json = cli.json;
    if json && let Some(refusal) = json_refusal(&cli.command) {
        return Err(machine::invalid_arguments(refusal));
    }

    // Config and Completions run before Config::load() (broken/missing config).
    match &cli.command {
        Some(Commands::Config { command }) => {
            if let Some(command) = command {
                return config_cli::run(command, json);
            }
            return config_cli::run(
                &config_cli::Command::List {
                    scope: Default::default(),
                },
                json,
            );
        }
        Some(Commands::Cargo { args }) => return cargo_proxy::run(args.clone()),
        _ => {}
    }

    let (config, config_provenance) = config::Config::load_with_provenance()?;
    // Doctor reports this as a failing check, and the editor refuses to open.
    if !matches!(
        cli.command,
        Some(Commands::Doctor { .. } | Commands::Config { .. })
    ) && let Some((path, reason)) = config::unusable_config_file()
    {
        eprintln!(
            "{} ignoring {}: {reason}. Using the defaults.",
            term::paint("warning:", term::Style::Warning),
            path.display()
        );
    }

    match cli.command {
        Some(Commands::List {
            crate_name,
            sort,
            no_pager,
        }) => cli::list(
            &config,
            crate_name.as_deref(),
            sort.as_str(),
            no_pager,
            json,
        ),
        Some(Commands::Gc {
            max_age,
            stale_schema,
        }) => {
            let hours = max_age
                .as_deref()
                .map(|value| parse_age_hours("--max-age", value))
                .transpose()?;
            cli::gc(&config, hours, stale_schema, json)
        }
        Some(Commands::Purge { crate_name }) => cli::purge(&config, crate_name.as_deref()),
        Some(Commands::Clean {
            dry_run,
            yes,
            stale,
            cache,
            all,
            crate_name,
            stale_schema,
        }) => {
            if let Some(name) = crate_name {
                return cli::clean_cache(&config, cli::CacheClean::Crate(name), dry_run, yes, json);
            }
            let _ = (cache, all);
            let hours = stale
                .as_deref()
                .map(|value| parse_age_hours("--stale", value))
                .transpose()?;
            let what = cache_clean_choice(stale_schema, hours);
            cli::clean_cache(&config, what, dry_run, yes, json)
        }
        #[cfg(unix)]
        Some(Commands::Init {
            shims: Some(dir),
            from_path,
            canonical_only,
            force,
            ..
        }) => install_shims(dir, force, from_path, canonical_only),
        #[cfg(not(unix))]
        Some(Commands::Init { shims: Some(_), .. }) => Err(anyhow::anyhow!(UNIX_ONLY_SHIMS)),
        Some(Commands::Init {
            yes,
            no_service,
            no_shell,
            check,
            ..
        }) => {
            if json && !yes && !check {
                return Err(machine::invalid_arguments(
                    "`kache init --json` needs `--check` to preview or `--yes` to apply setup.",
                ));
            }
            cli::init(yes, no_service, no_shell, check)
        }
        Some(Commands::Doctor {
            fix,
            purge_sccache,
            verify,
            checksums,
            repair,
            probe,
        }) => {
            if json && !doctor_json_is_read_only(fix, purge_sccache, verify, checksums, repair) {
                return Err(machine::invalid_arguments(
                    "`kache doctor --json` reports diagnostics only; run repair options without `--json`.",
                ));
            }
            cli::doctor(
                fix,
                purge_sccache,
                verify || checksums || repair,
                checksums,
                repair,
                probe,
                json,
            )
        }
        Some(Commands::Daemon { command: None }) => service::status(json),
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Status),
        }) => service::status(json),
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Run),
        }) => daemon::run_server(&config, &config_provenance, readiness),
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Start),
        }) => match daemon::start(&config) {
            Ok(outcome) => {
                eprintln!("{}", daemon_start_message(&outcome));
                if !outcome.is_ready() {
                    std::process::exit(1);
                }
                Ok(())
            }
            Err(e) => {
                eprintln!("failed to start daemon: {e}");
                std::process::exit(1);
            }
        },
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Stop),
        }) => daemon::send_shutdown_request(&config),
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Restart),
        }) => {
            let was_running = daemon::is_running(&config)?;
            let ready = daemon::restart(&config)?;
            eprintln!("{}", daemon_restart_message(was_running, ready));
            if !ready {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Install),
        }) => service::install(),
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Uninstall),
        }) => service::uninstall(),
        Some(Commands::Daemon {
            command: Some(DaemonCommands::Log),
        }) => service::log(),
        Some(Commands::Report {
            format,
            since,
            root,
            output,
            top,
            record,
            redact,
        }) => {
            let window = parse_since_window(&since)?;
            cli::report(
                &config,
                report_format(Some(format), json)?,
                window,
                report::ReportFilter { root },
                output,
                top,
                record,
                redact,
            )
        }
        Some(Commands::Stats {
            since,
            root,
            full,
            format,
            output,
            top,
            record,
            redact,
        }) => {
            let full = stats_wants_full(
                full,
                format.is_some(),
                output.is_some(),
                top.is_some(),
                record,
                redact,
            );
            if full {
                let format = report_format(format, json)?;
                let window = parse_since_window(&since)?;
                return cli::report(
                    &config,
                    format,
                    window,
                    report::ReportFilter { root },
                    output,
                    top.unwrap_or(DEFAULT_REPORT_TOP),
                    record,
                    redact,
                );
            }
            if root.is_some() {
                return Err(machine::invalid_arguments(
                    "`--root` needs `--full`",
                ));
            }
            let window = parse_since_window(&since)?;
            cli::stats(&config, &config_provenance, window, json)
        }
        Some(Commands::Telemetry {
            command:
                TelemetryCommands::Write {
                    dir,
                    scenario,
                    phase,
                },
        }) => cli::telemetry_write(
            &config,
            &dir,
            scenario.as_deref(),
            phase.map(BenchPhase::as_str),
        ),
        Some(Commands::Explain { crate_name }) => cli::why_miss(&config, &crate_name, json),
        #[cfg(unix)]
        Some(Commands::InstallShims {
            dir,
            force,
            from_path,
            canonical_only,
        }) => install_shims(dir, force, from_path, canonical_only),
        #[cfg(not(unix))]
        Some(Commands::InstallShims { .. }) => Err(anyhow::anyhow!(UNIX_ONLY_SHIMS)),
        Some(Commands::Cargo { .. }) => unreachable!(),
        Some(Commands::Config { .. }) => unreachable!(),
        None => {
            // No subcommand — print help.
            print!("{}", cli_args::top_help_text());
            Ok(())
        }
    }
}

/// Environment breadcrumb a kache wrapper sets before spawning any
/// child compiler. A kache process that sees it already set is running
/// *inside* another kache — see [`run_wrapper_mode`]'s re-entrancy
/// guard. The value counts the wrappers above the child: the outermost
/// writes `1` and each nested wrapper one more.
const KACHE_ACTIVE_ENV: &str = "KACHE_ACTIVE";

/// How many kache wrappers may sit above this one before the chain is
/// taken to be a loop. Real nesting stays at one or two, such as a
/// compiler or a script it runs calling a shimmed `cc`.
const MAX_WRAPPERS_ABOVE: u32 = 8;

/// How many kache wrappers sit above this process, from the value of
/// [`KACHE_ACTIVE_ENV`]. `None` when it is the outermost. A value that is
/// not a number counts as one: older kache versions always wrote `1`.
fn wrappers_above(active: Option<&std::ffi::OsStr>) -> Option<u32> {
    let active = active?;
    Some(active.to_str().and_then(|v| v.parse().ok()).unwrap_or(1))
}

/// The [`KACHE_ACTIVE_ENV`] value a nested wrapper hands to its compiler.
fn next_wrapper_depth(above: u32) -> u32 {
    above.saturating_add(1)
}

/// Stop a chain of nested wrappers that only a loop could produce.
///
/// Shim resolution skips every PATH entry it can identify as kache, so a
/// loop that still gets here runs through shims it cannot identify: a farm
/// of copies with no marker. Nothing left on PATH is known to be the real
/// compiler, so skipping ahead could pick the wrong one. Failing names the
/// cause, where the loop would otherwise start processes without end.
fn check_wrapper_depth(above: u32, compiler: Option<&str>) -> Result<()> {
    if above < MAX_WRAPPERS_ABOVE {
        return Ok(());
    }
    anyhow::bail!(
        "kache is running inside {above} other kache wrappers to run `{}`, so two \
         compiler shims are running each other in a loop. Another kache install's shim \
         directory is on PATH and holds copies that kache cannot tell apart from a \
         compiler. Remove that directory from PATH, or create an empty `.kache-shims` \
         file in it so every kache skips it.",
        compiler.unwrap_or("?")
    )
}

fn run_compiler_directly_for(
    config: &config::Config,
    args: &[String],
    env: &invocation::InvocationEnv,
) -> Result<i32> {
    if is_cc_compiler_invocation_for(args, env) {
        let parsed = compiler::cc::CcArgs::parse_for(args, env)?;
        wrapper::refuse_legacy_cc_blob_outputs(config, &parsed)?;
    }

    run_compiler_process_directly_for(args, env)
}

#[cfg(test)]
fn is_cc_compiler_invocation(args: &[String]) -> bool {
    is_cc_compiler_invocation_for(args, &invocation::InvocationEnv::capture())
}

fn is_cc_compiler_invocation_for(args: &[String], env: &invocation::InvocationEnv) -> bool {
    if compiler::is_passthrough_compiler_invocation_for(args, env) {
        return false;
    }
    if compiler::is_workspace_wrapper_chain_for(args, env) {
        return false;
    }
    compiler::detect_compiler_for(args, env)
        .is_some_and(|adapter| adapter.id() == compiler::cc::CC_ID)
}

fn run_workspace_wrapper_chain_for(
    config: &config::Config,
    args: &[String],
    env: &invocation::InvocationEnv,
) -> Result<i32> {
    let parsed =
        args::RustcArgs::parse_for(args, env).context("parsing workspace-wrapper rustc chain")?;
    let extra_inputs = wrapper::resolve_extra_inputs_for_passthrough(config, &parsed)?;
    let exit = run_compiler_directly_for(config, args, env)?;
    if exit == 0 {
        wrapper::complete_current_extra_inputs_after_success(
            config,
            &parsed,
            extra_inputs.as_ref(),
        )?;
    }
    Ok(exit)
}

#[cfg(all(test, unix))]
fn run_compiler_process_directly(args: &[String], _preserve_incremental: bool) -> Result<i32> {
    run_compiler_process_directly_for(args, &invocation::InvocationEnv::capture())
}

fn run_compiler_process_directly_for(
    args: &[String],
    env: &invocation::InvocationEnv,
) -> Result<i32> {
    // A previous cache-on build may have restored this crate's outputs
    // as read-only hardlinks into the store (0o444, shared inode).
    // Running the real compiler over them in place would fail with
    // EACCES — and a chmod could not help, since the inode is shared
    // with the store blob (truncating it would poison future restores).
    // Unlink them first: a plain remove breaks the hardlink and leaves
    // the store blob intact, exactly as the enabled cache-miss path does
    // before recompiling. Without this, `KACHE_DISABLED=1` (or a nested
    // re-entrant compile) over a warm target dir breaks the build.

    // C/C++ and unknown compiler response files use shell/MSVC tokenization,
    // not rustc's one-argument-per-line format. Their direct passthrough must
    // therefore remain byte-for-byte argv transport.
    let configured_rustc = args.first().is_some_and(|program| {
        env.var_os("RUSTC")
            .is_some_and(|rustc| rustc == std::ffi::OsStr::new(program.as_str()))
    });
    let rustc_invocation =
        configured_rustc || rustc_args_for_direct_preclean_for(args, env).is_some();
    if !rustc_invocation {
        let status = invocation::io::status(env.command(&args[0])?.args(&args[1..]))?;
        return Ok(status.code().unwrap_or(1));
    }

    // C/C++ output paths have compiler-specific symlink, hardlink, device,
    // and read-only behavior (#645). Only rustc-shaped invocations may remove
    // previous read-only cache restores before compiling.
    let parsed = args::RustcArgs::parse_for(args, env).ok();
    if let Some(parsed) = &parsed {
        let output = parsed
            .output
            .as_deref()
            .map(|path| {
                env.resolve(path)
                    .context("resolving direct rustc output path")
            })
            .transpose()?;
        let out_dir = parsed
            .out_dir
            .as_deref()
            .map(|path| {
                env.resolve(path)
                    .context("resolving direct rustc output directory")
            })
            .transpose()?;
        compile::pre_clean_outputs(
            output.as_deref(),
            out_dir.as_deref(),
            parsed.crate_name.as_deref(),
            parsed.extra_filename.as_deref(),
            &parsed.emit,
        );
    }

    let program = env
        .resolve_program(std::path::Path::new(&args[0]))
        .unwrap_or_else(|| std::path::PathBuf::from(&args[0]));
    let mut command = env.command(&program)?;
    toolchain_dylib::apply_for(&mut command, &program, env);
    command.args(&args[1..]);
    let status = invocation::io::status(&mut command)?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
fn rustc_args_for_direct_preclean(args: &[String]) -> Option<&[String]> {
    rustc_args_for_direct_preclean_for(args, &invocation::InvocationEnv::capture())
}

fn rustc_args_for_direct_preclean_for<'a>(
    args: &'a [String],
    env: &invocation::InvocationEnv,
) -> Option<&'a [String]> {
    if compiler::rustc::RustcCompiler::recognizes(args) {
        Some(args)
    } else if compiler::is_workspace_wrapper_chain_for(args, env) {
        Some(&args[1..])
    } else {
        None
    }
}

/// Which wrapper entry point an adapter dispatches to.
///
/// Pure so the dispatch choice is unit-testable: `run_wrapper_mode` only
/// executes it. `None` means the adapter has no wrapper entry point yet
/// (the caller bails with the adapter's identity).
fn wrapper_target(adapter: &compiler::CompilerAdapter) -> Option<WrapperTarget> {
    if adapter.id() == compiler::rustc::RUSTC_ID {
        Some(WrapperTarget::Rustc)
    } else if adapter.id() == compiler::cc::CC_ID {
        Some(WrapperTarget::Cc)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapperTarget {
    Rustc,
    Cc,
}

fn run_wrapper_mode(args: &[String]) -> Result<()> {
    let context = invocation::InvocationContext {
        daemon: None,
        env: invocation::InvocationEnv::capture(),
        io: invocation::io::InvocationIo::capture(),
        cancellation: None,
        #[cfg(unix)]
        jobserver: kache_ipc::jobserver::Descriptors::default(),
        started_at: opcounts::process_start().unwrap_or_else(std::time::Instant::now),
        shim_stem: platform::frontend_executable()
            .ok()
            .and_then(|path| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "kache".to_owned()),
    };
    #[cfg(unix)]
    let config = if wrappers_above(context.env.var_os(KACHE_ACTIVE_ENV).as_deref()).is_none() {
        let config = config::Config::load_for(&context.env)?;
        if let Some(exit) = invocation::execution::try_execute_with_trust(
            &config.socket_path(),
            args,
            &context,
            config.trust_daemon_execution,
        )? {
            std::process::exit(exit);
        }
        // A held run lock can belong to the release we just upgraded from.
        // The coordinator checks its version and drains it before replacing it.
        if !cfg!(test) && !config.disabled {
            match daemon::start(&config) {
                Ok(_) => {
                    if let Some(exit) = invocation::execution::try_execute_with_trust(
                        &config.socket_path(),
                        args,
                        &context,
                        config.trust_daemon_execution,
                    )? {
                        std::process::exit(exit);
                    }
                }
                Err(error) => log::debug!("compiler daemon did not start: {error}"),
            }
        }
        Some(config)
    } else {
        None
    };
    #[cfg(not(unix))]
    let config = None;
    let exit = run_compiler_invocation_with_config(args, context, config)?;
    std::process::exit(exit);
}

/// Decide whether raw argv can enter the compiler pipeline without probing a
/// program. Ambiguous entry points remain available to the local engine.
#[cfg(unix)]
fn original_compiler_request(args: &[String], env: &invocation::InvocationEnv) -> bool {
    if args.is_empty()
        || env.var_os("KACHE_FAMILY_PROBE_ACTIVE").is_some()
        || env.var_os(build_script::SHIM_PATH_ENV).is_some()
        || platform::is_self_spawn(args, env.var_os(platform::SELF_SPAWN_ENV).as_deref())
        || compiler::command_basename(&args[0]) == Some("cargo")
    {
        return false;
    }
    if kache_shims::is_compiler_shim_name(&args[0]) {
        return true;
    }
    let after = &args[1..];
    compiler::rustc::RustcCompiler::recognizes(after)
        || after
            .first()
            .is_some_and(|program| compiler::cc::CcCompiler::recognizes_name(program))
        || compiler::is_passthrough_compiler_invocation_for(after, env)
        || compiler::is_workspace_wrapper_chain_for(after, env)
        || compiler::cc::CcCompiler::recognizes_family_probe(after)
}

#[cfg(unix)]
fn run_original_compiler_invocation(
    args: &[String],
    context: invocation::InvocationContext,
    executable: &std::path::Path,
) -> Result<i32> {
    let _cancellation = invocation::process::CancellationScope::enter(context.cancellation.clone());
    let _io = invocation::io::InvocationIoScope::enter(context.io.clone());
    let _log_scope = invocation_log_scope(&context.env);
    invocation::process::check()?;
    let rewritten = if args
        .first()
        .is_some_and(|arg0| kache_shims::is_compiler_shim_name(arg0))
    {
        compiler::shim::wrapper_args_for(args, &context.env, Some(executable))
            .context("missing compiler shim arguments")?
            .map_err(anyhow::Error::msg)?
    } else {
        args[1..].to_vec()
    };
    run_compiler_invocation(&rewritten, context)
}

fn run_compiler_invocation(args: &[String], context: invocation::InvocationContext) -> Result<i32> {
    run_compiler_invocation_with_config(args, context, None)
}

fn run_compiler_invocation_with_config(
    args: &[String],
    context: invocation::InvocationContext,
    prepared_config: Option<config::Config>,
) -> Result<i32> {
    let _cancellation = invocation::process::CancellationScope::enter(context.cancellation.clone());
    invocation::process::check()?;
    context.env.validate()?;
    let result = run_compiler_invocation_inner(args, context, prepared_config);
    invocation::process::check()?;
    result
}

fn run_compiler_invocation_inner(
    args: &[String],
    context: invocation::InvocationContext,
    prepared_config: Option<config::Config>,
) -> Result<i32> {
    let invocation::InvocationContext {
        daemon,
        mut env,
        started_at,
        shim_stem,
        io,
        cancellation: _,
        #[cfg(unix)]
        jobserver,
    } = context;
    #[cfg(unix)]
    let _jobserver = kache_ipc::jobserver::Scope::enter(jobserver);
    let _io = invocation::io::InvocationIoScope::enter(io);
    let _log_scope = invocation_log_scope(&env);
    let _alias_state = out_dir_alias::InvocationAliasScope::enter();
    let _key_state = cache_key::InvocationKeyScope::enter();
    let _wrapper_state = wrapper::InvocationWrapperScope::enter_for(&env, daemon.clone());
    let _heartbeat = heartbeat::InvocationHeartbeatScope::enter();
    let _restore_policy = link::InvocationLinkScope::enter();
    let _accounting = opcounts::InvocationScope::enter(started_at);
    // Re-entrancy guard. If a child compiler kache spawns is itself a
    // kache wrapper — e.g. `cc` on PATH shadowed by `kache cc` — the
    // nested invocation must run the real compiler directly instead of
    // looping kache → cc → kache → … kache sets KACHE_ACTIVE before
    // spawning any child, so a wrapper that already sees it set knows
    // it is nested. This runs before the `disabled` check below, so
    // the loop is broken even when caching is turned off.
    if let Some(above) = wrappers_above(env.var_os(KACHE_ACTIVE_ENV).as_deref()) {
        // The compiler run here can itself be a kache shim that nothing
        // identified. Counting the depth ends that loop at a fixed bound.
        check_wrapper_depth(above, args.first().map(String::as_str))?;
        env.set(KACHE_ACTIVE_ENV, next_wrapper_depth(above).to_string());
        let config = config::Config::load_for(&env)?;
        return run_compiler_directly_for(&config, args, &env);
    }
    // Every compiler child gets this request's re-entrancy depth.
    env.set(KACHE_ACTIVE_ENV, "1");

    // Local fallback keeps the snapshot used to select the daemon endpoint.
    // Delegated requests load their own invocation's configuration here.
    let mut config = match prepared_config {
        Some(config) => config,
        None => config::Config::load_for(&env)?,
    };
    if daemon.is_some() {
        config.daemon_publish = false;
    }
    let _pressure =
        scheduler::pressure::InvocationPressureScope::enter(config.scheduler_memory_pressure);

    if config.disabled {
        return run_compiler_directly_for(&config, args, &env);
    }

    // Compiler-family probe (`kache -E <file>` from the `cc` Rust
    // crate) — handled before compiler dispatch because it is NOT a
    // compiler invocation: it's a passthrough to the system default
    // `cc` so the cc crate sees the real underlying compiler's
    // preprocessor output.
    if compiler::cc::CcCompiler::recognizes_family_probe(args) {
        return wrapper::run_cc_probe_for(args, &env, &shim_stem);
    }

    if compiler::is_workspace_wrapper_chain_for(args, &env)
        && !compiler::is_cacheable_workspace_wrapper_chain_for(args, &env)
    {
        log::debug!(
            "workspace-wrapper chain is uncached; retaining extra-input Cargo freshness; program={:?}",
            args.first()
        );
        return run_workspace_wrapper_chain_for(&config, args, &env);
    }

    let direct_passthrough = compiler::is_passthrough_compiler_invocation_for(args, &env)
        && !compiler::rustc::RustcCompiler::recognizes(args);
    if direct_passthrough {
        log::debug!(
            "compiler has no cache adapter; passing through uncached; program={:?}",
            args.first()
        );
        return run_compiler_directly_for(&config, args, &env);
    }

    let Some(adapter) = compiler::detect_compiler_for(args, &env) else {
        anyhow::bail!(
            "wrapper-mode dispatched but no compiler adapter matched argv[0] = {:?}",
            args.first()
        );
    };
    let exit_code = match wrapper_target(adapter) {
        Some(WrapperTarget::Rustc) => {
            // Started here so the trace also covers the OUT_DIR alias check.
            let trace = phase_trace::start_for("rustc", args, &env);
            let exit = match out_dir_alias::apply_for(&config, args, &mut env) {
                Some(exit) => exit,
                None => wrapper::run_for(&config, args, &env)?,
            };
            drop(trace);
            exit
        }
        Some(WrapperTarget::Cc) => wrapper::run_cc_for(&config, args, &env)?,
        None => anyhow::bail!(
            "detected compiler adapter {} ({}) has no wrapper dispatch",
            adapter.id(),
            adapter.display_name()
        ),
    };
    Ok(exit_code)
}

/// What `kache clean --cache` removes, from its hidden flags.
fn cache_clean_choice(stale_schema: bool, stale_hours: Option<u64>) -> cli::CacheClean {
    match stale_hours {
        _ if stale_schema => cli::CacheClean::StaleSchema,
        Some(hours) => cli::CacheClean::OlderThan(hours),
        None => cli::CacheClean::All,
    }
}

/// `kache init --shims` and the hidden `kache install-shims`: compiler-name
/// links to kache in `dir`, by default `~/.local/lib/kache/shims`.
#[cfg(unix)]
fn install_shims(
    dir: Option<PathBuf>,
    force: bool,
    from_path: bool,
    canonical_only: bool,
) -> Result<()> {
    let dir = dir
        .or_else(compiler::shim::default_shim_dir)
        .context("no home directory; pass the shim directory to use")?;
    let extra = if canonical_only {
        Vec::new()
    } else {
        let path = std::env::var_os("PATH").unwrap_or_default();
        compiler::shim::names_on_path(&path, from_path)
    };
    cli::install_shims_named(&dir, force, &extra)
}

/// Shims are Unix-only (symlinks); the Windows `.exe` shim story differs
/// (#310).
#[cfg(not(unix))]
const UNIX_ONLY_SHIMS: &str = "shim installation is Unix-only for now: it relies on symlinks, and \
     the Windows `.exe` shim story differs (kunobi-ninja/kache#310). Use `CC`/`CXX` there.";

/// Entries in each top list of the full report by default.
const DEFAULT_REPORT_TOP: usize = 10;

/// `--full`, or any flag that only the full report understands.
fn stats_wants_full(
    full: bool,
    format: bool,
    output: bool,
    top: bool,
    record: bool,
    redact: bool,
) -> bool {
    full || format || output || top || record || redact
}

/// The full report's format: the one asked for, else JSON under `--json`,
/// else text.
fn report_format(format: Option<cli::ReportFormat>, json: bool) -> Result<cli::ReportFormat> {
    use cli::ReportFormat;
    if json {
        if !matches!(format, None | Some(ReportFormat::Json | ReportFormat::Text)) {
            return Err(machine::invalid_arguments(
                "`--json` cannot be combined with a different `--format`.",
            ));
        }
        return Ok(ReportFormat::Json);
    }
    Ok(format.unwrap_or(ReportFormat::Text))
}

/// Parse a `--since` value, or fail loudly. Falling back to the default on a
/// value that did not parse is how `--since 15m` came to report a 24h window
/// labelled as such (kunobi-ninja/kache#897). An empty window is refused.
fn parse_since_window(value: &str) -> Result<since::SinceWindow> {
    let window = parse_duration("--since", value)?;
    if window.secs() == 0 {
        return Err(machine::invalid_arguments(
            "--since must be longer than zero",
        ));
    }
    Ok(window)
}

/// Parse `--stale` or `--max-age` with the `--since` grammar, in whole hours:
/// the store and the daemon protocol count age in hours. A sub-hour value
/// rounds up, so `30m` keeps what an hour keeps and never becomes 0. An
/// explicit 0 still means any age.
fn parse_age_hours(flag: &str, value: &str) -> Result<u64> {
    Ok(parse_duration(flag, value)?.secs().div_ceil(3600))
}

/// The one duration grammar: `90s`, `15m`, `2h`, `7d`, or a bare number of
/// hours.
fn parse_duration(flag: &str, value: &str) -> Result<since::SinceWindow> {
    since::SinceWindow::parse(value).ok_or_else(|| {
        machine::invalid_arguments(format!(
            "invalid {flag} {value:?}; expected a duration like 15m, 2h, or 7d (a bare number is hours)"
        ))
    })
}

/// What `kache daemon start` says it found or did.
fn daemon_start_message(outcome: &daemon::Ensured) -> String {
    match outcome {
        daemon::Ensured::AlreadyRunning(Some(version)) => {
            format!("daemon already running (version {version})")
        }
        daemon::Ensured::AlreadyRunning(None) => "daemon already running".to_string(),
        daemon::Ensured::Started => "daemon started".to_string(),
        daemon::Ensured::TimedOut => "daemon did not start within timeout".to_string(),
    }
}

/// What `kache daemon restart` says, from whether a daemon owned the cache
/// before and whether one is ready now.
fn daemon_restart_message(was_running: bool, ready: bool) -> &'static str {
    match (was_running, ready) {
        (_, false) => "daemon did not start within timeout",
        (true, true) => "daemon restarted",
        (false, true) => "daemon was not running; daemon started",
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn original_compiler_requests_use_only_the_supplied_arguments_and_environment() {
        let root = tempfile::tempdir().unwrap();
        let env = invocation::InvocationEnv::from_parts(
            Vec::<(std::ffi::OsString, std::ffi::OsString)>::new(),
            Some(root.path().to_owned()),
            None,
            root.path().to_owned(),
        );
        for arguments in [
            vec!["cc", "-c", "source.c"],
            vec!["kache", "rustc", "--version"],
            vec!["kache", "cc", "-c", "source.c"],
            vec!["kache", "/client/workspace-wrapper", "rustc", "--version"],
            vec!["kache", "-E", "probe.c"],
        ] {
            let arguments = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(original_compiler_request(&arguments, &env), "{arguments:?}");
        }
        for arguments in [
            vec![],
            vec!["kache"],
            vec!["kache", "stats"],
            vec!["cargo", "rustc"],
            vec!["kache", "unrecognized-tool", "--version"],
        ] {
            let arguments = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(
                !original_compiler_request(&arguments, &env),
                "{arguments:?}"
            );
        }
        let custom = ["kache", "custom-rust-compiler", "--version"].map(str::to_owned);
        let mut configured = env.clone();
        configured.set("RUSTC", "custom-rust-compiler");
        assert!(original_compiler_request(&custom, &configured));
        assert!(!original_compiler_request(&custom, &env));
        let rustc = ["kache", "rustc", "--version"].map(str::to_owned);
        for (name, value) in [
            ("KACHE_FAMILY_PROBE_ACTIVE", "1"),
            (build_script::SHIM_PATH_ENV, "shim"),
            (platform::SELF_SPAWN_ENV, "rustc"),
        ] {
            let mut blocked = env.clone();
            blocked.set(name, value);
            assert!(!original_compiler_request(&rustc, &blocked), "{name}");
        }
        assert!(original_compiler_request(&rustc, &env));
    }

    #[test]
    fn wrapper_file_logging_requires_an_explicit_filter() {
        let root = tempfile::tempdir().unwrap();
        let mut env = invocation::InvocationEnv::from_parts(
            [("KACHE_LOG", "off"), ("KACHE_LOG_FILE_PATH", "wrapper.log")],
            Some(root.path().to_owned()),
            None,
            root.path().to_owned(),
        );
        {
            let logging = logging_logger(LogMode::Wrapper, &env);
            logging.emit(log::Level::Info, "kache::request", "without-file-opt-in");
        }
        assert!(!root.path().join("wrapper.log").exists());
        env.set("KACHE_LOG_FILE", "kache=info");
        {
            let logging = logging_logger(LogMode::Wrapper, &env);
            logging.emit(log::Level::Info, "kache::request", "with-file-opt-in");
        }
        let logged = std::fs::read_to_string(root.path().join("wrapper.log")).unwrap();
        assert!(logged.contains("with-file-opt-in"));
        assert!(!logged.contains("without-file-opt-in"));
    }

    #[test]
    fn default_diagnostic_logs_use_the_clients_home_or_cache_directory() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("client-home");
        let cache = root.path().join("client-cache");
        let env = invocation::InvocationEnv::from_parts(
            [("KACHE_LOG", "off")],
            Some(root.path().to_owned()),
            Some(home.clone()),
            root.path().to_owned(),
        )
        .with_cache_dir(Some(cache.clone()));
        let expected = if cfg!(target_os = "macos") {
            home.join("Library/Logs/kache/kache.log")
        } else {
            cache.join("kache/kache.log")
        };
        assert_eq!(diagnostic_log_path_for(&env), expected);
        {
            let logging = logging_logger(LogMode::Cli, &env);
            logging.emit(log::Level::Info, "kache::request", "client-default-log");
        }
        assert!(
            std::fs::read_to_string(expected)
                .unwrap()
                .contains("client-default-log")
        );
    }

    #[test]
    fn client_routes_every_engine_command_and_alias_without_loading_config() {
        for name in kache_client::engine_command_names() {
            // `telemetry` requires a subcommand; probe it with its minimal
            // shape. Everything else must route bare.
            let argv: Vec<&str> = match *name {
                "telemetry" => vec!["kache", name, "write", "telemetry-probe-dir"],
                _ => vec!["kache", name],
            };
            match parsed(&argv) {
                Ok(cli_args::ParseOutcome::Run(_)) => {}
                Ok(_) if *name == "help" => {}
                other => panic!("{name} does not parse: {}", other.is_ok()),
            }
        }
    }

    fn logging_test_io(root: &std::path::Path, name: &str) -> invocation::io::InvocationIo {
        let input = root.join(format!("{name}.in"));
        std::fs::write(&input, b"").unwrap();
        invocation::io::InvocationIo::from_files(
            std::fs::File::open(input).unwrap(),
            std::fs::File::create(root.join(format!("{name}.out"))).unwrap(),
            std::fs::File::create(root.join(format!("{name}.err"))).unwrap(),
            None,
        )
    }

    #[test]
    fn concurrent_invocation_logging_uses_client_filters_paths_and_stderr() {
        let root = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|threads| {
            for (name, stderr_filter, file_filter) in [
                ("first", "kache=trace", "off"),
                ("second", "kache=error", "kache=warn"),
            ] {
                let root = root.path().join(name);
                std::fs::create_dir(&root).unwrap();
                let barrier = &barrier;
                threads.spawn(move || {
                    let env = invocation::InvocationEnv::from_parts(
                        [
                            ("KACHE_LOG", stderr_filter),
                            ("KACHE_LOG_FILE", file_filter),
                            ("KACHE_LOG_FILE_PATH", "logs/wrapper.log"),
                        ],
                        Some(root.clone()),
                        Some(root.clone()),
                        root.clone(),
                    );
                    let logging = logging_logger(LogMode::Wrapper, &env);
                    let _io =
                        invocation::io::InvocationIoScope::enter(logging_test_io(&root, name));
                    barrier.wait();
                    for _ in 0..2 {
                        logging.emit(log::Level::Warn, "kache::request", &format!("own-warning client={name}"));
                        logging.emit(log::Level::Error, "kache::request", &format!("own-error client={name}"));
                        logging.emit(log::Level::Debug, "kache::request", &format!("own-debug client={name}"));
                    }
                });
            }
        });
        let first = std::fs::read_to_string(root.path().join("first/first.err")).unwrap();
        let second = std::fs::read_to_string(root.path().join("second/second.err")).unwrap();
        assert_eq!(first.matches("own-warning").count(), 2);
        assert_eq!(first.matches("own-error").count(), 2);
        assert_eq!(first.matches("own-debug").count(), 2);
        assert!(!first.contains("second"));
        assert_eq!(second.matches("own-error").count(), 2);
        assert!(!second.contains("own-warning"));
        assert!(!second.contains("own-debug"));
        assert!(!second.contains("first"));
        assert_eq!(
            std::fs::read(root.path().join("first/logs/wrapper.log")).unwrap(),
            b""
        );
        let log = std::fs::read_to_string(root.path().join("second/logs/wrapper.log")).unwrap();
        assert_eq!(log.matches("own-warning").count(), 2);
        assert_eq!(log.matches("own-error").count(), 2);
        assert!(!log.contains("own-debug"));
        assert!(!log.contains("first"));
    }

    #[test]
    fn invocation_logging_restores_parent_and_monitors_do_not_own_client_pipes() {
        let root = tempfile::tempdir().unwrap();
        let env = |filter: &str| {
            invocation::InvocationEnv::from_parts(
                [("KACHE_LOG", filter)],
                Some(root.path().to_owned()),
                None,
                root.path().to_owned(),
            )
        };
        let parent = logging_logger(LogMode::Wrapper, &env("kache=warn"));
        let monitor = {
            let _io =
                invocation::io::InvocationIoScope::enter(logging_test_io(root.path(), "parent"));
            parent.emit(log::Level::Warn, "kache::request", "parent-before");
            {
                let nested = logging_logger(LogMode::Wrapper, &env("invalid["));
                nested.emit(log::Level::Error, "kache::request", "nested-default-off");
            }
            parent.emit(log::Level::Warn, "kache::request", "parent-after");
            invocation::io::current().borrow_for_monitor()
        };
        let log = std::fs::read_to_string(root.path().join("parent.err")).unwrap();
        assert!(log.contains("parent-before"));
        assert!(log.contains("parent-after"));
        assert!(!log.contains("nested-default-off"));
        assert!(!root.path().join("logs").exists());
        let _io = invocation::io::InvocationIoScope::enter(monitor);
        use std::io::Write;
        assert_eq!(
            invocation::io::stderr().write(b"late").unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        parent.emit(log::Level::Warn, "kache::request", "late-monitor");
        assert_eq!(
            std::fs::read_to_string(root.path().join("parent.err")).unwrap(),
            log
        );
    }

    #[test]
    fn cancelled_invocation_stops_before_dispatch() {
        let cancellation = invocation::process::Cancellation::default();
        cancellation.cancel();
        let error = run_compiler_invocation(
            &["missing-compiler".into()],
            invocation::InvocationContext {
                daemon: None,
                env: invocation::InvocationEnv::default(),
                io: invocation::io::InvocationIo::process(),
                cancellation: Some(cancellation),
                #[cfg(unix)]
                jobserver: kache_ipc::jobserver::Descriptors::default(),
                started_at: std::time::Instant::now(),
                shim_stem: "kache".into(),
            },
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::Interrupted
        );
        invocation::process::check().unwrap();
    }

    use super::*;

    #[cfg(unix)]
    #[test]
    fn compiler_entry_returns_each_clients_status_and_restores_request_state() {
        use std::os::unix::fs::PermissionsExt;

        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|threads| {
            for (root, client, nested, code) in [
                (first.path(), "first", true, 17),
                (second.path(), "second", false, 31),
            ] {
                std::fs::create_dir(root.join("bin")).unwrap();
                std::fs::write(root.join(".kache.toml"), "").unwrap();
                std::fs::write(root.join("lib.rs"), "pub fn value() {}\n").unwrap();
                for name in ["rustc", "cc-probe"] {
                    kache_fs::testutil::write_executable(
                        &root.join("bin").join(name),
                        "#!/bin/sh\nif [ \"$1\" != -E ]; then [ ! -e unit.rlib ] || exit 91; printf '%s' \"$CLIENT\" > unit.rlib; fi\nprintf '%s|%s|%s|%s' \"$CLIENT\" \"$KACHE_ACTIVE\" \"$CARGO_INCREMENTAL\" \"$PWD\" > observed-env\nexit \"$CLIENT_EXIT\"\n",
                    );
                }
                let barrier = &barrier;
                threads.spawn(move || {
                    let physical_root = std::fs::canonicalize(root).unwrap();
                    let _parent = wrapper::InvocationWrapperScope::enter();
                    demand::record(client);
                    verify_compare::record_report(client.to_owned());
                    let make_env = |disabled: bool| {
                        let mut env = invocation::InvocationEnv::from_parts(
                            [
                                ("PATH", "bin"),
                                ("CLIENT", client),
                                ("CARGO_INCREMENTAL", "1"),
                            ],
                            Some(root.to_owned()),
                            Some(root.to_owned()),
                            root.to_owned(),
                        );
                        env.set("KACHE_DISABLED", if disabled { "1" } else { "0" });
                        env.set("KACHE_CONFIG", root.join(".kache.toml"));
                        env.set("KACHE_CACHE_DIR", root.join("cache"));
                        env.set("KACHE_RUNTIME_DIR", root.join("runtime"));
                        env
                    };
                    barrier.wait();
                    for iteration in 0..2 {
                        let mut env = make_env(true);
                        if nested {
                            env.set(KACHE_ACTIVE_ENV, "2");
                        }
                        env.set("CLIENT_EXIT", (code + iteration).to_string());
                        std::fs::write(root.join("unit.rlib"), "old restore").unwrap();
                        std::fs::set_permissions(
                            root.join("unit.rlib"),
                            std::fs::Permissions::from_mode(0o444),
                        )
                        .unwrap();
                        let result = run_compiler_invocation(
                            &argv(&[
                                "rustc",
                                "--crate-name",
                                "demo",
                                "--crate-type",
                                "lib",
                                "lib.rs",
                                "-o",
                                "unit.rlib",
                            ]),
                            invocation::InvocationContext {
                                daemon: None,
                                env: env.clone(),
                                io: invocation::io::InvocationIo::capture(),
                                cancellation: None,
                                #[cfg(unix)]
                                jobserver: kache_ipc::jobserver::Descriptors::default(),
                                started_at: std::time::Instant::now(),
                                shim_stem: "client-kache".into(),
                            },
                        )
                        .unwrap();
                        assert_eq!(result, code + iteration);
                        assert_eq!(
                            std::fs::read_to_string(root.join("observed-env")).unwrap(),
                            format!(
                                "{client}|{}|1|{}",
                                if nested { "3" } else { "1" },
                                physical_root.display()
                            )
                        );
                        assert_eq!(
                            std::fs::read_to_string(root.join("unit.rlib")).unwrap(),
                            client
                        );
                        assert_eq!(
                            env.var_os(KACHE_ACTIVE_ENV),
                            nested.then(|| std::ffi::OsString::from("2"))
                        );
                    }
                    let mut env = make_env(false);
                    env.set("CC", "client-kache cc-probe");
                    env.set("CLIENT_EXIT", "43");
                    assert_eq!(
                        run_compiler_invocation(
                            &argv(&["-E", "input.c"]),
                            invocation::InvocationContext {
                                daemon: None,
                                env,
                                io: invocation::io::InvocationIo::capture(),
                                cancellation: None,
                                #[cfg(unix)]
                                jobserver: kache_ipc::jobserver::Descriptors::default(),
                                started_at: std::time::Instant::now(),
                                shim_stem: "client-kache".into(),
                            }
                        )
                        .unwrap(),
                        43
                    );
                    assert_eq!(
                        std::fs::read_to_string(root.join("observed-env")).unwrap(),
                        format!("{client}|1|1|{}", physical_root.display())
                    );
                    let demands = demand::take();
                    assert_eq!(demands.len(), 1);
                    assert_eq!(demands[0].cache_key, client);
                    assert_eq!(verify_compare::take_last_report(), client);
                });
            }
        });
    }

    #[cfg(unix)]
    fn write_success_program(path: &std::path::Path) {
        let shell = compiler::resolve_program_on_path("sh").expect("sh must be available on PATH");
        kache_fs::testutil::write_executable(path, format!("#!{}\nexit 0\n", shell.display()));
    }

    #[cfg(unix)]
    fn run_generated_compiler_process_directly(
        args: &[String],
        preserve_incremental: bool,
    ) -> Result<i32> {
        match run_compiler_process_directly(args, preserve_incremental) {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.raw_os_error() == Some(libc::ETXTBSY)) =>
            {
                // Some Linux filesystems briefly keep a just-written test
                // executable busy even after its writer has closed.
                std::thread::sleep(std::time::Duration::from_millis(20));
                run_compiler_process_directly(args, preserve_incremental)
            }
            result => result,
        }
    }

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    /// The auto-GC worker started from a wrapper behind a shim, on macOS where
    /// `current_exe` is the shim path. Before the fix it routed to shim mode
    /// and handed `gc` to the real compiler.
    #[test]
    fn a_self_spawned_gc_under_a_shim_path_routes_to_the_cli() {
        let gc = Some(std::ffi::OsStr::new("gc"));
        let args = argv(&["/x/shims/cc", "gc"]);
        assert_eq!(entry_route(&args, gc), EntryRoute::SelfSpawn);
        assert_eq!(
            detect_log_mode_with_rustc(&args, None),
            LogMode::Cli,
            "the rest of argv must then parse as the gc subcommand"
        );
        // A Windows shim is a copy, so argv[0] stays the compiler's name.
        let daemon = Some(std::ffi::OsStr::new("daemon"));
        let args = argv(&["C:/shims/gcc.exe", "daemon", "run"]);
        assert_eq!(entry_route(&args, daemon), EntryRoute::SelfSpawn);
        assert_eq!(detect_log_mode_with_rustc(&args, None), LogMode::Cli);
    }

    /// The marker alone is not enough: a compile run under a leaked marker
    /// still goes to the compiler.
    #[test]
    fn a_compile_through_a_shim_stays_a_compile() {
        let gc = Some(std::ffi::OsStr::new("gc"));
        let compile = argv(&["/x/shims/cc", "-c", "foo.c"]);
        assert_eq!(entry_route(&compile, gc), EntryRoute::Shim);
        assert_eq!(entry_route(&compile, None), EntryRoute::Shim);
        // Without the marker, `cc gc` compiles a file named gc.
        assert_eq!(
            entry_route(&argv(&["/x/shims/cc", "gc"]), None),
            EntryRoute::Shim
        );
    }

    #[test]
    fn kache_under_its_own_name_routes_on_the_rest_of_argv() {
        assert_eq!(entry_route(&argv(&["kache", "gc"]), None), EntryRoute::Argv);
        assert_eq!(
            entry_route(&argv(&["kache", "cc", "-c", "foo.c"]), None),
            EntryRoute::Argv
        );
        assert_eq!(entry_route(&[], None), EntryRoute::Argv);
        // The self-spawn marker wins over any argv[0].
        assert_eq!(
            entry_route(
                &argv(&["kache", "daemon", "run"]),
                Some(std::ffi::OsStr::new("daemon"))
            ),
            EntryRoute::SelfSpawn
        );
    }

    #[test]
    fn wrappers_above_reads_the_depth_and_treats_old_values_as_one() {
        use std::ffi::OsStr;
        assert_eq!(wrappers_above(None), None, "the outermost wrapper");
        assert_eq!(wrappers_above(Some(OsStr::new("1"))), Some(1));
        assert_eq!(wrappers_above(Some(OsStr::new("5"))), Some(5));
        assert_eq!(wrappers_above(Some(OsStr::new(""))), Some(1));
        assert_eq!(wrappers_above(Some(OsStr::new("yes"))), Some(1));
    }

    #[test]
    fn a_nested_wrapper_hands_down_one_more_level() {
        assert_eq!(next_wrapper_depth(1), 2);
        assert_eq!(next_wrapper_depth(7), 8);
        assert_eq!(next_wrapper_depth(u32::MAX), u32::MAX);
    }

    /// Pinned at the bound: one level short still runs, the bound itself
    /// stops the chain with a message that says how to fix it.
    #[test]
    fn wrapper_depth_is_refused_at_the_bound() {
        assert!(check_wrapper_depth(1, Some("cc")).is_ok());
        assert!(check_wrapper_depth(MAX_WRAPPERS_ABOVE - 1, Some("cc")).is_ok());
        let err = check_wrapper_depth(MAX_WRAPPERS_ABOVE, Some("/copies/cc"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("/copies/cc"), "{err}");
        assert!(err.contains("loop"), "{err}");
        assert!(err.contains(".kache-shims"), "{err}");
        assert!(check_wrapper_depth(MAX_WRAPPERS_ABOVE + 1, None).is_err());
    }

    #[test]
    fn stats_wants_the_full_report_for_any_full_flag() {
        assert!(!stats_wants_full(false, false, false, false, false, false));
        assert!(stats_wants_full(true, false, false, false, false, false));
        assert!(stats_wants_full(false, true, false, false, false, false));
        assert!(stats_wants_full(false, false, true, false, false, false));
        assert!(stats_wants_full(false, false, false, true, false, false));
        assert!(stats_wants_full(false, false, false, false, true, false));
        assert!(stats_wants_full(false, false, false, false, false, true));
    }

    fn is_invalid_arguments(error: &anyhow::Error) -> bool {
        machine::failure_exit(error) == Some(("invalid_arguments", 2))
    }

    fn parsed(argv: &[&str]) -> Result<cli_args::ParseOutcome, cli_args::ParseFail> {
        let args: Vec<std::ffi::OsString> =
            argv.iter().skip(1).map(std::ffi::OsString::from).collect();
        cli_args::parse_cli(&args)
    }

    fn parse(argv: &[&str]) -> Cli {
        match parsed(argv) {
            Ok(cli_args::ParseOutcome::Run(cli)) => cli,
            other => panic!("expected a parsed invocation for {argv:?}: {}", other.is_ok()),
        }
    }

    fn parse_err(argv: &[&str]) -> anyhow::Error {
        match parsed(argv) {
            Err(failure) => failure.error,
            Ok(_) => panic!("expected a parse failure for {argv:?}"),
        }
    }

    fn help_text(argv: &[&str]) -> String {
        match parsed(argv) {
            Ok(cli_args::ParseOutcome::Help { text, .. }) => text,
            other => panic!("expected help for {argv:?}: {}", other.is_ok()),
        }
    }

    #[test]
    fn report_format_prefers_an_explicit_name() {
        use cli::ReportFormat;
        assert_eq!(report_format(None, false).unwrap(), ReportFormat::Text);
        assert_eq!(report_format(None, true).unwrap(), ReportFormat::Json);
        assert_eq!(
            report_format(Some(ReportFormat::Markdown), false).unwrap(),
            ReportFormat::Markdown
        );
        assert_eq!(
            report_format(Some(ReportFormat::Text), true).unwrap(),
            ReportFormat::Json
        );
        assert_eq!(
            report_format(Some(ReportFormat::Json), true).unwrap(),
            ReportFormat::Json
        );
        for other in [
            ReportFormat::Markdown,
            ReportFormat::Github,
            ReportFormat::Trace,
        ] {
            let error = report_format(Some(other), true).unwrap_err();
            assert!(is_invalid_arguments(&error), "{error:#}");
        }
    }

    /// Every spelling the flag took as free text still parses, and a typo
    /// is a usage error that lists the values.
    #[test]
    fn report_formats_keep_every_old_spelling_and_refuse_typos() {
        use cli::ReportFormat;
        for (spelling, format) in [
            ("text", ReportFormat::Text),
            ("json", ReportFormat::Json),
            ("markdown", ReportFormat::Markdown),
            ("md", ReportFormat::Markdown),
            ("github", ReportFormat::Github),
            ("gh", ReportFormat::Github),
            ("trace", ReportFormat::Trace),
            ("perfetto", ReportFormat::Trace),
            ("chrome-trace", ReportFormat::Trace),
        ] {
            let stats = parse(&["kache", "stats", "--format", spelling]);
            assert!(
                matches!(stats.command, Some(Commands::Stats { format: Some(f), .. }) if f == format),
                "{spelling}"
            );
            let report = parse(&["kache", "report", "--format", spelling]);
            assert!(
                matches!(report.command, Some(Commands::Report { format: f, .. }) if f == format),
                "{spelling}"
            );
            assert_eq!(
                ReportFormat::parse(format.name()),
                Some(format)
            );
        }
        let report = parse(&["kache", "report"]);
        assert!(matches!(
            report.command,
            Some(Commands::Report {
                format: ReportFormat::Text,
                ..
            })
        ));
        for args in [
            ["kache", "stats", "--format", "jsno"],
            ["kache", "report", "--format", "html"],
        ] {
            let error = parse_err(&args);
            assert!(is_invalid_arguments(&error), "{error:#}");
            assert!(error.to_string().contains("markdown"), "{error}");
        }
    }

    #[test]
    fn list_sort_and_bench_phase_take_only_known_values() {
        for (spelling, sort) in [
            ("name", ListSort::Name),
            ("size", ListSort::Size),
            ("hits", ListSort::Hits),
            ("age", ListSort::Age),
        ] {
            let list = parse(&["kache", "list", "--sort", spelling]);
            assert!(matches!(list.command, Some(Commands::List { sort: s, .. }) if s == sort));
            assert_eq!(sort.as_str(), spelling);
        }
        let list = parse(&["kache", "list"]);
        assert!(matches!(
            list.command,
            Some(Commands::List {
                sort: ListSort::Name,
                ..
            })
        ));
        let error = parse_err(&["kache", "list", "--sort", "sise"]);
        assert!(is_invalid_arguments(&error), "{error:#}");

        for (spelling, phase) in [
            ("cold", BenchPhase::Cold),
            ("warm-same-tree", BenchPhase::WarmSameTree),
            ("warm", BenchPhase::Warm),
            ("pull", BenchPhase::Pull),
        ] {
            let write = parse(&["kache", "telemetry", "write", "d", "--phase", spelling]);
            assert!(matches!(
                write.command,
                Some(Commands::Telemetry {
                    command: TelemetryCommands::Write { phase: Some(p), .. }
                }) if p == phase
            ));
            assert_eq!(phase.as_str(), spelling);
        }
        let error = parse_err(&["kache", "telemetry", "write", "d", "--phase", "hot"]);
        assert!(is_invalid_arguments(&error), "{error:#}");
    }

    #[test]
    fn parse_since_window_accepts_minutes_and_rejects_garbage() {
        assert_eq!(parse_since_window("15m").unwrap().secs(), 900);
        assert_eq!(parse_since_window("2h").unwrap().secs(), 7200);
        assert_eq!(parse_since_window("24").unwrap().secs(), 86_400);
        assert_eq!(parse_since_window("1s").unwrap().secs(), 1);
        let err = parse_since_window("soon").unwrap_err();
        assert!(is_invalid_arguments(&err), "{err:#}");
        let err = err.to_string();
        assert!(err.contains("invalid --since \"soon\""), "{err}");
        assert!(err.contains("15m"), "{err}");
        for empty in ["0", "0h", "0s"] {
            let err = parse_since_window(empty).unwrap_err();
            assert!(is_invalid_arguments(&err), "{err:#}");
            assert!(err.to_string().contains("longer than zero"), "{err}");
        }
    }

    /// `--stale` and `--max-age` take the `--since` forms. Sub-hour values
    /// round up to a whole hour instead of down to 0, which means any age.
    #[test]
    fn age_flags_share_the_since_grammar_and_round_up_to_hours() {
        assert_eq!(parse_age_hours("--stale", "7d").unwrap(), 168);
        assert_eq!(parse_age_hours("--stale", "24h").unwrap(), 24);
        assert_eq!(parse_age_hours("--stale", "48").unwrap(), 48);
        assert_eq!(parse_age_hours("--stale", "30m").unwrap(), 1);
        assert_eq!(parse_age_hours("--stale", "1s").unwrap(), 1);
        assert_eq!(parse_age_hours("--stale", "60m").unwrap(), 1);
        assert_eq!(parse_age_hours("--stale", "61m").unwrap(), 2);
        assert_eq!(parse_age_hours("--max-age", "0h").unwrap(), 0);
        // Accepted before the flags shared the grammar: a leading `+`, and
        // hour counts too large to hold in seconds, which now saturate.
        assert_eq!(parse_age_hours("--stale", "+24h").unwrap(), 24);
        assert_eq!(parse_age_hours("--stale", "+48").unwrap(), 48);
        for huge in ["18446744073709551615h", "18446744073709551615d"] {
            assert_eq!(
                parse_age_hours("--max-age", huge).unwrap(),
                u64::MAX.div_ceil(3600),
                "{huge}"
            );
        }
        assert_eq!(parse_since_window("+2h").unwrap().secs(), 7200);
        assert!(is_invalid_arguments(&parse_since_window("+0").unwrap_err()));
        for bad in ["invalid", "+", "1.5h"] {
            let err = parse_age_hours("--max-age", bad).unwrap_err();
            assert!(is_invalid_arguments(&err), "{err:#}");
            assert!(err.to_string().contains("invalid --max-age"), "{err}");
        }
    }

    #[test]
    fn run_cli_rejects_unsupported_json_before_loading_configuration() {
        let cli = parse(&["kache", "purge", "--json"]);
        let error = run_cli(cli, None).unwrap_err();
        assert!(is_invalid_arguments(&error), "{error:#}");
        assert!(error.to_string().contains("purge"), "{error:#}");
    }

    #[test]
    fn json_refusals_name_the_command_or_list_visible_ones() {
        let general = json_refusal(&None).unwrap();
        assert!(general.contains("is supported on"), "{general}");
        for hidden in ["report", "gc", "diff", "targets"] {
            assert!(!general.contains(hidden), "{hidden} is hidden: {general}");
        }
        assert_eq!(
            json_refusal(&Some(Commands::Telemetry {
                command: TelemetryCommands::Write {
                    dir: std::path::PathBuf::new(),
                    scenario: None,
                    phase: None,
                }
            })),
            Some(general)
        );
        assert_eq!(json_refusal(&Some(Commands::Config { command: None })), None);
        assert!(
            json_refusal(&Some(Commands::Purge { crate_name: None }))
                .unwrap()
                .contains("kache purge")
        );
        let shims = parse(&["kache", "init", "--shims"]);
        assert!(json_refusal(&shims.command).unwrap().contains("--shims"));
        let init = parse(&["kache", "init", "--check"]);
        assert_eq!(json_refusal(&init.command), None);
    }

    #[test]
    fn daemon_lifecycle_messages_say_what_happened() {
        assert_eq!(
            daemon_start_message(&daemon::Ensured::AlreadyRunning(Some("1.2.3".into()))),
            "daemon already running (version 1.2.3)"
        );
        assert_eq!(
            daemon_start_message(&daemon::Ensured::AlreadyRunning(None)),
            "daemon already running"
        );
        assert_eq!(
            daemon_start_message(&daemon::Ensured::Started),
            "daemon started"
        );
        assert_eq!(
            daemon_start_message(&daemon::Ensured::TimedOut),
            "daemon did not start within timeout"
        );
        assert!(daemon::Ensured::Started.is_ready());
        assert!(daemon::Ensured::AlreadyRunning(None).is_ready());
        assert!(!daemon::Ensured::TimedOut.is_ready());

        assert_eq!(daemon_restart_message(true, true), "daemon restarted");
        assert_eq!(
            daemon_restart_message(false, true),
            "daemon was not running; daemon started"
        );
        for was_running in [true, false] {
            assert_eq!(
                daemon_restart_message(was_running, false),
                "daemon did not start within timeout"
            );
        }
    }

    #[test]
    fn errors_reaching_main_exit_one_and_a_closed_stdout_exits_zero() {
        assert_eq!(report_main_error(&anyhow::anyhow!("boom")), 1);
        assert_eq!(
            report_main_error(&machine::stdout_error(std::io::Error::from(
                std::io::ErrorKind::BrokenPipe
            ))),
            0
        );
    }

    #[test]
    fn status_runs_stats_and_daemon_status_stays_the_daemons() {
        let status = parse(&["kache", "status", "--since", "2h"]);
        assert!(matches!(
            status.command,
            Some(Commands::Stats { ref since, .. }) if since == "2h"
        ));
        let daemon = parse(&["kache", "daemon", "status"]);
        assert!(matches!(
            daemon.command,
            Some(Commands::Daemon {
                command: Some(DaemonCommands::Status)
            })
        ));
    }

    #[test]
    fn clean_flags_keep_targets_and_cache_apart() {
        assert!(parsed(&["kache", "clean", "--cache"]).is_ok());
        assert!(
            parsed(&["kache", "clean", "--cache", "--all"]).is_ok(),
            "older scripts still parse"
        );
        assert!(parsed(&["kache", "clean", "--crate", "serde", "--yes"]).is_ok());
        assert!(parsed(&["kache", "clean", "--cache", "--stale", "7d"]).is_ok());
        assert!(parsed(&["kache", "clean", "some/dir", "--dry-run"]).is_err());
        assert!(parsed(&["kache", "clean", "--all"]).is_err(), "--all needs --cache");
        assert!(
            parsed(&["kache", "clean", "--stale-schema"]).is_err(),
            "--stale-schema needs --cache"
        );
        assert!(parsed(&["kache", "clean", "some/dir", "--cache"]).is_err());
        for removed in ["--units", "--artifacts", "--stale", "--orphans", "--tracked"] {
            assert!(
                parsed(&["kache", "clean", "some/dir", removed]).is_err(),
                "{removed} is gone"
            );
        }
        assert!(parsed(&["kache", "clean", "--crate", "serde", "--cache"]).is_err());
        for hidden in ["gc", "purge", "report"] {
            assert!(parsed(&["kache", hidden]).is_ok(), "{hidden}");
        }
    }

    #[test]
    fn clean_flags_pick_what_to_remove() {
        use cli::CacheClean;
        assert_eq!(cache_clean_choice(false, None), CacheClean::All);
        assert_eq!(
            cache_clean_choice(false, Some(24)),
            CacheClean::OlderThan(24)
        );
        assert_eq!(cache_clean_choice(true, None), CacheClean::StaleSchema);
    }

    #[test]
    fn init_shims_and_why_miss_without_a_crate_parse() {
        let shims =
            parse(&["kache", "init", "--shims", "--from-path", "--force"]);
        assert!(matches!(
            shims.command,
            Some(Commands::Init {
                shims: Some(None),
                from_path: true,
                force: true,
                ..
            })
        ));
        let package =
            parse(&["kache", "init", "--shims", "/usr/lib/kache", "--force"]);
        assert!(matches!(
            package.command,
            Some(Commands::Init { shims: Some(Some(ref dir)), force: true, .. })
                if dir == std::path::Path::new("/usr/lib/kache")
        ));
        let plain = parse(&["kache", "init"]);
        assert!(matches!(
            plain.command,
            Some(Commands::Init { shims: None, .. })
        ));
        assert!(
            parsed(&["kache", "init", "--from-path"]).is_err(),
            "needs --shims"
        );
        assert!(parsed(&["kache", "init", "--shims", "--check"]).is_err());
        let canonical =
            parse(&["kache", "init", "--shims", "--canonical-only"]);
        assert!(matches!(
            canonical.command,
            Some(Commands::Init {
                canonical_only: true,
                from_path: false,
                ..
            })
        ));
        assert!(parsed(&["kache", "init", "--canonical-only"]).is_err());
        assert!(
            parsed(&[
                "kache",
                "init",
                "--shims",
                "--canonical-only",
                "--from-path"
            ])
            .is_err()
        );
        assert!(parsed(&["kache", "why-miss", "--root", "x"]).is_err());
        assert!(parsed(&["kache", "why-miss", "serde", "--root", "x"]).is_err());
        assert!(parsed(&["kache", "install-shims"]).is_ok());
    }

    #[test]
    fn dry_run_spellings_reach_every_preview() {
        for spelling in ["--check", "--dry-run", "-n"] {
            let parsed = parse(&["kache", "init", spelling, "--no-service"]);
            assert!(
                matches!(
                    parsed.command,
                    Some(Commands::Init {
                        check: true,
                        no_service: true,
                        ..
                    })
                ),
                "{spelling}"
            );
        }
        let invocation = parse(&["kache", "init", "--no-service"]);
        assert!(matches!(
            invocation.command,
            Some(Commands::Init { check: false, .. })
        ));
        for spelling in ["--dry-run", "-n"] {
            let invocation = parse(&["kache", "clean", spelling]);
            assert!(
                matches!(
                    invocation.command,
                    Some(Commands::Clean { dry_run: true, .. })
                ),
                "{spelling}"
            );
        }
        assert!(parsed(&["kache", "init", "--shims", "-n"]).is_err());
    }

    #[test]
    fn doctor_fix_help_names_only_what_fix_does() {
        let help = help_text(&["kache", "doctor", "--help"]);
        let fix = help
            .lines()
            .find(|line| line.contains("--fix"))
            .unwrap()
            .to_string();
        assert!(fix.contains("sccache") && fix.contains("daemon"), "{fix}");
        assert!(!fix.contains("repair config"), "{fix}");
    }

    #[test]
    fn help_lists_commands_by_task_and_leaves_out_plumbing() {
        let help = cli_args::top_help_text();
        let listed: Vec<&str> = help
            .lines()
            .skip_while(|line| *line != "Commands:")
            .skip(1)
            .take_while(|line| !line.is_empty())
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        assert_eq!(
            listed,
            [
                "init",
                "doctor",
                "config",
                "stats",
                "clean",
                "explain",
                "list",
                "daemon",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn init_shims_creates_the_links_in_the_named_directory() {
        let dir = tempfile::tempdir().unwrap();
        let farm = dir.path().join("farm");
        install_shims(Some(farm.clone()), false, false, true).unwrap();
        for name in ["cc", "c++", "cargo"] {
            assert!(
                farm.join(name)
                    .symlink_metadata()
                    .is_ok_and(|meta| meta.file_type().is_symlink()),
                "{name}"
            );
        }
    }

    #[test]
    fn json_support_is_limited_to_machine_readable_commands() {
        assert!(command_supports_json(&Some(Commands::Stats {
            since: "24h".to_string(),
            root: None,
            full: false,
            format: None,
            output: None,
            top: None,
            record: false,
            redact: false,
        })));
        assert!(command_supports_json(&Some(Commands::Daemon {
            command: Some(DaemonCommands::Status),
        })));
        assert!(command_supports_json(&Some(Commands::Config {
            command: None
        })));
        assert!(!command_supports_json(&Some(Commands::Telemetry {
            command: TelemetryCommands::Write {
                dir: PathBuf::from("/tmp"),
                scenario: None,
                phase: None,
            },
        })));
        assert!(!command_supports_json(&None));

        assert!(doctor_json_is_read_only(false, false, false, false, false));
        for flags in [
            (true, false, false, false, false),
            (false, true, false, false, false),
            (false, false, true, false, false),
            (false, false, false, true, false),
            (false, false, false, false, true),
        ] {
            assert!(!doctor_json_is_read_only(
                flags.0, flags.1, flags.2, flags.3, flags.4
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_compiler_directly_propagates_exit_code() {
        // `args[0]` is the program, `args[1..]` its args. The unix
        // `true` / `false` utilities are the simplest stand-ins for a
        // compiler: they exit 0 / 1 and ignore their arguments.
        assert_eq!(
            run_compiler_process_directly(&["true".to_string()], false).unwrap(),
            0
        );
        assert_eq!(
            run_compiler_process_directly(&["false".to_string()], false).unwrap(),
            1
        );
    }

    #[test]
    fn rustc_args_for_direct_preclean_truth_table() {
        let rustc = vec!["rustc".to_string()];
        assert_eq!(
            rustc_args_for_direct_preclean(&rustc),
            Some(rustc.as_slice())
        );

        for program in ["cc", "cl.exe", "CL.EXE", "unrecognized-driver"] {
            let cc = vec![program.to_string()];
            assert_eq!(rustc_args_for_direct_preclean(&cc), None, "{program}");
        }

        let chained = vec![
            "/tmp/outer-wrapper".to_string(),
            "rustc".to_string(),
            "--crate-name".to_string(),
        ];
        assert_eq!(
            rustc_args_for_direct_preclean(&chained),
            Some(&chained[1..])
        );

        let unknown = vec![
            "/tmp/outer-wrapper".to_string(),
            "other-tool".to_string(),
            "--crate-name".to_string(),
        ];
        assert_eq!(rustc_args_for_direct_preclean(&unknown), None);
    }

    #[test]
    fn direct_cc_safety_check_applies_only_to_cc_invocations() {
        assert!(is_cc_compiler_invocation(&["cc".to_string()]));
        assert!(!is_cc_compiler_invocation(&["rustc".to_string()]));
        assert!(!is_cc_compiler_invocation(&["other-tool".to_string()]));
        assert!(!is_cc_compiler_invocation(&["nvcc".to_string()]));
    }

    #[test]
    fn wrapper_target_routes_every_adapter() {
        let adapter_for = |argv: &[&str]| {
            compiler::detect_compiler(
                &argv
                    .iter()
                    .map(|arg| (*arg).to_string())
                    .collect::<Vec<_>>(),
            )
            .expect("adapter must match")
        };
        assert_eq!(
            wrapper_target(adapter_for(&["rustc", "--crate-name", "foo"])),
            Some(WrapperTarget::Rustc)
        );
        assert_eq!(
            wrapper_target(adapter_for(&["cc", "-c", "foo.c"])),
            Some(WrapperTarget::Cc)
        );
        let unknown = compiler::CompilerAdapter::new(
            compiler::CompilerId::new("unwired"),
            "unwired",
            |_, _| false,
        );
        assert_eq!(wrapper_target(&unknown), None);
    }

    #[cfg(unix)]
    #[test]
    fn run_compiler_directly_pre_cleans_readonly_restores() {
        // Regression for #238: a direct-exec passthrough (KACHE_DISABLED
        // / nested re-entrancy) over a target dir that still holds
        // read-only hardlinked restores must unlink them first, or the
        // real compiler hits EACCES overwriting them in place.
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_rustc = dir.path().join("rustc");
        write_success_program(&fake_rustc);
        let restored = dir.path().join("libfoo.rlib");
        std::fs::write(&restored, b"cached").unwrap();
        std::fs::set_permissions(&restored, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(
            std::fs::metadata(&restored)
                .unwrap()
                .permissions()
                .readonly()
        );

        // A `rustc`-named shell shim stands in for the compiler; the
        // rustc-shaped flags drive the pre-clean's out-dir branch.
        let code = run_generated_compiler_process_directly(
            &[
                fake_rustc.to_string_lossy().into_owned(),
                "--crate-name".to_string(),
                "foo".to_string(),
                "--out-dir".to_string(),
                dir.path().to_string_lossy().into_owned(),
            ],
            false,
        )
        .unwrap();

        assert_eq!(code, 0);
        assert!(
            !restored.exists(),
            "read-only restore should have been unlinked before exec"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_compiler_directly_pre_cleans_workspace_wrapper_chain_restores() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let outer_wrapper = dir.path().join("outer-wrapper");
        write_success_program(&outer_wrapper);
        let fake_rustc = dir.path().join("rustc");
        write_success_program(&fake_rustc);
        let restored = dir.path().join("libfoo.rlib");
        std::fs::write(&restored, b"cached").unwrap();
        std::fs::set_permissions(&restored, std::fs::Permissions::from_mode(0o444)).unwrap();

        let code = run_generated_compiler_process_directly(
            &[
                outer_wrapper.to_string_lossy().into_owned(),
                fake_rustc.to_string_lossy().into_owned(),
                "--crate-name".to_string(),
                "foo".to_string(),
                "--out-dir".to_string(),
                dir.path().to_string_lossy().into_owned(),
            ],
            false,
        )
        .unwrap();

        assert_eq!(code, 0);
        assert!(
            !restored.exists(),
            "workspace-wrapper chains must pre-clean the inner rustc outputs"
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_compiler_directly_keeps_non_rustc_response_argv_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let compiler = dir.path().join("fake-cc");
        let dump = dir.path().join("argv.txt");
        let response = dir.path().join("cc.args");
        kache_fs::testutil::write_executable(
            &compiler,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\n",
                dump.display()
            ),
        );
        std::fs::write(&response, "-DNAME='two words'\n").unwrap();

        let direct = "direct argument with spaces".to_string();
        let response_arg = format!("@{}", response.display());
        let code = run_generated_compiler_process_directly(
            &[
                compiler.to_string_lossy().into_owned(),
                direct.clone(),
                response_arg.clone(),
            ],
            false,
        )
        .unwrap();

        assert_eq!(code, 0);
        assert_eq!(
            std::fs::read_to_string(dump).unwrap(),
            format!("{direct}\n{response_arg}\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_compiler_directly_preserves_cc_readonly_output() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = tempfile::tempdir().unwrap();
        let fake_cc = dir.path().join("cc");
        write_success_program(&fake_cc);
        let output = dir.path().join("user-owned.o");
        std::fs::write(&output, b"original").unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o444)).unwrap();
        let before = std::fs::metadata(&output).unwrap();

        let code = run_generated_compiler_process_directly(
            &[
                fake_cc.to_string_lossy().into_owned(),
                "-c".to_string(),
                "foo.c".to_string(),
                "-o".to_string(),
                output.to_string_lossy().into_owned(),
            ],
            false,
        )
        .unwrap();

        assert_eq!(code, 0);
        let after = std::fs::metadata(&output).unwrap();
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        assert_eq!(std::fs::read(&output).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn run_compiler_directly_preserves_double_wrapper_rustc_response_file() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("workspace-wrapper");
        let inner = dir.path().join("rustc");
        let dump = dir.path().join("argv.txt");
        let response = dir.path().join("rustc.args");
        let incremental = dir.path().join("target/debug/incremental/unit");
        kache_fs::testutil::write_executable(
            &outer,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" > \"{}\"\nshift\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    @*) cat \"${{arg#@}}\" >> \"{}\";;\n    *) printf '%s\\n' \"$arg\" >> \"{}\";;\n  esac\ndone\n",
                dump.display(),
                dump.display(),
                dump.display()
            ),
        );
        std::fs::write(
            &response,
            format!(
                "--crate-name\nfixture\n-Cincremental={}\n",
                incremental.display()
            ),
        )
        .unwrap();

        let code = run_generated_compiler_process_directly(
            &[
                outer.to_string_lossy().into_owned(),
                inner.to_string_lossy().into_owned(),
                format!("@{}", response.display()),
            ],
            true,
        )
        .unwrap();

        assert_eq!(code, 0);
        let argv = std::fs::read_to_string(dump).unwrap();
        assert!(
            argv.starts_with(&format!("{}\n", inner.display())),
            "{argv}"
        );
        assert!(argv.contains(&format!("-Cincremental={}\n", incremental.display())));

    }

    #[test]
    fn test_runner_args_take_everything_after_the_subcommand() {
        let os = |args: &[&str]| -> Vec<std::ffi::OsString> {
            args.iter().map(std::ffi::OsString::from).collect()
        };
        let argv = os(&[
            "kache",
            "test-runner",
            "/t/probe-0123456789abcdef",
            "--exact",
        ]);
        assert_eq!(
            test_runner_args(&argv),
            Some(&argv[2..]),
            "the binary and its arguments"
        );
        let bare = os(&["kache", "test-runner"]);
        assert_eq!(test_runner_args(&bare), Some(&bare[2..]));
        assert_eq!(
            test_runner_args(&os(&["kache", "rustc", "test-runner"])),
            None
        );
        assert_eq!(test_runner_args(&os(&["kache"])), None);
        // A compile through a shim whose first input is named `test-runner`.
        assert_eq!(
            test_runner_args(&os(&["/x/shims/cc", "test-runner", "-o", "app"])),
            None
        );
    }

    #[test]
    fn test_detect_log_mode() {
        assert_eq!(detect_log_mode(&["kache".into()]), LogMode::Cli);
        assert_eq!(
            detect_log_mode(&["kache".into(), "config".into()]),
            LogMode::Cli
        );
        assert_eq!(
            detect_log_mode(&["kache".into(), "stats".into()]),
            LogMode::Cli
        );
        assert_eq!(
            detect_log_mode(&["kache".into(), "rustc".into(), "--crate-name".into()]),
            LogMode::Wrapper
        );
        assert_eq!(
            detect_log_mode(&[
                "kache".into(),
                "C:/Users/dev/.mozbuild/clang/bin/clang.exe".into(),
                "-E".into(),
                "conftest.c".into()
            ]),
            LogMode::Wrapper
        );
        // Issue #514: a cross-compiler basename is a wrapper invocation, not a
        // clap subcommand. Both bare and path-qualified forms take this route.
        assert_eq!(
            detect_log_mode(&[
                "kache".into(),
                "arm-linux-gnueabihf-gcc".into(),
                "--version".into(),
            ]),
            LogMode::Wrapper
        );
        assert_eq!(
            detect_log_mode(&[
                "kache".into(),
                "/opt/cross/bin/aarch64-linux-gnu-g++-13".into(),
                "-c".into(),
                "foo.cc".into(),
            ]),
            LogMode::Wrapper
        );
        // Regression for issue #287: `cargo clippy` on Windows invokes the
        // wrapper as `kache <…>\clippy-driver.exe rustc -vV`. The whole
        // dispatch (not just recognizes()) must select Wrapper mode — before
        // the fix this fell through to Cli and clap-errored with
        // "unrecognized subcommand". The backslash basename + `.exe` suffix
        // resolve identically on every host OS.
        assert_eq!(
            detect_log_mode(&[
                "kache".into(),
                r"G:\.rustup\toolchains\nightly-x86_64-pc-windows-msvc\bin\clippy-driver.exe"
                    .into(),
                "rustc".into(),
                "-vV".into(),
            ]),
            LogMode::Wrapper
        );
        assert_eq!(
            detect_log_mode(&[
                "kache".into(),
                r"C:\Program Files\Rust\bin\rustc.exe".into(),
                "--crate-name".into(),
            ]),
            LogMode::Wrapper
        );
        // Issue #656: Kani replaces Cargo's rustc with `kani-compiler` while
        // preserving RUSTC_WRAPPER, so the compiler path must enter wrapper
        // mode and be passed through instead of being parsed as a CLI command.
        assert_eq!(
            detect_log_mode_with_rustc(
                &[
                    "kache".into(),
                    "/home/user/.kani/kani-0.67.0/bin/kani-compiler".into(),
                    "-vV".into(),
                ],
                Some(std::ffi::OsStr::new(
                    "/home/user/.kani/kani-0.67.0/bin/kani-compiler",
                )),
            ),
            LogMode::Wrapper
        );
        // Issue #505: unrecognized RUSTC_WORKSPACE_WRAPPER tools like
        // dylint-driver. Cargo passes `kache <wrapper-path> rustc <args>`.
        // Detection keys off the inner rustc (position 2), not the
        // wrapper's name — so any workspace-wrapper tool works.
        assert_eq!(
            detect_log_mode(&[
                "kache".into(),
                "/Users/dev/.dylint_drivers/nightly/dylint-driver".into(),
                "rustc".into(),
                "--crate-name".into(),
            ]),
            LogMode::Wrapper
        );
        // Negative: no path separator in arg[1] → CLI subcommand, not a wrapper.
        assert_eq!(
            detect_log_mode(&["kache".into(), "init".into(), "rustc-project".into(),]),
            LogMode::Cli
        );
    }
}
