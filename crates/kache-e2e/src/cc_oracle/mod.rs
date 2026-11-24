//! C/C++ differential oracle: each case runs through the raw compiler, the
//! pinned ccache in its normal and forced-preprocessor configurations, and
//! kache, with independent state per arm, then judges kache against raw.
//! See `docs/roadmap.md` (CCP-01) for the contract.

pub mod case;
pub mod exec;
mod gcov;
pub mod judge;
pub mod pin;
mod report;

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use case::{Case, Cohort, Profile};
use exec::{Arm, ArmContext, Control, Toolchain};
use judge::{Judgement, Observation, Row, Verdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// Blocking cohort only.
    Gate,
    /// Every case, including open gaps.
    Full,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub kache: PathBuf,
    pub ccache: PathBuf,
    pub cases: PathBuf,
    pub out: PathBuf,
    pub toolchains: Vec<String>,
    pub only: Vec<String>,
    pub profiles: Vec<Profile>,
    pub selection: Selection,
    pub control: Option<Control>,
}

pub struct RowResult {
    pub step: String,
    pub compile: usize,
    pub reuse: case::Reuse,
    pub judgement: Judgement,
    pub arms: BTreeMap<Arm, (Observation, u64)>,
}

pub struct CaseRun {
    pub case: String,
    pub cohort: Cohort,
    pub profile: Profile,
    pub toolchain: String,
    pub verdict: Verdict,
    /// Set when the run could not execute the case at all.
    pub blocked: Option<String>,
    pub rows: Vec<RowResult>,
    pub expected_rows: usize,
    pub injected: bool,
}

pub struct Outcome {
    pub runs: Vec<CaseRun>,
    pub report_dir: PathBuf,
}

impl Outcome {
    pub fn gate_passed(&self) -> bool {
        self.runs.iter().any(|r| !r.rows.is_empty())
            && self
                .runs
                .iter()
                .all(|r| !r.verdict.fails_gate() && r.rows.len() == r.expected_rows)
    }

    /// A control passes when it was injected somewhere, and every run it was
    /// injected into reports the verdict class the fault must produce.
    pub fn control_detected(&self, control: Control) -> bool {
        let want = expected_verdict(control);
        let injected: Vec<&CaseRun> = self.runs.iter().filter(|r| r.injected).collect();
        !injected.is_empty() && injected.iter().all(|r| r.rows.iter().any(|row| row.judgement.verdict == want))
    }
}

pub fn expected_verdict(control: Control) -> Verdict {
    match control {
        Control::StaleOutput | Control::OmitSidecar | Control::DisabledInvalidation => Verdict::KacheWrongOutput,
        Control::CacheDisabled => Verdict::KacheMissedReuse,
        Control::ReferenceFailure | Control::WrongEffectiveArgs => Verdict::HarnessError,
    }
}

pub fn select<'a>(cases: &'a [Case], config: &Config, os: &str) -> Result<Vec<&'a Case>> {
    for id in &config.only {
        if !cases.iter().any(|c| &c.id == id) {
            bail!("unknown case `{id}`");
        }
    }
    let selected: Vec<&Case> = cases
        .iter()
        .filter(|c| config.only.is_empty() || config.only.contains(&c.id))
        .filter(|c| !config.only.is_empty() || config.selection == Selection::Full || c.cohort == Cohort::Blocking)
        .filter(|c| c.applies_to_os(os))
        .collect();
    if selected.is_empty() {
        bail!("no cases selected for {os}");
    }
    Ok(selected)
}

