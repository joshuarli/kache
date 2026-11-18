//! `kache-cc-oracle` command line.

use lexopt::prelude::*;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::cc_oracle::case::Profile;
use crate::cc_oracle::exec::Control;
use crate::cc_oracle::{self, Config, Selection};

const USAGE: &str = "usage: kache-cc-oracle --print-ccache-pin [--lock PATH]
       kache-cc-oracle --kache PATH --ccache PATH [--cases DIR] [--out DIR]
                       [--toolchain NAME]... [--case ID]... [--profile transparent|reproducible]...
                       [--full] [--negative-control KIND]

Runs C/C++ differential cases through raw, ccache and kache arms and judges
kache against the raw compiler. Exit 0 only when every selected run matches.
With --negative-control, exit 0 only when the injected fault is detected.
Controls: stale-output omit-sidecar disabled-invalidation cache-disabled reference-failure";

enum Mode {
    Help,
    PrintCcachePin(PathBuf),
    Run(Config),
}

fn parse(argv: &[OsString]) -> Result<Mode, String> {
    let mut print_pin = false;
    let mut lock = PathBuf::from("campaigns/cpp/lock.toml");
    let mut config = Config {
        kache: PathBuf::new(),
        ccache: PathBuf::new(),
        cases: PathBuf::from("campaigns/cpp/oracle"),
        out: PathBuf::from(".work/tmp/cc-oracle"),
        toolchains: Vec::new(),
        only: Vec::new(),
        profiles: Vec::new(),
        selection: Selection::Gate,
        control: None,
    };
    let mut parser = lexopt::Parser::from_args(argv.iter().skip(1));
    let value = |p: &mut lexopt::Parser, flag: &str| -> Result<String, String> {
        p.value().map(|v| v.to_string_lossy().into_owned()).map_err(|_| format!("{flag} requires a value"))
    };
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Long("kache") => config.kache = value(&mut parser, "--kache")?.into(),
            Long("ccache") => config.ccache = value(&mut parser, "--ccache")?.into(),
            Long("cases") => config.cases = value(&mut parser, "--cases")?.into(),
            Long("out") => config.out = value(&mut parser, "--out")?.into(),
            Long("toolchain") => config.toolchains.push(value(&mut parser, "--toolchain")?),
            Long("case") => config.only.push(value(&mut parser, "--case")?),
            Long("profile") => {
                let raw = value(&mut parser, "--profile")?;
                config.profiles.push(match raw.as_str() {
                    "transparent" => Profile::Transparent,
                    "reproducible" => Profile::Reproducible,
                    _ => return Err(format!("unknown --profile `{raw}`")),
                });
            }
            Long("full") => config.selection = Selection::Full,
            Long("negative-control") => {
                let raw = value(&mut parser, "--negative-control")?;
                config.control = Some(Control::parse(&raw).ok_or_else(|| format!("unknown control `{raw}`"))?);
            }
            Long("print-ccache-pin") => print_pin = true,
            Long("lock") => lock = value(&mut parser, "--lock")?.into(),
            Long("help") | Short('h') => return Ok(Mode::Help),
            other => return Err(other.unexpected().to_string()),
        }
    }
    if print_pin {
        return Ok(Mode::PrintCcachePin(lock));
    }
    if config.kache.as_os_str().is_empty() || config.ccache.as_os_str().is_empty() {
        return Err("--kache and --ccache are required".into());
    }
    if config.toolchains.is_empty() {
        config.toolchains.push("cc".into());
    }
    if config.profiles.is_empty() {
        config.profiles = vec![Profile::Transparent, Profile::Reproducible];
    }
    Ok(Mode::Run(config))
}

pub fn main(argv: Vec<OsString>) -> i32 {
    let config = match parse(&argv) {
        Ok(Mode::Run(config)) => config,
        Ok(Mode::Help) => {
            println!("{USAGE}");
            return 0;
        }
        Ok(Mode::PrintCcachePin(lock)) => {
            return match cc_oracle::pin::host_ccache_pin(&lock) {
                Ok(pin) => {
                    println!("{} {} {}", pin.lane, pin.url, pin.sha256);
                    0
                }
                Err(e) => {
                    eprintln!("kache-cc-oracle: {e:#}");
                    2
                }
            };
        }
        Err(e) => {
            eprintln!("kache-cc-oracle: {e}\n\n{USAGE}");
            return 2;
        }
    };
    let outcome = match cc_oracle::run(&config) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("kache-cc-oracle: {e:#}");
            return 2;
        }
    };
    let md = std::fs::read_to_string(outcome.report_dir.join("report.md")).unwrap_or_default();
    eprintln!("\n{md}");
    eprintln!("Report: {}", outcome.report_dir.join("report.json").display());
    match config.control {
        Some(control) => {
            if outcome.control_detected(control) {
                eprintln!(
                    "PASS: negative control {} detected as {}",
                    control.as_str(),
                    cc_oracle::expected_verdict(control).as_str()
                );
                0
            } else {
                eprintln!("FAIL: negative control {} was not detected", control.as_str());
                1
            }
        }
        None if outcome.gate_passed() => 0,
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        std::iter::once("kache-cc-oracle").chain(list.iter().copied()).map(OsString::from).collect()
    }

    #[test]
    fn defaults_cover_both_profiles_and_the_host_cc() {
        let Mode::Run(c) = parse(&args(&["--kache", "k", "--ccache", "c"])).unwrap() else { panic!() };
        assert_eq!(c.toolchains, ["cc"]);
        assert_eq!(c.profiles, [Profile::Transparent, Profile::Reproducible]);
        assert_eq!(c.selection, Selection::Gate);
        assert!(c.control.is_none());
    }

    #[test]
    fn repeated_options_accumulate() {
        let Mode::Run(c) = parse(&args(&[
            "--kache", "k", "--ccache", "c", "--toolchain", "gcc", "--toolchain", "clang", "--case", "x",
            "--profile", "reproducible", "--full", "--negative-control", "omit-sidecar",
        ]))
        .unwrap() else { panic!() };
        assert_eq!(c.toolchains, ["gcc", "clang"]);
        assert_eq!(c.only, ["x"]);
        assert_eq!(c.profiles, [Profile::Reproducible]);
        assert_eq!(c.selection, Selection::Full);
        assert_eq!(c.control, Some(Control::OmitSidecar));
    }

    #[test]
    fn rejects_missing_binaries_and_unknown_values() {
        assert!(parse(&args(&["--kache", "k"])).is_err());
        assert!(parse(&args(&["--kache", "k", "--ccache", "c", "--profile", "x"])).is_err());
        assert!(parse(&args(&["--kache", "k", "--ccache", "c", "--negative-control", "x"])).is_err());
        assert!(parse(&args(&["--bogus"])).is_err());
        assert!(matches!(parse(&args(&["--help"])).unwrap(), Mode::Help));
        assert!(matches!(
            parse(&args(&["--print-ccache-pin", "--lock", "l.toml"])).unwrap(),
            Mode::PrintCcachePin(p) if p == std::path::Path::new("l.toml")
        ));
    }
}
