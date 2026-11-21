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

macro_rules! declare_cli_names {
    ($table:ident, $kind:ty; $($name:ident => $spelling:literal),* $(,)?) => {
        $(macro_rules! $name { () => { $spelling }; })*
        #[cfg(test)]
        const $table: &[$kind] = &[$($spelling),*];
    };
}

declare_cli_names!(COMMAND_NAMES, &str;
    cli_command_cargo => "cargo",
    cli_command_clean => "clean",
    cli_command_config => "config",
    cli_command_daemon => "daemon",
    cli_command_doctor => "doctor",
    cli_command_explain => "explain",
    cli_command_gc => "gc",
    cli_command_help => "help",
    cli_command_init => "init",
    cli_command_install_shims => "install-shims",
    cli_command_list => "list",
    cli_command_purge => "purge",
    cli_command_report => "report",
    cli_command_stats => "stats",
    cli_command_status => "status",
    cli_command_telemetry => "telemetry",
    cli_command_why_miss => "why-miss",
    cli_command_get => "get",
    cli_command_set => "set",
    cli_command_unset => "unset",
    cli_command_path => "path",
    cli_command_run => "run",
    cli_command_start => "start",
    cli_command_stop => "stop",
    cli_command_restart => "restart",
    cli_command_install => "install",
    cli_command_uninstall => "uninstall",
    cli_command_log => "log",
    cli_command_write => "write",
);

declare_cli_names!(LONG_FLAGS, &str;
    cli_flag_all => "all",
    cli_flag_cache => "cache",
    cli_flag_canonical_only => "canonical-only",
    cli_flag_check => "check",
    cli_flag_checksums => "checksums",
    cli_flag_crate => "crate",
    cli_flag_crate_name => "crate-name",
    cli_flag_dry_run => "dry-run",
    cli_flag_file => "file",
    cli_flag_fix => "fix",
    cli_flag_force => "force",
    cli_flag_format => "format",
    cli_flag_from_path => "from-path",
    cli_flag_full => "full",
    cli_flag_global => "global",
    cli_flag_help => "help",
    cli_flag_json => "json",
    cli_flag_max_age => "max-age",
    cli_flag_no_pager => "no-pager",
    cli_flag_no_service => "no-service",
    cli_flag_no_shell => "no-shell",
    cli_flag_output => "output",
    cli_flag_phase => "phase",
    cli_flag_probe => "probe",
    cli_flag_project => "project",
    cli_flag_purge_sccache => "purge-sccache",
    cli_flag_record => "record",
    cli_flag_redact => "redact",
    cli_flag_repair => "repair",
    cli_flag_root => "root",
    cli_flag_scenario => "scenario",
    cli_flag_shims => "shims",
    cli_flag_since => "since",
    cli_flag_sort => "sort",
    cli_flag_stale => "stale",
    cli_flag_stale_schema => "stale-schema",
    cli_flag_top => "top",
    cli_flag_verify => "verify",
    cli_flag_version => "version",
    cli_flag_yes => "yes",
);

