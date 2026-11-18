//! Differential case definitions: `campaigns/cpp/oracle/<id>/case.toml`.

use anyhow::{Context, Result, bail, ensure};
use deser::Deserialize;
use std::path::{Path, PathBuf};

/// Which gate a case belongs to. `blocking` cases run in `make cc-oracle`
/// and must pass; `open` cases document a known gap and run only in the
/// full inventory, where they keep `cc-oracle-full` failing until fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[deser(rename_all = "kebab-case")]
pub enum Cohort {
    Blocking,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[deser(rename_all = "kebab-case")]
pub enum Profile {
    /// Kache path maps and epoch disabled; every arm sees the case's flags.
    Transparent,
    /// Kache defaults; raw and ccache get the flags and environment kache
    /// documents adding, so all three compile the same effective command.
    Reproducible,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Transparent => "transparent",
            Profile::Reproducible => "reproducible",
        }
    }
}

/// What a step must show about reuse, judged per compile invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[deser(rename_all = "kebab-case")]
pub enum Reuse {
    /// Inputs changed (or the cache is empty): the compiler must generate code.
    Miss,
    /// Kache must restore without generating code.
    Hit,
    /// Kache must avoid code generation wherever ccache's normal
    /// configuration did; otherwise either outcome is acceptable.
    LikeCcache,
    /// No reuse requirement.
    Any,
}

#[derive(Debug, Clone, Deserialize)]
#[deser(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    /// Create or replace a file. `keep_mtime` restores the previous
    /// modification time so only the content distinguishes the edit.
    Write {
        path: String,
        content: String,
        #[deser(default)]
        keep_mtime: bool,
    },
    Remove {
        path: String,
    },
    /// Remove every path listed in the case's `outputs`.
    DeleteOutputs {},
    /// Kache arm only: stop the daemon and wait for its queue to drain, so
    /// the next compile must find entries without the process that wrote them.
    RestartDaemon {},
}

impl Action {
    pub fn edits_files(&self) -> bool {
        !matches!(self, Action::RestartDaemon {})
    }
}