pub fn run(config: &Config) -> Result<Outcome> {
    let kache = std::fs::canonicalize(&config.kache)
        .with_context(|| format!("kache binary not found at {}", config.kache.display()))?;
    let ccache = std::fs::canonicalize(&config.ccache)
        .with_context(|| format!("ccache binary not found at {}", config.ccache.display()))?;
    let cases = case::discover(&config.cases)?;
    let selected = select(&cases, config, std::env::consts::OS)?;

    let run_id = format!(
        "{}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        std::process::id()
    );
    std::fs::create_dir_all(&config.out)?;
    let report_dir = std::fs::canonicalize(&config.out)?.join(&run_id);
    let work = report_dir.join("work");
    std::fs::create_dir_all(&work)?;
    let fixture = work.join("fixture");
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let system_path = exec::clean_path(&inherited);

    let mut toolchains: Vec<Result<Toolchain, String>> = Vec::new();
    for name in &config.toolchains {
        toolchains.push(exec::resolve_toolchain(name, &system_path));
    }

    let mut runs = Vec::new();
    for tc in &toolchains {
        let (tc, blocked) = match tc {
            Ok(tc) => (Some(tc), None),
            Err(e) => (None, Some(e.clone())),
        };
        let tc_name = tc.map_or_else(|| "unresolved".to_string(), |t| t.name.clone());
        let tc_dir = work.join("toolchains").join(&tc_name);
        if let Some(tc) = tc {
            exec::write_shims(&tc_dir.join("shims"), tc, None)?;
            exec::write_shims(&tc_dir.join("failing-shims"), tc, Some(Path::new("/usr/bin/false")))?;
        }
        for case in &selected {
            for profile in case.profiles.iter().copied().filter(|p| config.profiles.contains(p)) {
                if config.control.is_some_and(|control| !exec::control_can_inject(control, case, profile)) {
                    continue;
                }
                let expected_rows = case.steps.len() * case.compiles.len();
                let mut run = CaseRun {
                    case: case.id.clone(),
                    cohort: case.cohort,
                    profile,
                    toolchain: tc_name.clone(),
                    verdict: Verdict::Blocked,
                    blocked: blocked.clone(),
                    rows: Vec::new(),
                    expected_rows,
                    injected: false,
                };
                let unsupported = tc.and_then(|tc| unsupported_family(case, tc));
                if let Some(reason) = unsupported {
                    run.verdict = Verdict::UnsupportedByCompiler;
                    run.blocked = Some(reason);
                    run.expected_rows = 0;
                } else if let Some(tc) = tc {
                    eprintln!("=> {} [{} {}]", case.id, profile.as_str(), tc.name);
                    let ctx = ArmContext {
                        kache: &kache,
                        ccache: &ccache,
                        toolchain: tc,
                        shims: &tc_dir.join("shims"),
                        failing_shims: &tc_dir.join("failing-shims"),
                        fixture: &fixture,
                        state: &work.join("arms").join(&case.id).join(profile.as_str()).join(&tc.name),
                        system_path: &system_path,
                        control: config.control,
                    };
                    if let Err(e) = run_case(case, profile, &ctx, &mut run) {
                        run.verdict = Verdict::HarnessError;
                        run.blocked = Some(format!("{e:#}"));
                    }
                    eprintln!("   {}", run.verdict.as_str());
                }
                runs.push(run);
            }
        }
    }
    if fixture.exists() {
        let _ = std::fs::remove_dir_all(&fixture);
    }
    let outcome = Outcome { runs, report_dir };
    report::write(&outcome, config, &kache, &ccache, &toolchains)?;
    Ok(outcome)
}

/// Why `case` cannot run on `toolchain`, when its family is not one the
/// case exercises. An unidentified family cannot be ruled out, so it runs.
fn unsupported_family(case: &Case, toolchain: &Toolchain) -> Option<String> {
    let family = toolchain.family?;
    if case.families.is_empty() || case.families.contains(&family) {
        return None;
    }
    let wanted: Vec<&str> = case.families.iter().map(|f| f.as_str()).collect();
    Some(format!("case exercises {}; {} is {}", wanted.join("/"), toolchain.name, family.as_str()))
}

