//! End-to-end coverage for Kache's automatic incremental adaptation.

#![cfg(unix)]

use filetime::FileTime;
use deser_value::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[allow(dead_code)]
mod common;
use common::{build_kache, hermetic_command};

fn kache_binary() -> PathBuf {
    build_kache();
    common::kache_binary()
}

fn fixture_events(cache_dir: &Path) -> Vec<Value> {
    fs::read_to_string(cache_dir.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| deser_json::from_str::<Value>(line).ok())
        .filter(|event| event["crate_name"] == "adaptive_fixture")
        .collect()
}

/// Create the fixture package.
fn create_fixture(project: &Path) {
    fs::create_dir(project.join("src")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"adaptive-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
}

// Cold discovery rejects inputs dated after compilation starts. These tests
// force Cargo to revisit old source timestamps by clearing its fingerprint.
fn past_mtime() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 3_600
}

fn reset_fixture_fingerprint(target_dir: &Path) {
    common::clear_package_fingerprints(&target_dir.join("debug"), "adaptive-fixture");
}

fn write_source(project: &Path, source_mtime: i64, body: &str) {
    let source = project.join("src/lib.rs");
    fs::write(&source, format!("pub fn answer() -> u64 {{ {body} }}\n")).unwrap();
    filetime::set_file_mtime(&source, FileTime::from_unix_time(source_mtime, 0)).unwrap();
}

fn run_cargo(project: &Path, cache_dir: &Path, target_dir: &Path) -> Output {
    let mut command = hermetic_command(
        "cargo",
        cache_dir,
        Some(&project.join("missing-kache.toml")),
    );
    command
        .args(["build", "--offline", "--quiet", "--lib"])
        .current_dir(project)
        .env("RUSTC_WRAPPER", kache_binary())
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_INCREMENTAL", "1")
        .env("KACHE_LOG", "kache=debug")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("KACHE_DISABLED");
    reset_fixture_fingerprint(target_dir);
    command.output().expect("failed to build adaptive fixture")
}

/// The one fixture event logged after the first `before` events.
fn only_new_event(cache_dir: &Path, before: usize) -> Value {
    let events = fixture_events(cache_dir);
    assert_eq!(
        events.len(),
        before + 1,
        "each source edit should compile the fixture exactly once; events:\n{}",
        fs::read_to_string(cache_dir.join("events.jsonl")).unwrap_or_default(),
    );
    events.into_iter().nth(before).unwrap()
}

fn build_variant(
    project: &Path,
    cache_dir: &Path,
    target_dir: &Path,
    source_mtime: i64,
    answer: u64,
) -> Value {
    write_source(project, source_mtime, &answer.to_string());
    let event_count = fixture_events(cache_dir).len();
    let output = run_cargo(project, cache_dir, target_dir);
    assert!(
        output.status.success(),
        "fixture build failed for variant {answer}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Execute a tiny direct consumer of the produced rlib. Event transitions
    // alone would not catch stale incremental output.
    let rlibs: Vec<_> = common::profile_artifacts(&target_dir.join("debug"))
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("libadaptive_fixture-") && name.ends_with(".rlib")
                })
        })
        .collect();
    let rlib = rlibs
        .iter()
        .max_by_key(|path| std::fs::metadata(path).and_then(|meta| meta.modified()).ok())
        .expect("fixture rlib was not produced")
        .clone();
    let consumer_source = project.join("consumer.rs");
    let consumer_binary = project.join("consumer");
    fs::write(
        &consumer_source,
        "extern crate adaptive_fixture; fn main() { print!(\"{}\", adaptive_fixture::answer()); }\n",
    )
    .unwrap();
    let mut consumer_compile = Command::new("rustc");
    consumer_compile
        .arg(&consumer_source)
        .arg("--edition=2024")
        .arg("--extern")
        .arg(format!("adaptive_fixture={}", rlib.display()));
    let rmeta = rlib.with_extension("rmeta");
    if rmeta.is_file() {
        consumer_compile
            .arg("--extern")
            .arg(format!("adaptive_fixture={}", rmeta.display()));
    }
    let consumer_compile = consumer_compile
        .arg("-L")
        .arg(format!(
            "dependency={}",
            rlib.parent().expect("rlib has a directory").display()
        ))
        .arg("-o")
        .arg(&consumer_binary)
        .output()
        .expect("failed to compile fixture consumer");
    assert!(
        consumer_compile.status.success(),
        "consumer compile failed: {}",
        String::from_utf8_lossy(&consumer_compile.stderr),
    );
    let consumer = Command::new(&consumer_binary)
        .output()
        .expect("failed to run fixture consumer");
    assert!(consumer.status.success());
    assert_eq!(
        String::from_utf8_lossy(&consumer.stdout),
        answer.to_string(),
        "rlib {}",
        rlib.display()
    );
    only_new_event(cache_dir, event_count)
}

