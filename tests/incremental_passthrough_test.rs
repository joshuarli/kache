#![cfg(unix)]

use std::fs;

#[allow(dead_code)]
mod common;

#[test]
fn incremental_arguments_and_environment_pass_through_unchanged() {
    for disabled in [false, true] {
        for response_file in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let cache = dir.path().join("cache");
            let _daemon = common::DaemonGuard::new(common::kache_binary(), &cache);
            let rustc = dir.path().join("rustc");
            let recorded = dir.path().join("argv");
            let environment = dir.path().join("environment");
            kache_fs::testutil::write_executable(
                &rustc,
                "#!/bin/sh\n: > \"$ARGV_DUMP\"\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    @*) while IFS= read -r line || [ -n \"$line\" ]; do printf '%s\\n' \"$line\" >> \"$ARGV_DUMP\"; done < \"${arg#@}\";;\n    *) printf '%s\\n' \"$arg\" >> \"$ARGV_DUMP\";;\n  esac\ndone\nprintf '%s' \"${CARGO_INCREMENTAL-unset}\" > \"$ENV_DUMP\"\n",
            );
            let source = dir.path().join("lib.rs");
            fs::write(&source, "pub fn answer() -> u8 { 42 }\n").unwrap();
            let incremental = dir.path().join("target/debug/incremental");
            let out = dir.path().join("target/debug/deps");
            fs::create_dir_all(&incremental).unwrap();
            fs::create_dir_all(&out).unwrap();
            fs::write(incremental.join("state"), b"existing state").unwrap();
            let args = vec![
                "--crate-name".to_string(),
                "fixture".to_string(),
                "--crate-type".to_string(),
                "lib".to_string(),
                source.display().to_string(),
                "--out-dir".to_string(),
                out.display().to_string(),
                "--emit=metadata".to_string(),
                "-Cextra-filename=-1234abcd".to_string(),
                format!("-Cincremental={}", incremental.display()),
            ];
            let mut command = common::hermetic_command(
                common::kache_binary(),
                &cache,
                Some(&dir.path().join("missing.toml")),
            );
            command.arg(&rustc).current_dir(dir.path())
                .env("ARGV_DUMP", &recorded)
                .env("ENV_DUMP", &environment)
                .env("CARGO_INCREMENTAL", "1")
                .env_remove("CARGO_PRIMARY_PACKAGE")
                .env("KACHE_LOG", "kache=debug")
                .env("KACHE_DISABLED", if disabled { "1" } else { "0" });
            if response_file {
                let path = dir.path().join("rustc.args");
                fs::write(&path, format!("{}\n", args.join("\n"))).unwrap();
                command.arg(format!("@{}", path.display()));
            } else {
                command.args(&args);
            }
            let mut stderr = String::new();
            for _ in 0..2 {
                let output = command.output().unwrap();
                stderr.push_str(&String::from_utf8_lossy(&output.stderr));
                assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
                let actual = fs::read_to_string(&recorded).unwrap();
                assert_eq!(actual.lines().collect::<Vec<_>>(), args.iter().map(String::as_str).collect::<Vec<_>>());
                assert_eq!(fs::read_to_string(&environment).unwrap(), "1");
                assert_eq!(fs::read(incremental.join("state")).unwrap(), b"existing state");
            }
            if !disabled {
                let events = fs::read_to_string(cache.join("events.jsonl"))
                    .unwrap_or_else(|error| panic!("missing events: {error}; stderr: {stderr}"));
                let events: Vec<deser_value::Value> = events.lines()
                    .map(|line| deser_json::from_str(line).unwrap()).collect();
                assert_eq!(events.len(), 2);
                assert!(events.iter().all(|event| event["result"] == "passthrough"));
            }
        }
    }
}

/// A compile whose result may be stored runs without Cargo's incremental
/// directory: the key leaves the flag out, so a stored artifact must not
/// depend on incremental state. This layout is outside the adaptive policy
/// (the incremental directory is not `<profile>/incremental`), so the miss
/// takes the ordinary cache path.
#[test]
fn a_cached_compile_never_runs_with_cargos_incremental_directory() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let _daemon = common::DaemonGuard::new(common::kache_binary(), &cache);
    let rustc = dir.path().join("rustc");
    let recorded = dir.path().join("argv");
    let out = dir.path().join("target/debug/deps");
    let real_rustc = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map(|output| {
            std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
                .join("bin/rustc")
        })
        .unwrap();
    kache_fs::testutil::write_executable(
        &rustc,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$ARGV_DUMP\"\nexec '{}' \"$@\"\n",
            real_rustc.display()
        ),
    );
    let source = dir.path().join("lib.rs");
    fs::write(&source, "pub fn answer() -> u8 { 42 }\n").unwrap();
    let incremental = dir.path().join("elsewhere/incremental");
    fs::create_dir_all(&incremental).unwrap();
    fs::create_dir_all(&out).unwrap();
    let output = common::hermetic_command(
        common::kache_binary(),
        &cache,
        Some(&dir.path().join("missing.toml")),
    )
    .arg(&rustc)
    .args([
        "--crate-name",
        "fixture",
        "--crate-type",
        "lib",
        &source.display().to_string(),
        "--out-dir",
        &out.display().to_string(),
        "--emit=metadata",
        "-Cextra-filename=-1234abcd",
        &format!("-Cincremental={}", incremental.display()),
    ])
    .current_dir(dir.path())
    .env("ARGV_DUMP", &recorded)
    .env("CARGO_PRIMARY_PACKAGE", "1")
    .env("KACHE_LOG", "kache=debug")
    .output()
    .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let argv = fs::read_to_string(&recorded).unwrap();
    assert!(
        argv.lines().any(|arg| arg == "fixture"),
        "the compile ran: {argv}"
    );
    assert!(
        !argv.contains("incremental="),
        "a cacheable compile received Cargo's incremental directory: {argv}"
    );
    let events = fs::read_to_string(cache.join("events.jsonl")).unwrap();
    let last: deser_value::Value = deser_json::from_str(events.lines().last().unwrap()).unwrap();
    assert_eq!(last["result"], "miss", "{last:#?}");
}
