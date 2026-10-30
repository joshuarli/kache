#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::Command;

const KACHE_BIN: &str = env!("CARGO_BIN_EXE_kache");

// Only the env plumbing is shared; this file drives the Cargo-built binary,
// so the bootstrap helpers in `common` stay unused here.
#[allow(dead_code)]
mod common;
use common::hermetic_command;

/// `kache cargo` wired to a throwaway HOME, cache, and target directory.
fn proxied_cargo(home: &Path, cache: &Path, target: &Path) -> Command {
    let mut command = hermetic_command(KACHE_BIN, cache, Some(&cache.join("config.toml")));
    command
        .env("HOME", home)
        .env("CARGO_HOME", home.join(".cargo"))
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTC_WRAPPER", KACHE_BIN)
        .env("KACHE_LOG", "off")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env_remove("CARGO_BUILD_BUILD_DIR");
    command
}

#[test]
fn cargo_command_passes_arguments_and_installs_the_wrapper() {
    let dir = tempfile::tempdir().unwrap();
    let cargo = dir.path().join("cargo");
    let record = dir.path().join("record");
    kache_fs::testutil::write_executable(
        &cargo,
        "#!/bin/sh\nprintf '%s\\n' \"$RUSTC_WRAPPER\" \"$@\" > \"$PROXY_RECORD\"\nexit 23\n",
    );
    let output = hermetic_command(KACHE_BIN, &dir.path().join("cache"), None)
        .env("KACHE_REAL_CARGO", &cargo)
        .env("PROXY_RECORD", &record)
        .env_remove("RUSTC_WRAPPER")
        .args(["cargo", "--", "check", "--offline"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
    let recorded = std::fs::read_to_string(record).unwrap();
    let mut lines = recorded.lines();
    assert_eq!(lines.next(), Some(KACHE_BIN));
    assert_eq!(lines.collect::<Vec<_>>(), ["check", "--offline"]);
}

#[test]
fn cargo_command_preserves_a_signal_exit() {
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::tempdir().unwrap();
    let cargo = dir.path().join("cargo");
    kache_fs::testutil::write_executable(&cargo, "#!/bin/sh\nkill -TERM $$\n");
    let output = hermetic_command(KACHE_BIN, &dir.path().join("cache"), None)
        .env("KACHE_REAL_CARGO", &cargo)
        .args(["cargo", "--", "check"])
        .output()
        .unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGTERM));
}

#[test]
fn explicit_rustflags_keep_their_cargo_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = home.join("work/project");
    let cargo_home = home.join(".cargo");
    let cache = dir.path().join("cache");
    let _daemon = common::DaemonGuard::new(env!("CARGO_BIN_EXE_kache"), &cache);
    let target = dir.path().join("target");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(&cargo_home).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"proxy_env_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        project.join("src/lib.rs"),
        "#[cfg(not(from_env))]\ncompile_error!(\"explicit RUSTFLAGS was replaced\");\n",
    )
    .unwrap();
    std::fs::write(
        cargo_home.join("config.toml"),
        "[build]\nrustflags = [\"--cfg\", \"from_config\"]\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&cargo_home, home.join("work/.cargo")).unwrap();

    let mut command = proxied_cargo(&home, &cache, &target);
    command
        .args(["cargo", "--", "check", "--quiet"])
        .current_dir(&project)
        .env("KACHE_REAL_CARGO", env!("CARGO"));
    command.env("RUSTFLAGS", "--cfg from_env");
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "explicit RUSTFLAGS lost precedence: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn non_utf8_cargo_argument_passes_through_without_cli_panic() {
    let dir = tempfile::tempdir().unwrap();
    let fake_cargo = dir.path().join("cargo");
    kache_fs::testutil::write_executable(&fake_cargo, "#!/bin/sh\nexit 0\n");

    let output = Command::new(KACHE_BIN)
        .args(["cargo", "--", "build"])
        .arg(OsString::from_vec(vec![b'f', b'o', 0x80]))
        .env("KACHE_REAL_CARGO", &fake_cargo)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "non-UTF-8 passthrough failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn non_utf8_wrapper_argument_fails_closed_before_compiler_execution() {
    let dir = tempfile::tempdir().unwrap();
    let fake_rustc = dir.path().join("rustc");
    let sentinel = dir.path().join("compiler-ran");
    kache_fs::testutil::write_executable(
        &fake_rustc,
        format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    );

    let output = Command::new(KACHE_BIN)
        .arg(&fake_rustc)
        .arg(OsString::from_vec(vec![b's', b'r', b'c', b'/', 0x80]))
        .env("RUSTC", &fake_rustc)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!sentinel.exists(), "unsafe compiler fallback was executed");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot be cached safely"),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn configured_build_dir_reaches_cargo_without_proxy_override() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = dir.path().join("project");
    let config_dir = project.join(".cargo");
    let fake_cargo = dir.path().join("cargo");
    let capture = dir.path().join("captured-build-dir");
    std::fs::create_dir_all(home.join(".cargo")).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[build]\nbuild-dir = \"configured-build\"\n",
    )
    .unwrap();
    kache_fs::testutil::write_executable(
        &fake_cargo,
        "#!/bin/sh\nprintf '%s %s' \"${CARGO_BUILD_BUILD_DIR-unset}\" \"$*\" > \"$KACHE_TEST_CAPTURE\"\n",
    );

    let output = Command::new(KACHE_BIN)
        .args(["cargo", "--", "build"])
        .current_dir(&project)
        .env("HOME", &home)
        .env("CARGO_HOME", home.join(".cargo"))
        .env("KACHE_REAL_CARGO", &fake_cargo)
        .env("KACHE_TEST_CAPTURE", &capture)
        .env_remove("CARGO_BUILD_BUILD_DIR")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "proxy failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(capture).unwrap(),
        "unset build",
        "a config-defined build-dir must retain Cargo precedence"
    );
}
