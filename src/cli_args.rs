//! Command-line parsing: the `kache` argument grammar without proc macros.
//!
//! One function owns the surface: [`parse_cli`]. It returns the parsed
//! invocation, or [`ParseOutcome::Help`]/[`ParseOutcome::Version`] for the
//! flags that print and exit 0. Every other failure is
//! [`machine::InvalidArguments`], which the caller reports with exit 2.

use crate::config_cli;
use crate::machine;
use crate::{BenchPhase, Commands, DaemonCommands, ListSort, TelemetryCommands};
use std::ffi::OsString;
use std::path::PathBuf;

/// A parsed invocation: the global `--json` flag and the subcommand.
pub(crate) struct Cli {
    pub json: bool,
    pub command: Option<Commands>,
}

impl Cli {
    /// The dotted command path `machine::configure` files JSON documents
    /// under: `daemon-status`, `config-list`, or `cli` with no subcommand.
    pub(crate) fn command_path(&self) -> String {
        let mut names = Vec::new();
        match &self.command {
            None => {}
            Some(Commands::Cargo { .. }) => names.push("cargo"),
            Some(Commands::List { .. }) => names.push("list"),
            Some(Commands::Gc { .. }) => names.push("gc"),
            Some(Commands::Purge { .. }) => names.push("purge"),
            Some(Commands::Clean { .. }) => names.push("clean"),
            Some(Commands::Init { .. }) => names.push("init"),
            Some(Commands::Doctor { .. }) => names.push("doctor"),
            Some(Commands::Daemon { command }) => {
                names.push("daemon");
                match command {
                    None => {}
                    Some(DaemonCommands::Status) => names.push("status"),
                    Some(DaemonCommands::Run) => names.push("run"),
                    Some(DaemonCommands::Start) => names.push("start"),
                    Some(DaemonCommands::Stop) => names.push("stop"),
                    Some(DaemonCommands::Restart) => names.push("restart"),
                    Some(DaemonCommands::Install) => names.push("install"),
                    Some(DaemonCommands::Uninstall) => names.push("uninstall"),
                    Some(DaemonCommands::Log) => names.push("log"),
                }
            }
            Some(Commands::Stats { .. }) => names.push("stats"),
            Some(Commands::Telemetry { command }) => {
                names.push("telemetry");
                if matches!(command, TelemetryCommands::Write { .. }) {
                    names.push("write");
                }
            }
            Some(Commands::Explain { .. }) => names.push("explain"),
            Some(Commands::Diff { .. }) => names.push("diff"),
            Some(Commands::Report { .. }) => names.push("report"),
            Some(Commands::Config { command }) => {
                names.push("config");
                match command {
                    None => {}
                    Some(config_cli::Command::Get { .. }) => names.push("get"),
                    Some(config_cli::Command::List { .. }) => names.push("list"),
                    Some(config_cli::Command::Set { .. }) => names.push("set"),
                    Some(config_cli::Command::Unset { .. }) => names.push("unset"),
                    Some(config_cli::Command::Path { .. }) => names.push("path"),
                }
            }
            Some(Commands::InstallShims { .. }) => names.push("install-shims"),
        }
        if names.is_empty() {
            "cli".to_owned()
        } else {
            names.join("-")
        }
    }
}

/// What argument parsing decided: run, print help, or print the version.
/// Help and version carry whether `--json` was seen: with it they print
/// machine-readable documents instead of text.
pub(crate) enum ParseOutcome {
    Run(Cli),
    Help { text: String, json: bool },
    Version { json: bool },
}

type ParseResult = Result<ParseOutcome, ParseFail>;

fn dispatch(name: &str, parser: &mut lexopt::Parser, json: bool) -> ParseResult {
    match name {
        "cargo" => parse_cargo(parser, json),
        "help" => parse_help(parser, json),
        "status" => parse_stats(parser, json),
        "stats" => parse_stats(parser, json),
        "list" => parse_list(parser, json),
        "gc" => parse_gc(parser, json),
        "purge" => parse_purge(parser, json),
        "clean" => parse_clean(parser, json),
        "init" => parse_init(parser, json),
        "doctor" => parse_doctor(parser, json),
        "daemon" => parse_daemon(parser, json),
        "telemetry" => parse_telemetry(parser, json),
        "why-miss" => parse_explain(parser, json),
        "explain" => parse_explain(parser, json),
        "diff" => parse_diff(parser, json),
        "report" => parse_report(parser, json),
        "config" => parse_config(parser, json),
        "install-shims" => parse_install_shims(parser, json),
        _ => Err(fail(
            json,
            format!("unrecognized subcommand '{name}'\n\nFor more information, try '--help'."),
        )),
    }
}

fn run(json: bool, command: Commands) -> ParseResult {
    Ok(ParseOutcome::Run(Cli {
        json,
        command: Some(command),
    }))
}

/// Everything after `cargo`, verbatim: Cargo owns its own flag grammar.
/// A `--` right after the subcommand is the escape, not an argument.
fn parse_cargo(parser: &mut lexopt::Parser, json: bool) -> ParseResult {
    let mut rest = parser
        .raw_args()
        .map_err(|error| lexopt_error(error, json))?;
    if rest.peek().is_some_and(|arg| arg.to_string_lossy() == "--") {
        rest.next();
    }
    run(
        json,
        Commands::Cargo {
            args: rest.collect(),
        },
    )
}

fn parse_help(parser: &mut lexopt::Parser, json: bool) -> ParseResult {
    let mut path = Vec::new();
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Value(value) => path.push(utf8(value, json)?),
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    let text = match path.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => top_help(),
        [command] => command_help(command, json)?,
        [command, subcommand] => subcommand_help(command, subcommand, json)?,
        _ => return Err(fail(json, "help takes at most a command and a subcommand")),
    };
    Ok(ParseOutcome::Help { text, json })
}

