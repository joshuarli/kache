//! Verdicts from per-arm observations. Pure: no process or filesystem access.

use deser::Serialize;
use std::collections::BTreeMap;

use super::case::Reuse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[deser(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Match,
    KacheWrongOutput,
    KacheMissedReuse,
    CcacheWrongOutput,
    CompilerNondeterminism,
    HarnessError,
    UnsupportedByCompiler,
    Blocked,
}

impl Verdict {
    /// Whether this verdict fails a gate. A ccache disagreement with raw is
    /// recorded as a reference defect; it says nothing about kache.
    pub fn fails_gate(self) -> bool {
        !matches!(self, Verdict::Match | Verdict::CcacheWrongOutput | Verdict::UnsupportedByCompiler)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Match => "MATCH",
            Verdict::KacheWrongOutput => "KACHE_WRONG_OUTPUT",
            Verdict::KacheMissedReuse => "KACHE_MISSED_REUSE",
            Verdict::CcacheWrongOutput => "CCACHE_WRONG_OUTPUT",
            Verdict::CompilerNondeterminism => "COMPILER_NONDETERMINISM",
            Verdict::HarnessError => "HARNESS_ERROR",
            Verdict::UnsupportedByCompiler => "UNSUPPORTED_BY_COMPILER",
            Verdict::Blocked => "BLOCKED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[deser(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub kind: EntryKind,
    pub mode: u32,
    /// blake3 of the file bytes, or the symlink target.
    pub digest: String,
    pub size: u64,
    /// For one half of a GCC coverage pair whose stamp was located: the
    /// digest with that stamp zeroed, which is what two raw compiles share.
    pub stamp_free: Option<String>,
    /// For one half of a coverage pair whose stamp could not be located
    /// safely: why. Its bytes are compared as they are.
    pub stamp_unresolved: Option<String>,
}

/// Every path under the fixture root after an invocation.
pub type Inventory = BTreeMap<String, Entry>;

/// Compiler executions an arm caused for one invocation, read from the
/// observation shim's log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Executions {
    /// Code generation for the unit: an execution naming the source that is
    /// not preprocessing, dependency listing, syntax checking or a query
    /// (`-c`, `-S`, a header's PCH).
    pub codegen: u32,
    /// Preprocessing the unit (`-E` naming the source).
    pub preprocess: u32,
    /// Everything else: version queries, driver dumps, capability probes.
    pub probe: u32,
    /// Arguments after the driver for each code-generation execution.
    pub codegen_argv: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct Observation {
    /// `None` when the process died from a signal.
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub tree: Inventory,
    pub executions: Executions,
    /// Kache's own per-invocation result from its event log; kache arm only.
    pub kache_result: Option<String>,
    /// The arguments each `@file` in the command held when it ran, split on
    /// whitespace (the oracle's own response files never quote).
    pub response_files: BTreeMap<String, Vec<String>>,
}

/// One invocation in every arm, ready to judge.
pub struct Row<'a> {
    pub reuse: Reuse,
    pub expected_status: i32,
    pub raw: &'a Observation,
    pub raw_repeat: &'a Observation,
    /// The case's own command, run unchanged; reproducible profile only.
    pub raw_passthrough: Option<&'a Observation>,
    pub ccache: &'a Observation,
    pub ccache_preprocessor: &'a Observation,
    pub kache: &'a Observation,
    /// Arguments after the driver that the raw arm ran. Kache's code
    /// generation must run exactly these, apart from its dependency capture.
    pub effective_argv: &'a [String],
    /// The case's own arguments. Kache may also run these unchanged when it
    /// passes the compile through.
    pub passthrough_argv: &'a [String],
    /// Leading arguments a fixture driver script adds before the case's own.
    pub driver_args: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Judgement {
    pub verdict: Verdict,
    pub reasons: Vec<String>,
    /// `MATCH` or `CCACHE_WRONG_OUTPUT` per ccache configuration.
    pub ccache: Verdict,
    pub ccache_preprocessor: Verdict,
}

pub fn judge(row: &Row<'_>) -> Judgement {
    let ccache = reference_verdict(row.raw, row.ccache);
    let ccache_preprocessor = reference_verdict(row.raw, row.ccache_preprocessor);
    let (verdict, reasons) = kache_verdict(row, ccache);
    Judgement {
        verdict,
        reasons,
        ccache,
        ccache_preprocessor,
    }
}

fn reference_verdict(raw: &Observation, arm: &Observation) -> Verdict {
    if differences(raw, arm).is_empty() {
        Verdict::Match
    } else {
        Verdict::CcacheWrongOutput
    }
}

fn kache_verdict(row: &Row<'_>, ccache: Verdict) -> (Verdict, Vec<String>) {
    if row.raw.status != Some(row.expected_status) {
        return (
            Verdict::HarnessError,
            vec![format!(
                "reference raw compile exited {}, the case declares {}",
                describe_status(row.raw.status),
                row.expected_status
            )],
        );
    }
    let unstable = differences(row.raw, row.raw_repeat);
    if !unstable.is_empty() {
        return (
            Verdict::CompilerNondeterminism,
            unstable.into_iter().map(|d| format!("raw vs raw: {d}")).collect(),
        );
    }
    // A passthrough runs the case's own command, not the profile's, so
    // the raw run of that command is what it must match.
    let ran_passthrough = !row.kache.executions.codegen_argv.is_empty()
        && row.kache.executions.codegen_argv.iter().all(|argv| argv.as_slice() == row.passthrough_argv);
    let reference = match row.raw_passthrough {
        Some(raw) if ran_passthrough => raw,
        _ => row.raw,
    };
    let wrong = differences(reference, row.kache);
    if !wrong.is_empty() {
        return (Verdict::KacheWrongOutput, wrong);
    }
    let codegen = row.kache.executions.codegen;
    match row.reuse {
        Reuse::Miss if codegen == 0 => {
            return (
                Verdict::KacheWrongOutput,
                vec!["stale hit: the inputs changed but kache generated no code".to_string()],
            );
        }
        Reuse::Hit if codegen > 0 => {
            return (
                Verdict::KacheMissedReuse,
                vec![format!(
                    "kache ran code generation {codegen} time(s) where a restore was required (kache result: {})",
                    row.kache.kache_result.as_deref().unwrap_or("none")
                )],
            );
        }
        Reuse::Hit if !row.kache.kache_result.as_deref().is_some_and(is_hit_result) => {
            return (
                Verdict::HarnessError,
                vec![format!(
                    "no code generation observed but kache recorded `{}`, not a hit",
                    row.kache.kache_result.as_deref().unwrap_or("no event")
                )],
            );
        }
        Reuse::LikeCcache if ccache == Verdict::Match && row.ccache.executions.codegen == 0 && codegen > 0 => {
            let how = if row.ccache.executions.preprocess > 0 { "preprocessor" } else { "direct" };
            return (
                Verdict::KacheMissedReuse,
                vec![format!("ccache reused the result ({how} mode) but kache generated code")],
            );
        }
        _ => {}
    }
    let expand = |args: &[String]| expand_response_files(args, &row.kache.response_files);
    let (effective_expanded, passthrough_expanded) = (expand(row.effective_argv), expand(row.passthrough_argv));
    for argv in &row.kache.executions.codegen_argv {
        let argv = &argv[row.driver_args.min(argv.len())..].to_vec();
        let effective = without_dependency_capture(argv);
        let keyed = effective == row.effective_argv || effective == effective_expanded;
        let passthrough = argv.as_slice() == row.passthrough_argv || *argv == passthrough_expanded;
        if !keyed && !passthrough {
            return (
                Verdict::HarnessError,
                vec![format!(
                    "kache's code generation ran {:?}, the profile's effective command is {:?}",
                    argv, row.effective_argv
                )],
            );
        }
    }
    (Verdict::Match, Vec::new())
}

/// `args` with each `@file` replaced by the arguments it held.
pub fn expand_response_files(args: &[String], files: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut expanded = args.to_vec();
    for _ in 0..32 {
        let next: Vec<_> = expanded.iter()
            .flat_map(|arg| files.get(arg).cloned().unwrap_or_else(|| vec![arg.clone()]))
            .take(16_385)
            .collect();
        if next.len() > 16_384 {
            return args.to_vec();
        }
        if next == expanded {
            return next;
        }
        expanded = next;
    }
    args.to_vec()
}

fn is_hit_result(result: &str) -> bool {
    result.ends_with("hit")
}

/// Drop the `-MD -MF <tmp> -MT __kache_direct_capture` triple kache adds to
/// learn the read set; it does not change the compiler's other outputs.
pub fn without_dependency_capture(argv: &[String]) -> Vec<String> {
    let Some(at) = argv.windows(2).position(|w| w[0] == "-MT" && w[1] == "__kache_direct_capture") else {
        return argv.to_vec();
    };
    let start = at.checked_sub(3).filter(|&s| argv[s] == "-MD" && argv[s + 1] == "-MF");
    match start {
        Some(start) => argv[..start].iter().chain(&argv[at + 2..]).cloned().collect(),
        None => argv.to_vec(),
    }
}

fn describe_status(status: Option<i32>) -> String {
    status.map_or_else(|| "from a signal".to_string(), |s| s.to_string())
}

/// Human-readable differences between a reference and another arm,
/// in a fixed order: status, streams, then the tree path by path.
pub fn differences(reference: &Observation, other: &Observation) -> Vec<String> {
    let mut out = Vec::new();
    if reference.status != other.status {
        out.push(format!(
            "exit status {} vs {}",
            describe_status(reference.status),
            describe_status(other.status)
        ));
    }
    for (name, a, b) in [
        ("stdout", &reference.stdout, &other.stdout),
        ("stderr", &reference.stderr, &other.stderr),
    ] {
        if a != b {
            out.push(format!(
                "{name} differs: {:?} vs {:?}",
                excerpt(a),
                excerpt(b)
            ));
        }
    }
    for (path, entry) in &reference.tree {
        match other.tree.get(path) {
            None => out.push(format!("missing {path}")),
            Some(got) if got.kind != entry.kind => out.push(format!("{path}: {:?} vs {:?}", entry.kind, got.kind)),
            Some(got)
                if got.digest != entry.digest
                    && (entry.stamp_free.is_none() || got.stamp_free != entry.stamp_free) =>
            {
                match entry.stamp_unresolved.as_ref().or(got.stamp_unresolved.as_ref()) {
                    Some(why) => out.push(format!("{path}: content differs (coverage stamp unresolved: {why})")),
                    None => out.push(format!("{path}: content differs")),
                }
            }
            Some(got) if got.mode != entry.mode => {
                out.push(format!("{path}: mode {:o} vs {:o}", entry.mode, got.mode))
            }
            Some(_) => {}
        }
    }
    for path in other.tree.keys() {
        if !reference.tree.contains_key(path) {
            out.push(format!("unexpected {path}"));
        }
    }
    out
}

fn excerpt(bytes: &[u8]) -> String {
    const LIMIT: usize = 160;
    let text = String::from_utf8_lossy(bytes);
    if text.chars().count() <= LIMIT {
        text.into_owned()
    } else {
        format!("{}…", text.chars().take(LIMIT).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(digest: &str) -> Entry {
        Entry {
            kind: EntryKind::File,
            mode: 0o644,
            digest: digest.to_string(),
            size: 1,
            stamp_free: None,
            stamp_unresolved: None,
        }
    }

    fn obs(codegen: u32) -> Observation {
        let mut tree = Inventory::new();
        tree.insert("a.c".into(), file("src"));
        tree.insert("a.o".into(), file("obj"));
        Observation {
            status: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
            tree,
            executions: Executions {
                codegen,
                codegen_argv: (0..codegen).map(|_| argv()).collect(),
                ..Executions::default()
            },
            kache_result: Some(if codegen == 0 { "local_hit" } else { "miss" }.to_string()),
            response_files: BTreeMap::new(),
        }
    }

    fn argv() -> Vec<String> {
        ["-c", "a.c", "-o", "a.o"].map(String::from).to_vec()
    }

    fn run(reuse: Reuse, raw: &Observation, ccache: &Observation, kache: &Observation) -> Judgement {
        let effective = argv();
        judge(&Row {
            reuse,
            expected_status: 0,
            raw,
            raw_repeat: raw,
            raw_passthrough: None,
            ccache,
            ccache_preprocessor: ccache,
            kache,
            effective_argv: &effective,
            passthrough_argv: &effective,
            driver_args: 0,
        })
    }

    #[test]
    fn identical_outputs_with_the_required_reuse_match() {
        let raw = obs(1);
        assert_eq!(run(Reuse::Miss, &raw, &obs(1), &obs(1)).verdict, Verdict::Match);
        assert_eq!(run(Reuse::Hit, &raw, &obs(0), &obs(0)).verdict, Verdict::Match);
        assert_eq!(run(Reuse::Any, &raw, &obs(0), &obs(1)).verdict, Verdict::Match);
    }

    #[test]
    fn a_failed_reference_is_a_harness_error_even_when_kache_agrees() {
        let mut raw = obs(1);
        raw.status = Some(1);
        let mut kache = obs(1);
        kache.status = Some(1);
        let judgement = run(Reuse::Miss, &raw, &raw, &kache);
        assert_eq!(judgement.verdict, Verdict::HarnessError);
        assert!(judgement.reasons[0].contains("exited 1"));
    }

    #[test]
    fn a_signalled_reference_is_a_harness_error() {
        let mut raw = obs(1);
        raw.status = None;
        assert_eq!(run(Reuse::Miss, &raw, &raw, &raw).verdict, Verdict::HarnessError);
    }

    #[test]
    fn unstable_raw_output_is_nondeterminism() {
        let raw = obs(1);
        let mut repeat = obs(1);
        repeat.tree.get_mut("a.o").unwrap().digest = "other".into();
        let effective = argv();
        let judgement = judge(&Row {
            reuse: Reuse::Miss,
            expected_status: 0,
            raw: &raw,
            raw_repeat: &repeat,
            raw_passthrough: None,
            ccache: &raw,
            ccache_preprocessor: &raw,
            kache: &raw,
            effective_argv: &effective,
            passthrough_argv: &effective,
            driver_args: 0,
        });
        assert_eq!(judgement.verdict, Verdict::CompilerNondeterminism);
    }

    #[test]
    fn wrong_bytes_missing_and_unexpected_files_are_wrong_output() {
        let raw = obs(1);
        let mut stale = obs(1);
        stale.tree.get_mut("a.o").unwrap().digest = "stale".into();
        let j = run(Reuse::Miss, &raw, &raw, &stale);
        assert_eq!(j.verdict, Verdict::KacheWrongOutput);
        assert_eq!(j.reasons, vec!["a.o: content differs"]);

        let mut missing = obs(1);
        missing.tree.remove("a.o");
        assert_eq!(run(Reuse::Miss, &raw, &raw, &missing).reasons, vec!["missing a.o"]);

        let mut extra = obs(1);
        extra.tree.insert("a.d".into(), file("dep"));
        assert_eq!(run(Reuse::Miss, &raw, &raw, &extra).reasons, vec!["unexpected a.d"]);
    }

    #[test]
    fn status_stream_mode_and_kind_differences_are_reported() {
        let raw = obs(1);
        let mut other = obs(1);
        other.status = Some(1);
        other.stderr = b"warning".to_vec();
        other.tree.get_mut("a.c").unwrap().mode = 0o755;
        other.tree.get_mut("a.o").unwrap().kind = EntryKind::Symlink;
        assert_eq!(
            differences(&raw, &other),
            vec![
                "exit status 0 vs 1".to_string(),
                "stderr differs: \"\" vs \"warning\"".to_string(),
                "a.c: mode 644 vs 755".to_string(),
                "a.o: File vs Symlink".to_string(),
            ]
        );
    }

    #[test]
    fn a_required_miss_without_codegen_is_a_stale_hit() {
        let raw = obs(1);
        let j = run(Reuse::Miss, &raw, &raw, &obs(0));
        assert_eq!(j.verdict, Verdict::KacheWrongOutput);
        assert!(j.reasons[0].starts_with("stale hit"));
    }

    #[test]
    fn a_required_hit_with_codegen_is_missed_reuse() {
        let raw = obs(1);
        assert_eq!(run(Reuse::Hit, &raw, &raw, &obs(1)).verdict, Verdict::KacheMissedReuse);
    }

    #[test]
    fn a_hit_needs_kaches_own_event_to_agree() {
        let raw = obs(1);
        let mut silent = obs(0);
        silent.kache_result = None;
        assert_eq!(run(Reuse::Hit, &raw, &raw, &silent).verdict, Verdict::HarnessError);
        silent.kache_result = Some("passthrough".into());
        assert_eq!(run(Reuse::Hit, &raw, &raw, &silent).verdict, Verdict::HarnessError);
    }

    #[test]
    fn like_ccache_requires_reuse_only_where_ccache_reused_correctly() {
        let raw = obs(1);
        let mut preprocessed = obs(0);
        preprocessed.executions.preprocess = 1;
        let j = run(Reuse::LikeCcache, &raw, &preprocessed, &obs(1));
        assert_eq!(j.verdict, Verdict::KacheMissedReuse);
        assert!(j.reasons[0].contains("preprocessor mode"));
        assert!(run(Reuse::LikeCcache, &raw, &obs(0), &obs(1)).reasons[0].contains("direct mode"));

        assert_eq!(run(Reuse::LikeCcache, &raw, &obs(1), &obs(1)).verdict, Verdict::Match);

        let mut wrong = obs(0);
        wrong.tree.get_mut("a.o").unwrap().digest = "bad".into();
        let j = run(Reuse::LikeCcache, &raw, &wrong, &obs(1));
        assert_eq!(j.verdict, Verdict::Match);
        assert_eq!(j.ccache, Verdict::CcacheWrongOutput);
        assert!(!j.ccache.fails_gate());
    }

    #[test]
    fn kache_codegen_must_run_the_profiles_effective_command() {
        let raw = obs(1);
        let mut kache = obs(1);
        kache.executions.codegen_argv[0].push("-ffile-prefix-map=/x=/y".into());
        let j = run(Reuse::Miss, &raw, &raw, &kache);
        assert_eq!(j.verdict, Verdict::HarnessError);

        let mut captured = obs(1);
        captured.executions.codegen_argv[0]
            .extend(["-MD", "-MF", "/t/inputs.d", "-MT", "__kache_direct_capture"].map(String::from));
        assert_eq!(run(Reuse::Miss, &raw, &raw, &captured).verdict, Verdict::Match);
    }

    #[test]
    fn an_unchanged_passthrough_satisfies_the_reproducible_profile() {
        let raw = obs(1);
        let reproducible: Vec<String> = argv().into_iter().chain(["-ffile-prefix-map=/f=/kache/cc-root".to_string()]).collect();
        let case_args = argv();
        let row = |kache: &Observation| {
            judge(&Row {
                reuse: Reuse::Miss,
                expected_status: 0,
                raw: &raw,
                raw_repeat: &raw,
                raw_passthrough: None,
                ccache: &raw,
                ccache_preprocessor: &raw,
                kache,
                effective_argv: &reproducible,
                passthrough_argv: &case_args,
                driver_args: 0,
            })
            .verdict
        };
        assert_eq!(row(&obs(1)), Verdict::Match);
        let mut captured_passthrough = obs(1);
        captured_passthrough.executions.codegen_argv[0]
            .extend(["-MD", "-MF", "/t/inputs.d", "-MT", "__kache_direct_capture"].map(String::from));
        assert_eq!(row(&captured_passthrough), Verdict::HarnessError);
    }

    #[test]
    fn a_passthrough_is_judged_against_the_unchanged_command() {
        let raw = obs(1);
        let mut unchanged = obs(1);
        unchanged.tree.get_mut("a.o").unwrap().digest = "no-prefix-map".into();
        let reproducible: Vec<String> = argv().into_iter().chain(["-ffile-prefix-map=/f=/kache/cc-root".to_string()]).collect();
        let case_args = argv();
        let verdict = |kache: &Observation| {
            judge(&Row {
                reuse: Reuse::Miss,
                expected_status: 0,
                raw: &raw,
                raw_repeat: &raw,
                raw_passthrough: Some(&unchanged),
                ccache: &raw,
                ccache_preprocessor: &raw,
                kache,
                effective_argv: &reproducible,
                passthrough_argv: &case_args,
                driver_args: 0,
            })
            .verdict
        };
        assert_eq!(verdict(&unchanged), Verdict::Match, "a passthrough matching the unchanged command is correct");
        assert_eq!(verdict(&raw), Verdict::KacheWrongOutput, "a passthrough that matches only the profile's command is not");
        let mut keyed = raw.clone();
        keyed.executions.codegen_argv = vec![reproducible.clone()];
        assert_eq!(verdict(&keyed), Verdict::Match, "a keyed compile is judged against the profile's command");
    }

    #[test]
    fn kache_may_run_a_response_file_expanded() {
        let raw = obs(1);
        let with_file: Vec<String> = ["@args.rsp"].map(String::from).to_vec();
        let mut kache = obs(1);
        kache.response_files.insert("@args.rsp".into(), argv());
        let verdict = |kache: &Observation| {
            judge(&Row {
                reuse: Reuse::Miss,
                expected_status: 0,
                raw: &raw,
                raw_repeat: &raw,
                raw_passthrough: None,
                ccache: &raw,
                ccache_preprocessor: &raw,
                kache,
                effective_argv: &with_file,
                passthrough_argv: &with_file,
                driver_args: 0,
            })
            .verdict
        };
        assert_eq!(verdict(&kache), Verdict::Match);
        kache.response_files.insert("@args.rsp".into(), vec!["@inner.rsp".into()]);
        kache.response_files.insert("@inner.rsp".into(), argv());
        assert_eq!(verdict(&kache), Verdict::Match);
        kache.response_files.insert("@inner.rsp".into(), vec!["-c".into(), "other.c".into()]);
        assert_eq!(verdict(&kache), Verdict::HarnessError);
        kache.response_files.insert("@args.rsp".into(), vec!["-c".into(), "other.c".into()]);
        assert_eq!(verdict(&kache), Verdict::HarnessError);
        assert_eq!(
            expand_response_files(&["-O2".into(), "@x".into()], &BTreeMap::from([("@x".to_string(), vec!["-c".to_string(), "a.c".to_string()])])),
            ["-O2", "-c", "a.c"]
        );
        assert!(expand_response_files(&[], &BTreeMap::new()).is_empty());
        assert_eq!(expand_response_files(&["@x".into()], &BTreeMap::from([("@x".into(), vec!["@x".into()])])), ["@x"]);
    }

    #[test]
    fn dependency_capture_is_removed_only_as_the_exact_triple() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            without_dependency_capture(&v(&["-c", "a.c", "-MD", "-MF", "t.d", "-MT", "__kache_direct_capture", "-g"])),
            v(&["-c", "a.c", "-g"])
        );
        let user = v(&["-c", "a.c", "-MD", "-MF", "a.d", "-MT", "a.o"]);
        assert_eq!(without_dependency_capture(&user), user);
        let partial = v(&["-MF", "t.d", "-MT", "__kache_direct_capture"]);
        assert_eq!(without_dependency_capture(&partial), partial);
    }

    #[test]
    fn only_kache_failures_and_unresolved_rows_fail_the_gate() {
        for v in [
            Verdict::KacheWrongOutput,
            Verdict::KacheMissedReuse,
            Verdict::CompilerNondeterminism,
            Verdict::HarnessError,
            Verdict::Blocked,
        ] {
            assert!(v.fails_gate(), "{v:?}");
        }
        for v in [Verdict::Match, Verdict::CcacheWrongOutput, Verdict::UnsupportedByCompiler] {
            assert!(!v.fails_gate(), "{v:?}");
        }
    }

    #[test]
    fn long_streams_are_excerpted() {
        let long = "x".repeat(400);
        assert_eq!(excerpt(long.as_bytes()).chars().count(), 161);
        assert_eq!(excerpt(b"short"), "short");
    }
}