#[derive(Debug, Clone, Deserialize)]
#[deser(deny_unknown_fields)]
pub struct Compile {
    /// `{cc}` and `{cxx}` expand to the selected toolchain's drivers.
    pub argv: Vec<String>,
    /// The translation unit's source path as spelled in `argv`. Compiler
    /// executions naming it are the unit's; every other execution is a probe.
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[deser(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    #[deser(default)]
    pub action: Vec<Action>,
    pub reuse: Reuse,
    /// Exit status the raw compiler must return. A different raw status
    /// means the reference arm failed, not that kache matched it.
    #[deser(default)]
    pub status: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[deser(deny_unknown_fields)]
struct CaseFile {
    summary: String,
    cohort: Cohort,
    #[deser(default)]
    os: Vec<String>,
    profiles: Vec<Profile>,
    #[deser(default)]
    outputs: Vec<String>,
    compile: Vec<Compile>,
    step: Vec<Step>,
}

#[derive(Debug, Clone)]
pub struct Case {
    pub id: String,
    pub summary: String,
    pub cohort: Cohort,
    pub os: Vec<String>,
    pub profiles: Vec<Profile>,
    pub outputs: Vec<String>,
    pub compiles: Vec<Compile>,
    pub steps: Vec<Step>,
    pub source_dir: PathBuf,
}

impl Case {
    pub fn applies_to_os(&self, os: &str) -> bool {
        self.os.is_empty() || self.os.iter().any(|o| o == os)
    }
}

pub fn parse(id: &str, text: &str, source_dir: PathBuf) -> Result<Case> {
    let file: CaseFile = deser_toml::from_str(text).with_context(|| format!("parsing case {id}"))?;
    ensure!(!file.profiles.is_empty(), "case {id}: profiles is empty");
    ensure!(!file.compile.is_empty(), "case {id}: no [[compile]]");
    ensure!(!file.step.is_empty(), "case {id}: no [[step]]");
    ensure!(
        file.step[0].reuse == Reuse::Miss,
        "case {id}: the first step runs against an empty cache, so it must declare reuse = \"miss\""
    );
    for compile in &file.compile {
        ensure!(
            compile.argv.first().is_some_and(|a| a == "{cc}" || a == "{cxx}"),
            "case {id}: argv must start with {{cc}} or {{cxx}}"
        );
        ensure!(
            compile.argv.iter().any(|a| a == &compile.source),
            "case {id}: source `{}` does not appear in argv",
            compile.source
        );
    }
    for step in &file.step {
        for action in &step.action {
            if matches!(action, Action::DeleteOutputs {}) {
                ensure!(!file.outputs.is_empty(), "case {id}: delete-outputs with no outputs declared");
            }
        }
    }
    Ok(Case {
        id: id.to_string(),
        summary: file.summary,
        cohort: file.cohort,
        os: file.os,
        profiles: file.profiles,
        outputs: file.outputs,
        compiles: file.compile,
        steps: file.step,
        source_dir,
    })
}

/// Load every case directory under `root`, sorted by id.
pub fn discover(root: &Path) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let dir = entry?.path();
        let toml = dir.join("case.toml");
        if !toml.is_file() {
            continue;
        }
        let id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .context("case directory name is not UTF-8")?
            .to_string();
        let text = std::fs::read_to_string(&toml).with_context(|| format!("reading {}", toml.display()))?;
        let source = dir.join("source");
        if !source.is_dir() {
            bail!("case {id}: missing source/ directory");
        }
        cases.push(parse(&id, &text, source)?);
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
summary = "s"
cohort = "blocking"
profiles = ["transparent", "reproducible"]
outputs = ["a.o"]

[[compile]]
argv = ["{cc}", "-c", "a.c", "-o", "a.o"]
source = "a.c"

[[step]]
name = "populate"
reuse = "miss"

[[step]]
name = "restore"
reuse = "hit"
action = [{ op = "delete-outputs" }, { op = "write", path = "a.h", content = "x", keep_mtime = true }]
"#;

    #[test]
    fn parses_actions_and_defaults() {
        let case = parse("c", MINIMAL, PathBuf::from("s")).unwrap();
        assert_eq!(case.cohort, Cohort::Blocking);
        assert_eq!(case.steps[0].status, 0);
        assert_eq!(case.steps[1].reuse, Reuse::Hit);
        assert!(matches!(case.steps[1].action[0], Action::DeleteOutputs {}));
        assert!(case.steps[1].action.iter().all(Action::edits_files));
        assert!(!Action::RestartDaemon {}.edits_files());
        assert!(matches!(
            &case.steps[1].action[1],
            Action::Write { path, keep_mtime: true, .. } if path == "a.h"
        ));
        assert!(case.applies_to_os("linux"));
    }

    #[test]
    fn os_list_restricts_cases() {
        let text = MINIMAL.replace("cohort = \"blocking\"", "cohort = \"open\"\nos = [\"macos\"]");
        let case = parse("c", &text, PathBuf::from("s")).unwrap();
        assert_eq!(case.cohort, Cohort::Open);
        assert!(case.applies_to_os("macos"));
        assert!(!case.applies_to_os("linux"));
    }

    #[test]
    fn first_step_must_expect_a_miss() {
        let text = MINIMAL.replacen("reuse = \"miss\"", "reuse = \"hit\"", 1);
        let err = parse("c", &text, PathBuf::from("s")).unwrap_err();
        assert!(format!("{err:#}").contains("must declare reuse"));
    }

    #[test]
    fn source_must_appear_in_argv() {
        let text = MINIMAL.replace("source = \"a.c\"", "source = \"b.c\"");
        assert!(parse("c", &text, PathBuf::from("s")).is_err());
    }

    #[test]
    fn argv_must_name_a_toolchain_driver() {
        let text = MINIMAL.replace("\"{cc}\", \"-c\"", "\"gcc\", \"-c\"");
        assert!(parse("c", &text, PathBuf::from("s")).is_err());
    }

    #[test]
    fn delete_outputs_requires_declared_outputs() {
        let text = MINIMAL.replace("outputs = [\"a.o\"]\n", "");
        assert!(parse("c", &text, PathBuf::from("s")).is_err());
    }
}
