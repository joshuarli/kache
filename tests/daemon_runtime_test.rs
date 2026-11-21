#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

mod common;

struct Fixture {
    root: tempfile::TempDir,
    cache: PathBuf,
    runtime: Option<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let cache = root.path().join("deep-cache-".repeat(13)).join("store");
        std::fs::write(
            root.path().join("config.toml"),
            "[cache]\nlocal_only=true\nauto_gc=false\nscheduler=false\nprefetch_enabled=false\n",
        )
        .unwrap();
        Self {
            root,
            cache,
            runtime: None,
        }
    }

    fn command(&self) -> Command {
        let mut command = common::hermetic_command(
            common::kache_binary(),
            &self.cache,
            Some(&self.root.path().join("config.toml")),
        );
        command
            .env_remove("KACHE_RUNTIME_DIR")
            .env_remove("KACHE_DISABLED")
            .env("KACHE_LOG", "off")
            .env("KACHE_DAEMON_IDLE_TIMEOUT", "30")
            .env("HOME", self.root.path())
            .current_dir(self.root.path());
        command
    }

    fn start(&mut self) -> u64 {
        let output = successful(self.command().args(["--json", "daemon", "status"]));
        let status: deser_value::Value = deser_json::from_slice(&output.stdout).unwrap();
        let socket = PathBuf::from(status["socket"].as_str().unwrap());
        assert!(socket.as_os_str().len() < 100);
        let runtime = socket.parent().unwrap().to_path_buf();
        assert_ne!(runtime, self.cache);
        assert_eq!(
            std::fs::metadata(&runtime).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        self.runtime = Some(runtime.clone());
        successful(self.command().args(["daemon", "start"]));
        let output = successful(self.command().args(["--json", "daemon", "status"]));
        let status: deser_value::Value = deser_json::from_slice(&output.stdout).unwrap();
        assert_eq!(status["daemon_running"], true);
        let state: deser_value::Value =
            deser_json::from_slice(&std::fs::read(runtime.join("daemon.state.json")).unwrap())
                .unwrap();
        state["pid"].as_u64().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(runtime) = &self.runtime {
            common::stop_daemon(self.command().args(["daemon", "stop"]), runtime);
            let _ = std::fs::remove_dir_all(runtime);
        }
    }
}

fn successful(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{:?}\n{}",
        command,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn deep_caches_get_distinct_private_runtimes_and_warm_daemon_handoffs() {
    let mut a = Fixture::new();
    let mut b = Fixture::new();
    let pid = a.start();
    b.start();
    assert_ne!(a.runtime, b.runtime);
    let sysroot = successful(Command::new("rustc").args(["--print", "sysroot"]));
    let rustc = PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim()).join("bin/rustc");
    let source = a.root.path().join("lib.rs");
    std::fs::write(&source, "pub fn answer() -> u8 { 42 }\n").unwrap();
    let out = a.root.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let traces = a.root.path().join("traces");
    let compile = || {
        let output = successful(
            a.command()
                .arg(&rustc)
                .arg(&source)
                .args([
                    "--crate-name",
                    "runtime_probe",
                    "--crate-type=rlib",
                    "--emit=link,dep-info",
                    "--out-dir",
                ])
                .arg(&out)
                .env("KACHE_PHASE_TRACE_DIR", &traces),
        );
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    compile();
    let object = std::fs::read(out.join("libruntime_probe.rlib")).unwrap();
    std::fs::remove_dir_all(&out).unwrap();
    std::fs::create_dir(&out).unwrap();
    compile();
    assert_eq!(
        std::fs::read(out.join("libruntime_probe.rlib")).unwrap(),
        object
    );
    common::stop_daemon(
        a.command().args(["daemon", "stop"]),
        a.runtime.as_ref().unwrap(),
    );
    let events = std::fs::read_to_string(a.runtime.as_ref().unwrap().join("events.jsonl")).unwrap();
    assert!(
        events.lines().any(|line| {
            let e: deser_value::Value = deser_json::from_str(line).unwrap();
            e["result"] == "local_hit"
        }),
        "{events}"
    );
    let mut count = 0;
    for trace in std::fs::read_dir(traces).unwrap() {
        let trace: deser_value::Value =
            deser_json::from_slice(&std::fs::read(trace.unwrap().path()).unwrap()).unwrap();
        for event in trace["traceEvents"].as_seq().unwrap() {
            assert_eq!(event["pid"].as_u64(), Some(pid));
        }
        count += 1;
    }
    assert_eq!(
        count, 2,
        "both compiler invocations must run in the same daemon"
    );
}

#[test]
fn explicit_overlong_runtime_is_an_error_not_an_automatic_redirect() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["daemon", "start"])
        .env("KACHE_RUNTIME_DIR", &fixture.cache)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("a Unix socket path can be at most"),
        "{stderr}"
    );
    assert!(stderr.contains("KACHE_RUNTIME_DIR"), "{stderr}");
}

#[test]
fn failed_bootstrap_warns_once_but_disabled_execution_is_quiet() {
    let fixture = Fixture::new();
    let runtime = fixture.root.path().join("runtime\nwith-newline");
    std::fs::create_dir_all(runtime.join("daemon.sock")).unwrap();
    let compiler = fixture.root.path().join("rustc");
    kache_fs::testutil::write_executable(&compiler, "#!/bin/sh\nprintf 'fake-rustc\\n'\n");
    for disabled in [false, true] {
        let output = successful(
            fixture
                .command()
                .arg(&compiler)
                .arg("--version")
                .env("KACHE_RUNTIME_DIR", &runtime)
                .env("KACHE_DISABLED", if disabled { "1" } else { "0" }),
        );
        assert_eq!(output.stdout, b"fake-rustc\n");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let warnings = stderr
            .lines()
            .filter(|l| l.contains("warning: daemon handoff failed"))
            .count();
        assert_eq!(warnings, usize::from(!disabled), "{stderr}");
        if !disabled {
            assert_eq!(stderr.lines().count(), 1, "{stderr}");
        } else {
            assert!(stderr.is_empty(), "{stderr}");
        }
    }
    assert!(runtime.join("daemon.sock").is_dir());
}