/// Build a variant that fails to type-check and return its one event.
fn build_failing(project: &Path, cache_dir: &Path, target_dir: &Path, source_mtime: i64) -> Value {
    write_source(project, source_mtime, &format!("\"{source_mtime}\""));
    let event_count = fixture_events(cache_dir).len();
    let output = run_cargo(project, cache_dir, target_dir);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() && stderr.contains("E0308"),
        "the fixture should fail with a type error\nstdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&output.stdout),
    );
    only_new_event(cache_dir, event_count)
}

/// rustc's newest published session in the private state: the greatest
/// `s-*` directory name that is not still `-working`.
fn newest_finalized_session(policy_root: &Path) -> Option<PathBuf> {
    let children = |dir: &Path| -> Vec<PathBuf> {
        fs::read_dir(dir)
            .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    children(&policy_root.join("v3"))
        .iter()
        .flat_map(|unit| children(&unit.join("rustc")))
        .flat_map(|crate_dir| children(&crate_dir))
        .filter(|session| {
            let name = session.file_name().unwrap().to_string_lossy();
            name.starts_with("s-") && !name.ends_with("-working") && session.is_dir()
        })
        .max_by(|left, right| left.file_name().cmp(&right.file_name()))
}

fn assert_passthrough(event: &Value, reason: &str) {
    assert_eq!(event["result"], "passthrough", "event: {event:#?}");
    assert!(
        event["passthrough_reason"]
            .as_str()
            .is_some_and(|value| value.contains(reason)),
        "expected passthrough reason containing {reason:?}; event: {event:#?}",
    );
}

fn assert_no_private_graph(policy_root: &Path) {
    for unit in fs::read_dir(policy_root.join("v3")).unwrap() {
        let rustc = unit.unwrap().path().join("rustc");
        if rustc.exists() {
            assert!(
                fs::read_dir(&rustc).unwrap().next().is_none(),
                "a cache hit retained a provisional graph in {}",
                rustc.display(),
            );
        }
    }
}

fn finished_output(mut child: Child, status: ExitStatus) -> Output {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_end(&mut stdout).unwrap();
    }
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_end(&mut stderr).unwrap();
    }
    Output {
        status,
        stdout,
        stderr,
    }
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> Result<Output, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(finished_output(child, status)),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let status = child.wait().map_err(|error| error.to_string())?;
                let output = finished_output(child, status);
                return Err(format!(
                    "child timed out\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                ));
            }
            Err(error) => return Err(format!("polling child: {error}")),
        }
    }
}

