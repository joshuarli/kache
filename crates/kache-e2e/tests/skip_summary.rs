use std::process::Command;

#[test]
fn a_selection_whose_fixtures_all_skip_fails() {
    let temp = tempfile::tempdir().unwrap();
    let scenario = temp.path().join("e2e-missing-tool");
    std::fs::create_dir_all(scenario.join("source")).unwrap();
    std::fs::write(
        scenario.join("scenario.toml"),
        r#"
name = "e2e-missing-tool"
tags = ["suite:e2e"]
requires = ["kache-e2e-tool-that-does-not-exist"]

[source]
kind = "fixture"
path = "source"

[commands]
build = "unused"
clean = "unused"
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kache-scenario"))
        .arg("--kache")
        .arg(env!("CARGO_BIN_EXE_kache-scenario"))
        .arg("--scenarios")
        .arg(temp.path())
        .arg("--select")
        .arg("suite:e2e")
        .arg("--out")
        .arg(temp.path().join("results.json"))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("FAIL: every selected fixture was skipped (1 skipped); nothing was verified")
    );
}