declare_cli_names!(SHORT_FLAGS, char;
    cli_short_V => 'V',
    cli_short_h => 'h',
    cli_short_n => 'n',
    cli_short_o => 'o',
    cli_short_y => 'y',
);

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
            Some(Commands::Cargo { .. }) => names.push(cli_command_cargo!()),
            Some(Commands::List { .. }) => names.push(cli_command_list!()),
            Some(Commands::Gc { .. }) => names.push(cli_command_gc!()),
            Some(Commands::Purge { .. }) => names.push(cli_command_purge!()),
            Some(Commands::Clean { .. }) => names.push(cli_command_clean!()),
            Some(Commands::Init { .. }) => names.push(cli_command_init!()),
            Some(Commands::Doctor { .. }) => names.push(cli_command_doctor!()),
            Some(Commands::Daemon { command }) => {
                names.push(cli_command_daemon!());
                match command {
                    None => {}
                    Some(DaemonCommands::Status) => names.push(cli_command_status!()),
                    Some(DaemonCommands::Run) => names.push(cli_command_run!()),
                    Some(DaemonCommands::Start) => names.push(cli_command_start!()),
                    Some(DaemonCommands::Stop) => names.push(cli_command_stop!()),
                    Some(DaemonCommands::Restart) => names.push(cli_command_restart!()),
                    Some(DaemonCommands::Install) => names.push(cli_command_install!()),
                    Some(DaemonCommands::Uninstall) => names.push(cli_command_uninstall!()),
                    Some(DaemonCommands::Log) => names.push(cli_command_log!()),
                }
            }
            Some(Commands::Stats { .. }) => names.push(cli_command_stats!()),
            Some(Commands::Telemetry { command }) => {
                names.push(cli_command_telemetry!());
                if matches!(command, TelemetryCommands::Write { .. }) {
                    names.push(cli_command_write!());
                }
            }
            Some(Commands::Explain { .. }) => names.push(cli_command_explain!()),
            Some(Commands::Report { .. }) => names.push(cli_command_report!()),
            Some(Commands::Config { command }) => {
                names.push(cli_command_config!());
                match command {
                    None => {}
                    Some(config_cli::Command::Get { .. }) => names.push(cli_command_get!()),
                    Some(config_cli::Command::List { .. }) => names.push(cli_command_list!()),
                    Some(config_cli::Command::Set { .. }) => names.push(cli_command_set!()),
                    Some(config_cli::Command::Unset { .. }) => names.push(cli_command_unset!()),
                    Some(config_cli::Command::Path { .. }) => names.push(cli_command_path!()),
                }
            }
            Some(Commands::InstallShims { .. }) => names.push(cli_command_install_shims!()),
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
        cli_command_cargo!() => parse_cargo(parser, json),
        cli_command_help!() => parse_help(parser, json),
        cli_command_status!() => parse_stats(parser, json),
        cli_command_stats!() => parse_stats(parser, json),
        cli_command_list!() => parse_list(parser, json),
        cli_command_gc!() => parse_gc(parser, json),
        cli_command_purge!() => parse_purge(parser, json),
        cli_command_clean!() => parse_clean(parser, json),
        cli_command_init!() => parse_init(parser, json),
        cli_command_doctor!() => parse_doctor(parser, json),
        cli_command_daemon!() => parse_daemon(parser, json),
        cli_command_telemetry!() => parse_telemetry(parser, json),
        cli_command_why_miss!() => parse_explain(parser, json),
        cli_command_explain!() => parse_explain(parser, json),
        cli_command_report!() => parse_report(parser, json),
        cli_command_config!() => parse_config(parser, json),
        cli_command_install_shims!() => parse_install_shims(parser, json),
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help { text: top_help(), json });
            }
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: list_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_sort!()) => sort = parse_sort(&next_string(&mut *parser, "sort", json)?, json)?,
            lexopt::Arg::Long(cli_flag_no_pager!()) => {
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: gc_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_max_age!()) => {
                max_age = Some(next_string(&mut *parser, "max-age", json)?);
            }
            lexopt::Arg::Long(cli_flag_stale_schema!()) => {
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: purge_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_crate_name!()) => {
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: clean_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Short(cli_short_n!()) | lexopt::Arg::Long(cli_flag_dry_run!()) => {
                no_value(&mut *parser, "dry-run", json)?;
                dry_run = true;
            }
            lexopt::Arg::Short(cli_short_y!()) | lexopt::Arg::Long(cli_flag_yes!()) => {
                no_value(&mut *parser, "yes", json)?;
                yes = true;
            }
            lexopt::Arg::Long(cli_flag_stale!()) => stale = Some(next_string(&mut *parser, "stale", json)?),
            lexopt::Arg::Long(cli_flag_cache!()) => {
                no_value(&mut *parser, "cache", json)?;
                cache = true;
            }
            lexopt::Arg::Long(cli_flag_all!()) => {
                no_value(&mut *parser, "all", json)?;
                all = true;
            }
            lexopt::Arg::Long(cli_flag_crate!()) => {
                crate_name = Some(next_string(&mut *parser, "crate", json)?);
            }
            lexopt::Arg::Long(cli_flag_stale_schema!()) => {
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: init_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Short(cli_short_y!()) | lexopt::Arg::Long(cli_flag_yes!()) => {
                no_value(&mut *parser, "yes", json)?;
                yes = true;
            }
            lexopt::Arg::Long(cli_flag_no_service!()) => {
                no_value(&mut *parser, "no-service", json)?;
                no_service = true;
            }
            lexopt::Arg::Long(cli_flag_no_shell!()) => {
                no_value(&mut *parser, "no-shell", json)?;
                no_shell = true;
            }
            lexopt::Arg::Short(cli_short_n!()) | lexopt::Arg::Long(cli_flag_check!()) | lexopt::Arg::Long(cli_flag_dry_run!()) => {
                no_value(&mut *parser, "check", json)?;
                check = true;
            }
            lexopt::Arg::Long(cli_flag_shims!()) => {
                shims = Some(parse_optional_dir(&mut *parser, json)?);
            }
            lexopt::Arg::Long(cli_flag_from_path!()) => {
                no_value(&mut *parser, "from-path", json)?;
                from_path = true;
            }
            lexopt::Arg::Long(cli_flag_canonical_only!()) => {
                no_value(&mut *parser, "canonical-only", json)?;
                canonical_only = true;
            }
            lexopt::Arg::Long(cli_flag_force!()) => {
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
    Ok(raw.next_if(|next| !looks_like_flag(next)).map(PathBuf::from))
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: match subcommand.as_deref() {
                        Some(sub) => subcommand_help(cli_command_daemon!(), sub, json)?,
                        None => daemon_help(),
                    },
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
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
        Some(cli_command_status!()) => Some(DaemonCommands::Status),
        Some(cli_command_run!()) => Some(DaemonCommands::Run),
        Some(cli_command_start!()) => Some(DaemonCommands::Start),
        Some(cli_command_stop!()) => Some(DaemonCommands::Stop),
        Some(cli_command_restart!()) => Some(DaemonCommands::Restart),
        Some(cli_command_install!()) => Some(DaemonCommands::Install),
        Some(cli_command_uninstall!()) => Some(DaemonCommands::Uninstall),
        Some(cli_command_log!()) => Some(DaemonCommands::Log),
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: match subcommand.as_deref() {
                        Some(sub) => subcommand_help(cli_command_telemetry!(), sub, json)?,
                        None => telemetry_help(),
                    },
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
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
            lexopt::Arg::Long(cli_flag_scenario!()) => {
                scenario = Some(next_string(&mut *parser, "scenario", json)?);
            }
            lexopt::Arg::Long(cli_flag_phase!()) => {
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
        Some(cli_command_write!()) => {
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: stats_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_since!()) => {
                since = next_string(&mut *parser, "since", json)?;
            },
            lexopt::Arg::Long(cli_flag_root!()) => root = Some(next_path(&mut *parser, "root", json)?),
            lexopt::Arg::Long(cli_flag_full!()) => {
                no_value(&mut *parser, "full", json)?;
                full = true;
            }
            lexopt::Arg::Long(cli_flag_format!()) => {
                format = Some(parse_format(&next_string(&mut *parser, "format", json)?, json)?);
            }
            lexopt::Arg::Short(cli_short_o!()) | lexopt::Arg::Long(cli_flag_output!()) => {
                output = Some(next_path(&mut *parser, "output", json)?);
            }
            lexopt::Arg::Long(cli_flag_top!()) => top = Some(next_usize(&mut *parser, "top", json)?),
            lexopt::Arg::Long(cli_flag_record!()) => {
                no_value(&mut *parser, "record", json)?;
                record = true;
            }
            lexopt::Arg::Long(cli_flag_redact!()) => {
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
    run(
        json,
        Commands::Stats {
            since,
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
    loop {
        let Some(arg) = parser.next().map_err(|error| lexopt_error(error, json))? else {
            break;
        };
        match arg {
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: explain_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
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
    let crate_name = crate_name.ok_or_else(|| missing(json, "crate name"))?;
    run(json, Commands::Explain { crate_name })
}

fn parse_report(parser: &mut lexopt::Parser, mut json: bool) -> ParseResult {
    let mut format = crate::cli::ReportFormat::Text;
    let mut since = "24h".to_owned();
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: report_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_format!()) => {
                format = parse_format(&next_string(&mut *parser, "format", json)?, json)?;
            }
            lexopt::Arg::Long(cli_flag_since!()) => {
                since = next_string(&mut *parser, "since", json)?;
            },
            lexopt::Arg::Long(cli_flag_root!()) => root = Some(next_path(&mut *parser, "root", json)?),
            lexopt::Arg::Short(cli_short_o!()) | lexopt::Arg::Long(cli_flag_output!()) => {
                output = Some(next_path(&mut *parser, "output", json)?);
            }
            lexopt::Arg::Long(cli_flag_top!()) => top = next_usize(&mut *parser, "top", json)?,
            lexopt::Arg::Long(cli_flag_record!()) => {
                no_value(&mut *parser, "record", json)?;
                record = true;
            }
            lexopt::Arg::Long(cli_flag_redact!()) => {
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
    run(
        json,
        Commands::Report {
            format,
            since,
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: match subcommand.as_deref() {
                        Some(sub) => subcommand_help(cli_command_config!(), sub, json)?,
                        None => config_help(),
                    },
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_project!()) => {
                no_value(&mut *parser, "project", json)?;
                project = true;
            }
            lexopt::Arg::Long(cli_flag_global!()) => {
                no_value(&mut *parser, "global", json)?;
                global = true;
            }
            lexopt::Arg::Long(cli_flag_file!()) => file = Some(next_path(&mut *parser, "file", json)?),
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
        (Some(cli_command_get!()), [key]) => Some(config_cli::Command::Get {
            scope,
            key: key.clone(),
        }),
        (Some(cli_command_get!()), _) => return Err(missing(json, "<KEY>")),
        (Some(cli_command_set!()), [key, value]) => Some(config_cli::Command::Set {
            scope,
            key: key.clone(),
            value: value.clone(),
        }),
        (Some(cli_command_set!()), _) => return Err(missing(json, "<KEY> <VALUE>")),
        (Some(cli_command_unset!()), [key]) => Some(config_cli::Command::Unset {
            scope,
            key: key.clone(),
        }),
        (Some(cli_command_unset!()), _) => return Err(missing(json, "<KEY>")),
        (Some(cli_command_list!()), []) => Some(config_cli::Command::List { scope }),
        (Some(cli_command_list!()), [other, ..]) => {
            return Err(fail(json, format!("unexpected argument '{other}'")));
        }
        (Some(cli_command_path!()), []) => Some(config_cli::Command::Path { scope }),
        (Some(cli_command_path!()), [other, ..]) => {
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: install_shims_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_force!()) => {
                no_value(&mut *parser, "force", json)?;
                force = true;
            }
            lexopt::Arg::Long(cli_flag_from_path!()) => {
                no_value(&mut *parser, "from-path", json)?;
                from_path = true;
            }
            lexopt::Arg::Long(cli_flag_canonical_only!()) => {
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

const OWNED_TOOL_FOOTER: &str = "\
Maintained by Joshua Li: https://github.com/joshuarli/kache
Bugs and features: TODO.md in that repository (follow its AGENTS.md), or its issue tracker.
Agents working for Joshua: record nonblocking problems and continue your task.
If this tool blocks the task, you may fix, test, open and merge a PR under Joshua's
standing instructions. Release only when needed to deliver the verified fix.
Procedure: https://github.com/joshuarli/usr/blob/main/dotfiles/base/.agents/instructions/owned-tools.md";

const REFERENCE: &[(&[&str], &str)] = &[
    (&[cli_command_init!()], concat!("kache ", cli_command_init!(), " ", "--", cli_flag_check!())),
    (&[cli_command_doctor!()], concat!("kache ", cli_command_doctor!(), " ", "--", cli_flag_verify!())),
    (&[cli_command_config!()], concat!("kache ", cli_command_config!())),
    (&[cli_command_config!(), cli_command_get!()], concat!("kache ", cli_command_config!(), " ", cli_command_get!(), " cache.local_store")),
    (&[cli_command_config!(), cli_command_list!()], concat!("kache ", cli_command_config!(), " ", cli_command_list!())),
    (&[cli_command_config!(), cli_command_set!()], concat!("kache ", cli_command_config!(), " ", cli_command_set!(), " cache.local_store .work/tmp/kache")),
    (&[cli_command_config!(), cli_command_unset!()], concat!("kache ", cli_command_config!(), " ", cli_command_unset!(), " cache.local_store")),
    (&[cli_command_config!(), cli_command_path!()], concat!("kache ", cli_command_config!(), " ", cli_command_path!())),
    (&[cli_command_stats!()], concat!("kache ", cli_command_stats!(), " ", "--", cli_flag_since!(), " 7d")),
    (&[cli_command_clean!()], concat!("kache ", cli_command_clean!(), " ", "--", cli_flag_dry_run!())),
    (&[cli_command_explain!()], concat!("kache ", cli_command_explain!(), " serde")),
    (&[cli_command_list!()], concat!("kache ", cli_command_list!(), " ", "--", cli_flag_sort!(), " size")),
    (&[cli_command_daemon!()], concat!("kache ", cli_command_daemon!())),
    (&[cli_command_daemon!(), cli_command_status!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_status!())),
    (&[cli_command_daemon!(), cli_command_run!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_run!())),
    (&[cli_command_daemon!(), cli_command_start!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_start!())),
    (&[cli_command_daemon!(), cli_command_stop!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_stop!())),
    (&[cli_command_daemon!(), cli_command_restart!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_restart!())),
    (&[cli_command_daemon!(), cli_command_install!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_install!())),
    (&[cli_command_daemon!(), cli_command_uninstall!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_uninstall!())),
    (&[cli_command_daemon!(), cli_command_log!()], concat!("kache ", cli_command_daemon!(), " ", cli_command_log!())),
    (&[cli_command_cargo!()], concat!("kache ", cli_command_cargo!(), " -- build")),
    (&[cli_command_gc!()], concat!("kache ", cli_command_gc!(), " ", "--", cli_flag_max_age!(), " 7d")),
    (&[cli_command_purge!()], concat!("kache ", cli_command_purge!(), " ", "--", cli_flag_crate_name!(), " serde")),
    (&[cli_command_telemetry!()], concat!("kache ", cli_command_telemetry!(), " ", cli_command_write!(), " /tmp/kache-metrics")),
    (&[cli_command_telemetry!(), cli_command_write!()], concat!("kache ", cli_command_telemetry!(), " ", cli_command_write!(), " /tmp/kache-metrics")),
    (&[cli_command_report!()], concat!("kache ", cli_command_report!(), " ", "--", cli_flag_format!(), " json")),
    (&[cli_command_install_shims!()], concat!("kache ", cli_command_install_shims!(), " /tmp/kache-shims")),
    (&[cli_command_help!()], concat!("kache ", cli_command_help!(), " ", cli_command_list!())),
];

fn top_help() -> String {
    let mut text = String::from(concat!("Compiler cache for Rust and C/C++.\n\nUsage: kache [OPTIONS] [COMMAND]\n\nCommon options:\n  ", "--", cli_flag_json!(), "       Machine-readable JSON on commands that support it\n  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "   Print ", cli_command_help!(), "\n  ", "-", cli_short_V!(), ", ", "--", cli_flag_version!(), "  Print version\n\nCommand reference:\n"));
    for &(path, _) in REFERENCE {
        if path == [cli_command_help!()] {
            text.push_str(concat!("\nUsage: kache ", cli_command_help!(), " [COMMAND] [SUBCOMMAND]\n\nShow command ", cli_command_help!(), ".\n"));
            continue;
        }
        let page = if path.len() == 1 {
            command_help(path[0], false).unwrap_or_else(|_| panic!("invalid reference command"))
        } else {
            subcommand_help(path[0], path[1], false).unwrap_or_else(|_| panic!("invalid reference subcommand"))
        };
        text.push('\n');
        for line in page.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with(concat!("-", cli_short_h!(), ", ", "--", cli_flag_help!())) || trimmed.starts_with(concat!("--", cli_flag_json!(), " ")) {
                continue;
            }
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str("\nEnvironment:\n  Unset overrides use file/default configuration. File-backed overrides:\n");
    for (variable, key) in crate::config::file_environment_overrides() {
        text.push_str(&format!("  {variable}  Override {key}\n"));
    }
    text.push_str("  KACHE_CONFIG, KACHE_HOST_CONFIG  Select configuration files\n  KACHE_DISABLED  Disable caching\n  KACHE_SOCKET_PATH, KACHE_TRUST_DOMAIN  Select daemon socket and trust domain\n  KACHE_LOG, KACHE_LOG_FILE, KACHE_PROGRESS  Logging and progress controls\n  KACHE_NAMESPACE, KACHE_BASE_DIR  Cache namespace and relocation base\n\nExamples:\n");
    for &(_, example) in REFERENCE {
        text.push_str("  ");
        text.push_str(example);
        text.push('\n');
    }
    text.push('\n');
    text.push_str(OWNED_TOOL_FOOTER);
    text.push('\n');
    text
}

fn cargo_help() -> String {
    concat!("\
Run Cargo with Kache as its Rust compiler wrapper

Usage: kache ", cli_command_cargo!(), " [ARGS]...

Arguments:
  [ARGS]...  Arguments passed verbatim to Cargo (use `--` first)
")
    .to_owned()
}

fn list_help() -> String {
    concat!("\
List cache entries, or one crate's entries in detail

Usage: kache ", cli_command_list!(), " [OPTIONS] [CRATE]

Arguments:
  [CRATE]  Crate name to show details for (omit to ", cli_command_list!(), " all)

Options:
  ", "--", cli_flag_sort!(), " <name|size|hits|age>  Sort by name, size, hits or age [default: name]
  ", "--", cli_flag_no_pager!(), "                   Print directly instead of using a pager
  ", "--", cli_flag_json!(), "                       Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "                   Print ", cli_command_help!(), "
")
    .to_owned()
}

fn gc_help() -> String {
    concat!("\
Evict cache entries by the size and age limits, as the ", cli_command_daemon!(), " does.
Kept for scripts

Usage: kache ", cli_command_gc!(), " [OPTIONS]

Options:
  ", "--", cli_flag_max_age!(), " <DURATION>  Evict entries older than this duration (e.g. 7d, 24h)
  ", "--", cli_flag_stale_schema!(), "        Remove entries from old or unrecorded cache-key schemas
  ", "--", cli_flag_json!(), "                Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "            Print ", cli_command_help!(), "
")
    .to_owned()
}

fn purge_help() -> String {
    concat!("\
Remove the whole cache or one crate's entries. Kept for scripts;
`kache ", cli_command_clean!(), " ", "--", cli_flag_cache!(), "` empties the cache too

Usage: kache ", cli_command_purge!(), " [OPTIONS]

Options:
  ", "--", cli_flag_crate_name!(), " <NAME>  Only ", cli_command_purge!(), " entries for this crate
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "           Print ", cli_command_help!(), "
")
    .to_owned()
}

fn clean_help() -> String {
    concat!("\
Remove cache entries

Usage: kache ", cli_command_clean!(), " [OPTIONS]

Options:
  ", "-", cli_short_n!(), ", ", "--", cli_flag_dry_run!(), "        Show what would be removed and what kache keeps, and remove nothing
  ", "-", cli_short_y!(), ", ", "--", cli_flag_yes!(), "            Remove without asking. For scripts and cron; preview first with ", "--", cli_flag_dry_run!(), "
  ", "--", cli_flag_cache!(), "              Empty the cache instead. Asks first. The ", cli_command_daemon!(), " already keeps it under its size limit
  ", "--", cli_flag_all!(), "                Remove all cache entries (requires ", "--", cli_flag_cache!(), ")
  ", "--", cli_flag_stale!(), " <DURATION>   Remove old entries (requires ", "--", cli_flag_cache!(), "; conflicts with ", "--", cli_flag_all!(), " and ", "--", cli_flag_stale_schema!(), ")
  ", "--", cli_flag_stale_schema!(), "       Remove old schema entries (requires ", "--", cli_flag_cache!(), "; conflicts with ", "--", cli_flag_all!(), ")
  ", "--", cli_flag_crate!(), " <NAME>       Remove one crate (conflicts with ", "--", cli_flag_cache!(), ")
  ", "--", cli_flag_json!(), "               Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "           Print ", cli_command_help!(), "
")
    .to_owned()
}

fn init_help() -> String {
    concat!("\
Set up caching for Cargo and C/C++ builds

Usage: kache ", cli_command_init!(), " [OPTIONS]

Options:
  ", "-", cli_short_y!(), ", ", "--", cli_flag_yes!(), "            Accept all default answers (non-interactive)
  ", "--", cli_flag_no_service!(), "         Do not ", cli_command_install!(), " the ", cli_command_daemon!(), " as a login service
  ", "--", cli_flag_no_shell!(), "           Leave shell setup alone: no C/C++ shims on PATH
  ", "-", cli_short_n!(), ", ", "--", cli_flag_check!(), "          Print what would change without modifying anything [alias: ", "--", cli_flag_dry_run!(), "]
  ", "--", cli_flag_shims!(), " [<DIR>]      Only create the compiler-name links to kache (Unix), in DIR or ~/.local/lib/kache/shims
  ", "--", cli_flag_from_path!(), "          With ", "--", cli_flag_shims!(), ", also link every other compiler name on PATH (target triplets)
  ", "--", cli_flag_canonical_only!(), "     With ", "--", cli_flag_shims!(), ", link only the unversioned names, not the versioned compilers on PATH
  ", "--", cli_flag_force!(), "              With ", "--", cli_flag_shims!(), ", replace existing entries instead of refusing
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "           Print ", cli_command_help!(), "
")
    .to_owned()
}

fn doctor_help() -> String {
    concat!("\
Diagnose setup issues and verify cache integrity

Usage: kache ", cli_command_doctor!(), " [OPTIONS]

Options:
  ", "--", cli_flag_fix!(), "              Migrate from sccache (Cargo wrapper, running server) and ", cli_command_restart!(), " a ", cli_command_daemon!(), " older than this binary
  ", "--", cli_flag_purge_sccache!(), "    Also remove sccache cache and binary (requires ", "--", cli_flag_fix!(), ")
  ", "--", cli_flag_verify!(), "           Verify cache integrity (entries, blobs, metadata)
  ", "--", cli_flag_checksums!(), "        Also verify blob checksums (slower, implies ", "--", cli_flag_verify!(), ")
  ", "--", cli_flag_repair!(), "           Remove corrupted entries (implies ", "--", cli_flag_verify!(), ")
  ", "--", cli_flag_probe!(), "            Compile a temporary Rust library and verify a local cache hit
  ", "--", cli_flag_json!(), "             Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "         Print ", cli_command_help!(), "
")
    .to_owned()
}

fn daemon_help() -> String {
    concat!("\
Show the background ", cli_command_daemon!(), "'s ", cli_command_status!(), ", or ", cli_command_start!(), ", ", cli_command_stop!(), " or ", cli_command_install!(), " it

Usage: kache ", cli_command_daemon!(), " [COMMAND]

Commands:
  ", cli_command_status!(), "     Show ", cli_command_daemon!(), " ", cli_command_status!(), " (also bare `kache ", cli_command_daemon!(), "`)
  ", cli_command_run!(), "        Run the ", cli_command_daemon!(), " server in the foreground
  ", cli_command_start!(), "      Start the ", cli_command_daemon!(), " in the background and wait until it is ready
  ", cli_command_stop!(), "       Stop a running ", cli_command_daemon!(), "
  ", cli_command_restart!(), "    Restart the ", cli_command_daemon!(), ", through its login service when one is installed
  ", cli_command_install!(), "    Install the ", cli_command_daemon!(), " as a per-user login service
  ", cli_command_uninstall!(), "  Remove the ", cli_command_daemon!(), " service
  ", cli_command_log!(), "        Stream ", cli_command_daemon!(), " logs

Options:
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "  Print ", cli_command_help!(), "
")
    .to_owned()
}

fn stats_help() -> String {
    concat!("\
Show hit rate, time saved and cache size; ", "--", cli_flag_full!(), " for the whole ", cli_command_report!(), " [alias: ", cli_command_status!(), "]

Usage: kache ", cli_command_stats!(), " [OPTIONS]

Options:
  ", "--", cli_flag_since!(), " <SINCE>    Event window (e.g. 15m, 2h, 7d; a bare number is hours) [default: 24h]
  ", "--", cli_flag_root!(), " <PATH>      Only this build tree/root
  ", "--", cli_flag_full!(), "             The full build ", cli_command_report!(), " instead of the summary
  ", "--", cli_flag_format!(), " <FORMAT>  Write the full ", cli_command_report!(), " as json, markdown, github, or trace (Perfetto / Chrome trace)
  ", "-", cli_short_o!(), ", ", "--", cli_flag_output!(), " <PATH>
                     Write the full ", cli_command_report!(), " to a file instead of stdout
  ", "--", cli_flag_top!(), " <N>          Entries in each top ", cli_command_list!(), " of the full ", cli_command_report!(), "
  ", "--", cli_flag_record!(), "           Also append the full ", cli_command_report!(), "'s summary and timing, with the host's load, to ", cli_command_telemetry!(), "
  ", "--", cli_flag_redact!(), "           Replace cache keys, roots, and object paths with `redacted`
  ", "--", cli_flag_json!(), "             Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "         Print ", cli_command_help!(), "
")
    .to_owned()
}

fn telemetry_help() -> String {
    concat!("\
Write cache counters as OTLP JSON

Usage: kache ", cli_command_telemetry!(), " <COMMAND>

Commands:
  ", cli_command_write!(), "  Snapshot live cache counters into `metrics.otlp.json` + `schema_version`

Options:
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "  Print ", cli_command_help!(), "
")
    .to_owned()
}

fn telemetry_write_help() -> String {
    concat!("\
Snapshot live cache counters into `metrics.otlp.json` + `schema_version`

Usage: kache ", cli_command_telemetry!(), " ", cli_command_write!(), " [OPTIONS] <DIR>

Arguments:
  <DIR>  Directory to ", cli_command_write!(), ". Created if missing

Options:
  ", "--", cli_flag_scenario!(), " <NAME>   Bench scenario this dump belongs to
  ", "--", cli_flag_phase!(), " <PHASE>     Bench phase this dump belongs to (cold, warm-same-tree, warm, pull)
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "          Print ", cli_command_help!(), "
")
    .to_owned()
}

fn explain_help() -> String {
    concat!(
        "Explain why a crate missed the cache [alias: ", cli_command_why_miss!(), "]\n\n",
        "Usage: kache ", cli_command_explain!(), " [OPTIONS] <CRATE>\n\n",
        "Arguments:\n  <CRATE>       Crate name to explain\n\nOptions:\n  --",
        cli_flag_json!(), "        Machine-readable JSON on stdout\n  -",
        cli_short_h!(), ", --", cli_flag_help!(), "    Print help\n",
    ).to_owned()
}

fn report_help() -> String {
    concat!("\
The full build ", cli_command_report!(), ". Kept for scripts; `kache ", cli_command_stats!(), " ", "--", cli_flag_full!(), "` is the
same ", cli_command_report!(), "

Usage: kache ", cli_command_report!(), " [OPTIONS]

Options:
  ", "--", cli_flag_format!(), " <FORMAT>  Output format: json, trace (also perfetto, chrome-trace), markdown, github, text [default: text]
  ", "--", cli_flag_since!(), " <SINCE>    Event window (e.g. 15m, 2h, 7d; a bare number is hours) [default: 24h]
  ", "--", cli_flag_root!(), " <PATH>      Only include compiler events from this build tree/root
  ", "-", cli_short_o!(), ", ", "--", cli_flag_output!(), " <PATH>
                     Write output to a file instead of stdout
  ", "--", cli_flag_top!(), " <N>          Number of top entries to show [default: 10]
  ", "--", cli_flag_record!(), "           Also append this ", cli_command_report!(), "'s summary and timing, with the host's load, to ", cli_command_telemetry!(), "
  ", "--", cli_flag_redact!(), "           Replace cache keys, roots, and object paths with `redacted`
  ", "--", cli_flag_json!(), "             Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "         Print ", cli_command_help!(), "
")
    .to_owned()
}

fn config_help() -> String {
    concat!("\
Inspect or change configuration values (lists values without a subcommand)

Usage: kache ", cli_command_config!(), " [COMMAND]

Commands:
  ", cli_command_get!(), "    Read a value explicitly ", cli_command_set!(), " in the selected file
  ", cli_command_list!(), "   List values explicitly ", cli_command_set!(), " in the selected file
  ", cli_command_set!(), "    Set a known configuration key; accepts TOML values or an unquoted string
  ", cli_command_unset!(), "  Remove a value from the selected file
  ", cli_command_path!(), "   Print the selected file's ", cli_command_path!(), ", even if it does not exist

Options:
  ", "--", cli_flag_json!(), "      Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "  Print ", cli_command_help!(), "
")
    .to_owned()
}

fn config_sub_help(subcommand: &str) -> String {
    let (usage, about) = match subcommand {
        cli_command_get!() => (concat!("kache ", cli_command_config!(), " ", cli_command_get!(), " [OPTIONS] <KEY>"), "Read a value explicitly set in the selected file"),
        cli_command_set!() => (concat!("kache ", cli_command_config!(), " ", cli_command_set!(), " [OPTIONS] <KEY> <VALUE>"), "Set a known configuration key"),
        cli_command_unset!() => (concat!("kache ", cli_command_config!(), " ", cli_command_unset!(), " [OPTIONS] <KEY>"), "Remove a value from the selected file"),
        cli_command_list!() => (concat!("kache ", cli_command_config!(), " ", cli_command_list!(), " [OPTIONS]"), "List values explicitly set in the selected file"),
        cli_command_path!() => (concat!("kache ", cli_command_config!(), " ", cli_command_path!(), " [OPTIONS]"), "Print the selected file's path"),
        _ => unreachable!("config subcommand checked before this call"),
    };
    format!(
        concat!("\
{about}

Usage: {usage}

Arguments:
  <KEY>          Configuration key (where required)
  <VALUE>        TOML value or unquoted string (", cli_command_set!(), " only)

Options:
  ", "--", cli_flag_project!(), "      Use the nearest project ", cli_command_config!(), ", or create .kache.toml in this directory
  ", "--", cli_flag_global!(), "       Use the user ", cli_command_config!(), ", ignoring KACHE_CONFIG and project files
  ", "--", cli_flag_file!(), " <PATH>  Use this configuration file
  ", "--", cli_flag_json!(), "         Machine-readable JSON on stdout
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "     Print ", cli_command_help!(), "
"),
        about = about,
        usage = usage,
    )
}

fn install_shims_help() -> String {
    concat!("\
Create compiler-name links to kache in a directory you put first on
PATH. Kept for scripts and packages; `kache ", cli_command_init!(), " ", "--", cli_flag_shims!(), "` does the
same in the default directory

Usage: kache ", cli_command_install_shims!(), " [OPTIONS] [DIR]

Arguments:
  [DIR]  Directory to populate. Defaults to ~/.local/lib/kache/shims

Options:
  ", "--", cli_flag_force!(), "           Replace existing entries instead of refusing
  ", "--", cli_flag_from_path!(), "       Also wrap every other compiler name on PATH (target triplets)
  ", "--", cli_flag_canonical_only!(), "  Link only the unversioned names, not the versioned compilers on PATH
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "        Print ", cli_command_help!(), "
")
    .to_owned()
}

fn command_help(name: &str, json: bool) -> Result<String, ParseFail> {
    match name {
        cli_command_cargo!() => Ok(cargo_help()),
        cli_command_help!() => Ok(top_help()),
        cli_command_status!() | cli_command_stats!() => Ok(stats_help()),
        cli_command_list!() => Ok(list_help()),
        cli_command_gc!() => Ok(gc_help()),
        cli_command_purge!() => Ok(purge_help()),
        cli_command_clean!() => Ok(clean_help()),
        cli_command_init!() => Ok(init_help()),
        cli_command_doctor!() => Ok(doctor_help()),
        cli_command_daemon!() => Ok(daemon_help()),
        cli_command_telemetry!() => Ok(telemetry_help()),
        cli_command_why_miss!() | cli_command_explain!() => Ok(explain_help()),
        cli_command_report!() => Ok(report_help()),
        cli_command_config!() => Ok(config_help()),
        cli_command_install_shims!() => Ok(install_shims_help()),
        _ => Err(fail(json, format!("unrecognized subcommand '{name}'"))),
    }
}

fn subcommand_help(command: &str, subcommand: &str, json: bool) -> Result<String, ParseFail> {
    match (command, subcommand) {
        (cli_command_daemon!(), sub) => Ok(format!(
            concat!("\
{about}

Usage: kache ", cli_command_daemon!(), " {sub}

Options:
  ", "-", cli_short_h!(), ", ", "--", cli_flag_help!(), "  Print ", cli_command_help!(), "
"),
            about = match sub {
                cli_command_status!() => "Show daemon status (also bare `kache daemon`)",
                cli_command_run!() => "Run the daemon server in the foreground",
                cli_command_start!() => "Start the daemon in the background and wait until it is ready",
                cli_command_stop!() => "Stop a running daemon",
                cli_command_restart!() => "Restart the daemon, through its login service when one is installed",
                cli_command_install!() => "Install the daemon as a per-user login service",
                cli_command_uninstall!() => "Remove the daemon service",
                cli_command_log!() => "Stream daemon logs",
                _ => return Err(fail(json, format!("unrecognized daemon subcommand '{sub}'"))),
            },
            sub = sub,
        )),
        (cli_command_telemetry!(), cli_command_write!()) => Ok(telemetry_write_help()),
        (cli_command_config!(), sub @ (cli_command_get!() | cli_command_set!() | cli_command_unset!() | cli_command_list!() | cli_command_path!())) => Ok(config_sub_help(sub)),
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
                name: cli_command_init!(),
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
                name: cli_command_doctor!(),
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
                name: cli_command_config!(),
                description: Some("Inspect or change configuration values"),
                usage: "Usage: kache config [COMMAND]",
                arguments: vec![json_arg()],
                subcommands: vec![
                    HelpCommand {
                        name: cli_command_get!(),
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
                        name: cli_command_list!(),
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
                        name: cli_command_set!(),
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
                        name: cli_command_unset!(),
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
                        name: cli_command_path!(),
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
                name: cli_command_stats!(),
                description: Some("Show hit rate, time saved and cache size"),
                usage: "Usage: kache stats [OPTIONS]",
                arguments: vec![
                    arg("since", Some("since"), None, false, "Event window", &[]),
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
                name: cli_command_clean!(),
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
                name: cli_command_explain!(),
                description: Some("Explain why a crate missed the cache"),
                usage: "Usage: kache explain [OPTIONS] <CRATE>",
                arguments: vec![
                    arg("crate-name", None, None, true, "The crate to explain", &[]),
                    json_arg(),
                ],
                subcommands: vec![],
            },
            HelpCommand {
                name: cli_command_list!(),
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
                name: cli_command_daemon!(),
                description: Some("Show the background daemon's status, or start, stop or install it"),
                usage: "Usage: kache daemon [COMMAND]",
                arguments: vec![],
                subcommands: vec![
                    HelpCommand {
                        name: cli_command_status!(),
                        description: Some("Show daemon status (also bare `kache daemon`)"),
                        usage: "Usage: kache daemon status",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: cli_command_run!(),
                        description: Some("Run the daemon server in the foreground"),
                        usage: "Usage: kache daemon run",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: cli_command_start!(),
                        description: Some("Start the daemon in the background and wait until it is ready"),
                        usage: "Usage: kache daemon start",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: cli_command_stop!(),
                        description: Some("Stop a running daemon"),
                        usage: "Usage: kache daemon stop",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: cli_command_restart!(),
                        description: Some("Restart the daemon"),
                        usage: "Usage: kache daemon restart",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: cli_command_install!(),
                        description: Some("Install the daemon as a per-user login service"),
                        usage: "Usage: kache daemon install",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: cli_command_uninstall!(),
                        description: Some("Remove the daemon service"),
                        usage: "Usage: kache daemon uninstall",
                        arguments: vec![],
                        subcommands: vec![],
                    },
                    HelpCommand {
                        name: cli_command_log!(),
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
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: doctor_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
                return Ok(ParseOutcome::Version { json });
            }
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut *parser, "json", json)?;
                json = true;
            },
            lexopt::Arg::Long(cli_flag_fix!()) => {
                no_value(&mut *parser, "fix", json)?;
                fix = true;
            }
            lexopt::Arg::Long(cli_flag_purge_sccache!()) => {
                no_value(&mut *parser, "purge-sccache", json)?;
                purge_sccache = true;
            }
            lexopt::Arg::Long(cli_flag_verify!()) => {
                no_value(&mut *parser, "verify", json)?;
                verify = true;
            }
            lexopt::Arg::Long(cli_flag_checksums!()) => {
                no_value(&mut *parser, "checksums", json)?;
                checksums = true;
            }
            lexopt::Arg::Long(cli_flag_repair!()) => {
                no_value(&mut *parser, "repair", json)?;
                repair = true;
            }
            lexopt::Arg::Long(cli_flag_probe!()) => {
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
            lexopt::Arg::Long(cli_flag_json!()) => {
                no_value(&mut parser, "json", json)?;
                json = true;
            }
            lexopt::Arg::Short(cli_short_h!()) | lexopt::Arg::Long(cli_flag_help!()) => {
                return Ok(ParseOutcome::Help {
                    text: top_help(),
                    json,
                });
            }
            lexopt::Arg::Short(cli_short_V!()) | lexopt::Arg::Long(cli_flag_version!()) => {
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

#[cfg(test)]
mod reference_tests {
    use super::*;

    #[test]
    fn primary_help_covers_declared_spellings() {
        let help = top_help();
        for (index, flag) in LONG_FLAGS.iter().enumerate() {
            assert!(!LONG_FLAGS[..index].contains(flag));
            assert!(help.contains(&format!("--{flag}")), "missing --{flag}");
        }
        for (index, flag) in SHORT_FLAGS.iter().enumerate() {
            assert!(!SHORT_FLAGS[..index].contains(flag));
            assert!(help.contains(&format!("-{flag}")), "missing -{flag}");
        }
        for name in COMMAND_NAMES { assert!(help.contains(name), "missing {name}"); }
    }

    #[test]
    fn reference_examples_parse_without_execution() {
        let mut paths = std::collections::HashSet::new();
        for &(path, example) in REFERENCE {
            assert!(paths.insert(path), "duplicate command path: {path:?}");
            let args: Vec<OsString> = example.split_whitespace().skip(1).map(OsString::from).collect();
            match parse_cli(&args) {
                Ok(ParseOutcome::Run(cli)) => {
                    let expected = if path == ["telemetry"] { "telemetry-write".to_string() } else { path.join("-") };
                    assert_eq!(cli.command_path(), expected, "{example}");
                }
                Ok(ParseOutcome::Help { .. }) if path == ["help"] => {}
                _ => panic!("invalid example for {path:?}: {example}"),
            }
        }
    }

    #[test]
    fn primary_help_describes_configuration_overrides_without_reading_them() {
        let help = top_help();
        assert!(help.contains("KACHE_CACHE_DIR  Override cache.local_store"));
        assert!(help.contains("KACHE_RUNTIME_DIR  Override cache.runtime_dir"));
        assert!(help.contains("KACHE_CONFIG, KACHE_HOST_CONFIG"));
        assert!(help.contains("Unset overrides use file/default configuration"));
    }
}