#[test]
fn source_churn_adapts_then_returns_to_exact_cache_hits() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");
    create_fixture(project.path());
    let config_path = project.path().join("missing-kache.toml");

    let first_mtime = past_mtime();
    let mut tick = 0;
    let mut build = |answer| {
        let event = build_variant(
            project.path(),
            cache.path(),
            &target,
            first_mtime + tick,
            answer,
        );
        tick += 1;
        event
    };

    let first = build(1);
    assert_eq!(first["result"], "miss", "event: {first:#?}");
    assert_eq!(first["compiler_runs"], 1);

    // The edit changes the package's tree, so the record no longer applies.
    // The seed keys from the closure its own compile emits: no pre-pass.
    let seed = build(2);
    assert_passthrough(&seed, "adaptive seed");
    assert_eq!(seed["compiler_runs"], 1);
    assert_eq!(
        seed["dep_info_runs"], 0,
        "a seed discovers its closure in its own compile: {seed:#?}"
    );

    let active = build(3);
    assert_passthrough(&active, "adaptive active");
    assert_eq!(active["key_ms"], 0, "event: {active:#?}");

    // A newly configured exclusion must win over already-active policy state.
    fs::write(&config_path, "[cache]\nexclude = [\"src/lib.rs\"]\n").unwrap();
    let excluded = build(4);
    assert_passthrough(&excluded, "source excluded");
    fs::remove_file(&config_path).unwrap();

    // Exclusion resets learned state. The next miss learns normally, then a
    // second source-only miss seeds a fresh adaptive run.
    let after_exclusion = build(5);
    assert_eq!(
        after_exclusion["result"], "miss",
        "event: {after_exclusion:#?}"
    );
    let reseed = build(6);
    assert_passthrough(&reseed, "adaptive seed");

    for answer in 7..=14 {
        let active = build(answer);
        assert_passthrough(&active, "adaptive active");
        assert_eq!(active["compiler_runs"], 1);
        assert_eq!(
            active["key_ms"], 0,
            "active adaptation must run before cache-key work: {active:#?}",
        );
    }

    let policy_root = target.join("debug/incremental.kache-auto");
    assert!(policy_root.is_dir(), "missing {}", policy_root.display());
    assert!(
        fs::read_dir(&policy_root).unwrap().next().is_some(),
        "adaptive policy tree is empty: {}",
        policy_root.display(),
    );

    let restored = build(1);
    assert_eq!(restored["result"], "local_hit", "event: {restored:#?}");
    assert_eq!(restored["compiler_runs"], 0);

    // A hit discards the incremental state it could not have used, but it
    // is still a build of the unit: the next edit seeds state rather than
    // compiling once more without it. The consumer check in `build_variant`
    // proves the seeded compile produced the new answer.
    let after_hit = build(15);
    assert_passthrough(&after_hit, "adaptive seed");
    assert_eq!(after_hit["compiler_runs"], 1);
    assert_eq!(
        after_hit["dep_info_runs"], 0,
        "the first edit after a hit discovers its closure in its own compile: {after_hit:#?}"
    );

    // The predicted key may select incremental compilation, but must not
    // publish its output. With policy state removed, this variant still misses.
    fs::remove_dir_all(&policy_root).unwrap();
    let unseeded = build(15);
    assert_eq!(unseeded["result"], "miss", "event: {unseeded:#?}");
}

/// A type error keeps the private rustc state, as plain Cargo keeps its own
/// incremental directory, so the fix compiles on the same adaptive lane
/// instead of in full on the normal path.
#[test]
fn compile_errors_keep_adaptive_state_through_the_fix() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");
    let policy_root = target.join("debug/incremental.kache-auto");
    create_fixture(project.path());

    let mtime = std::cell::Cell::new(past_mtime());
    let next_mtime = || {
        let current = mtime.get();
        mtime.set(current + 1);
        current
    };
    let build = |answer| {
        build_variant(
            project.path(),
            cache.path(),
            &target,
            next_mtime(),
            answer,
        )
    };
    let break_build = || {
        build_failing(
            project.path(),
            cache.path(),
            &target,
            next_mtime(),
        )
    };

    let first = build(1);
    assert_eq!(first["result"], "miss", "event: {first:#?}");

    // A failed seed leaves the unit as it found it, so the fix seeds.
    let failed_seed = break_build();
    assert_passthrough(&failed_seed, "adaptive seed");
    assert_eq!(failed_seed["exit_code"], 1, "event: {failed_seed:#?}");
    assert_passthrough(&build(2), "adaptive seed");

    assert_passthrough(&build(3), "adaptive active");
    let kept = newest_finalized_session(&policy_root).expect("no finalized rustc session");

    let failed_active = break_build();
    assert_passthrough(&failed_active, "adaptive active");
    assert_eq!(failed_active["exit_code"], 1, "event: {failed_active:#?}");
    assert!(
        kept.is_dir(),
        "the failed compile removed {}",
        kept.display()
    );
    assert_eq!(
        newest_finalized_session(&policy_root).as_ref(),
        Some(&kept),
        "a failed compile must not publish a session"
    );

    // The consumer check in `build_variant` proves the fix's output is current.
    let fixed = build(4);
    assert_passthrough(&fixed, "adaptive active");
    assert_eq!(fixed["key_ms"], 0, "event: {fixed:#?}");
    assert_passthrough(&build(5), "adaptive active");

    let events = fixture_events(cache.path());
    assert!(
        events[1..]
            .iter()
            .all(|event| event["result"] == "passthrough"),
        "a compile after the first went through the normal path: {events:#?}"
    );
}