fn parse_list(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut crate_name = None;
    let mut sort = ListSort::Name;
    let mut no_pager = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: list_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("sort") => sort = parse_sort(&next_string(&mut *parser, "sort", json)?, json)?,
            lexopt::Arg::Long("no-pager") => {
                no_value(&mut *parser, "no-pager", json)?;
                no_pager = true;
            }
            lexopt::Arg::Value(value) => {
                if crate_name.is_some() {
                    return Err(fail(json, "unexpected argument: list takes at most one crate name"));
                }
                crate_name = Some(utf8(value, json)?);
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    run(
        json,
        Commands::List {
            crate_name,
            sort,
            no_pager,
        },
    )
}

fn parse_gc(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut max_age = None;
    let mut stale_schema = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: gc_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("max-age") => {
                max_age = Some(next_string(&mut *parser, "max-age", json)?);
            }
            lexopt::Arg::Long("stale-schema") => {
                no_value(&mut *parser, "stale-schema", json)?;
                stale_schema = true;
            }
            lexopt::Arg::Value(value) => {
                return Err(fail(
                    json,
                    format!("unexpected argument '{}'", value.to_string_lossy()),
                ));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if max_age.is_some() && stale_schema {
        return Err(conflict(json, "'--max-age'", "'--stale-schema'"));
    }
    run(json, Commands::Gc { max_age, stale_schema })
}

fn parse_purge(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut crate_name = None;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: purge_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("crate-name") => {
                crate_name = Some(next_string(&mut *parser, "crate-name", json)?);
            }
            lexopt::Arg::Value(value) => {
                return Err(fail(
                    json,
                    format!("unexpected argument '{}'", value.to_string_lossy()),
                ));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    run(json, Commands::Purge { crate_name })
}

fn parse_clean(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut dry_run = false;
    let mut yes = false;
    let mut stale = None;
    let mut cache = false;
    let mut all = false;
    let mut crate_name = None;
    let mut stale_schema = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: clean_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Short('n') | lexopt::Arg::Long("dry-run") => {
                no_value(&mut *parser, "dry-run", json)?;
                dry_run = true;
            }
            lexopt::Arg::Short('y') | lexopt::Arg::Long("yes") => {
                no_value(&mut *parser, "yes", json)?;
                yes = true;
            }
            lexopt::Arg::Long("stale") => stale = Some(next_string(&mut *parser, "stale", json)?),
            lexopt::Arg::Long("cache") => {
                no_value(&mut *parser, "cache", json)?;
                cache = true;
            }
            lexopt::Arg::Long("all") => {
                no_value(&mut *parser, "all", json)?;
                all = true;
            }
            lexopt::Arg::Long("crate") => {
                crate_name = Some(next_string(&mut *parser, "crate", json)?);
            }
            lexopt::Arg::Long("stale-schema") => {
                no_value(&mut *parser, "stale-schema", json)?;
                stale_schema = true;
            }
            lexopt::Arg::Value(value) => {
                return Err(fail(json, format!("unexpected argument '{}'", value.to_string_lossy())));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if all && !cache {
        return Err(requires(json, "'--all'", "'--cache'"));
    }
    if all && stale_schema {
        return Err(conflict(json, "'--all'", "'--stale-schema'"));
    }
    if stale.is_some() && !cache {
        return Err(requires(json, "'--stale'", "'--cache'"));
    }
    if stale.is_some() && all {
        return Err(conflict(json, "'--stale'", "'--all'"));
    }
    if stale.is_some() && stale_schema {
        return Err(conflict(json, "'--stale'", "'--stale-schema'"));
    }
    if crate_name.is_some() && cache {
        return Err(conflict(json, "'--crate'", "'--cache'"));
    }
    if stale_schema && !cache {
        return Err(requires(json, "'--stale-schema'", "'--cache'"));
    }
    run(
        json,
        Commands::Clean {
            dry_run,
            yes,
            stale,
            cache,
            all,
            crate_name,
            stale_schema,
        },
    )
}

fn parse_init(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut yes = false;
    let mut no_service = false;
    let mut no_shell = false;
    let mut check = false;
    let mut shims: Option<Option<PathBuf>> = None;
    let mut from_path = false;
    let mut canonical_only = false;
    let mut force = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: init_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Short('y') | lexopt::Arg::Long("yes") => {
                no_value(&mut *parser, "yes", json)?;
                yes = true;
            }
            lexopt::Arg::Long("no-service") => {
                no_value(&mut *parser, "no-service", json)?;
                no_service = true;
            }
            lexopt::Arg::Long("no-shell") => {
                no_value(&mut *parser, "no-shell", json)?;
                no_shell = true;
            }
            lexopt::Arg::Short('n') | lexopt::Arg::Long("check") | lexopt::Arg::Long("dry-run") => {
                no_value(&mut *parser, "check", json)?;
                check = true;
            }
            lexopt::Arg::Long("shims") => {
                shims = Some(parse_optional_dir(&mut *parser, json)?);
            }
            lexopt::Arg::Long("from-path") => {
                no_value(&mut *parser, "from-path", json)?;
                from_path = true;
            }
            lexopt::Arg::Long("canonical-only") => {
                no_value(&mut *parser, "canonical-only", json)?;
                canonical_only = true;
            }
            lexopt::Arg::Long("force") => {
                no_value(&mut *parser, "force", json)?;
                force = true;
            }
            lexopt::Arg::Value(value) => {
                return Err(fail(
                    json,
                    format!("unexpected argument '{}'", value.to_string_lossy()),
                ));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if shims.is_some() && check {
        return Err(conflict(json, "'--shims'", "'--check'"));
    }
    if shims.is_some() && no_service {
        return Err(conflict(json, "'--shims'", "'--no-service'"));
    }
    if shims.is_some() && no_shell {
        return Err(conflict(json, "'--shims'", "'--no-shell'"));
    }
    if from_path && shims.is_none() {
        return Err(requires(json, "'--from-path'", "'--shims'"));
    }
    if canonical_only && shims.is_none() {
        return Err(requires(json, "'--canonical-only'", "'--shims'"));
    }
    if canonical_only && from_path {
        return Err(conflict(json, "'--canonical-only'", "'--from-path'"));
    }
    if force && shims.is_none() {
        return Err(requires(json, "'--force'", "'--shims'"));
    }
    run(
        json,
        Commands::Init {
            yes,
            no_service,
            no_shell,
            check,
            shims,
            from_path,
            canonical_only,
            force,
        },
    )
}

/// The value of `--shims`: attached (`--shims=DIR`), separate
/// (`--shims DIR`), or absent (the default directory). A following flag is
/// never consumed as the directory.
fn parse_optional_dir(
    parser: &mut lexopt::Parser,
    json: bool,
) -> Result<Option<PathBuf>, ParseFail> {
    if let Some(value) = parser.optional_value() {
        return Ok(Some(PathBuf::from(value)));
    }
    let mut raw = parser.try_raw_args().ok_or_else(|| missing(json, "'--shims <DIR>'"))?;
    match raw.peek() {
        Some(next) if !looks_like_flag(next) => Ok(Some(PathBuf::from(raw.next().unwrap()))),
        _ => Ok(None),
    }
}

fn looks_like_flag(arg: &std::ffi::OsStr) -> bool {
    arg.len() > 1 && arg.to_string_lossy().starts_with('-')
}

fn parse_daemon(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut subcommand: Option<String> = None;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: daemon_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Value(value) => {
                if subcommand.is_some() {
                    return Err(fail(
                        json,
                        format!("unexpected argument '{}'", value.to_string_lossy()),
                    ));
                }
                subcommand = Some(utf8(value, json)?);
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    let command = match subcommand.as_deref() {
        None => None,
        Some("status") => Some(DaemonCommands::Status),
        Some("run") => Some(DaemonCommands::Run),
        Some("start") => Some(DaemonCommands::Start),
        Some("stop") => Some(DaemonCommands::Stop),
        Some("restart") => Some(DaemonCommands::Restart),
        Some("install") => Some(DaemonCommands::Install),
        Some("uninstall") => Some(DaemonCommands::Uninstall),
        Some("log") => Some(DaemonCommands::Log),
        Some(other) => {
            return Err(fail(json, format!("unrecognized daemon subcommand '{other}'")));
        }
    };
    run(json, Commands::Daemon { command })
}

fn parse_telemetry(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut subcommand: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    let mut scenario = None;
    let mut phase = None;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: telemetry_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Value(value) if subcommand.is_none() => {
                subcommand = Some(utf8(value, json)?);
            }
            lexopt::Arg::Value(value) => {
                if dir.is_some() {
                    return Err(fail(
                        json,
                        format!("unexpected argument '{}'", value.to_string_lossy()),
                    ));
                }
                dir = Some(PathBuf::from(value));
            }
            lexopt::Arg::Long("scenario") => {
                scenario = Some(next_string(&mut *parser, "scenario", json)?);
            }
            lexopt::Arg::Long("phase") => {
                phase = Some(parse_phase(&next_string(&mut *parser, "phase", json)?, json)?);
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    match subcommand.as_deref() {
        None => Err(missing(json, "telemetry subcommand (write)")),
        Some("write") => {
            let Some(dir) = dir else {
                return Err(missing(json, "<DIR>"));
            };
            run(
                json,
                Commands::Telemetry {
                    command: TelemetryCommands::Write { dir, scenario, phase },
                },
            )
        }
        Some(other) => Err(fail(json, format!("unrecognized telemetry subcommand '{other}'"))),
    }
}

fn parse_stats(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut since = "24h".to_owned();
    let mut since_set = false;
    let mut last_build = false;
    let mut root = None;
    let mut full = false;
    let mut format = None;
    let mut output = None;
    let mut top = None;
    let mut record = false;
    let mut redact = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: stats_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("since") => {
                since = next_string(&mut *parser, "since", json)?;
                since_set = true;
            },
            lexopt::Arg::Long("last-build") => {
                no_value(&mut *parser, "last-build", json)?;
                last_build = true;
            }
            lexopt::Arg::Long("root") => root = Some(next_path(&mut *parser, "root", json)?),
            lexopt::Arg::Long("full") => {
                no_value(&mut *parser, "full", json)?;
                full = true;
            }
            lexopt::Arg::Long("format") => {
                format = Some(parse_format(&next_string(&mut *parser, "format", json)?, json)?);
            }
            lexopt::Arg::Short('o') | lexopt::Arg::Long("output") => {
                output = Some(next_path(&mut *parser, "output", json)?);
            }
            lexopt::Arg::Long("top") => top = Some(next_usize(&mut *parser, "top", json)?),
            lexopt::Arg::Long("record") => {
                no_value(&mut *parser, "record", json)?;
                record = true;
            }
            lexopt::Arg::Long("redact") => {
                no_value(&mut *parser, "redact", json)?;
                redact = true;
            }
            lexopt::Arg::Value(value) => {
                return Err(fail(
                    json,
                    format!("unexpected argument '{}'", value.to_string_lossy()),
                ));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if last_build && since_set {
        return Err(conflict(json, "'--last-build'", "'--since'"));
    }
    run(
        json,
        Commands::Stats {
            since,
            last_build,
            root,
            full,
            format,
            output,
            top,
            record,
            redact,
        },
    )
}

fn parse_explain(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut crate_name = None;
    let mut all = false;
    let mut root = None;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: explain_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("all") => {
                no_value(&mut *parser, "all", json)?;
                all = true;
            }
            lexopt::Arg::Long("root") => root = Some(next_path(&mut *parser, "root", json)?),
            lexopt::Arg::Value(value) => {
                if crate_name.is_some() {
                    return Err(fail(
                        json,
                        "unexpected argument: explain takes at most one crate name",
                    ));
                }
                crate_name = Some(utf8(value, json)?);
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if all && crate_name.is_some() {
        return Err(conflict(json, "'--all'", "'<CRATE>'"));
    }
    if root.is_some() && crate_name.is_some() {
        return Err(conflict(json, "'--root'", "'<CRATE>'"));
    }
    run(json, Commands::Explain { crate_name, all, root })
}

fn parse_diff(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut root = None;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: diff_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("root") => root = Some(next_path(&mut *parser, "root", json)?),
            lexopt::Arg::Value(value) => {
                return Err(fail(
                    json,
                    format!("unexpected argument '{}'", value.to_string_lossy()),
                ));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    run(json, Commands::Diff { root })
}

fn parse_report(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut format = crate::cli::ReportFormat::Text;
    let mut since = "24h".to_owned();
    let mut since_set = false;
    let mut last_build = false;
    let mut root = None;
    let mut output = None;
    let mut top = 10;
    let mut record = false;
    let mut redact = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: report_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("format") => {
                format = parse_format(&next_string(&mut *parser, "format", json)?, json)?;
            }
            lexopt::Arg::Long("since") => {
                since = next_string(&mut *parser, "since", json)?;
                since_set = true;
            },
            lexopt::Arg::Long("last-build") => {
                no_value(&mut *parser, "last-build", json)?;
                last_build = true;
            }
            lexopt::Arg::Long("root") => root = Some(next_path(&mut *parser, "root", json)?),
            lexopt::Arg::Short('o') | lexopt::Arg::Long("output") => {
                output = Some(next_path(&mut *parser, "output", json)?);
            }
            lexopt::Arg::Long("top") => top = next_usize(&mut *parser, "top", json)?,
            lexopt::Arg::Long("record") => {
                no_value(&mut *parser, "record", json)?;
                record = true;
            }
            lexopt::Arg::Long("redact") => {
                no_value(&mut *parser, "redact", json)?;
                redact = true;
            }
            lexopt::Arg::Value(value) => {
                return Err(fail(
                    json,
                    format!("unexpected argument '{}'", value.to_string_lossy()),
                ));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if last_build && since_set {
        return Err(conflict(json, "'--last-build'", "'--since'"));
    }
    run(
        json,
        Commands::Report {
            format,
            since,
            last_build,
            root,
            output,
            top,
            record,
            redact,
        },
    )
}

fn parse_config(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut subcommand: Option<String> = None;
    let mut project = false;
    let mut global = false;
    let mut file = None;
    let mut positionals = Vec::new();
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: config_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("project") => {
                no_value(&mut *parser, "project", json)?;
                project = true;
            }
            lexopt::Arg::Long("global") => {
                no_value(&mut *parser, "global", json)?;
                global = true;
            }
            lexopt::Arg::Long("file") => file = Some(next_path(&mut *parser, "file", json)?),
            lexopt::Arg::Value(value) if subcommand.is_none() => {
                subcommand = Some(utf8(value, json)?);
            }
            lexopt::Arg::Value(value) => positionals.push(utf8(value, json)?),
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if project && global {
        return Err(conflict(json, "'--project'", "'--global'"));
    }
    if project && file.is_some() {
        return Err(conflict(json, "'--project'", "'--file'"));
    }
    if global && file.is_some() {
        return Err(conflict(json, "'--global'", "'--file'"));
    }
    let scope = config_cli::Scope {
        project,
        global,
        file,
    };
    let command = match (subcommand.as_deref(), positionals.as_slice()) {
        (None, []) => None,
        (None, [other, ..]) => {
            return Err(fail(
                json,
                format!("unexpected argument '{other}': expected a config subcommand"),
            ));
        }
        (Some("get"), [key]) => Some(config_cli::Command::Get {
            scope,
            key: key.clone(),
        }),
        (Some("get"), _) => return Err(missing(json, "<KEY>")),
        (Some("set"), [key, value]) => Some(config_cli::Command::Set {
            scope,
            key: key.clone(),
            value: value.clone(),
        }),
        (Some("set"), _) => return Err(missing(json, "<KEY> <VALUE>")),
        (Some("unset"), [key]) => Some(config_cli::Command::Unset {
            scope,
            key: key.clone(),
        }),
        (Some("unset"), _) => return Err(missing(json, "<KEY>")),
        (Some("list"), []) => Some(config_cli::Command::List { scope }),
        (Some("list"), [other, ..]) => {
            return Err(fail(json, format!("unexpected argument '{other}'")));
        }
        (Some("path"), []) => Some(config_cli::Command::Path { scope }),
        (Some("path"), [other, ..]) => {
            return Err(fail(json, format!("unexpected argument '{other}'")));
        }
        (Some(other), _) => {
            return Err(fail(json, format!("unrecognized config subcommand '{other}'")));
        }
    };
    run(json, Commands::Config { command })
}

fn parse_install_shims(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut dir = None;
    let mut force = false;
    let mut from_path = false;
    let mut canonical_only = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: install_shims_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("force") => {
                no_value(&mut *parser, "force", json)?;
                force = true;
            }
            lexopt::Arg::Long("from-path") => {
                no_value(&mut *parser, "from-path", json)?;
                from_path = true;
            }
            lexopt::Arg::Long("canonical-only") => {
                no_value(&mut *parser, "canonical-only", json)?;
                canonical_only = true;
            }
            lexopt::Arg::Value(value) => {
                if dir.is_some() {
                    return Err(fail(
                        json,
                        "unexpected argument: install-shims takes at most one directory",
                    ));
                }
                dir = Some(PathBuf::from(value));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if canonical_only && from_path {
        return Err(conflict(json, "'--canonical-only'", "'--from-path'"));
    }
    run(
        json,
        Commands::InstallShims {
            dir,
            force,
            from_path,
            canonical_only,
        },
    )
}

/// The top-level help text, for the no-subcommand path.
pub(crate) fn top_help_text() -> String {
    top_help()
}

fn top_help() -> String {
    "\
Compiler cache for Rust and C/C++.

kache runs in front of rustc and the C/C++ compilers, so builds use the cache without changing how you run them.

Usage: kache [OPTIONS] [COMMAND]

Commands:
  init         Set up caching for Cargo and C/C++ builds
  doctor       Diagnose setup issues and verify cache integrity
  config       Inspect or change configuration values
  stats        Show hit rate, time saved and cache size [alias: status]
  clean        Remove cache entries
  explain      Explain why a crate, or the latest build, missed the cache
  list         List cache entries, or one crate's entries in detail
  daemon       Show the background daemon's status, or start, stop or install it

Options:
  --json       Machine-readable JSON on stdout, on commands that support it
  -h, --help   Print help
  -V, --version
               Print version

Start with `kache init`. Keep using cargo as usual.
Use `kache stats` for a summary.
"
    .to_owned()
}

fn cargo_help() -> String {
    "\
Run Cargo with Kache as its Rust compiler wrapper

Usage: kache cargo [ARGS]...

Arguments:
  [ARGS]...  Arguments passed verbatim to Cargo (use `--` first)
"
    .to_owned()
}

fn list_help() -> String {
    "\
List cache entries, or one crate's entries in detail

Usage: kache list [OPTIONS] [CRATE]

Arguments:
  [CRATE]  Crate name to show details for (omit to list all)

Options:
  --sort <name|size|hits|age>  Sort by name, size, hits or age [default: name]
  --no-pager                   Print directly instead of using a pager
  --json                       Machine-readable JSON on stdout
  -h, --help                   Print help
"
    .to_owned()
}

fn gc_help() -> String {
    "\
Evict cache entries by the size and age limits, as the daemon does.
Kept for scripts

Usage: kache gc [OPTIONS]

Options:
  --max-age <DURATION>  Evict entries older than this duration (e.g. 7d, 24h)
  --stale-schema        Remove entries from old or unrecorded cache-key schemas
  --json                Machine-readable JSON on stdout
  -h, --help            Print help
"
    .to_owned()
}

fn purge_help() -> String {
    "\
Remove the whole cache or one crate's entries. Kept for scripts;
`kache clean --cache` empties the cache too

Usage: kache purge [OPTIONS]

Options:
  --crate-name <NAME>  Only purge entries for this crate
  -h, --help           Print help
"
    .to_owned()
}

fn clean_help() -> String {
    "\
Remove cache entries

Usage: kache clean [OPTIONS]

Options:
  -n, --dry-run        Show what would be removed and what kache keeps, and remove nothing
  -y, --yes            Remove without asking. For scripts and cron; preview first with --dry-run
  --cache              Empty the cache instead. Asks first. The daemon already keeps it under its size limit
  --json               Machine-readable JSON on stdout
  -h, --help           Print help
"
    .to_owned()
}

fn init_help() -> String {
    "\
Set up caching for Cargo and C/C++ builds

Usage: kache init [OPTIONS]

Options:
  -y, --yes            Accept all default answers (non-interactive)
  --no-service         Do not install the daemon as a login service
  --no-shell           Leave shell setup alone: no C/C++ shims on PATH or test runner
  -n, --check          Print what would change without modifying anything [alias: --dry-run]
  --shims [<DIR>]      Only create the compiler-name links to kache (Unix), in DIR or ~/.local/lib/kache/shims
  --from-path          With --shims, also link every other compiler name on PATH (target triplets)
  --canonical-only     With --shims, link only the unversioned names, not the versioned compilers on PATH
  --force              With --shims, replace existing entries instead of refusing
  -h, --help           Print help
"
    .to_owned()
}

fn doctor_help() -> String {
    "\
Diagnose setup issues and verify cache integrity

Usage: kache doctor [OPTIONS]

Options:
  --fix              Migrate from sccache (Cargo wrapper, running server) and restart a daemon older than this binary
  --purge-sccache    Also remove sccache cache and binary (requires --fix)
  --verify           Verify cache integrity (entries, blobs, metadata)
  --checksums        Also verify blob checksums (slower, implies --verify)
  --repair           Remove corrupted entries (implies --verify)
  --probe            Compile a temporary Rust library and verify a local cache hit
  --json             Machine-readable JSON on stdout
  -h, --help         Print help
"
    .to_owned()
}

fn daemon_help() -> String {
    "\
Show the background daemon's status, or start, stop or install it

Usage: kache daemon [COMMAND]

Commands:
  status     Show daemon status (also bare `kache daemon`)
  run        Run the daemon server in the foreground
  start      Start the daemon in the background and wait until it is ready
  stop       Stop a running daemon
  restart    Restart the daemon, through its login service when one is installed
  install    Install the daemon as a per-user login service
  uninstall  Remove the daemon service
  log        Stream daemon logs

Options:
  -h, --help  Print help
"
    .to_owned()
}

fn stats_help() -> String {
    "\
Show hit rate, time saved and cache size; --full for the whole report [alias: status]

Usage: kache stats [OPTIONS]

Options:
  --since <SINCE>    Event window (e.g. 15m, 2h, 7d; a bare number is hours) [default: 24h]
  --last-build       Report the latest recorded activity session
  --root <PATH>      Only this build tree/root
  --full             The full build report instead of the summary
  --format <FORMAT>  Write the full report as json, markdown, github, or trace (Perfetto / Chrome trace)
  -o, --output <PATH>
                     Write the full report to a file instead of stdout
  --top <N>          Entries in each top list of the full report
  --record           Also append the full report's summary and timing, with the host's load, to telemetry
  --redact           Replace cache keys, roots, and object paths with `redacted`
  --json             Machine-readable JSON on stdout
  -h, --help         Print help
"
    .to_owned()
}

fn telemetry_help() -> String {
    "\
Write cache counters as OTLP JSON

Usage: kache telemetry <COMMAND>

Commands:
  write  Snapshot live cache counters into `metrics.otlp.json` + `schema_version`

Options:
  -h, --help  Print help
"
    .to_owned()
}

fn telemetry_write_help() -> String {
    "\
Snapshot live cache counters into `metrics.otlp.json` + `schema_version`

Usage: kache telemetry write [OPTIONS] <DIR>

Arguments:
  <DIR>  Directory to write. Created if missing

Options:
  --scenario <NAME>   Bench scenario this dump belongs to
  --phase <PHASE>     Bench phase this dump belongs to (cold, warm-same-tree, warm, pull)
  -h, --help          Print help
"
    .to_owned()
}

fn explain_help() -> String {
    "\
Explain why a crate, or the latest build, missed the cache [alias: why-miss]

Usage: kache explain [OPTIONS] [CRATE]

Arguments:
  [CRATE]  The crate to explain. Without one, compare the latest build's misses with the build before it

Options:
  --all         List every skip and passthrough across the retained build history, with counts by reason
  --root <PATH> Without a crate: only builds recorded for this build tree
  --json        Machine-readable JSON on stdout
  -h, --help    Print help
"
    .to_owned()
}

fn diff_help() -> String {
    "\
Compare misses in the two newest sessions of one build root. Kept
for scripts; `kache explain` with no crate explains the same build

Usage: kache diff [OPTIONS]

Options:
  --root <PATH>  Only sessions recorded for this build tree
  --json         Machine-readable JSON on stdout
  -h, --help     Print help
"
    .to_owned()
}

fn report_help() -> String {
    "\
The full build report. Kept for scripts; `kache stats --full` is the
same report

Usage: kache report [OPTIONS]

Options:
  --format <FORMAT>  Output format: json, trace (also perfetto, chrome-trace), markdown, github, text [default: text]
  --since <SINCE>    Event window (e.g. 15m, 2h, 7d; a bare number is hours) [default: 24h]
  --last-build       Report the latest recorded activity session
  --root <PATH>      Only include compiler events from this build tree/root
  -o, --output <PATH>
                     Write output to a file instead of stdout
  --top <N>          Number of top entries to show [default: 10]
  --record           Also append this report's summary and timing, with the host's load, to telemetry
  --redact           Replace cache keys, roots, and object paths with `redacted`
  --json             Machine-readable JSON on stdout
  -h, --help         Print help
"
    .to_owned()
}

fn config_help() -> String {
    "\
Inspect or change configuration values (lists values without a subcommand)

Usage: kache config [COMMAND]

Commands:
  get    Read a value explicitly set in the selected file
  list   List values explicitly set in the selected file
  set    Set a known configuration key; accepts TOML values or an unquoted string
  unset  Remove a value from the selected file
  path   Print the selected file's path, even if it does not exist

Options:
  --json      Machine-readable JSON on stdout
  -h, --help  Print help
"
    .to_owned()
}

fn config_sub_help(subcommand: &str) -> String {
    let (usage, about) = match subcommand {
        "get" => ("kache config get [OPTIONS] <KEY>", "Read a value explicitly set in the selected file"),
        "set" => ("kache config set [OPTIONS] <KEY> <VALUE>", "Set a known configuration key"),
        "unset" => ("kache config unset [OPTIONS] <KEY>", "Remove a value from the selected file"),
        "list" => ("kache config list [OPTIONS]", "List values explicitly set in the selected file"),
        "path" => ("kache config path [OPTIONS]", "Print the selected file's path"),
        _ => unreachable!("config subcommand checked before this call"),
    };
    format!(
        "\
{about}

Usage: {usage}

Options:
  --project      Use the nearest project config, or create .kache.toml in this directory
  --global       Use the user config, ignoring KACHE_CONFIG and project files
  --file <PATH>  Use this configuration file
  --json         Machine-readable JSON on stdout
  -h, --help     Print help
"
    )
}

fn install_shims_help() -> String {
    "\
Create compiler-name links to kache in a directory you put first on
PATH. Kept for scripts and packages; `kache init --shims` does the
same in the default directory

Usage: kache install-shims [OPTIONS] [DIR]

Arguments:
  [DIR]  Directory to populate. Defaults to ~/.local/lib/kache/shims

Options:
  --force           Replace existing entries instead of refusing
  --from-path       Also wrap every other compiler name on PATH (target triplets)
  --canonical-only  Link only the unversioned names, not the versioned compilers on PATH
  -h, --help        Print help
"
    .to_owned()
}

fn command_help(name: &str, json: bool) -> Result<String, ParseFail> {
    match name {
        "cargo" => Ok(cargo_help()),
        "help" => Ok(top_help()),
        "status" | "stats" => Ok(stats_help()),
        "list" => Ok(list_help()),
        "gc" => Ok(gc_help()),
        "purge" => Ok(purge_help()),
        "clean" => Ok(clean_help()),
        "init" => Ok(init_help()),
        "doctor" => Ok(doctor_help()),
        "daemon" => Ok(daemon_help()),
        "telemetry" => Ok(telemetry_help()),
        "why-miss" | "explain" => Ok(explain_help()),
        "diff" => Ok(diff_help()),
        "report" => Ok(report_help()),
        "config" => Ok(config_help()),
        "install-shims" => Ok(install_shims_help()),
        _ => Err(fail(json, format!("unrecognized subcommand '{name}'"))),
    }
}

fn subcommand_help(command: &str, subcommand: &str, json: bool) -> Result<String, ParseFail> {
    match (command, subcommand) {
        ("daemon", sub) => Ok(format!(
            "\
{about}

Usage: kache daemon {sub}

Options:
  -h, --help  Print help
",
            about = match sub {
                "status" => "Show daemon status (also bare `kache daemon`)",
                "run" => "Run the daemon server in the foreground",
                "start" => "Start the daemon in the background and wait until it is ready",
                "stop" => "Stop a running daemon",
                "restart" => "Restart the daemon, through its login service when one is installed",
                "install" => "Install the daemon as a per-user login service",
                "uninstall" => "Remove the daemon service",
                "log" => "Stream daemon logs",
                _ => return Err(fail(json, format!("unrecognized daemon subcommand '{sub}'"))),
            },
        )),
        ("telemetry", "write") => Ok(telemetry_write_help()),
        ("config", sub @ ("get" | "set" | "unset" | "list" | "path")) => Ok(config_sub_help(sub)),
        _ => Err(fail(
            json,
            format!("'{command} {subcommand}' is not a command; see 'kache {command} --help'"),
        )),
    }
}

/// One flag or positional for the machine-readable help document. Field
/// names match the document clients parse, so keep them in sync with
/// `machine::emit_help` tests.
#[derive(deser::Serialize)]
pub(crate) struct HelpArgument {
    name: &'static str,
    long: Option<&'static str>,
    short: Option<char>,
    required: bool,
    description: Option<&'static str>,
    values: Vec<&'static str>,
}

/// One command for the machine-readable help document.
#[derive(deser::Serialize)]
pub(crate) struct HelpCommand {
    name: &'static str,
    description: Option<&'static str>,
    usage: &'static str,
    arguments: Vec<HelpArgument>,
    subcommands: Vec<HelpCommand>,
}

fn arg(
    name: &'static str,
    long: Option<&'static str>,
    short: Option<char>,
    required: bool,
    description: &'static str,
    values: &'static [&'static str],
) -> HelpArgument {
    HelpArgument {
        name,
        long,
        short,
        required,
        description: Some(description),
        values: values.to_vec(),
    }
}

fn flag(name: &'static str, long: &'static str, description: &'static str) -> HelpArgument {
    arg(name, Some(long), None, false, description, &[])
}

fn flag_short(
    name: &'static str,
    long: &'static str,
    short: char,
    description: &'static str,
) -> HelpArgument {
    arg(name, Some(long), Some(short), false, description, &[])
}

fn json_arg() -> HelpArgument {
    flag(
        "json",
        "json",
        "Machine-readable JSON on stdout, on commands that support it",
    )
}

/// The whole command tree for `machine::emit_help`: visible commands only,
/// in help order. Descriptions match the text help above.
pub(crate) fn help_tree() -> HelpCommand {
    HelpCommand {
        name: "kache",
        description: Some("Compiler cache for Rust and C/C++."),
        usage: "Usage: kache [OPTIONS] [COMMAND]",
        arguments: vec![json_arg()],
        subcommands: vec![
            HelpCommand {
                name: "init",
                description: Some("Set up caching for Cargo and C/C++ builds"),
                usage: "Usage: kache init [OPTIONS]",
                arguments: vec![
                    flag_short("yes", "yes", 'y', "Accept all default answers (non-interactive)"),
                    flag("no-service", "no-service", "Do not install the daemon as a login service"),
                    flag("no-shell", "no-shell", "Leave shell setup alone"),
                    flag_short("check", "check", 'n', "Print what would change without modifying anything"),
                    arg("shims", Some("shims"), None, false, "Only create the compiler-name links to kache", &[]),
                    flag("from-path", "from-path", "With --shims, also link every other compiler name on PATH"),
                    flag("canonical-only", "canonical-only", "With --shims, link only the unversioned names"),
                    flag("force", "force", "With --shims, replace existing entries instead of refusing"),
                ],
                subcommands: vec![],
            },
            HelpCommand {
                name: "doctor",
                description: Some("Diagnose setup issues and verify cache integrity"),
                usage: "Usage: kache doctor [OPTIONS]",
                arguments: vec![
                    flag("fix", "fix", "Migrate from sccache and restart a daemon older than this binary"),
                    flag("purge-sccache", "purge-sccache", "Also remove sccache cache and binary (requires --fix)"),
                    flag("verify", "verify", "Verify cache integrity (entries, blobs, metadata)"),
                    flag("checksums", "checksums", "Also verify blob checksums (slower, implies --verify)"),
                    flag("repair", "repair", "Remove corrupted entries (implies --verify)"),
                    flag("probe", "probe", "Compile a temporary Rust library and verify a local cache hit"),
                    json_arg(),
                ],
                subcommands: vec![],
            },
            HelpCommand {
                name: "config",
                description: Some("Inspect or change configuration values"),
                usage: "Usage: kache config [COMMAND]",
                arguments: vec![json_arg()],
                subcommands: vec![
                    HelpCommand {
                        name: "get",
                        description: Some("Read a value explicitly set in the selected file"),
                        usage: "Usage: kache config get [OPTIONS] <KEY>",
                        arguments: vec![
                            arg("key", None, None, true, "Configuration key", &[]),
                            flag("project", "project", "Use the nearest project config"),
                            flag("global", "global", "Use the user config"),
                            arg("file", Some("file"), None, false, "Use this configuration file", &[]),
                        ],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "list",
                        description: Some("List values explicitly set in the selected file"),
                        usage: "Usage: kache config list [OPTIONS]",
                        arguments: vec![
                            flag("project", "project", "Use the nearest project config"),
                            flag("global", "global", "Use the user config"),
                            arg("file", Some("file"), None, false, "Use this configuration file", &[]),
                        ],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "set",
                        description: Some("Set a known configuration key"),
                        usage: "Usage: kache config set [OPTIONS] <KEY> <VALUE>",
                        arguments: vec![
                            arg("key", None, None, true, "Configuration key", &[]),
                            arg("value", None, None, true, "TOML value or unquoted string", &[]),
                            flag("project", "project", "Use the nearest project config"),
                            flag("global", "global", "Use the user config"),
                            arg("file", Some("file"), None, false, "Use this configuration file", &[]),
                        ],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "unset",
                        description: Some("Remove a value from the selected file"),
                        usage: "Usage: kache config unset [OPTIONS] <KEY>",
                        arguments: vec![
                            arg("key", None, None, true, "Configuration key", &[]),
                            flag("project", "project", "Use the nearest project config"),
                            flag("global", "global", "Use the user config"),
                            arg("file", Some("file"), None, false, "Use this configuration file", &[]),
                        ],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "path",
                        description: Some("Print the selected file's path"),
                        usage: "Usage: kache config path [OPTIONS]",
                        arguments: vec![
                            flag("project", "project", "Use the nearest project config"),
                            flag("global", "global", "Use the user config"),
                            arg("file", Some("file"), None, false, "Use this configuration file", &[]),
                        ],
                        subcommands: vec![],
                    },
                ],
            },
            HelpCommand {
                name: "stats",
                description: Some("Show hit rate, time saved and cache size"),
                usage: "Usage: kache stats [OPTIONS]",
                arguments: vec![
                    arg("since", Some("since"), None, false, "Event window", &[]),
                    flag("last-build", "last-build", "Report the latest recorded activity session"),
                    arg("root", Some("root"), None, false, "Only this build tree/root", &[]),
                    flag("full", "full", "The full build report instead of the summary"),
                    arg("format", Some("format"), None, false, "Write the full report in this format", crate::cli::ReportFormat::spellings()),
                    arg("output", Some("output"), Some('o'), false, "Write the full report to a file instead of stdout", &[]),
                    arg("top", Some("top"), None, false, "Entries in each top list of the full report", &[]),
                    flag("record", "record", "Also append the report summary and timing to telemetry"),
                    flag("redact", "redact", "Replace cache keys, roots, and object paths with `redacted`"),
                    json_arg(),
                ],
                subcommands: vec![],
            },
            HelpCommand {
                name: "clean",
                description: Some("Remove cache entries"),
                usage: "Usage: kache clean [OPTIONS]",
                arguments: vec![
                    flag_short("dry-run", "dry-run", 'n', "Show what would be removed and remove nothing"),
                    flag_short("yes", "yes", 'y', "Remove without asking"),
                    flag("cache", "cache", "Empty the cache instead"),
                    json_arg(),
                ],
                subcommands: vec![],
            },
            HelpCommand {
                name: "explain",
                description: Some("Explain why a crate, or the latest build, missed the cache"),
                usage: "Usage: kache explain [OPTIONS] [CRATE]",
                arguments: vec![
                    arg("crate-name", None, None, false, "The crate to explain", &[]),
                    flag("all", "all", "List every skip and passthrough across the retained build history"),
                    arg("root", Some("root"), None, false, "Only builds recorded for this build tree", &[]),
                    json_arg(),
                ],
                subcommands: vec![],
            },
            HelpCommand {
                name: "list",
                description: Some("List cache entries, or one crate's entries in detail"),
                usage: "Usage: kache list [OPTIONS] [CRATE]",
                arguments: vec![
                    arg("crate-name", None, None, false, "Crate name to show details for", &[]),
                    arg("sort", Some("sort"), None, false, "Sort by name, size, hits or age", &["name", "size", "hits", "age"]),
                    flag("no-pager", "no-pager", "Print directly instead of using a pager"),
                    json_arg(),
                ],
                subcommands: vec![],
            },
            HelpCommand {
                name: "daemon",
                description: Some("Show the background daemon's status, or start, stop or install it"),
                usage: "Usage: kache daemon [COMMAND]",
                arguments: vec![],
                subcommands: vec![
                    HelpCommand {
                        name: "status",
                        description: Some("Show daemon status (also bare `kache daemon`)"),
                        usage: "Usage: kache daemon status",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "run",
                        description: Some("Run the daemon server in the foreground"),
                        usage: "Usage: kache daemon run",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "start",
                        description: Some("Start the daemon in the background and wait until it is ready"),
                        usage: "Usage: kache daemon start",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "stop",
                        description: Some("Stop a running daemon"),
                        usage: "Usage: kache daemon stop",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "restart",
                        description: Some("Restart the daemon"),
                        usage: "Usage: kache daemon restart",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "install",
                        description: Some("Install the daemon as a per-user login service"),
                        usage: "Usage: kache daemon install",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "uninstall",
                        description: Some("Remove the daemon service"),
                        usage: "Usage: kache daemon uninstall",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: "log",
                        description: Some("Stream daemon logs"),
                        usage: "Usage: kache daemon log",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                ],
            },
        ],
    }
}

fn parse_doctor(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut fix = false;
    let mut purge_sccache = false;
    let mut verify = false;
    let mut checksums = false;
    let mut repair = false;
    let mut probe = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: doctor_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long("json") => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long("fix") => {
                no_value(&mut *parser, "fix", json)?;
                fix = true;
            }
            lexopt::Arg::Long("purge-sccache") => {
                no_value(&mut *parser, "purge-sccache", json)?;
                purge_sccache = true;
            }
            lexopt::Arg::Long("verify") => {
                no_value(&mut *parser, "verify", json)?;
                verify = true;
            }
            lexopt::Arg::Long("checksums") => {
                no_value(&mut *parser, "checksums", json)?;
                checksums = true;
            }
            lexopt::Arg::Long("repair") => {
                no_value(&mut *parser, "repair", json)?;
                repair = true;
            }
            lexopt::Arg::Long("probe") => {
                no_value(&mut *parser, "probe", json)?;
                probe = true;
            }
            lexopt::Arg::Value(value) => {
                return Err(fail(
                    json,
                    format!("unexpected argument '{}'", value.to_string_lossy()),
                ));
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    if purge_sccache && !fix {
        return Err(requires(json, "'--purge-sccache'", "'--fix'"));
    }
    run(
        json,
        Commands::Doctor {
            fix,
            purge_sccache,
            verify,
            checksums,
            repair,
            probe,
        },
    )
}

/// A parse failure, with whether `--json` was seen: JSON failures are
/// reported as a document, text failures as `error:` on stderr. Both exit 2.
pub(crate) struct ParseFail {
    pub json: bool,
    pub error: anyhow::Error,
}

fn fail(json: bool, message: impl Into<String>) -> ParseFail {
    ParseFail {
        json,
        error: machine::invalid_arguments(message),
    }
}

fn lexopt_error(error: lexopt::Error, json: bool) -> ParseFail {
    fail(json, error.to_string())
}

/// Parse `args` without the program name. Non-UTF-8 arguments fail closed
/// at the value that needs text; `cargo` arguments stay verbatim bytes.
pub(crate) fn parse_cli(args: &[OsString]) -> Result<ParseOutcome, ParseFail> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut json = false;
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Long("json") => {
                no_value(&mut parser, "json", json)?;
                json = true;
            }
            lexopt::Arg::Short('h') | lexopt::Arg::Long("help") => {
                return Ok(ParseOutcome::Help {
                    text: top_help(),
                    json,
                });
            }
            lexopt::Arg::Short('V') | lexopt::Arg::Long("version") => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Value(name) => {
                let name = utf8(name, json)?;
                return dispatch(&name, &mut parser, json);
            }
            lexopt::Arg::Short(flag) => {
                return Err(fail(json, format!("unrecognized flag '-{flag}'")));
            }
            lexopt::Arg::Long(flag) => {
                return Err(fail(json, format!("unrecognized option '--{flag}'")));
            },
        }
    }
    Ok(ParseOutcome::Run(Cli { json, command: None }))
}

/// Reject `--flag=value` on a flag that takes no value.
fn no_value(parser: &mut lexopt::Parser, flag: &str, json: bool) -> Result<(), ParseFail> {
    if let Some(value) = parser.optional_value() {
        return Err(fail(
            json,
            format!(
                "unexpected value '{}' for '--{flag}': no more expected",
                value.to_string_lossy()
            ),
        ));
    }
    Ok(())
}

fn utf8(value: OsString, json: bool) -> Result<String, ParseFail> {
    value.into_string().map_err(|value| {
        fail(
            json,
            format!(
                "invalid UTF-8 in command-line arguments: '{}'",
                value.to_string_lossy()
            ),
        )
    })
}

fn conflict(json: bool, first: &str, second: &str) -> ParseFail {
    fail(json, format!("{first} cannot be used with {second}"))
}

fn requires(json: bool, flag: &str, missing: &str) -> ParseFail {
    fail(
        json,
        format!("{flag} requires {missing} to also be set"),
    )
}

fn missing(json: bool, what: &str) -> ParseFail {
    fail(json, format!("missing required argument {what}"))
}

/// The next argument as a UTF-8 string.
fn next_string(
    parser: &mut lexopt::Parser,
    flag: &str,
    json: bool,
) -> Result<String, ParseFail> {
    let value = parser.value().map_err(|error| lexopt_error(error, json))?;
    utf8(value, json).map_err(|_| fail(json, format!("invalid UTF-8 value for '{flag}'")))
}

/// The next argument as a path. Paths stay byte-preserving: unlike flag
/// values they are never required to be UTF-8.
fn next_path(
    parser: &mut lexopt::Parser,
    flag: &str,
    json: bool,
) -> Result<PathBuf, ParseFail> {
    parser
        .value()
        .map(PathBuf::from)
        .map_err(|_| fail(json, format!("missing value for '{flag}'")))
}

fn next_usize(
    parser: &mut lexopt::Parser,
    flag: &str,
    json: bool,
) -> Result<usize, ParseFail> {
    let raw = next_string(&mut *parser, flag, json)?;
    raw.parse::<usize>().map_err(|_| {
        fail(
            json,
            format!("invalid value '{raw}' for '{flag}': expected a number"),
        )
    })
}

fn parse_sort(value: &str, json: bool) -> Result<ListSort, ParseFail> {
    match value {
        "name" => Ok(ListSort::Name),
        "size" => Ok(ListSort::Size),
        "hits" => Ok(ListSort::Hits),
        "age" => Ok(ListSort::Age),
        _ => Err(fail(
            json,
            "invalid value for '--sort': expected one of name, size, hits, age",
        )),
    }
}

fn parse_format(value: &str, json: bool) -> Result<crate::cli::ReportFormat, ParseFail> {
    crate::cli::ReportFormat::parse(value).ok_or_else(|| {
        fail(
            json,
            format!(
                "invalid value for '--format': expected one of {}",
                crate::cli::ReportFormat::spellings().join(", ")
            ),
        )
    })
}

fn parse_phase(value: &str, json: bool) -> Result<BenchPhase, ParseFail> {
    match value {
        "cold" => Ok(BenchPhase::Cold),
        "warm-same-tree" => Ok(BenchPhase::WarmSameTree),
        "warm" => Ok(BenchPhase::Warm),
        "pull" => Ok(BenchPhase::Pull),
        _ => Err(fail(
            json,
            "invalid value for '--phase': expected one of cold, warm-same-tree, warm, pull",
        )),
    }
}
