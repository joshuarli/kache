//! Shared scenario metadata and selection.
//!
//! Fixtures and clone-scale benchmark scenarios still have different source
//! materialization needs, but they now expose the same identity surface:
//! name, source kind, and tags. Selection is intentionally declarative and
//! source-kind aware, so CI can say "run the gate tier" without knowing which
//! directory layout backs each scenario.

use anyhow::{Context, Result};
use deser::Deserialize;
use std::path::{Path, PathBuf};

use rustc_hash::FxHashSet;

/// Where a scenario's source comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[deser(rename_all = "kebab-case")]
pub enum SourceKind {
    /// Checked-in fixture source directory.
    Fixture,
    /// External repository materialized as cloned worktrees.
    Clone,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Fixture => "fixture",
            SourceKind::Clone => "clone",
        }
    }

    pub fn source_tag(self) -> &'static str {
        match self {
            SourceKind::Fixture => "source:fixture",
            SourceKind::Clone => "source:clone",
        }
    }

    pub fn default_tier_tag(self) -> &'static str {
        match self {
            // Fast checked-in fixtures are the PR correctness gate by default.
            SourceKind::Fixture => "tier:gate",
            // Large cloned repositories are opt-in/nightly by default.
            SourceKind::Clone => "tier:nightly",
        }
    }
}

/// Selector parsed from `--select`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    raw: String,
}