#[test]
fn local_source_churn_seeds_from_emitted_dep_info_without_a_pre_pass() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");
    let policy_root = target.join("debug/incremental.kache-auto");
    create_fixture(project.path());

    let first_mtime = past_mtime();
    let mut tick = 0;
    let mut build = |answer| {
        let event = build_variant(
            project.path(),
            cache.path(),
            &target,
            first_mtime + tick,
            answer,
        );
        tick += 1;
        event
    };

    let first = build(1);
    assert_eq!(first["result"], "miss", "event: {first:#?}");
    assert_eq!(first["compiler_runs"], 1, "event: {first:#?}");

    let seed = build(2);
    assert_passthrough(&seed, "adaptive seed");
    assert_eq!(seed["compiler_runs"], 1, "event: {seed:#?}");
    assert_eq!(seed["dep_info_runs"], 0, "event: {seed:#?}");
    assert_eq!(
        seed["rebuilt_package"], "adaptive-fixture",
        "event: {seed:#?}"
    );
    assert!(!seed["rebuilt_paths"].as_seq().unwrap().is_empty());
    assert!(
        newest_finalized_session(&policy_root).is_some(),
        "the cold seed did not publish a private rustc session",
    );

    let active = build(3);
    assert_passthrough(&active, "adaptive active");
    assert_eq!(active["compiler_runs"], 1, "event: {active:#?}");
    assert_eq!(active["dep_info_runs"], 0, "event: {active:#?}");
    assert_eq!(active["key_ms"], 0, "event: {active:#?}");

    // Removing policy state exposes any accidental artifact publication from
    // either the active compile or its seed. Each consumer checks the answer.
    fs::remove_dir_all(&policy_root).unwrap();
    let active_output = build(3);
    assert_eq!(active_output["result"], "miss", "event: {active_output:#?}");
    assert_eq!(active_output["compiler_runs"], 1);
    fs::remove_dir_all(&policy_root).unwrap();
    let seed_output = build(2);
    assert_eq!(seed_output["result"], "miss", "event: {seed_output:#?}");
    assert_eq!(seed_output["compiler_runs"], 1);
}

#[test]
fn local_learning_cache_hit_discards_the_provisional_graph() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");
    let policy_root = target.join("debug/incremental.kache-auto");
    create_fixture(project.path());

    let first_mtime = past_mtime();
    let mut tick = 0;
    let mut build = |answer| {
        let event = build_variant(
            project.path(),
            cache.path(),
            &target,
            first_mtime + tick,
            answer,
        );
        tick += 1;
        event
    };

    let first = build(1);
    assert_eq!(first["result"], "miss", "event: {first:#?}");
    assert_eq!(first["compiler_runs"], 1, "event: {first:#?}");
    assert_no_private_graph(&policy_root);
    let weight = cache
        .path()
        .join("scheduler/weights")
        .join(blake3::hash(b"adaptive_fixture").to_hex().as_str());
    let initial_rss: u64 = fs::read_to_string(&weight).unwrap().parse().unwrap();
    assert!(initial_rss > 1024, "the first compile did not record RSS");
    fs::remove_file(&weight).unwrap();

    // The changed mtime invalidates the learned closure; the cleared Cargo
    // fingerprint forces a build. The compiler must stop at the exact hit.
    let restored = build(1);
    assert_eq!(restored["result"], "local_hit", "event: {restored:#?}");
    assert_eq!(restored["dep_info_runs"], 1, "event: {restored:#?}");
    assert_eq!(restored["compiler_runs"], 0, "event: {restored:#?}");
    assert_no_private_graph(&policy_root);
    assert!(
        !weight.exists(),
        "an interrupted cache-hit compile recorded an incomplete RSS sample",
    );

    let seed = build(2);
    assert_passthrough(&seed, "adaptive seed");
    assert_eq!(seed["compiler_runs"], 1, "event: {seed:#?}");
    assert_eq!(seed["dep_info_runs"], 0, "event: {seed:#?}");
    assert!(
        newest_finalized_session(&policy_root).is_some(),
        "a cache hit prevented the next edit from seeding private state",
    );
}

