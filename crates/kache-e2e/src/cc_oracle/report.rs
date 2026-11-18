//! `report.json` (versioned, complete) and `report.md` (one line per run).

use anyhow::Result;
use deser::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use super::exec::Toolchain;
use super::judge::{Executions, Judgement, Verdict};
use super::{Config, Outcome, Selection};

const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    run_id: String,
    mode: String,
    passed: bool,
    host: Host,
    tools: Vec<Tool>,
    summary: BTreeMap<&'static str, usize>,
    runs: Vec<RunReport>,
}

#[derive(Serialize)]
struct Host {
    os: &'static str,
    arch: &'static str,
    uname: String,
}

#[derive(Serialize)]
struct Tool {
    role: String,
    path: String,
    blake3: String,
    version: String,
}

#[derive(Serialize)]
struct RunReport {
    case: String,
    cohort: String,
    profile: &'static str,
    toolchain: String,
    verdict: &'static str,
    fails_gate: bool,
    #[deser(skip_serializing_if = Option::is_none)]
    blocked: Option<String>,
    expected_rows: usize,
    injected: bool,
    rows: Vec<RowReport>,
}

#[derive(Serialize)]
struct RowReport {
    step: String,
    compile: usize,
    reuse: String,
    judgement: Judgement,
    arms: BTreeMap<&'static str, ArmReport>,
}

#[derive(Serialize)]
struct ArmReport {
    status: Option<i32>,
    stdout_blake3: String,
    stderr_blake3: String,
    stderr: String,
    tree_blake3: String,
    executions: Executions,
    #[deser(skip_serializing_if = Option::is_none)]
    kache_result: Option<String>,
    duration_ms: u64,
}

fn digest_file(path: &Path) -> String {
    std::fs::read(path)
        .map(|b| blake3::hash(&b).to_hex().to_string())
        .unwrap_or_else(|e| format!("unreadable: {e}"))
}

fn first_line(program: &Path, arg: &str) -> String {
    Command::new(program)
        .arg(arg)
        .output()
        .map(|o| {
            let text = if o.stdout.is_empty() { o.stderr } else { o.stdout };
            String::from_utf8_lossy(&text).lines().next().unwrap_or("").to_string()
        })
        .unwrap_or_else(|e| format!("failed: {e}"))
}

fn tool(role: String, path: &Path) -> Tool {
    Tool {
        role,
        path: path.display().to_string(),
        blake3: digest_file(path),
        version: first_line(path, "--version"),
    }
}

fn mode(config: &Config) -> String {
    let base = if !config.only.is_empty() {
        format!("case:{}", config.only.join(","))
    } else {
        match config.selection {
            Selection::Gate => "gate".to_string(),
            Selection::Full => "full".to_string(),
        }
    };
    match config.control {
        Some(c) => format!("{base}+negative-control:{}", c.as_str()),
        None => base,
    }
}

pub fn write(
    outcome: &Outcome,
    config: &Config,
    kache: &Path,
    ccache: &Path,
    toolchains: &[Result<Toolchain, String>],
) -> Result<()> {
    let mut tools = vec![tool("kache".into(), kache), tool("ccache".into(), ccache)];
    for tc in toolchains.iter().flatten() {
        tools.push(tool(format!("{} cc", tc.name), &tc.cc));
        tools.push(tool(format!("{} cxx", tc.name), &tc.cxx));
    }
    let mut summary = BTreeMap::new();
    for run in &outcome.runs {
        *summary.entry(run.verdict.as_str()).or_insert(0) += 1;
    }
    let report = Report {
        schema_version: SCHEMA_VERSION,
        run_id: outcome
            .report_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        mode: mode(config),
        passed: outcome.gate_passed(),
        host: Host {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            uname: first_line(Path::new("uname"), "-a"),
        },
        tools,
        summary,
        runs: outcome.runs.iter().map(run_report).collect(),
    };
    let json = deser_json::SerializerConfig::builder()
        .pretty(deser_json::Indent::Spaces(2))
        .build()
        .to_string(&report)?;
    std::fs::write(outcome.report_dir.join("report.json"), json)?;
    std::fs::write(outcome.report_dir.join("report.md"), markdown(&report))?;
    Ok(())
}

fn run_report(run: &super::CaseRun) -> RunReport {
    RunReport {
        case: run.case.clone(),
        cohort: format!("{:?}", run.cohort).to_lowercase(),
        profile: run.profile.as_str(),
        toolchain: run.toolchain.clone(),
        verdict: run.verdict.as_str(),
        fails_gate: run.verdict.fails_gate(),
        blocked: run.blocked.clone(),
        expected_rows: run.expected_rows,
        injected: run.injected,
        rows: run
            .rows
            .iter()
            .map(|row| RowReport {
                step: row.step.clone(),
                compile: row.compile,
                reuse: format!("{:?}", row.reuse),
                judgement: row.judgement.clone(),
                arms: row
                    .arms
                    .iter()
                    .map(|(arm, (obs, ms))| {
                        let tree = deser_json::to_string(&obs.tree).unwrap_or_default();
                        (
                            arm.as_str(),
                            ArmReport {
                                status: obs.status,
                                stdout_blake3: blake3::hash(&obs.stdout).to_hex().to_string(),
                                stderr_blake3: blake3::hash(&obs.stderr).to_hex().to_string(),
                                stderr: String::from_utf8_lossy(&obs.stderr).into_owned(),
                                tree_blake3: blake3::hash(tree.as_bytes()).to_hex().to_string(),
                                executions: obs.executions.clone(),
                                kache_result: obs.kache_result.clone(),
                                duration_ms: *ms,
                            },
                        )
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn markdown(report: &Report) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# cc-oracle {} ({})\n", report.run_id, report.mode);
    let _ = writeln!(
        md,
        "Result: **{}** on {} {}\n",
        if report.passed { "PASS" } else { "FAIL" },
        report.host.os,
        report.host.arch
    );
    for t in &report.tools {
        let _ = writeln!(md, "- {}: `{}` {} (blake3 {})", t.role, t.path, t.version, &t.blake3[..t.blake3.len().min(16)]);
    }
    let _ = writeln!(md, "\n| case | profile | toolchain | verdict | first reason |\n|---|---|---|---|---|");
    for run in &report.runs {
        let reason = run
            .blocked
            .clone()
            .or_else(|| {
                run.rows
                    .iter()
                    .find(|r| r.judgement.verdict.as_str() == run.verdict && r.judgement.verdict != Verdict::Match)
                    .map(|r| format!("{}: {}", r.step, r.judgement.reasons.join("; ")))
            })
            .unwrap_or_default();
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} |",
            run.case,
            run.profile,
            run.toolchain,
            run.verdict,
            reason.replace('|', "\\|").replace('\n', " ")
        );
    }
    let failing: std::collections::BTreeSet<String> = report
        .runs
        .iter()
        .filter(|r| r.fails_gate)
        .map(|r| format!("- `make cc-oracle CASE={} TOOLCHAINS={} PROFILES={}`", r.case, r.toolchain, r.profile))
        .collect();
    if !failing.is_empty() {
        let _ = writeln!(md, "\nReproduce:\n");
        for line in failing {
            let _ = writeln!(md, "{line}");
        }
    }
    md
}
