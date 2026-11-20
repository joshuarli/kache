//! `kache-cc-bench` command line.

use lexopt::prelude::*;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::cc_bench::{self, Config, workload::Archives};

const USAGE: &str = "usage: kache-cc-bench --kache PATH --ccache PATH [--toolchain NAME] [--out DIR]
                      [--workload NAME]... [--pairs N] [--seed N] [--jobs N] [--validate-only]
                      [--acceptance] [--zlib-archive PATH] [--llvm-archive PATH]

Times raw, ccache and kache builds of the fixed comparison set (corpus-c-tiny,
corpus-cxx-headers, zlib, llvm-subset) under the protocol locked in
campaigns/cpp/lock.toml. Every workload is first validated untimed through
compiler shims; a workload that fails validation is not timed. Writes
report.json and report.md under --out. Every arm runs with a declared
environment, recorded in the report; nothing else is inherited.

Diagnostic runs (the default) time --pairs pairs and exit 0 when every
selected workload validated and every timed replay was admitted.

--acceptance applies the locked acceptance rules instead of --pairs: at least
five complete admitted pairs, up to ten only while a target's 95% interval
crosses it, on a host where unrelated processes stay under 0.5 cores around
every timed build. Exit 0 only when every workload PASSes, 3 when the host
was ineligible, 1 otherwise (FAIL, INCONCLUSIVE or undersampled).";

pub fn main(argv: Vec<OsString>) -> i32 {
    if argv.get(1).is_some_and(|a| a == "--meter") {
        let (Some(report), Some(sep)) = (argv.get(2), argv.get(3)) else {
            eprintln!("usage: kache-cc-bench --meter REPORT -- COMMAND...");
            return 2;
        };
        if sep != "--" {
            eprintln!("usage: kache-cc-bench --meter REPORT -- COMMAND...");
            return 2;
        }
        return match cc_bench::meter(std::path::Path::new(report), &argv[4..]) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("kache-cc-bench --meter: {error:#}");
                1
            }
        };
    }
    let cfg = match parse(&argv) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(error) => {
            eprintln!("kache-cc-bench: {error}\n{USAGE}");
            return 2;
        }
    };
    match cc_bench::run(&cfg) {
        Ok(report) => {
            let json = deser_json::to_string(&report).unwrap_or_default();
            let md = cc_bench::markdown(&report);
            let written = std::fs::write(cfg.out.join("report.json"), json).and_then(|_| std::fs::write(cfg.out.join("report.md"), &md));
            if let Err(error) = written {
                eprintln!("kache-cc-bench: writing the report: {error}");
                return 1;
            }
            println!("{md}");
            if cfg.acceptance {
                return acceptance_exit(&report);
            }
            let ok = report.workloads.iter().all(|w| w.validation_failures.is_empty() && w.timed.iter().all(|r| r.admitted));
            if ok { 0 } else { 1 }
        }
        Err(error) => {
            eprintln!("kache-cc-bench: {error:#}");
            1
        }
    }
}

fn parse(argv: &[OsString]) -> Result<Option<Config>, String> {
    let mut cfg = Config {
        kache: PathBuf::new(),
        ccache: PathBuf::new(),
        toolchain: "cc".into(),
        out: PathBuf::from(".work/tmp/cc-bench"),
        workloads: Vec::new(),
        pairs: 5,
        seed: 20261117,
        validate_only: false,
        acceptance: false,
        archives: Archives::default(),
        jobs: std::thread::available_parallelism().map_or(4, |n| n.get()),
        meter: std::env::current_exe().map_err(|e| e.to_string())?,
        path: std::env::var("PATH").unwrap_or_default(),
    };
    let mut parser = lexopt::Parser::from_args(argv.iter().skip(1));
    let value = |p: &mut lexopt::Parser, flag: &str| -> Result<String, String> {
        p.value().map(|v| v.to_string_lossy().into_owned()).map_err(|_| format!("{flag} requires a value"))
    };
    let number = |text: String, flag: &str| text.parse::<u64>().map_err(|_| format!("{flag} takes a number"));
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Long("kache") => cfg.kache = absolute(value(&mut parser, "--kache")?)?,
            Long("ccache") => cfg.ccache = absolute(value(&mut parser, "--ccache")?)?,
            Long("toolchain") => cfg.toolchain = value(&mut parser, "--toolchain")?,
            Long("out") => cfg.out = value(&mut parser, "--out")?.into(),
            Long("workload") => cfg.workloads.push(value(&mut parser, "--workload")?),
            Long("pairs") => cfg.pairs = number(value(&mut parser, "--pairs")?, "--pairs")? as usize,
            Long("seed") => cfg.seed = number(value(&mut parser, "--seed")?, "--seed")?,
            Long("jobs") => cfg.jobs = number(value(&mut parser, "--jobs")?, "--jobs")? as usize,
            Long("validate-only") => cfg.validate_only = true,
            Long("acceptance") => cfg.acceptance = true,
            Long("zlib-archive") => cfg.archives.zlib = Some(absolute(value(&mut parser, "--zlib-archive")?)?),
            Long("llvm-archive") => cfg.archives.llvm = Some(absolute(value(&mut parser, "--llvm-archive")?)?),
            Long("help") | Short('h') => return Ok(None),
            other => return Err(other.unexpected().to_string()),
        }
    }
    if cfg.acceptance && cfg.validate_only {
        return Err("--acceptance times builds; it cannot be combined with --validate-only".into());
    }
    if cfg.kache.as_os_str().is_empty() || cfg.ccache.as_os_str().is_empty() {
        return Err("--kache and --ccache are required".into());
    }
    if !(5..=10).contains(&cfg.pairs) {
        return Err("--pairs must be 5..=10, the locked sample budget".into());
    }
    for name in &cfg.workloads {
        if !cc_bench::workload::all().iter().any(|w| w.name == name) {
            return Err(format!("unknown --workload `{name}`"));
        }
    }
    Ok(Some(cfg))
}

fn absolute(path: String) -> Result<PathBuf, String> {
    std::fs::canonicalize(&path).map_err(|e| format!("{path}: {e}"))
}

/// 0 when every workload PASSed, 3 when any ran on an ineligible host,
/// 1 otherwise.
fn acceptance_exit(report: &cc_bench::Report) -> i32 {
    use cc_bench::AcceptanceVerdict as V;
    let verdicts: Vec<V> = report.workloads.iter().filter_map(|w| w.acceptance.as_ref().map(|a| a.verdict)).collect();
    if verdicts.is_empty() {
        1
    } else if verdicts.contains(&V::Ineligible) {
        3
    } else if verdicts.iter().all(|v| *v == V::Pass) {
        0
    } else {
        1
    }
}