#[test]
fn local_empty_cache_entry_does_not_stop_the_cold_compile() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");
    let policy_root = target.join("debug/incremental.kache-auto");
    create_fixture(project.path());

    let first_mtime = past_mtime();
    let first = build_variant(project.path(), cache.path(), &target, first_mtime, 1);
    assert_eq!(first["result"], "miss", "event: {first:#?}");
    let metadata_path = cache
        .path()
        .join("store")
        .join(first["cache_key"].as_str().unwrap())
        .join("meta.json");
    let mut metadata: Value = deser_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    assert!(!metadata["files"].as_seq().unwrap().is_empty());
    metadata["files"] = deser_value::value!([]);
    fs::write(&metadata_path, deser_json::to_string(&metadata).map(String::into_bytes).unwrap()).unwrap();

    // The store can read this metadata, but it has no artifact to restore.
    // Cold discovery must finish compilation and discard unqualified state.
    let compiled = build_variant(
        project.path(),
        cache.path(),
        &target,
        first_mtime + 1,
        1,
    );
    assert_passthrough(&compiled, "adaptive seed");
    assert_eq!(compiled["compiler_runs"], 1, "event: {compiled:#?}");
    assert_eq!(compiled["dep_info_runs"], 0, "event: {compiled:#?}");
    assert_no_private_graph(&policy_root);
}

#[test]
fn local_unqualified_cold_seed_resets_learning_so_the_next_build_can_store() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");
    let policy_root = target.join("debug/incremental.kache-auto");
    create_fixture(project.path());

    let first_mtime = past_mtime();
    let mut tick = 0;
    let mut build = || {
        let event = build_variant(
            project.path(),
            cache.path(),
            &target,
            first_mtime + tick,
            1,
        );
        tick += 1;
        event
    };

    let first = build();
    assert_eq!(first["result"], "miss", "event: {first:#?}");
    assert_eq!(first["compiler_runs"], 1, "event: {first:#?}");
    let policy_state = fs::read_dir(policy_root.join("v3"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("state.json"))
        .find(|path| path.is_file())
        .expect("the first miss did not leave Learning state");

    // Eviction removes the exact artifact while preserving its learned key.
    // Unchanged inputs cannot qualify this cold compile as an adaptive seed.
    fs::remove_dir_all(cache.path().join("store")).unwrap();
    let unqualified = build();
    assert_passthrough(&unqualified, "adaptive seed");
    assert_eq!(unqualified["compiler_runs"], 1, "event: {unqualified:#?}");
    assert_eq!(unqualified["dep_info_runs"], 0, "event: {unqualified:#?}");
    assert_no_private_graph(&policy_root);
    assert!(
        !policy_state.exists(),
        "an unqualified cold seed retained Learning state in {}",
        policy_state.display(),
    );

    let stored = build();
    assert_eq!(stored["result"], "miss", "event: {stored:#?}");
    assert_eq!(stored["compiler_runs"], 1, "event: {stored:#?}");
    let restored = build();
    assert_eq!(restored["result"], "local_hit", "event: {restored:#?}");
    assert_eq!(restored["compiler_runs"], 0, "event: {restored:#?}");
}