fn unsupported_requirements(
    case: &Case,
    toolchain: &Toolchain,
    state: &Path,
    system_path: &std::ffi::OsStr,
) -> Result<Option<String>> {
    for argv in &case.requires {
        let driver = if argv[0] == "{cc}" { &toolchain.cc } else { &toolchain.cxx };
        let scratch = tempfile::tempdir_in(state)?;
        let stdin = scratch.path().join("stdin");
        std::fs::write(&stdin, &case.requires_stdin)?;
        let output = std::process::Command::new(driver)
            .args(&argv[1..])
            .env_clear()
            .env("PATH", system_path)
            .env("HOME", scratch.path())
            .env("TMPDIR", scratch.path())
            .env("LC_ALL", "C")
            .envs(case.env.iter().map(|(key, value)| (key, value.replace("{fixture}", &scratch.path().to_string_lossy()))))
            .current_dir(scratch.path())
            .stdin(std::fs::File::open(stdin)?)
            .output()
            .with_context(|| format!("running capability probe {argv:?}"))?;
        anyhow::ensure!(output.status.code().is_some(), "capability probe {argv:?} terminated by signal");
        if !output.status.success() {
            return Ok(Some(format!(
                "compiler rejected capability probe {argv:?}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    Ok(None)
}

fn run_case(case: &Case, profile: Profile, ctx: &ArmContext<'_>, run: &mut CaseRun) -> Result<()> {
    std::fs::create_dir_all(ctx.state)?;
    if let Some(reason) = unsupported_requirements(case, ctx.toolchain, ctx.state, ctx.system_path)? {
        run.verdict = Verdict::UnsupportedByCompiler;
        run.blocked = Some(reason);
        run.expected_rows = 0;
        return Ok(());
    }
    let fixture = {
        std::fs::create_dir_all(ctx.fixture)?;
        std::fs::canonicalize(ctx.fixture)?
    };
    let ctx = ArmContext {
        fixture: &fixture,
        ..*ctx
    };
    let mut arms = BTreeMap::new();
    for arm in Arm::for_profile(profile) {
        let result = exec::run_arm(case, profile, arm, &ctx).with_context(|| format!("{} arm", arm.as_str()))?;
        run.injected |= result.injected;
        arms.insert(arm, result);
    }
    for (s, step) in case.steps.iter().enumerate() {
        for (c, compile) in case.compiles.iter().enumerate() {
            let obs = |arm: Arm| &arms[&arm].observations[s][c];
            let effective = exec::effective_args(&compile.argv, step.profile_or(profile), &fixture);
            let passthrough = exec::effective_args(&compile.argv, Profile::Transparent, &fixture);
            let judgement = judge::judge(&Row {
                reuse: step.reuse_for(compile),
                expected_status: step.status,
                raw: obs(Arm::Raw),
                raw_repeat: obs(Arm::RawRepeat),
                raw_passthrough: arms.get(&Arm::RawPassthrough).map(|run| &run.observations[s][c]),
                ccache: obs(Arm::Ccache),
                ccache_preprocessor: obs(Arm::CcachePreprocessor),
                kache: obs(Arm::Kache),
                effective_argv: &effective,
                passthrough_argv: &passthrough,
                driver_args: compile.driver_args,
            });
            run.rows.push(RowResult {
                step: step.name.clone(),
                compile: c,
                reuse: step.reuse_for(compile),
                judgement,
                arms: arms
                    .iter()
                    .map(|(&a, run)| (a, (run.observations[s][c].clone(), run.durations_ms[s][c])))
                    .collect(),
            });
        }
    }
    run.verdict = run
        .rows
        .iter()
        .map(|r| r.judgement.verdict)
        .filter(|v| v.fails_gate())
        .min()
        .unwrap_or(Verdict::Match);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(selection: Selection, only: &[&str]) -> Config {
        Config {
            kache: PathBuf::new(),
            ccache: PathBuf::new(),
            cases: PathBuf::new(),
            out: PathBuf::new(),
            toolchains: vec![],
            only: only.iter().map(|s| s.to_string()).collect(),
            profiles: vec![Profile::Transparent],
            selection,
            control: None,
        }
    }

    fn case(id: &str, cohort: &str, os: &str) -> Case {
        let text = format!(
            r#"summary = "s"
cohort = "{cohort}"
os = [{os}]
profiles = ["transparent"]
[[compile]]
argv = ["{{cc}}", "-c", "a.c"]
source = "a.c"
[[step]]
name = "populate"
reuse = "miss"
"#
        );
        case::parse(id, &text, PathBuf::new()).unwrap()
    }

    #[test]
    fn gate_selects_the_blocking_cohort_and_full_selects_everything() {
        let cases = vec![case("a", "blocking", ""), case("b", "open", ""), case("c", "blocking", "\"windows\"")];
        let ids = |v: Vec<&Case>| v.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(select(&cases, &config(Selection::Gate, &[]), "macos").unwrap()), ["a"]);
        assert_eq!(ids(select(&cases, &config(Selection::Full, &[]), "macos").unwrap()), ["a", "b"]);
        assert_eq!(ids(select(&cases, &config(Selection::Gate, &["b"]), "macos").unwrap()), ["b"]);
        assert!(select(&cases, &config(Selection::Gate, &["c"]), "macos").is_err());
        assert!(select(&cases, &config(Selection::Gate, &["zzz"]), "macos").is_err());
    }

    fn row(verdict: Verdict) -> RowResult {
        RowResult {
            step: "s".into(),
            compile: 0,
            reuse: case::Reuse::Any,
            judgement: Judgement {
                verdict,
                reasons: vec![],
                ccache: Verdict::Match,
                ccache_preprocessor: Verdict::Match,
            },
            arms: BTreeMap::new(),
        }
    }

    fn case_run(verdict: Verdict, rows: Vec<RowResult>, expected_rows: usize, injected: bool) -> CaseRun {
        CaseRun {
            case: "c".into(),
            cohort: Cohort::Blocking,
            profile: Profile::Transparent,
            toolchain: "cc".into(),
            verdict,
            blocked: None,
            rows,
            expected_rows,
            injected,
        }
    }

    fn outcome(runs: Vec<CaseRun>) -> Outcome {
        Outcome {
            runs,
            report_dir: PathBuf::new(),
        }
    }

    #[test]
    fn the_gate_fails_on_no_runs_dropped_rows_or_a_failing_verdict() {
        assert!(!outcome(vec![]).gate_passed());
        assert!(!outcome(vec![case_run(Verdict::UnsupportedByCompiler, vec![], 0, false)]).gate_passed());
        assert!(
            outcome(vec![
                case_run(Verdict::UnsupportedByCompiler, vec![], 0, false),
                case_run(Verdict::Match, vec![row(Verdict::Match)], 1, false),
            ])
            .gate_passed()
        );
        assert!(outcome(vec![case_run(Verdict::Match, vec![row(Verdict::Match)], 1, false)]).gate_passed());
        assert!(!outcome(vec![case_run(Verdict::Match, vec![], 1, false)]).gate_passed());
        assert!(!outcome(vec![case_run(Verdict::Blocked, vec![], 0, false)]).gate_passed());
        assert!(
            outcome(vec![case_run(Verdict::CcacheWrongOutput, vec![row(Verdict::Match)], 1, false)]).gate_passed()
        );
    }

    #[test]
    fn a_control_must_be_injected_and_detected_in_every_injected_run() {
        let detected = || case_run(Verdict::KacheMissedReuse, vec![row(Verdict::KacheMissedReuse)], 1, true);
        let missed = || case_run(Verdict::Match, vec![row(Verdict::Match)], 1, true);
        let untouched = || case_run(Verdict::Match, vec![row(Verdict::Match)], 1, false);
        assert!(outcome(vec![detected(), untouched()]).control_detected(Control::CacheDisabled));
        assert!(!outcome(vec![detected(), missed()]).control_detected(Control::CacheDisabled));
        assert!(!outcome(vec![untouched()]).control_detected(Control::CacheDisabled));
        assert!(!outcome(vec![detected()]).control_detected(Control::StaleOutput));
    }

    #[test]
    fn family_specific_cases_skip_other_families_with_a_reason() {
        let mut clang_only = case("a", "blocking", "");
        clang_only.families = vec![case::Family::Clang];
        let tc = |family| Toolchain {
            name: "gcc".into(),
            family,
            cc_name: "gcc".into(),
            cxx_name: "g++".into(),
            cc: PathBuf::new(),
            cxx: PathBuf::new(),
        };
        assert_eq!(
            unsupported_family(&clang_only, &tc(Some(case::Family::Gcc))).as_deref(),
            Some("case exercises clang; gcc is gcc")
        );
        assert_eq!(unsupported_family(&clang_only, &tc(Some(case::Family::Clang))), None);
        assert_eq!(unsupported_family(&clang_only, &tc(None)), None);
        assert_eq!(unsupported_family(&case("b", "blocking", ""), &tc(Some(case::Family::Gcc))), None);
    }

    #[test]
    fn each_control_expects_its_own_verdict_class() {
        assert_eq!(expected_verdict(Control::StaleOutput), Verdict::KacheWrongOutput);
        assert_eq!(expected_verdict(Control::OmitSidecar), Verdict::KacheWrongOutput);
        assert_eq!(expected_verdict(Control::DisabledInvalidation), Verdict::KacheWrongOutput);
        assert_eq!(expected_verdict(Control::CacheDisabled), Verdict::KacheMissedReuse);
        assert_eq!(expected_verdict(Control::ReferenceFailure), Verdict::HarnessError);
    }

    #[test]
    #[cfg(unix)]
    fn capability_probes_receive_source_and_case_environment() {
        let root = tempfile::tempdir().unwrap();
        let driver = root.path().join("cc");
        kache_fs::testutil::write_executable(&driver,
            "#!/bin/sh\nread -r source\n[ \"$source\" = epoch-source ] && [ \"$SOURCE_DATE_EPOCH\" = 1700000000 ]\n");
        let tc = Toolchain {
            name: "cc".into(), family: None, cc_name: "cc".into(), cxx_name: "c++".into(),
            cc: driver.clone(), cxx: driver,
        };
        let mut c = case("a", "blocking", "");
        c.requires = vec![vec!["{cc}".into(), "-".into()]];
        c.env.insert("SOURCE_DATE_EPOCH".into(), "1700000000".into());
        let path = std::ffi::OsStr::new("/usr/bin:/bin");
        assert!(unsupported_requirements(&c, &tc, root.path(), path).unwrap().is_some());
        c.requires_stdin = "epoch-source\n".into();
        assert!(unsupported_requirements(&c, &tc, root.path(), path).unwrap().is_none());
        c.env.insert("SOURCE_DATE_EPOCH".into(), "0".into());
        assert!(unsupported_requirements(&c, &tc, root.path(), path).unwrap().is_some());
        c.requires_stdin = "wrong-source\n".into();
        c.env.insert("SOURCE_DATE_EPOCH".into(), "1700000000".into());
        assert!(unsupported_requirements(&c, &tc, root.path(), path).unwrap().is_some());
    }

    #[test]
    #[cfg(unix)]
    fn capability_probes_select_the_driver_and_only_skip_rejected_requirements() {
        let root = tempfile::tempdir().unwrap();
        let path = std::ffi::OsStr::new("/usr/bin:/bin");
        let mut c = case("a", "blocking", "");
        let cc = root.path().join("cc");
        let cxx = root.path().join("cxx");
        kache_fs::testutil::write_executable(&cc, "#!/bin/sh\nexit 0\n");
        kache_fs::testutil::write_executable(&cxx, "#!/bin/sh\nexit 1\n");
        let mut tc = Toolchain {
            name: "cc".into(), family: None, cc_name: "cc".into(), cxx_name: "c++".into(),
            cc, cxx,
        };
        assert!(unsupported_requirements(&c, &tc, root.path(), path).unwrap().is_none());
        c.requires = vec![vec!["{cc}".into(), "-a-feature".into()]];
        assert!(unsupported_requirements(&c, &tc, root.path(), path).unwrap().is_none());
        c.requires[0][0] = "{cxx}".into();
        assert!(unsupported_requirements(&c, &tc, root.path(), path).unwrap().unwrap().contains("compiler rejected capability probe"));
        tc.cxx = root.path().join("missing-compiler");
        assert!(unsupported_requirements(&c, &tc, root.path(), path).is_err());
        tc.cxx = "/bin/sh".into();
        c.requires[0] = vec!["{cxx}".into(), "-c".into(), "kill -TERM $$".into()];
        assert!(unsupported_requirements(&c, &tc, root.path(), path).is_err());
    }
}
