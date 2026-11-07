use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
mod common;

#[test]
fn explicit_binary_selector_identifies_binary_and_engine_pair() {
    let kache = common::kache_binary();
    let engine = common::kache_engine_binary();

    assert!(kache.is_file(), "kache binary must exist at {}", kache.display());
    assert!(engine.is_file(), "kache-engine companion must exist at {}", engine.display());
    assert_eq!(
        engine,
        kache.with_file_name(format!("kache-engine{}", std::env::consts::EXE_SUFFIX)),
        "engine companion must sit beside kache frontend"
    );

    let output = Command::new(&kache).arg("--version").output().expect("kache --version");
    assert!(output.status.success(), "kache --version failed");
    let version_str = String::from_utf8_lossy(&output.stdout);
    assert!(version_str.starts_with("kache "), "expected version output, got {version_str}");

    if let Some(explicit) = std::env::var_os("KACHE_TEST_BINARY") {
        let explicit_path = PathBuf::from(explicit);
        assert_eq!(
            kache, explicit_path,
            "selected kache binary must match KACHE_TEST_BINARY"
        );
    }
}

#[test]
fn explicit_binary_selector_fails_on_missing_or_stub_override() {
    let missing_path = Path::new("/nonexistent/bin/kache_missing_binary_stub");
    let res = common::resolve_kache_binary_with(Some(missing_path));
    assert!(res.is_err(), "missing binary override must fail");
    let err = res.unwrap_err();
    assert!(
        err.contains("KACHE_TEST_BINARY set to") && err.contains("file does not exist"),
        "error must identify missing selector: {err}"
    );

    let temp = tempfile::tempdir().unwrap();
    let stub_kache = temp.path().join("kache");
    kache_fs::testutil::write_executable(&stub_kache, b"#!/bin/sh\nexit 1\n");

    let res = common::resolve_kache_binary_with(Some(&stub_kache));
    assert_eq!(res.unwrap(), stub_kache);

    let engine_res = common::resolve_kache_engine_binary_with(&stub_kache, true);
    assert!(engine_res.is_err(), "missing engine beside explicit kache must fail");
    let engine_err = engine_res.unwrap_err();
    assert!(
        engine_err.contains("kache-engine companion at") && engine_err.contains("does not exist"),
        "error must identify missing companion: {engine_err}"
    );
}

#[test]
fn default_binary_selector_bootstraps_companion_without_override() {
    let res = common::resolve_kache_binary_with(None);
    assert!(res.is_ok(), "default selector must resolve a binary: {:?}", res.err());
    let bin = res.unwrap();
    assert!(bin.is_file(), "default binary must exist");

    let engine_res = common::resolve_kache_engine_binary_with(&bin, false);
    assert!(engine_res.is_ok(), "default engine companion must resolve: {:?}", engine_res.err());
    let engine = engine_res.unwrap();
    assert!(engine.is_file(), "default engine companion must exist");
}