#[test]
fn local_failed_cold_seed_keeps_the_next_fix_on_the_seed_lane() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");
    create_fixture(project.path());

    let mtime = std::cell::Cell::new(past_mtime());
    let next_mtime = || {
        let current = mtime.get();
        mtime.set(current + 1);
        current
    };
    let build = |answer| {
        build_variant(
            project.path(),
            cache.path(),
            &target,
            next_mtime(),
            answer,
        )
    };

    let first = build(1);
    assert_eq!(first["result"], "miss", "event: {first:#?}");
    let failed_seed = build_failing(project.path(), cache.path(), &target, next_mtime());
    assert_passthrough(&failed_seed, "adaptive seed");
    assert_eq!(failed_seed["exit_code"], 1, "event: {failed_seed:#?}");
    assert_eq!(failed_seed["compiler_runs"], 1, "event: {failed_seed:#?}");
    assert_eq!(failed_seed["dep_info_runs"], 0, "event: {failed_seed:#?}");
    assert!(
        failed_seed.get("rebuilt_paths").is_none(),
        "event: {failed_seed:#?}"
    );

    let fixed = build(2);
    assert_passthrough(&fixed, "adaptive seed");
    assert_eq!(fixed["compiler_runs"], 1, "event: {fixed:#?}");
    assert_eq!(fixed["dep_info_runs"], 0, "event: {fixed:#?}");
    let active = build(3);
    assert_passthrough(&active, "adaptive active");
    assert_eq!(active["compiler_runs"], 1, "event: {active:#?}");
    assert_eq!(active["dep_info_runs"], 0, "event: {active:#?}");
}