impl Selector {
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        anyhow::ensure!(!raw.is_empty(), "--select cannot be empty");
        Ok(Self {
            raw: raw.to_string(),
        })
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    fn matches(&self, name: &str, tags: &FxHashSet<String>, source_kind: SourceKind) -> bool {
        let raw = self.raw.as_str();
        if tags.contains(raw) {
            return true;
        }
        if let Some(needle) = raw.strip_prefix("name:") {
            return name.contains(needle);
        }
        // Convenience aliases for the natural source-kind selector.
        raw == source_kind.as_str()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Selectors {
    items: Vec<Selector>,
}

impl Selectors {
    pub fn parse_many(raw: &[String]) -> Result<Self> {
        let items = raw
            .iter()
            .map(|s| Selector::parse(s).with_context(|| format!("parsing --select `{s}`")))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { items })
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn describe(&self) -> String {
        self.items
            .iter()
            .map(|s| s.raw())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn matches(&self, name: &str, source_kind: SourceKind, explicit_tags: &[String]) -> bool {
        let tags = effective_tags(source_kind, explicit_tags);
        self.items
            .iter()
            .all(|selector| selector.matches(name, &tags, source_kind))
    }

    pub fn matches_metadata(&self, metadata: &ScenarioMetadata) -> bool {
        self.matches(&metadata.name, metadata.source_kind, &metadata.tags)
    }

    /// Needles from any `name:<needle>` selectors. `name:` is a substring
    /// filter, so callers that must resolve to a single scenario (the bench
    /// runner) use these to prefer an exact-name match when the substring is
    /// ambiguous — e.g. `hk` also catches `bench-hk-pull-mbx`.
    pub fn name_needles(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter_map(|s| s.raw().strip_prefix("name:"))
            .collect()
    }
}

/// Tags used for matching. Source kind is always implicit, and the default
/// tier is implicit unless the TOML declares an explicit `tier:*` tag.
pub fn effective_tags(source_kind: SourceKind, explicit_tags: &[String]) -> FxHashSet<String> {
    let mut tags = FxHashSet::default();
    tags.insert(source_kind.source_tag().to_string());
    if !explicit_tags.iter().any(|tag| tag.starts_with("tier:")) {
        tags.insert(source_kind.default_tier_tag().to_string());
    }
    tags.extend(explicit_tags.iter().cloned());
    tags
}

#[derive(Debug, Clone)]
pub struct ScenarioMetadata {
    pub name: String,
    pub tags: Vec<String>,
    pub source_kind: SourceKind,
    pub dir: PathBuf,
    pub toml_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ScenarioMetadataConfig {
    name: String,
    #[deser(default)]
    tags: Vec<String>,
    source: Option<ScenarioSourceConfig>,
}

pub fn discover_metadata(root: &Path) -> Result<Vec<ScenarioMetadata>> {
    let mut scenarios = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let toml_path = dir.join("scenario.toml");
        if !toml_path.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&toml_path)
            .with_context(|| format!("reading {}", toml_path.display()))?;
        let cfg: ScenarioMetadataConfig =
            deser_toml::from_str(&raw).with_context(|| format!("parsing {}", toml_path.display()))?;
        scenarios.push(ScenarioMetadata {
            name: cfg.name,
            tags: cfg.tags,
            source_kind: cfg
                .source
                .map(|source| source.kind())
                .unwrap_or(SourceKind::Fixture),
            dir,
            toml_path,
        });
    }
    scenarios.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(scenarios)
}

/// Common scenario checks surface. The legacy fixture `[assertions]` table
/// remains the blocking correctness contract for fixture TOML; `checks.assert`
/// gives clone-style scenarios a shared blocking verdict surface, while
/// `checks.measure` is advisory only.
#[derive(Debug, Clone, Default, Deserialize)]
#[deser(deny_unknown_fields)]
pub struct ScenarioChecks {
    #[deser(default, rename = "assert")]
    pub assertions: PhaseAssertSpecs,
    #[deser(default)]
    pub measure: PhaseMeasureSpecs,
}

/// Blocking assertion specs by phase.
#[derive(Debug, Clone, Default, Deserialize)]
#[deser(default, deny_unknown_fields)]
pub struct PhaseAssertSpecs {
    pub cold: Option<ScenarioAssertSpec>,
    #[deser(rename = "warm-same-tree")]
    pub warm_same_tree: Option<ScenarioAssertSpec>,
    pub warm: Option<ScenarioAssertSpec>,
    pub pull: Option<ScenarioAssertSpec>,
    pub noop: Option<ScenarioAssertSpec>,
    pub relocate: Option<ScenarioAssertSpec>,
    #[deser(rename = "relocate-modified")]
    pub relocate_modified: Option<ScenarioAssertSpec>,
    #[deser(rename = "relocate-noop")]
    pub relocate_noop: Option<ScenarioAssertSpec>,
}

impl PhaseAssertSpecs {
    pub fn for_phase(&self, phase: &str) -> Option<&ScenarioAssertSpec> {
        match phase {
            "cold" => self.cold.as_ref(),
            "warm-same-tree" => self.warm_same_tree.as_ref(),
            "warm" => self.warm.as_ref(),
            "pull" => self.pull.as_ref(),
            "noop" => self.noop.as_ref(),
            "relocate" => self.relocate.as_ref(),
            "relocate-modified" => self.relocate_modified.as_ref(),
            "relocate-noop" => self.relocate_noop.as_ref(),
            _ => None,
        }
    }
}

/// Blocking thresholds currently used by clone-scale scenarios.
#[derive(Debug, Clone, Default, Deserialize)]
#[deser(deny_unknown_fields)]
pub struct ScenarioAssertSpec {
    pub min_key_stability_pct: Option<f64>,
    pub max_passthrough_pct: Option<f64>,
    pub max_errors: Option<u64>,
    /// Minimum cache hits the phase must report. Zero hits means the phase
    /// compiled everything from scratch, so its wall-clock says nothing about
    /// the cache — the flattering-number failure mode a timing gate has to
    /// rule out before it trusts a delta.
    pub min_hits: Option<u64>,
    /// Minimum bytes the phase must have restored out of the store
    /// (`storage.restored_bytes`). Hits alone can be satisfied by entries that
    /// carry no output files; this asserts real artifacts landed in the objdir.
    pub min_restored_bytes: Option<u64>,
}

/// Warn-only measurement specs by phase.
#[derive(Debug, Clone, Default, Deserialize)]
#[deser(default, deny_unknown_fields)]
pub struct PhaseMeasureSpecs {
    pub cold: Option<MeasureSpec>,
    #[deser(rename = "warm-same-tree")]
    pub warm_same_tree: Option<MeasureSpec>,
    pub warm: Option<MeasureSpec>,
    pub pull: Option<MeasureSpec>,
    pub noop: Option<MeasureSpec>,
    pub relocate: Option<MeasureSpec>,
    #[deser(rename = "relocate-modified")]
    pub relocate_modified: Option<MeasureSpec>,
    #[deser(rename = "relocate-noop")]
    pub relocate_noop: Option<MeasureSpec>,
}

impl PhaseMeasureSpecs {
    pub fn for_phase(&self, phase: &str) -> Option<&MeasureSpec> {
        match phase {
            "cold" => self.cold.as_ref(),
            "warm-same-tree" => self.warm_same_tree.as_ref(),
            "warm" => self.warm.as_ref(),
            "pull" => self.pull.as_ref(),
            "noop" => self.noop.as_ref(),
            "relocate" => self.relocate.as_ref(),
            "relocate-modified" => self.relocate_modified.as_ref(),
            "relocate-noop" => self.relocate_noop.as_ref(),
            _ => None,
        }
    }
}

/// Advisory thresholds. Violations are recorded as warnings in results JSON
/// and never change phase/scenario status.
#[derive(Debug, Clone, Default, Deserialize)]
#[deser(deny_unknown_fields)]
pub struct MeasureSpec {
    pub max_wall_s: Option<u64>,
    pub min_hit_rate_pct: Option<f64>,
    pub min_speedup: Option<f64>,
    /// Passthrough reasons this scenario already has, as prefixes of the
    /// exported label (`category|detail`, paths shown as `<path>`). When the
    /// list is not empty, a real compile passed through for any other reason
    /// raises a warning, so a newly unsupported flag or file shows up the first
    /// night it appears. Probes (`not-a-compile`) are never checked. Fixing a
    /// reason means deleting its entry.
    #[deser(default)]
    pub known_passthrough: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[deser(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ScenarioSourceConfig {
    Fixture {
        path: PathBuf,
    },
    Clone {
        repo: String,
        #[deser(rename = "ref")]
        git_ref: String,
        #[deser(default)]
        ref_next: Option<String>,
        objdir: String,
    },
}

impl ScenarioSourceConfig {
    pub fn kind(&self) -> SourceKind {
        match self {
            ScenarioSourceConfig::Fixture { .. } => SourceKind::Fixture,
            ScenarioSourceConfig::Clone { .. } => SourceKind::Clone,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    #[test]
    fn fixture_gets_gate_and_source_tags_by_default() {
        let tags = effective_tags(SourceKind::Fixture, &[]);
        assert!(tags.contains("source:fixture"));
        assert!(tags.contains("tier:gate"));
    }

    #[test]
    fn clone_gets_nightly_and_source_tags_by_default() {
        let tags = effective_tags(SourceKind::Clone, &[]);
        assert!(tags.contains("source:clone"));
        assert!(tags.contains("tier:nightly"));
    }

    #[test]
    fn explicit_tier_replaces_default_tier() {
        let fixture_tags = effective_tags(SourceKind::Fixture, &["tier:nightly".to_string()]);
        assert!(fixture_tags.contains("source:fixture"));
        assert!(fixture_tags.contains("tier:nightly"));
        assert!(!fixture_tags.contains("tier:gate"));

        let clone_tags = effective_tags(SourceKind::Clone, &["tier:gate".to_string()]);
        assert!(clone_tags.contains("source:clone"));
        assert!(clone_tags.contains("tier:gate"));
        assert!(!clone_tags.contains("tier:nightly"));
    }

    #[test]
    fn selectors_match_all_requested_tags() {
        let selectors =
            Selectors::parse_many(&["tier:gate".to_string(), "lang:rust".to_string()]).unwrap();
        assert!(selectors.matches(
            "rust-check",
            SourceKind::Fixture,
            &["lang:rust".to_string()]
        ));
        assert!(!selectors.matches("c-hello", SourceKind::Fixture, &["lang:cc".to_string()]));
    }

    #[test]
    fn selectors_support_name_filter_and_source_alias() {
        let selectors =
            Selectors::parse_many(&["fixture".to_string(), "name:hello".to_string()]).unwrap();
        assert!(selectors.matches("c-hello", SourceKind::Fixture, &[]));
        assert!(!selectors.matches("firefox", SourceKind::Clone, &[]));
    }

    #[test]
    fn discovers_scenario_metadata_from_shared_root() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
        let scenarios = discover_metadata(&root).unwrap();

        let firefox = scenarios
            .iter()
            .find(|scenario| scenario.name == "bench-firefox")
            .unwrap();
        assert_eq!(firefox.source_kind, SourceKind::Clone);
        assert!(firefox.tags.contains(&"suite:bench".to_string()));

        let opendal = scenarios
            .iter()
            .find(|scenario| scenario.name == "bench-opendal")
            .unwrap();
        assert_eq!(opendal.source_kind, SourceKind::Clone);
        assert!(opendal.tags.contains(&"suite:bench".to_string()));
        assert!(opendal.tags.contains(&"project:opendal".to_string()));

        let cuda_oxide = scenarios
            .iter()
            .find(|scenario| scenario.name == "bench-cuda-oxide")
            .unwrap();
        assert_eq!(cuda_oxide.source_kind, SourceKind::Clone);
        assert!(cuda_oxide.tags.contains(&"project:cuda-oxide".to_string()));
        assert!(
            scenarios
                .iter()
                .any(|scenario| scenario.name == "bench-cuda-oxide-patched")
        );

        let c_hello = scenarios
            .iter()
            .find(|scenario| scenario.name == "e2e-c-hello")
            .unwrap();
        assert_eq!(c_hello.source_kind, SourceKind::Fixture);
        assert!(c_hello.tags.contains(&"suite:e2e".to_string()));
    }

    #[test]
    fn fixture_negative_control_exempt_must_be_top_level() {
        let cfg: Fixture = deser_toml::from_str(
            r#"
name = "e2e-exempt"
negative_control_exempt = true

[source]
kind = "fixture"
path = "source"

[commands]
build = "make"
clean = "make clean"
"#,
        )
        .unwrap();
        assert!(cfg.negative_control_exempt);
    }

    #[test]
    fn source_table_rejects_misplaced_fixture_fields() {
        let err = deser_toml::from_str::<Fixture>(
            r#"
name = "e2e-exempt"

[source]
kind = "fixture"
path = "source"
negative_control_exempt = true

[commands]
build = "make"
clean = "make clean"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn pull_phase_gates_bind() {
        let checks: ScenarioChecks = deser_toml::from_str(
            r#"
[assert.pull]
max_passthrough_pct = 40.0
max_errors = 0

[measure.pull]
min_hit_rate_pct = 50.0
"#,
        )
        .unwrap();

        let assert = checks
            .assertions
            .for_phase("pull")
            .expect("pull assert gate must bind");
        assert_eq!(assert.max_passthrough_pct, Some(40.0));
        assert_eq!(assert.max_errors, Some(0));

        let measure = checks
            .measure
            .for_phase("pull")
            .expect("pull measure gate must bind");
        assert_eq!(measure.min_hit_rate_pct, Some(50.0));
    }

    /// The same-worktree warm phase has its OWN gate tables, distinct from the
    /// cross-clone `warm` ones. Both `for_phase` lookups must return them
    /// rather than falling through to "no spec configured": a validity gate
    /// that silently stops binding is a validity gate that no longer runs.
    #[test]
    fn warm_same_tree_phase_gates_bind() {
        let checks: ScenarioChecks = deser_toml::from_str(
            r#"
[assert.warm-same-tree]
max_passthrough_pct = 40.0
min_hits = 100
min_restored_bytes = 67108864

[assert.warm]
min_key_stability_pct = 50.0
min_hits = 7

[measure.warm-same-tree]
max_wall_s = 300

[measure.warm]
max_wall_s = 900
"#,
        )
        .unwrap();

        let assert = checks
            .assertions
            .for_phase("warm-same-tree")
            .expect("warm-same-tree assert gate must bind");
        assert_eq!(assert.max_passthrough_pct, Some(40.0));
        assert_eq!(assert.min_hits, Some(100));
        assert_eq!(assert.min_restored_bytes, Some(67_108_864));
        // And it is not the cross-clone table: that one declares a different
        // min_hits and a key-stability floor this phase cannot have.
        assert_eq!(assert.min_key_stability_pct, None);

        let measure = checks
            .measure
            .for_phase("warm-same-tree")
            .expect("warm-same-tree measure gate must bind");
        assert_eq!(measure.max_wall_s, Some(300));
    }

    #[test]
    fn scenario_checks_reject_misspelled_fields() {
        for raw in [
            "[assert.warm]\nmax_erors = 0\n",
            "[measure.warm]\nmin_hit_rate_pxt = 90.0\n",
        ] {
            let err = deser_toml::from_str::<ScenarioChecks>(raw).unwrap_err();
            assert!(err.to_string().contains("unknown field"), "{err}");
        }
    }

    #[test]
    fn selectors_are_trimmed_and_never_empty() {
        assert_eq!(Selector::parse("  tier:gate ").unwrap().raw(), "tier:gate");
        for blank in ["", "   "] {
            let error = Selector::parse(blank).unwrap_err().to_string();
            assert!(error.contains("cannot be empty"), "{error}");
        }
        let error = Selectors::parse_many(&["suite:e2e".into(), " ".into()]).unwrap_err();
        assert!(format!("{error:#}").contains("parsing --select ` `"), "{error:#}");
    }

    #[test]
    fn the_source_kind_alias_selects_only_that_kind() {
        let selectors = Selectors::parse_many(&["fixture".into()]).unwrap();
        assert!(selectors.matches("anything", SourceKind::Fixture, &[]));
        assert!(!selectors.matches("anything", SourceKind::Clone, &[]));
    }
}
