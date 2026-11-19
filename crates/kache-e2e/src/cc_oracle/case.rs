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

/// Compiler family, for cases that exercise one family's options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[deser(rename_all = "kebab-case")]
pub enum Family {
    Gcc,
    Clang,
}

impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Family::Gcc => "gcc",
            Family::Clang => "clang",
        }
    }

    /// The family a driver's `--version` banner names.
    pub fn from_version(banner: &str) -> Option<Family> {
        if banner.contains("clang") {
            Some(Family::Clang)
        } else if banner.contains("Free Software Foundation") || banner.starts_with("gcc") {
            Some(Family::Gcc)
        } else {
            None
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
    /// Write a sibling temporary file and rename it over `path`, so the
    /// file gets a new inode as editors and code generators produce.
    Replace {
        path: String,
        content: String,
    },
    /// Create or retarget a symlink.
    Symlink {
        path: String,
        target: String,
    },
    /// Advance the modification time without changing content.
    Touch {
        path: String,
    },
    Remove {
        path: String,
    },
    /// Remove every path listed in the case's `outputs`.
    DeleteOutputs {},
    /// Kache arm only: stop the daemon and wait for its queue to drain, so
    /// the next compile must find entries without the process that wrote them.
    RestartDaemon {},
    /// Kache arm only: damage every stored entry the way a crash, a full
    /// disk or bit rot would. The daemon is stopped first so the fault is
    /// what the next compile reads.
    KacheStoreFault {
        fault: StoreFault,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[deser(rename_all = "kebab-case")]
pub enum StoreFault {
    /// Cut every blob to half its length.
    TruncateBlobs,
    /// Flip one byte in the middle of every blob, keeping its length.
    CorruptBlobs,
    /// Remove every blob, leaving the entry metadata.
    DeleteBlobs,
    /// Replace every entry's metadata with bytes that do not parse.
    CorruptMeta,
}

impl Action {
    pub fn edits_files(&self) -> bool {
        !matches!(self, Action::RestartDaemon {} | Action::KacheStoreFault { .. })
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
    /// Other arguments that name the unit in an execution, such as the
    /// `@file` a response-file compile passes while a cache that expands
    /// the file passes `source` instead.
    #[deser(default)]
    pub also_names: Vec<String>,
    /// Kache passes this invocation through, so it must generate code in
    /// every step, whatever the step requires of the others: the module
    /// interface a consumer case builds first, for example.
    #[deser(default)]
    pub uncached: bool,
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

impl Step {
    pub fn reuse_for(&self, compile: &Compile) -> Reuse {
        if compile.uncached { Reuse::Miss } else { self.reuse }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[deser(deny_unknown_fields)]
struct CaseFile {
    summary: String,
    cohort: Cohort,
    #[deser(default)]
    os: Vec<String>,
    #[deser(default)]
    families: Vec<Family>,
    profiles: Vec<Profile>,
    #[deser(default)]
    outputs: Vec<String>,
    #[deser(default)]
    env: std::collections::BTreeMap<String, String>,
    #[deser(default)]
    ccache_config: std::collections::BTreeMap<String, String>,
    #[deser(default)]
    kache_env: std::collections::BTreeMap<String, String>,
    compile: Vec<Compile>,
    step: Vec<Step>,
}

#[derive(Debug, Clone)]
pub struct Case {
    pub id: String,
    pub summary: String,
    pub cohort: Cohort,
    pub os: Vec<String>,
    /// Empty means every family.
    pub families: Vec<Family>,
    pub profiles: Vec<Profile>,
    pub outputs: Vec<String>,
    /// Set in every arm; `{fixture}` expands to the fixture root.
    pub env: std::collections::BTreeMap<String, String>,
    /// ccache settings for a conditional profile, such as the sloppiness
    /// ccache needs before it caches PCH consumers. Both ccache arms get
    /// them; kache and raw never do.
    pub ccache_config: std::collections::BTreeMap<String, String>,
    /// Kache settings for a conditional profile, such as
    /// `KACHE_VERIFY_RESTORES`. The kache arm alone gets them; profile
    /// variables stay the harness's.
    pub kache_env: std::collections::BTreeMap<String, String>,
    pub compiles: Vec<Compile>,
    pub steps: Vec<Step>,
    pub source_dir: PathBuf,
}

impl Case {
    pub fn applies_to_os(&self, os: &str) -> bool {
        self.os.is_empty() || self.os.iter().any(|o| o == os)
    }
}

/// Variables each arm sets itself; a case may not override them.
const RESERVED_ENV: &[&str] = &["PATH", "HOME", "TMPDIR", "LC_ALL", "LANG", "CC_ORACLE_LOG"];

/// Kache variables the harness sets for the profile, its state or a control.
const PROFILE_KACHE_ENV: &[&str] = &[
    "KACHE_CC_PATH_NORMALIZE",
    "KACHE_CC_SOURCE_DATE_EPOCH",
    "KACHE_CACHE_DIR",
    "KACHE_CONFIG",
    "KACHE_DISABLED",
];

pub fn parse(id: &str, text: &str, source_dir: PathBuf) -> Result<Case> {
    let file: CaseFile = deser_toml::from_str(text).with_context(|| format!("parsing case {id}"))?;
    ensure!(!file.profiles.is_empty(), "case {id}: profiles is empty");
    ensure!(!file.compile.is_empty(), "case {id}: no [[compile]]");
    ensure!(!file.step.is_empty(), "case {id}: no [[step]]");
    ensure!(
        file.step[0].reuse == Reuse::Miss,
        "case {id}: the first step runs against an empty cache, so it must declare reuse = \"miss\""
    );
    for key in file.env.keys() {
        ensure!(
            !RESERVED_ENV.contains(&key.as_str()) && !key.starts_with("KACHE_") && !key.starts_with("CCACHE_"),
            "case {id}: env {key} is owned by the harness"
        );
    }
    for key in file.kache_env.keys() {
        ensure!(
            key.starts_with("KACHE_") && !PROFILE_KACHE_ENV.contains(&key.as_str()),
            "case {id}: kache_env {key} is not a kache setting the case may choose"
        );
    }
    for compile in &file.compile {
        ensure!(
            compile.argv.first().is_some_and(|a| a == "{cc}" || a == "{cxx}"),
            "case {id}: argv must start with {{cc}} or {{cxx}}"
        );
        ensure!(
            std::iter::once(&compile.source)
                .chain(&compile.also_names)
                .any(|name| compile.argv.contains(name)),
            "case {id}: neither source `{}` nor also_names appears in argv",
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
        families: file.families,
        profiles: file.profiles,
        outputs: file.outputs,
        env: file.env,
        ccache_config: file.ccache_config,
        kache_env: file.kache_env,
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
        assert_eq!(case.steps[1].reuse_for(&case.compiles[0]), Reuse::Hit);
    }

    #[test]
    fn an_uncached_compile_must_generate_code_in_every_step() {
        let text = MINIMAL.replace("source = \"a.c\"\n", "source = \"a.c\"\nuncached = true\n");
        let case = parse("c", &text, PathBuf::from("s")).unwrap();
        assert_eq!(case.steps[1].reuse_for(&case.compiles[0]), Reuse::Miss);
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
    fn families_come_from_the_case_and_the_version_banner() {
        let text = MINIMAL.replace("cohort = \"blocking\"", "cohort = \"blocking\"\nfamilies = [\"clang\"]");
        assert_eq!(parse("c", &text, PathBuf::from("s")).unwrap().families, [Family::Clang]);
        assert!(parse("c", MINIMAL, PathBuf::from("s")).unwrap().families.is_empty());
        assert_eq!(Family::from_version("Apple clang version 21.0.0 (clang-2100.3.34.2)"), Some(Family::Clang));
        assert_eq!(Family::from_version("Debian clang version 19.1.7 (3+b1)"), Some(Family::Clang));
        assert_eq!(Family::from_version("gcc (Debian 14.2.0-19) 14.2.0\nCopyright (C) 2024 Free Software Foundation, Inc."), Some(Family::Gcc));
        assert_eq!(Family::from_version("cc (Alpine 15.2.0) 15.2.0\nCopyright (C) 2025 Free Software Foundation, Inc."), Some(Family::Gcc));
        assert_eq!(Family::from_version("icx 2025"), None);
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
    fn case_env_may_not_override_harness_variables() {
        let with_env = |kv: &str| MINIMAL.replace("outputs = [\"a.o\"]\n", &format!("outputs = [\"a.o\"]\nenv = {{ {kv} }}\n"));
        let case = parse("c", &with_env("CPATH = \"{fixture}/inc\""), PathBuf::from("s")).unwrap();
        assert_eq!(case.env["CPATH"], "{fixture}/inc");
        for key in ["PATH", "HOME", "TMPDIR", "LC_ALL", "LANG", "CC_ORACLE_LOG", "KACHE_DISABLED", "CCACHE_DIR"] {
            assert!(parse("c", &with_env(&format!("{key} = \"x\"")), PathBuf::from("s")).is_err(), "{key}");
        }
    }

    #[test]
    fn ccache_config_is_parsed_for_conditional_profiles() {
        let text = MINIMAL.replace("outputs = [\"a.o\"]\n", "outputs = [\"a.o\"]\nccache_config = { sloppiness = \"pch_defines,time_macros\" }\n");
        let case = parse("c", &text, PathBuf::from("s")).unwrap();
        assert_eq!(case.ccache_config["sloppiness"], "pch_defines,time_macros");
        assert!(parse("c", MINIMAL, PathBuf::from("s")).unwrap().ccache_config.is_empty());
    }

    #[test]
    fn kache_env_accepts_only_case_level_kache_settings() {
        let with = |kv: &str| MINIMAL.replace("outputs = [\"a.o\"]\n", &format!("outputs = [\"a.o\"]\nkache_env = {{ {kv} }}\n"));
        let case = parse("c", &with("KACHE_VERIFY_RESTORES = \"always\""), PathBuf::from("s")).unwrap();
        assert_eq!(case.kache_env["KACHE_VERIFY_RESTORES"], "always");
        for key in ["KACHE_DISABLED", "KACHE_CACHE_DIR", "KACHE_CC_PATH_NORMALIZE", "PATH"] {
            assert!(parse("c", &with(&format!("{key} = \"1\"")), PathBuf::from("s")).is_err(), "{key}");
        }
    }

    #[test]
    fn parses_store_faults() {
        let text = MINIMAL.replace(
            "{ op = \"delete-outputs\" }, ",
            "{ op = \"kache-store-fault\", fault = \"corrupt-blobs\" }, ",
        );
        let case = parse("c", &text, PathBuf::from("s")).unwrap();
        assert!(matches!(
            case.steps[1].action[0],
            Action::KacheStoreFault { fault: StoreFault::CorruptBlobs }
        ));
        assert!(!case.steps[1].action[0].edits_files());
    }

    #[test]
    fn parses_file_identity_actions() {
        let text = MINIMAL.replace(
            "{ op = \"delete-outputs\" }, ",
            "{ op = \"replace\", path = \"a.h\", content = \"y\" }, { op = \"symlink\", path = \"l.h\", target = \"a.h\" }, { op = \"touch\", path = \"a.h\" }, ",
        );
        let case = parse("c", &text, PathBuf::from("s")).unwrap();
        assert!(matches!(&case.steps[1].action[0], Action::Replace { path, .. } if path == "a.h"));
        assert!(matches!(&case.steps[1].action[1], Action::Symlink { target, .. } if target == "a.h"));
        assert!(matches!(&case.steps[1].action[2], Action::Touch { path } if path == "a.h"));
        assert!(case.steps[1].action.iter().all(Action::edits_files));
    }

    #[test]
    fn delete_outputs_requires_declared_outputs() {
        let text = MINIMAL.replace("outputs = [\"a.o\"]\n", "");
        assert!(parse("c", &text, PathBuf::from("s")).is_err());
    }
}