#[test]
fn an_uncached_user_facing_executable_takes_the_immediate_adaptive_lane() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let target = project.path().join("target");

    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"adaptive-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        project.path().join("src/main.rs"),
        "fn main() { print!(\"adaptive executable\"); }\n",
    )
    .unwrap();
    let output = hermetic_command(
        "cargo",
        cache.path(),
        Some(&project.path().join("missing-kache.toml")),
    )
    .args(["build", "--offline", "--quiet", "--bin", "adaptive-fixture"])
    .current_dir(project.path())
    .env("RUSTC_WRAPPER", kache_binary())
    .env("CARGO_TARGET_DIR", &target)
    .env("CARGO_INCREMENTAL", "1")
    .env("KACHE_CACHE_EXECUTABLES", "0")
    .env_remove("RUSTC_WORKSPACE_WRAPPER")
    .env_remove("KACHE_DISABLED")
    .output()
    .expect("failed to build adaptive executable fixture");
    assert!(
        output.status.success(),
        "adaptive executable build failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let executable = target.join("debug/adaptive-fixture");
    let run = Command::new(&executable)
        .output()
        .expect("failed to run adaptive executable fixture");
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout), "adaptive executable");

    let events = fixture_events(cache.path());
    assert_eq!(events.len(), 1, "unexpected fixture events: {events:#?}");
    let event = &events[0];
    assert_passthrough(event, "adaptive passthrough");
    assert_eq!(event["compiler_runs"], 1, "event: {event:#?}");

    let policy_version = target.join("debug/incremental.kache-auto/v3");
    let unit_dirs: Vec<_> = fs::read_dir(&policy_version)
        .unwrap_or_else(|error| panic!("missing {}: {error}", policy_version.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    assert_eq!(unit_dirs.len(), 1, "adaptive unit dirs: {unit_dirs:#?}");
    let rustc_dir = unit_dirs[0].join("rustc");
    assert!(rustc_dir.is_dir(), "missing {}", rustc_dir.display());
    assert!(
        fs::read_dir(&rustc_dir).unwrap().next().is_some(),
        "isolated rustc incremental state is empty: {}",
        rustc_dir.display(),
    );
}

#[test]
fn contended_immediate_lane_never_shares_incremental_state() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _daemon = common::DaemonGuard::new(kache_binary(), cache.path());
    let profile = project.path().join("target/debug");
    let deps = profile.join("deps");
    let incremental = profile.join("incremental");
    fs::create_dir_all(&deps).unwrap();
    fs::create_dir(&incremental).unwrap();

    let source = project.path().join("main.rs");
    let fake_rustc = project.path().join("rustc");
    let owner = project.path().join("compiler-owner");
    let ready = project.path().join("first-ready");
    let release = project.path().join("release-first");
    let contender = project.path().join("contender-seen");
    let first_argv = project.path().join("first-argv.txt");
    let second_argv = project.path().join("second-argv.txt");
    fs::write(&source, "fn main() {}\n").unwrap();
    kache_fs::testutil::write_executable(
        &fake_rustc,
        r#"#!/bin/sh
printf '%s\n' "$@" > "$ARGV_DUMP"
incremental=
for arg in "$@"; do
    case "$arg" in
        -Cincremental=*) incremental=${arg#-Cincremental=} ;;
        --codegen=incremental=*) incremental=${arg#--codegen=incremental=} ;;
    esac
done
if mkdir "$OWNER_DIR" 2>/dev/null; then
    if [ -n "$incremental" ]; then
        mkdir -p "$incremental" || exit 91
        printf 'seeded' > "$incremental/fake-state" || exit 92
    fi
    : > "$READY_FILE"
    attempts=0
    while [ ! -f "$RELEASE_FILE" ] && [ "$attempts" -lt 600 ]; do
        sleep 0.05
        attempts=$((attempts + 1))
    done
    [ -f "$RELEASE_FILE" ] || exit 93
else
    : > "$CONTENDER_FILE"
fi
exit 0
"#,
    );

    let wrapper = |argv_dump: &Path| {
        let mut command = hermetic_command(
            kache_binary(),
            cache.path(),
            Some(&project.path().join("missing-kache.toml")),
        );
        command
            .arg(&fake_rustc)
            .args(["--crate-name", "adaptive_fixture", "--crate-type", "bin"])
            .arg(&source)
            .arg("--out-dir")
            .arg(&deps)
            .arg("--emit=link")
            .arg("-Cextra-filename=-1234abcd")
            .arg(format!("-Cincremental={}", incremental.display()))
            .current_dir(project.path())
            .env("ARGV_DUMP", argv_dump)
            .env("OWNER_DIR", &owner)
            .env("READY_FILE", &ready)
            .env("RELEASE_FILE", &release)
            .env("CONTENDER_FILE", &contender)
            .env_remove("CARGO_PRIMARY_PACKAGE")
            .env("CARGO_INCREMENTAL", "1")
            .env("KACHE_CACHE_EXECUTABLES", "0")
            .env("KACHE_LOG", "kache=warn")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .env_remove("KACHE_DISABLED")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };

    let mut first = wrapper(&first_argv).spawn().unwrap();
    let ready_deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        if let Some(status) = first.try_wait().unwrap() {
            let output = finished_output(first, status);
            panic!(
                "first wrapper exited before holding incremental state\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        if Instant::now() >= ready_deadline {
            let _ = first.kill();
            let status = first.wait().unwrap();
            let output = finished_output(first, status);
            panic!(
                "first fake rustc did not reach its blocking point\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        thread::sleep(Duration::from_millis(20));
    }

    let second = wrapper(&second_argv).spawn().unwrap();
    // A cold local-store/daemon fallback can consume its own five-second
    // deadline. Keep the process bound above that without relying on the fake
    // compiler's longer self-release deadline.
    let second_output = match wait_with_timeout(second, Duration::from_secs(20)) {
        Ok(output) => output,
        Err(error) => {
            fs::write(&release, b"release").unwrap();
            let _ = first.kill();
            let _ = first.wait();
            panic!("contending wrapper did not fall back promptly: {error}");
        }
    };
    fs::write(&release, b"release").unwrap();
    let first_output = wait_with_timeout(first, Duration::from_secs(10)).unwrap();

    assert!(
        first_output.status.success(),
        "first wrapper failed: {}",
        String::from_utf8_lossy(&first_output.stderr),
    );
    assert!(
        second_output.status.success(),
        "contending wrapper failed: {}",
        String::from_utf8_lossy(&second_output.stderr),
    );
    assert!(contender.exists(), "fake rustc never saw the contender");

    let first_args = fs::read_to_string(&first_argv).unwrap();
    assert!(
        first_args.lines().any(|arg| {
            arg.starts_with("-Cincremental=")
                && arg.contains("/incremental.kache-auto/v3/")
                && arg.ends_with("/rustc")
        }),
        "first wrapper did not hold isolated incremental state: {first_args:?}",
    );
    let second_args = fs::read_to_string(&second_argv).unwrap();
    assert!(
        !second_args
            .lines()
            .any(|arg| arg.contains("incremental.kache-auto")),
        "contender borrowed the lease holder's private state: {second_args:?}",
    );
}
