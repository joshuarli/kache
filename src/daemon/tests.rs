use super::*;
use std::fs;
use std::sync::atomic::AtomicU64;

#[test]
fn compiler_request_capacity_handles_bursts_and_recovers_after_drop() {
    let root = tempfile::tempdir().unwrap();
    let mut config = test_config(root.path());
    config.daemon_execution_concurrency = Some(32);
    let daemon = Daemon::new(config);
    let workers = daemon.execution_workers.clone();
    let mut requests = Vec::new();

    // A cache-hit burst must fit even on a host with fewer than 32 CPUs.
    for _ in 0..32 {
        requests.push(
            workers
                .clone()
                .try_acquire_arc()
                .expect("pipeline request rejected before the burst limit"),
        );
    }
    assert!(workers.clone().try_acquire_arc().is_none());

    drop(requests.pop());
    let replacement = workers
        .clone()
        .try_acquire_arc()
        .expect("a finished request must release its slot");
    assert!(workers.clone().try_acquire_arc().is_none());

    drop(replacement);
    drop(requests);
    let mut next_burst = Vec::new();
    for _ in 0..32 {
        next_burst.push(
            workers
                .clone()
                .try_acquire_arc()
                .expect("all slots must be reusable after the burst finishes"),
        );
    }
    assert!(workers.try_acquire_arc().is_none());
    drop(next_burst);
}



/// A wrapper behind a compiler shim can start the daemon with the shim as
/// its executable. The child must still parse `daemon run` as the CLI.
#[test]
fn daemon_spawn_is_marked_as_a_self_spawn() {
    let exe = Path::new("/x/shims/cc");
    let command = daemon_run_command(exe);
    assert_eq!(command.get_program(), exe.as_os_str());
    let args: Vec<&std::ffi::OsStr> = command.get_args().collect();
    assert_eq!(args, ["daemon", "run"]);
    let argv: Vec<String> = std::iter::once("/x/shims/cc")
        .chain(args.iter().filter_map(|arg| arg.to_str()))
        .map(str::to_string)
        .collect();
    let marker = command
        .get_envs()
        .find(|(name, _)| *name == crate::platform::SELF_SPAWN_ENV)
        .and_then(|(_, value)| value);
    assert!(crate::platform::is_self_spawn(&argv, marker));
}

/// Env var naming the fixture directory for
/// `detached_daemon_caller_fixture`; unset, the fixture does nothing.
#[cfg(unix)]
const DETACH_FIXTURE_DIR: &str = "KACHE_TEST_DETACH_FIXTURE_DIR";

/// Plays the build that auto-starts the daemon: it spawns a stand-in
/// daemon through `spawn_detached_daemon`, records its PID, then waits to
/// be interrupted. Run only by
/// `detached_daemon_survives_sigint_to_the_callers_group`.
#[cfg(unix)]
#[test]
#[ignore = "fixture for detached_daemon_survives_sigint_to_the_callers_group"]
fn detached_daemon_caller_fixture() {
    let Some(dir) = std::env::var_os(DETACH_FIXTURE_DIR).map(PathBuf::from) else {
        return;
    };
    let log = std::fs::File::create(dir.join("daemon.log")).unwrap();
    let child = spawn_detached_daemon(
        &dir.join("kache"),
        kunobi_daemon::launch::DaemonOutput::File(log),
    )
    .unwrap();
    std::fs::write(dir.join("pid.tmp"), child.id().to_string()).unwrap();
    std::fs::rename(dir.join("pid.tmp"), dir.join("pid")).unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

/// kunobi-ninja/kache#1205: Ctrl-C in the terminal signals the whole
/// foreground process group, and an auto-started daemon used to be part of
/// it. A caller in its own group starts a stand-in daemon through the real
/// spawn path; interrupting that group must leave the daemon running, with
/// the arguments, environment and stderr the daemon is meant to get.
#[cfg(unix)]
#[test]
fn detached_daemon_survives_sigint_to_the_callers_group() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let script = root.join("kache");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{0}/args'\nenv > '{0}/env'\n\
                 echo daemon-stderr >&2\ntouch '{0}/ready'\nexec sleep 30\n",
            root.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut caller = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::tests::detached_daemon_caller_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env(DETACH_FIXTURE_DIR, root)
        .process_group(0)
        .spawn()
        .unwrap();
    // The caller is a second copy of this debug test binary; on a loaded
    // machine its start alone can take seconds.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !(root.join("pid").exists() && root.join("ready").exists()) {
        assert!(
            caller.try_wait().unwrap().is_none(),
            "the caller exited before starting the daemon"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let daemon: i32 = std::fs::read_to_string(root.join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    let caller_group = caller.id() as i32;

    assert_eq!(unsafe { libc::kill(-caller_group, libc::SIGINT) }, 0);
    let status = caller.wait().unwrap();
    // Give a daemon that shared the group time to die, then look.
    std::thread::sleep(Duration::from_millis(300));
    let survived = unsafe { libc::kill(daemon, 0) } == 0;
    let session = unsafe { libc::getsid(daemon) };
    unsafe { libc::kill(daemon, libc::SIGKILL) };

    assert!(!status.success(), "SIGINT never reached the caller's group");
    assert!(survived, "the daemon died with its caller's process group");
    assert_eq!(session, daemon, "the daemon must lead its own session");

    let args = std::fs::read_to_string(root.join("args")).unwrap();
    assert_eq!(args, "daemon\nrun\n");
    let env = std::fs::read_to_string(root.join("env")).unwrap();
    let marker = format!("{}=", crate::platform::SELF_SPAWN_ENV);
    assert!(
        env.lines().any(|line| line.starts_with(&marker)),
        "the daemon must be marked as a self-spawn: {env}"
    );
    let log = std::fs::read_to_string(root.join("daemon.log")).unwrap();
    assert_eq!(log, "daemon-stderr\n", "stderr must go to the daemon log");
}

/// Only an OS that proves the process gone makes it dead; PIDs that can
/// never name a daemon are dead without asking.
#[test]
fn process_liveness_maps_states_conservatively() {
    use kunobi_daemon::local::ProcessState;
    assert!(alive_from_state(2, || ProcessState::Alive));
    assert!(alive_from_state(2, || ProcessState::Unknown));
    assert!(!alive_from_state(2, || ProcessState::Exited));
    assert!(alive_from_state(i32::MAX as u32, || ProcessState::Alive));
    for pid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
        assert!(
            !alive_from_state(pid, || ProcessState::Alive),
            "PID {pid} must never read as a live daemon"
        );
    }
}

#[test]
fn current_process_is_alive() {
    assert!(process_is_alive(std::process::id()));
}

#[cfg(unix)]
#[test]
fn reaped_child_is_not_alive() {
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id();
    assert!(process_is_alive(pid));
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!process_is_alive(pid), "a reaped child must read as exited");
}

// Tests use the same transport as production, which resolves to UDS.
#[cfg(test)]
use crate::transport::AsyncListener;
use crate::transport::AsyncStream;

#[cfg(test)]
async fn smol_opt_timeout<T>(
    duration: Duration,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    use futures::FutureExt as _;
    let mut future = Box::pin(future.fuse());
    let mut timer = Box::pin(async_io::Timer::after(duration).fuse());
    futures::select! {
        value = future => Some(value),
        _ = timer => None,
    }
}

/// A sibling of `socket_path` that no earlier bind in this process has
/// used.
///
/// The endpoint is a filesystem path and rebinding the same name
/// after the listener is gone is harmless. Two binds on one name could race
/// (kunobi-ninja/kache#1107). The process-wide counter keeps every bind on
/// its own name; the caller's temp dir keeps it off the names concurrent
/// nextest processes use.
fn fresh_endpoint(socket_path: &Path) -> std::path::PathBuf {
    static BINDS: AtomicU64 = AtomicU64::new(0);
    let n = BINDS.fetch_add(1, Ordering::Relaxed);
    socket_path.with_file_name(format!("daemon-{n}.sock"))
}

/// #1107: two binds derived from one socket path must be live at the same
/// time. A repeated name is `EADDRINUSE`, so `bind_listener` panics here
/// instead of intermittently in whichever test happened to ask for a second
/// roundtrip.
#[test]
fn each_bind_gets_an_endpoint_name_of_its_own() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("daemon.sock");
        let first = fresh_endpoint(&socket_path);
        let second = fresh_endpoint(&socket_path);
        assert_ne!(first, second, "a reused name is the collision itself");
        assert_eq!(
            (first.parent(), second.parent()),
            (socket_path.parent(), socket_path.parent()),
            "endpoints stay beside the socket path the caller gave"
        );
        let _first = bind_listener(&first).await;
        let _second = bind_listener(&second).await;
    });
}

/// Bind a daemon-style listener at `path`, taking the cross-platform
/// transport. Used by every roundtrip test to remove boilerplate.
async fn bind_listener(path: &Path) -> AsyncListener {
    crate::transport::bind_daemon_listener(path)
        .await
        .expect("bind listener")
        .expect("listener won the endpoint")
}

/// Client-side connect mirror of bind_listener.
async fn connect_stream(path: &Path) -> AsyncStream {
    AsyncStream::connect(path).await.expect("connect")
}

/// Bind a *synchronous* listener at `path` so `transport::is_reachable`
/// reports the endpoint as live. It needs no async runtime — so it can be
/// created inside a plain `std::thread`.
fn bind_sync_listener(path: &Path) -> interprocess::local_socket::Listener {
    use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName as _};
    let name = path.to_fs_name::<GenericFilePath>().expect("socket name");
    ListenerOptions::new()
        .name(name)
        .create_sync()
        .expect("create_sync listener")
}

/// Send one request, read one response, and deliberately keep the socket
/// open so shutdown tests can prove the server does not require client EOF.
async fn client_request_keep_open(socket_path: &Path, req: &Request) -> (Response, AsyncStream) {
    let mut stream = connect_stream(socket_path).await;

    let mut line = serde_json::to_string(req).expect("serialize request");
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .await
        .expect("write request");

    let mut resp_line = String::new();
    {
        let mut reader = BufReader::new(stream.clone());
        reader
            .read_line(&mut resp_line)
            .await
            .expect("read response");
    }

    (
        serde_json::from_str(&resp_line).expect("parse response"),
        stream,
    )
}

/// Run one client request→response roundtrip against a daemon socket and
/// return the parsed response.
///
/// This mirrors the production client (`send_request_with_timeout`):
/// connect, write the request line, read exactly one response line, then
/// **drop the stream** so the server's read loop sees EOF and
/// `handle_connection` returns.
///
/// Tests must NOT instead half-close: the server never sees EOF on its read
/// half and the test hangs forever waiting on the server task. Dropping the
/// whole stream closes both halves and behaves identically on every
/// platform — which is also exactly what the real client does.
async fn client_roundtrip(socket_path: &Path, req: &Request) -> Response {
    let (response, stream) = client_request_keep_open(socket_path, req).await;
    drop(stream);
    response
}

/// Bind a fresh daemon socket, serve exactly one connection with
/// `handle_connection`, run a single client roundtrip against it, and
/// join the server task. Returns the parsed response.
///
/// Every socket integration test funnels through this so the
/// connect/serve/teardown ordering lives in one place and the macOS EOF
/// hang (see `client_roundtrip`) cannot be reintroduced piecemeal.
async fn one_shot_request(daemon: &Arc<Daemon>, socket_path: &Path, req: &Request) -> Response {
    // One endpoint per call: a test that asks for two roundtrips would
    // otherwise bind the same name twice (#1107). Both the listener and
    // the client below use the derived path, so callers keep passing the
    // socket path their config reports.
    let socket_path = &fresh_endpoint(socket_path);
    let listener = bind_listener(socket_path).await;

    let server_daemon = daemon.clone();
    let server = smol::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        handle_connection(stream, &server_daemon, &Arc::new(Lifecycle::default()))
            .await
            .expect("handle_connection");
    });

    let resp = client_roundtrip(socket_path, req).await;
    server.await;
    resp
}

#[test]
/// #131: the in-flight registry upserts by pid, deregisters on finish,
/// prunes dead/ancient entries, and snapshots with derived elapsed/ETA.
fn in_flight_registry_upserts_prunes_and_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::new(test_config(dir.path()));
    let now = unix_time_ms();
    // Use our own (certainly alive) pid so liveness pruning keeps it.
    let pid = std::process::id();

    daemon.handle_compile_started(CompileStartedRequest {
        crate_name: "gkrust".into(),
        root: "/w".into(),
        pid,
        started_at_ms: now.saturating_sub(10_000),
        typical_ms: None,
        client_epoch: 0,
        client_version: None,
    });
    // Upsert: the first-tick refresh with typical_ms replaces, not duplicates.
    daemon.handle_compile_started(CompileStartedRequest {
        crate_name: "gkrust".into(),
        root: "/w".into(),
        pid,
        started_at_ms: now.saturating_sub(10_000),
        typical_ms: Some(471_000),
        client_epoch: 0,
        client_version: None,
    });
    // An entry older than the max age is pruned even with a live pid
    // (PID reuse must not resurrect ghosts).
    daemon.handle_compile_started(CompileStartedRequest {
        crate_name: "ghost".into(),
        root: "/w".into(),
        pid: pid.wrapping_add(1),
        started_at_ms: now.saturating_sub(IN_FLIGHT_MAX_AGE_MS + 60_000),
        typical_ms: None,
        client_epoch: 0,
        client_version: None,
    });

    let snapshot = daemon.in_flight_snapshot();
    assert_eq!(
        snapshot.len(),
        1,
        "ghost pruned, upsert deduped: {snapshot:?}"
    );
    let entry = &snapshot[0];
    assert_eq!(entry.crate_name, "gkrust");
    assert_eq!(entry.pid, pid);
    assert!(entry.elapsed_s >= 10);
    assert_eq!(entry.typical_s, Some(471));
    assert_eq!(entry.eta_s, Some(471u64.saturating_sub(entry.elapsed_s)));

    // A stale Finished with a mismatched start token must NOT remove it.
    daemon.handle_compile_finished(&CompileFinishedRequest {
        pid,
        started_at_ms: 12345,
    });
    assert_eq!(daemon.in_flight_snapshot().len(), 1);
    daemon.handle_compile_finished(&CompileFinishedRequest {
        pid,
        started_at_ms: now.saturating_sub(10_000),
    });
    assert!(daemon.in_flight_snapshot().is_empty());
}

/// #131 wire shape: the new variants serialize under snake_case tags an
/// old daemon will reject as a parse error (fire-and-forget client
/// ignores), and StatsResponse's `in_flight` defaults for old daemons.
#[test]
fn compile_started_wire_tags_and_stats_default() {
    let req = Request::CompileStarted(CompileStartedRequest {
        crate_name: "c".into(),
        root: String::new(),
        pid: 1,
        started_at_ms: 2,
        typical_ms: None,
        client_epoch: 0,
        client_version: None,
    });
    let wire = serde_json::to_string(&req).unwrap();
    assert!(wire.contains("\"compile_started\""), "{wire}");
    let round: Request = serde_json::from_str(&wire).unwrap();
    assert_eq!(round, req);

    // A StatsResponse serialized by an OLD daemon (no in_flight field)
    // must deserialize with an empty registry view.
    let mut old = serde_json::to_value(StatsResponse {
        stores: Vec::new(),
        total_size: 0,
        max_size: 0,
        entry_count: 0,
        entries: None,
        events: EventStatsResponse {
            local_hits: 0,
            prefetch_hits: 0,
            remote_hits: 0,
            dups: 0,
            misses: 0,
            errors: 0,
            total_elapsed_ms: 0,
            hit_elapsed_ms: 0,
            miss_elapsed_ms: 0,
            hit_compile_time_ms: 0,
            miss_compile_time_ms: 0,
            store_output_blobs: 0,
            store_duplicate_blobs: 0,
            store_new_blobs: 0,
        },
        blob_stats: None,
        recent_summaries: Vec::new(),
        version: String::new(),
        build_epoch: 0,
        gc_policy_version: GC_POLICY_PROTOCOL_VERSION,
        in_flight: vec![InFlightEntry {
            crate_name: "x".into(),
            root: String::new(),
            pid: 1,
            elapsed_s: 1,
            typical_s: None,
            eta_s: None,
        }],
        effective_config: Some(EffectiveConfig {
            process: None,
            max_size: 1,
            cache_dir: "/c".into(),
            runtime_dir: "/c".into(),
            config_path: "/c/config.toml".into(),
            config_fingerprint: Some("fingerprint".into()),
                                        socket_path: "/c/daemon.sock".into(),
            started_at_ms: 1,
            auto_recover_min_free_bytes: None,
            auto_recover_targets: None,
        }),
    })
    .unwrap();
    {
        let old_obj = old.as_object_mut().unwrap();
        old_obj.remove("in_flight");
        old_obj.remove("blob_stats");
        old_obj.remove("recent_summaries");
        old_obj.remove("gc_policy_version");
    }
    let mut old_effective = old.get("effective_config").unwrap().clone();
    let old_effective_obj = old_effective.as_object_mut().unwrap();
    old_effective_obj.remove("runtime_dir");
    let parsed_effective: EffectiveConfig = serde_json::from_value(old_effective).unwrap();
    assert!(parsed_effective.runtime_dir.is_empty());
    // A pre-#689 daemon reports no effective config either; the CLI must
    // see `None` (and fall back to labeled client-config values), not a
    // parse error or a zeroed report.
    old.as_object_mut().unwrap().remove("effective_config");
    let parsed: StatsResponse = serde_json::from_value(old).unwrap();
    assert!(parsed.in_flight.is_empty());
    assert!(parsed.blob_stats.is_none());
    assert!(parsed.recent_summaries.is_empty());
    assert!(parsed.effective_config.is_none());
    assert_eq!(parsed.gc_policy_version, 0);
}

#[test]
fn test_shutdown_request_sets_flag_and_stores_notify_permit() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let socket_path = config.socket_path();
        std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();

        let listener = bind_listener(&socket_path).await;
        let daemon = Arc::new(Daemon::new(config));
        let lifecycle = Arc::new(Lifecycle::default());

        let server_daemon = daemon.clone();
        let server_flag = lifecycle.clone();

        let server = smol::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            handle_connection(stream, &server_daemon, &server_flag)
                .await
                .expect("handle_connection");
        });

        let resp = client_roundtrip(&socket_path, &Request::Shutdown).await;
        server.await;

        assert!(resp.ok, "stop request should return ok");
        assert!(
            !lifecycle.accepting_calls(),
            "stop request must close admission"
        );
        // A later observer must also see the shared drain transition.
        smol_opt_timeout(Duration::from_secs(1), lifecycle.draining())
            .await
            .expect("stop request must wake a later drain observer");
    });
}

#[test]
fn connection_handler_drain_aborts_silent_handler_at_deadline() {
    smol::block_on(async {
        let mut handlers = Vec::new();
        handlers.push(smol::spawn(std::future::pending::<()>()));

        assert!(
            drain_connection_handlers(&mut handlers, Duration::from_millis(20)).await,
            "a silent handler must be aborted at the drain deadline"
        );
        assert!(handlers.is_empty(), "aborted handlers must still be joined");
    });
}

#[test]
fn connection_handler_drain_completes_finished_handlers_without_abort() {
    smol::block_on(async {
        let mut handlers = Vec::new();
        handlers.push(smol::spawn(async {}));

        assert!(
            !drain_connection_handlers(&mut handlers, Duration::from_secs(1)).await,
            "finished handlers drain without cancellation"
        );
        assert!(handlers.is_empty(), "drained handlers must still be joined");
    });
}

/// Regression for #288 (loop side): a quiet `stop` must wake the accept loop
/// immediately rather than leaving it parked until the periodic idle tick.
/// We drive the real `accept_loop` with the idle timeout disabled and an
/// OS shutdown signal that never fires, so the *only* thing that can break
/// the loop within the assertion window is the stop-request wakeup. Before
/// the fix the loop would stay parked for `ACCEPT_LOOP_IDLE_TICK` (~60s) and
/// this 5s timeout would elapse.
#[test]
fn test_accept_loop_breaks_promptly_on_stop_request() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let socket_path = config.socket_path();
        std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();

        let listener = bind_listener(&socket_path).await;
        let daemon = Arc::new(Daemon::new(config));
        let lifecycle = Arc::new(Lifecycle::default());

        // Client sends a one-shot `stop` once the loop is up.
        let client_socket = socket_path.clone();
        let client =
            smol::spawn(async move { client_roundtrip(&client_socket, &Request::Shutdown).await });

        // `accept_loop` borrows the listener, so run it in this task (not a
        // spawned 'static one) under a timeout. `future::pending` stands in for
        // an OS shutdown signal that never arrives.
        let outcome = smol_opt_timeout(
            Duration::from_secs(5),
            accept_loop(
                &listener,
                &daemon,
                &lifecycle,
                None,
                Duration::from_secs(1),
                std::future::pending::<()>(),
            ),
        )
        .await;

        assert!(
            outcome.is_some(),
            "accept_loop did not break within 5s of a stop request (issue #288 regression)"
        );
        assert!(
            !lifecycle.accepting_calls(),
            "shutdown flag should be set after the stop request"
        );
        let resp = smol_opt_timeout(Duration::from_secs(5), client)
            .await
            .expect("accept loop never acknowledged shutdown");
        assert!(resp.ok, "stop request should return ok");
    });
}

#[test]
fn test_send_request_with_timeout_bounds_unresponsive_daemon() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("daemon.sock");
        let listener = bind_listener(&socket_path).await;

        let server = smol::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut request_line = String::new();
            {
                let mut reader = BufReader::new(stream.clone());
                reader
                    .read_line(&mut request_line)
                    .await
                    .expect("read request");
            }
            assert!(request_line.contains("\"stats\""));
            async_io::Timer::after(Duration::from_secs(1)).await;
            drop(stream);
        });

        let req = Request::Stats(StatsRequest {
            include_entries: false,
            include_summaries: false,
            sort_by: None,
            event_hours: None,
            event_secs: None,
            client_epoch: 0,
            client_version: None,
        });
        let client_socket_path = socket_path.clone();
        let started = Instant::now();
        let result = smol::unblock(move || {
            send_request_with_timeout(&client_socket_path, &req, Duration::from_millis(75))
        })
        .await;

        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_millis(750));
        let _ = server.cancel().await;
    });
}

pub(super) fn test_config(dir: &Path) -> Config {
    Config {
        readonly_store: None,
        fallback: None,
        key_salt: None,
        cc_extra_allowlist_flags: Vec::new(),
        modified_input_guard: false,
        input_predictions: false,
        record_sessions: false,
        volume_stores: Vec::new(),
        shared_hardlink_restores: false,
        deferred_discovery: true,
        out_dir_alias: true,
        out_dir_alias_root: None,
        deferred_durability: false,
        daemon_publish: false,
        trust_daemon_execution: false,
        daemon_execution_concurrency: None,
        project_rules: crate::config::ProjectRules::default(),
        auto_gc: true,
        index_auto_compact: true,
        auto_clean_orphaned_targets: false,
        auto_share_target_files: false,
        auto_clean_idle_targets_days: 0,
        auto_recover_min_free_bytes: 0,
        auto_recover_targets: false,
        scheduler_memory_pressure: true,
        auto_clean_unused_units_days: 0,
        seed_new_targets: false,
        build_script_hermetic: false,
        gc_evict_shared: false,
        storage_layout_advice: true,
        heartbeat_secs: 30,
        explain_miss: false,
        scheduler: true,
        test_lease: None,
        path_only_env_vars: Vec::new(),
        incremental_crates: Vec::new(),
        key_env_vars: Vec::new(),
        base_dirs: Vec::new(),
        cache_dir: dir.to_path_buf(),
        runtime_dir: dir.to_path_buf(),
        socket_path_override: None,
        max_size: 50 * 1024 * 1024, // 50 MiB
        disabled: false,
        cache_executables: false,
        cache_cc_links: false,
        trust_codegen_backends: false,
        target_liveness: false,
        clean_incremental: false,
        preserve_incremental: false,
        adaptive_incremental: true,
        seed_incremental: true,
        incremental_seed_from: None,
        unstable: Default::default(),
        event_log_max_size: 10 * 1024 * 1024,
        event_log_keep_lines: 1000,
        compression_level: 3,
        min_store_compile_ms: crate::config::DEFAULT_MIN_STORE_COMPILE_MS,
        gc_max_age_hours: crate::config::DEFAULT_GC_MAX_AGE_HOURS,
        daemon_idle_timeout_secs: crate::config::DEFAULT_DAEMON_IDLE_TIMEOUT_SECS,
    }
}













#[test]
fn request_release_controls_drain_and_legacy_timestamps_do_not() {
    smol::block_on(async {
        for (version, should_drain) in [
            (None, false),
            (Some(VERSION.to_owned()), false),
            (Some(format!("{VERSION}+rebuilt")), false),
            (Some("0.0.1".into()), false),
            (Some("legacy".into()), false),
            (Some("999.0.0".into()), true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let config = test_config(dir.path());
            let daemon = Arc::new(Daemon::new(config.clone()));
            let lifecycle = Arc::new(Lifecycle::default());
            let listener = bind_listener(&config.socket_path()).await;
            let (serving, control) = (daemon.clone(), lifecycle.clone());
            let server = smol::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                handle_connection(stream, &serving, &control).await.unwrap();
            });
            let response = client_roundtrip(
                &config.socket_path(),
                &Request::CompileStarted(CompileStartedRequest {
                    crate_name: "release-test".into(),
                    root: String::new(),
                    pid: 1234,
                    started_at_ms: 1,
                    typical_ms: None,
                    client_epoch: u64::MAX,
                    client_version: version.clone(),
                }),
            )
            .await;
            server.await;
            assert!(response.ok, "{response:?}");
            assert_eq!(!lifecycle.accepting_calls(), should_drain, "{version:?}");
            assert_eq!(
                daemon.in_flight_compiles.lock().unwrap().len(),
                1,
                "accepted work must survive"
            );
        }
    });
}

fn stats_at_version(version: &str) -> StatsResponse {
    serde_json::from_value(serde_json::json!({
        "total_size": 0, "max_size": 0, "entry_count": 0,
        "entries": null, "version": version, "build_epoch": 999,
        "events": { "local_hits": 0, "remote_hits": 0, "misses": 0, "errors": 0,
                    "total_elapsed_ms": 0 }
    }))
    .unwrap()
}

#[test]
fn diagnostic_stats_send_no_upgrade_metadata_to_legacy_daemons() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let listener = bind_listener(&config.socket_path()).await;
        let server = smol::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(stream.clone()).read_line(&mut line).await.unwrap();
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(value["stats"]["client_epoch"], 0);
            assert!(value["stats"].get("client_version").is_none());
            let response = Response {
                stats: Some(stats_at_version("0.0.1")),
                ..Response::ok()
            };
            let line = format!("{}\n", serde_json::to_string(&response).unwrap());
            stream.write_all(line.as_bytes()).await.unwrap();
        });
        let stats =
            smol::unblock(move || send_stats_request_without_restart(&config, false))
                .await
                .unwrap();
        server.await;
        assert_eq!(stats.version, "0.0.1");
    });
}

/// 1.0.x orders daemons by this value, so it must be the running
/// executable's modification time, not just any non-zero number.
#[test]
fn the_build_epoch_is_the_executables_mtime() {
    let modified = std::fs::metadata(std::env::current_exe().unwrap())
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(build_epoch(), modified);
}


#[test]
fn only_build_requests_count_as_activity() {
    assert!(!Request::Shutdown.is_build_activity());
    let probe = Request::ExecutionStatus {
        client_pid: 1,
        client_version: "fixture".into(),
        trusted_pipeline: false,
    };
    assert!(!probe.is_build_activity());
    assert_eq!(probe.client_version(), None);
    let stats: Request = serde_json::from_str(
        r#"{"stats":{"include_entries":false,"sort_by":null,"event_hours":null}}"#,
    )
    .unwrap();
    assert!(matches!(stats, Request::Stats(_)));
    assert!(!stats.is_build_activity());
    assert!(!Request::Gc(GcRequest::automatic(0)).is_build_activity());
    assert!(!Request::GcV2(GcRequest::automatic(0)).is_build_activity());
    assert!(Request::HashFiles(HashFilesRequest { files: Vec::new() }).is_build_activity());
}

#[test]
fn idle_timeout_truth_table() {
    assert_eq!(daemon_idle_timeout(0), None);
    assert_eq!(daemon_idle_timeout(1), Some(Duration::from_secs(1)));
    assert_eq!(daemon_idle_timeout(60), Some(Duration::from_secs(60)));
}

#[test]
fn foreground_startup_rejects_a_file_in_place_of_the_runtime_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.runtime_dir = dir.path().join("runtime");
    fs::write(&config.runtime_dir, b"not a directory").unwrap();
    let provenance = crate::config::config_file_provenance_at(dir.path().join("config.toml"));
    assert!(run_server(&config, &provenance, None).is_err());
}

#[test]
fn server_main_binds_socket_and_handles_shutdown() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path());
        config.daemon_idle_timeout_secs = 0;
        let socket_path = config.socket_path();
        let coord = DaemonCoordFile::for_socket(&socket_path);
        let server_config = config.clone();
        let provenance = crate::config::config_file_provenance_at(dir.path().join("config.toml"));
        let server = smol::spawn(async move {
            server_main(
                &server_config,
                &provenance,
                coord,
                None,
                DaemonCapacity::discover(server_config.daemon_execution_concurrency),
            )
            .await
        });

        let ready_socket = socket_path.clone();
        let ready = smol::unblock(move || {
            kunobi_daemon::readiness::wait_until(Instant::now() + Duration::from_secs(5), |deadline| {
                lifecycle_client::current_socket(&ready_socket, deadline)
            })
            .map(|proof| proof.is_some())
        })
        .await
        .unwrap();
        assert!(ready, "server_main must bind its configured socket");

        assert_eq!(
            read_daemon_state(&socket_path).unwrap().control_version,
            Some(kunobi_daemon::wire::VERSION),
            "stop must use the advertised binary DRAIN endpoint"
        );
        let shutdown_config = config.clone();
        smol::unblock(move || send_shutdown_request(&shutdown_config))
            .await
            .unwrap();

        let result = smol_opt_timeout(Duration::from_secs(10), server)
            .await
            .expect("server_main should stop after a shutdown request");
    assert!(
        result.is_ok(),
        "server_main should exit cleanly: {result:?}"
    );
        assert!(
            !socket_path.exists(),
            "server_main should remove its socket during shutdown"
        );
    });
}

// ── Protocol serde round-trips ───────────────────────────────


#[test]
fn readiness_rejects_an_accepting_socket_that_does_not_answer() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let _listener = bind_sync_listener(&socket);
    let start = Instant::now();
    assert!(
        lifecycle_client::current_socket(&socket, start + Duration::from_millis(100))
            .unwrap()
            .is_none()
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn readiness_requires_a_successful_compatible_health_response() {
    smol::block_on(async {
    for (response, accepted) in [
        (
            Response {
                health: Some(DaemonHealth {
                    version: VERSION.into(),
                    build_epoch: build_epoch(),
                }),
                ..Response::ok()
            },
            true,
        ),
        (
            Response {
                health: Some(DaemonHealth {
                    version: "0.0.1".into(),
                    build_epoch: 1,
                }),
                ..Response::ok()
            },
            false,
        ),
        (
            Response {
                health: Some(DaemonHealth {
                    version: String::new(),
                    build_epoch: 1,
                }),
                ..Response::ok()
            },
            true,
        ),
        (
            Response {
                health: Some(DaemonHealth {
                    version: "999.0.0".into(),
                    build_epoch: 1,
                }),
                ..Response::ok()
            },
            true,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("daemon.sock");
        let listener = bind_listener(&socket).await;
        let server = smol::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut request = String::new();
            BufReader::new(peer.clone()).read_line(&mut request).await.unwrap();
            assert!(matches!(
                serde_json::from_str::<Request>(&request).unwrap(),
                Request::Health
            ));
            let mut response = serde_json::to_vec(&response).unwrap();
            response.push(b'\n');
            peer.write_all(&response).await.unwrap();
        });
        let proof = smol::unblock(move || {
            lifecycle_client::current_socket(&socket, Instant::now() + Duration::from_secs(5))
        })
        .await
        .unwrap();
        smol_opt_timeout(Duration::from_secs(5), server)
            .await
            .expect("readiness probe never completed the health exchange");
        assert_eq!(proof.is_some(), accepted);
    }
    });
}

#[test]
fn decode_request_frame_strips_trailing_carriage_return() {
    assert_eq!(decode_request_frame(b"{\"x\":1}\r"), "{\"x\":1}");
    assert_eq!(decode_request_frame(b"{\"x\":1}"), "{\"x\":1}");
    assert_eq!(decode_request_frame(b""), "");
}

#[test]
fn is_client_disconnect_matches_disconnect_kinds() {
    use std::io::{Error, ErrorKind};
    assert!(is_client_disconnect(&Error::from(ErrorKind::BrokenPipe)));
    assert!(is_client_disconnect(&Error::from(
        ErrorKind::ConnectionReset
    )));
    assert!(is_client_disconnect(&Error::from_raw_os_error(32))); // EPIPE
    assert!(!is_client_disconnect(&Error::from(ErrorKind::NotFound)));
    assert!(!is_client_disconnect(&Error::from(ErrorKind::TimedOut)));
}

#[test]
fn request_failure_marks_only_withdrawn_handoffs_routine() {
    assert_eq!(request_failure(&Response::ok()), None);
    assert_eq!(
        request_failure(&Response::err("publish refused: queue full")),
        Some(("publish refused: queue full", false))
    );
    let withdrawn = "publish refused: the wrapper withdrew the hand-off first";
    assert_eq!(
        request_failure(&Response::err(withdrawn)),
        Some((withdrawn, true))
    );
}

/// Descriptor exhaustion pauses the accept loop; any other accept error
/// retries at once, as before.
#[cfg(unix)]
#[test]
fn accept_backoff_pauses_only_on_descriptor_exhaustion() {
    use std::io::{Error, ErrorKind};
    for errno in [libc::EMFILE, libc::ENFILE] {
        assert_eq!(
            accept_backoff(&Error::from_raw_os_error(errno)),
            Some(ACCEPT_EXHAUSTED_BACKOFF)
        );
    }
    assert_eq!(
        accept_backoff(&Error::from_raw_os_error(libc::ECONNABORTED)),
        None
    );
    assert_eq!(accept_backoff(&Error::from(ErrorKind::Other)), None);
}


#[test]
fn release_precedence_ignores_build_metadata_and_unknown_versions() {
    for (client, daemon, newer) in [
        ("1.0.0", "0.9.0", true),
        ("1.0.1", "1.0.0", true),
        ("1.0.0", "1.0.0-rc.1", true),
        ("1.0.0-rc.2", "1.0.0-rc.1", true),
        ("1.0.0+new", "1.0.0+old", false),
        ("1.0.0-rc.1", "1.0.0", false),
        ("1.0.0", "2.0.0", false),
        ("1.0.0", "", false),
        ("legacy", "1.0.0", false),
        ("1.0.0", "legacy", false),
    ] {
        assert_eq!(
            client_version_is_newer(client, daemon),
            newer,
            "{client} vs {daemon}"
        );
    }
    assert_eq!(
        serde_json::from_str::<DaemonHealth>("{}").unwrap().version,
        ""
    );
}



#[test]
fn rotate_daemon_log_if_large_truncates_only_oversized_logs() {
    // Branch: daemon startup log rotation size gate.
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.log");
    std::fs::write(&small, b"small log").unwrap();
    rotate_daemon_log_if_large(&small);
    assert_eq!(std::fs::read(&small).unwrap(), b"small log");

    let large = dir.path().join("large.log");
    std::fs::write(&large, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    rotate_daemon_log_if_large(&large);
    assert_eq!(std::fs::read(&large).unwrap(), b"--- log rotated ---\n");
}

#[test]
fn daemon_state_path_uses_state_json_extension() {
    assert_eq!(
        daemon_state_path(Path::new("/tmp/kache/daemon.sock")),
        Path::new("/tmp/kache/daemon.state.json")
    );
}

#[test]
fn daemon_state_is_recent_distinguishes_fresh_from_stale() {
    let fresh = DaemonCoordState {
        pid: 1,
        build_epoch: build_epoch(),
        phase: DaemonPhase::Ready,
        updated_at_ms: unix_time_ms(),
        control_version: None,
    };
    assert!(daemon_state_is_recent(&fresh));

    let stale = DaemonCoordState {
        control_version: None,
        pid: 1,
        build_epoch: build_epoch(),
        phase: DaemonPhase::Ready,
        updated_at_ms: unix_time_ms()
            .saturating_sub(DAEMON_COORD_STALE_AFTER.as_millis() as u64 * 2),
    };
    assert!(!daemon_state_is_recent(&stale));

    // Clock moved backwards: a record stamped in the future is not fresh.
    let future = DaemonCoordState {
        pid: 1,
        build_epoch: build_epoch(),
        phase: DaemonPhase::Ready,
        updated_at_ms: unix_time_ms() + DAEMON_COORD_STALE_AFTER.as_millis() as u64 * 2,
        control_version: None,
    };
    assert!(!daemon_state_is_recent(&future));
}

#[test]
fn coordinator_cleanup_removes_only_this_process_and_build() {
    let root = tempfile::tempdir().unwrap();
    for (same_pid, same_build) in [(true, true), (false, true), (true, false), (false, false)] {
        let mut coord = DaemonCoordFile::for_socket(&root.path().join("daemon.sock"));
        if !same_pid {
            coord.pid = std::process::id() + 1;
        }
        if !same_build {
            coord.build_epoch = build_epoch().wrapping_add(1);
        }
        coord.write_phase(DaemonPhase::Ready).unwrap();
        let bytes = std::fs::read(&coord.path).unwrap();
        drop(DaemonCoordGuard::new(coord.path.clone()));
        if same_pid && same_build {
            assert!(!coord.path.exists(), "own record must be removed");
        } else {
            assert_eq!(
                std::fs::read(&coord.path).unwrap(),
                bytes,
                "replacement record must survive"
            );
        }
    }
}

#[test]
fn test_daemon_coord_state_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("daemon.sock");
    let coord = DaemonCoordFile::for_socket(&socket_path);

    coord.write_phase(DaemonPhase::Starting).unwrap();
    let state = read_daemon_state(&socket_path).unwrap();
    assert_eq!(state.pid, std::process::id());
    assert_eq!(state.build_epoch, build_epoch());
    assert_eq!(state.phase, DaemonPhase::Starting);
    assert!(daemon_state_is_recent(&state));
}

#[cfg(unix)]
#[test]
fn pid_alive_rejects_broadcast_pids() {
    // This prunes the in-flight compile map. `kill(0, 0)` and `kill(-1, 0)`
    // succeed whenever anything signalable exists, so without the guard
    // entries recorded under a bogus PID would look alive forever.
    assert!(super::pid_alive(std::process::id()));

    assert!(!super::pid_alive(0), "0 is the caller's process group");
    assert!(!super::pid_alive(1), "1 is init/launchd, never a compile");
    assert!(
        !super::pid_alive(u32::MAX),
        "u32::MAX casts to the -1 broadcast"
    );
}

#[test]
fn shutdown_wait_requires_ownership_release() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("daemon.sock");
    let lock = kunobi_daemon::ProcessLock::try_acquire(daemon_run_lock_path(&socket))
        .unwrap()
        .unwrap();
    assert!(!wait_for_run_lock_release(&socket, Duration::from_millis(30)).unwrap());
    drop(lock);
    assert!(wait_for_run_lock_release(&socket, Duration::from_secs(1)).unwrap());
}

/// A missing run lock file means "nobody holds it", which is a different
/// answer from "the probe failed": callers treat an error as unknown and
/// stop, so collapsing the two would make a host that never ran a daemon
/// indistinguishable from one whose lock could not be read.
#[test]
fn existing_run_lock_probe_separates_missing_from_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("daemon.sock");

    assert!(!existing_daemon_run_lock_is_held(&socket_path).unwrap());
    // Answering must not have created the file it was asked about.
    assert!(!daemon_run_lock_path(&socket_path).exists());

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(daemon_run_lock_path(&socket_path))
        .unwrap();

    // Present but free.
    assert!(!existing_daemon_run_lock_is_held(&socket_path).unwrap());

    lock.try_lock().unwrap();
    assert!(existing_daemon_run_lock_is_held(&socket_path).unwrap());
}

/// A lock file that cannot be opened must surface as an error, not as
/// "nobody holds it".
///
/// The probe special-cases exactly one failure — `NotFound`, meaning the
/// file was never created — and propagates everything else. The test above
/// covers missing, present-and-free, and held, but never an unreadable
/// lock, so the `NotFound` guard could be widened to match every error and
/// nothing would notice: an unreadable lock would then read as free, and
/// doctor would report a host as clean precisely when it could not tell.
///
/// A directory where the file belongs is the portable way to fail an open
/// with something that is not `NotFound` (EISDIR) without depending on
/// running as an unprivileged user, which chmod-based variants do.
#[test]
fn existing_run_lock_probe_propagates_an_unreadable_lock() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("daemon.sock");
    std::fs::create_dir_all(daemon_run_lock_path(&socket_path)).unwrap();

    assert!(
        existing_daemon_run_lock_is_held(&socket_path).is_err(),
        "an unopenable lock file must not be reported as unheld"
    );
}

/// A starting record is a waiting hint only while its owner holds the lock.
#[test]
fn starting_daemon_epoch_reports_only_a_live_starting_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let socket_path = config.socket_path();
    std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();
    let state_path = daemon_state_path(&socket_path);

    // No coordinator file at all.
    assert_eq!(starting_daemon_epoch(&config), None);

    let mut state = DaemonCoordState {
        pid: std::process::id(),
        build_epoch: 4242,
        phase: DaemonPhase::Starting,
        updated_at_ms: unix_time_ms(),
        control_version: None,
    };
    write_json_atomically(&state_path, &state).unwrap();

    // A fresh record naming a live process is exactly what PID reuse looks
    // like. Without the run lock it must not read as a starting daemon — and
    // probing must not create the lock file, which `doctor` would then report
    // as leftover cruft.
    assert_eq!(starting_daemon_epoch(&config), None);
    assert!(!daemon_run_lock_path(&socket_path).exists());

    let run_lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(daemon_run_lock_path(&socket_path))
        .unwrap();
    run_lock.try_lock().unwrap();
    assert_eq!(starting_daemon_epoch(&config), Some(4242));

    // Already serving: the socket answers, so this is not the starting window.
    state.phase = DaemonPhase::Ready;
    write_json_atomically(&state_path, &state).unwrap();
    assert_eq!(starting_daemon_epoch(&config), None);

    // Starting, but the heartbeat went cold — a crashed starter, not a live one.
    state.phase = DaemonPhase::Starting;
    state.updated_at_ms =
        unix_time_ms().saturating_sub(DAEMON_COORD_STALE_AFTER.as_millis() as u64 * 2);
    write_json_atomically(&state_path, &state).unwrap();
    assert_eq!(starting_daemon_epoch(&config), None);

    // Fresh record, dead PID: the starter died between writing the file and
    // binding the socket. Use a reaped child rather than a made-up PID so
    // the process really is gone.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = child.id();
    child.wait().unwrap();

    state.updated_at_ms = unix_time_ms();
    state.pid = dead_pid;
    write_json_atomically(&state_path, &state).unwrap();
    assert_eq!(starting_daemon_epoch(&config), None);
}

#[test]
fn test_request_gc_serde() {
    let req = Request::Gc(GcRequest::explicit_age(168));
    let json = serde_json::to_string(&req).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, parsed);

    assert!(json.contains("\"gc\""));
    assert!(json.contains("\"max_age_hours\":168"));
}

#[test]
fn test_request_gc_null_age_serde() {
    let req = Request::Gc(GcRequest::legacy(None));
    let json = serde_json::to_string(&req).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(req, parsed);
    assert!(json.contains("\"max_age_hours\":null"));

    let old_wire: Request = serde_json::from_str(r#"{"gc":{"max_age_hours":null}}"#).unwrap();
    assert_eq!(old_wire, Request::Gc(GcRequest::legacy(None)));
}

#[test]
fn test_request_gc_automatic_carries_effective_age() {
    let req = Request::Gc(GcRequest::automatic(72));
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"mode\":\"automatic\""));
    assert!(json.contains("\"effective_max_age_hours\":72"));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
}

#[test]
fn gc_v2_is_atomic_compatibility_gate_for_old_daemons() {
    // Only the serde shape matters; the payloads are never read.
    #[allow(dead_code)]
    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum LegacyRequest {
        Gc(GcRequest),
        Stats(StatsRequest),
    }

    let req = Request::GcV2(GcRequest::automatic(72));
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"gc_v2\""));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
    assert!(
        serde_json::from_str::<LegacyRequest>(&json).is_err(),
        "a pre-v2 daemon must reject the request before mutation"
    );
}

#[test]
fn gc_rejects_any_response_without_policy_reporting() {
    let error = match gc_outcome_from_response(Response::ok_evicted(1)) {
        Ok(_) => panic!("legacy aggregate response must be rejected"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("omitted GC policy reporting"));
}

#[test]
fn daemon_start_requirement_distinguishes_success_and_failure() {
    assert!(require_daemon_started(true).is_ok());
    let error = match require_daemon_started(false) {
        Ok(()) => panic!("a failed daemon start must stop the request"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "could not reach or start daemon");
}




#[test]
fn test_response_ok_serde() {
    let resp = Response::ok();
    let json = serde_json::to_string(&resp).unwrap();
    assert_eq!(json, r#"{"ok":true}"#);
}

#[test]
fn test_response_ok_evicted_serde() {
    let resp = Response::ok_evicted(5);
    let json = serde_json::to_string(&resp).unwrap();
    assert_eq!(json, r#"{"ok":true,"evicted":5}"#);
}

#[test]
fn test_response_gc_skipped_serde() {
    let resp = Response::ok_gc_skipped(GcRunReport::skipped(GcRequestMode::Automatic).breakdown());
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: Response = serde_json::from_str(&json).unwrap();
    assert!(parsed.skipped);
    assert_eq!(parsed.evicted, Some(0));
    assert_eq!(parsed.gc.unwrap().mode, GcRequestMode::Automatic);
}






#[test]
fn test_invalid_request_json() {
    let result = serde_json::from_str::<Request>(r#"{"bogus": 42}"#);
    assert!(result.is_err());
}

// ── S3 Key Cache unit tests ──────────────────────────────────







// ── Daemon logic (no sockets) ────────────────────────────────

#[test]
fn test_handle_gc_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let daemon = Daemon::new(config);

    let resp = daemon.handle_gc(&GcRequest::automatic(daemon.config.gc_max_age_hours));
    assert!(resp.ok);
    assert_eq!(resp.evicted, Some(0));
    assert_eq!(resp.gc.as_ref().unwrap().mode, GcRequestMode::Automatic);
}

#[test]
fn gc_does_not_hold_the_daemon_store_mutex() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Arc::new(Daemon::new(test_config(dir.path())));
    daemon.with_store(|_| Ok(())).unwrap();
    let main_store = daemon.store.get().unwrap().lock().unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker_daemon = Arc::clone(&daemon);
    let worker = std::thread::spawn(move || {
        done_tx
            .send(worker_daemon.run_gc(
                GcPolicy::Automatic { max_age_hours: 0 },
                GcDriver::Requested,
            ))
            .unwrap();
    });

    let finished = done_rx.recv_timeout(Duration::from_secs(2));
    drop(main_store);
    worker.join().unwrap();
    assert!(
        finished
            .expect("GC must finish while the daemon store is in use")
            .is_ok()
    );
}

/// The daemon's GC sweeps shared build-script runs too: a sandbox a
/// crashed hermetic attempt left is gone after a run.
#[test]
fn daemon_gc_sweeps_hermetic_build_script_runs() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::new(test_config(dir.path()));
    let leftover = daemon
        .config
        .cache_dir
        .join("out-dirs/v2")
        .join("ab".repeat(16));
    std::fs::create_dir_all(leftover.join("out")).unwrap();
    daemon
        .run_gc(
            GcPolicy::Automatic { max_age_hours: 0 },
            GcDriver::Requested,
        )
        .unwrap();
    assert!(!leftover.exists());
}

/// The daemon's sweep is what flushes entries a miss stored without an
/// fsync: it marks them durable, does nothing once the queue is empty,
/// and stands aside while another flusher holds the lock.
#[test]
fn the_daemon_sweep_flushes_entries_pending_durability() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.deferred_durability = true;
    let store = Store::open(&config).unwrap();
    let output = dir.path().join("out.rlib");
    std::fs::write(&output, b"artifact-bytes").unwrap();
    store
        .put(
            "pending",
            "pending_crate",
            &["lib".to_string()],
            &[],
            "x86_64-unknown-linux-gnu",
            "dev",
            &[(output, "libout.rlib".to_string())],
            "",
            "",
        )
        .unwrap();
    assert_eq!(store.pending_durability().unwrap(), 1);

    // Another flusher holds the lock: the sweep leaves the entry alone.
    let held = store
        .try_durability_flush_lock()
        .unwrap()
        .expect("flush lock");
    let daemon = Daemon::new(config);
    daemon.flush_pending_durability();
    assert_eq!(
        store.pending_durability().unwrap(),
        1,
        "a held lock means another flusher is draining the queue"
    );
    drop(held);

    daemon.flush_pending_durability();
    assert_eq!(store.pending_durability().unwrap(), 0);
    // Nothing pending: the sweep is a no-op and takes no lock.
    daemon.flush_pending_durability();
    assert_eq!(store.pending_durability().unwrap(), 0);
    assert!(store.try_durability_flush_lock().unwrap().is_some());
}

/// A busy or missing shard must not prevent the main store and later shards
/// from flushing. Opening a missing volume would create its store directory.
#[test]
fn the_daemon_sweep_flushes_volume_shards_and_skips_busy_or_missing_stores() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(&dir.path().join("main"));
    config.deferred_durability = true;
    config.volume_stores = ["busy", "missing", "ready"]
        .into_iter()
        .map(|name| crate::config::VolumeStore {
            volume: dir.path().join(name).to_string_lossy().into_owned(),
            store: dir.path().join(name),
            max_size: None,
        })
        .collect();
    let main = Store::open(&config).unwrap();
    let busy = Store::open(&config.for_volume_store(&config.volume_stores[0], |_| None)).unwrap();
    let ready = Store::open(&config.for_volume_store(&config.volume_stores[2], |_| None)).unwrap();
    for (name, store) in [("main", &main), ("busy", &busy), ("ready", &ready)] {
        let output = dir.path().join(format!("{name}.rlib"));
        std::fs::write(&output, name.as_bytes()).unwrap();
        store
            .put(
                "pending",
                "pending_crate",
                &["lib".to_string()],
                &[],
                "x86_64-unknown-linux-gnu",
                "dev",
                &[(output, "libout.rlib".to_string())],
                "",
                "",
            )
            .unwrap();
        assert_eq!(store.pending_durability().unwrap(), 1);
    }
    let held = busy.try_durability_flush_lock().unwrap().unwrap();
    let missing = config.volume_stores[1].store.clone();
    let daemon = Daemon::new(config);
    daemon.flush_pending_durability();
    assert_eq!(main.pending_durability().unwrap(), 0);
    assert_eq!(busy.pending_durability().unwrap(), 1);
    assert_eq!(ready.pending_durability().unwrap(), 0);
    assert!(!missing.exists(), "an unmounted shard must not be created");
    drop(held);
    daemon.flush_pending_durability();
    assert_eq!(busy.pending_durability().unwrap(), 0);
}

#[test]
fn test_handle_gc_reports_lock_skip() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let store = Store::open(&config).unwrap();
    let _gc_lock = store.try_gc_lock().unwrap().expect("gc lock");
    let daemon = Daemon::new(config);

    let report = daemon
        .run_gc(
            GcPolicy::Automatic {
                max_age_hours: daemon.config.gc_max_age_hours,
            },
            GcDriver::Periodic,
        )
        .unwrap();
    assert!(report.total.skipped);
    assert_eq!(report.total.entries_evicted, 0);
}

#[test]
fn test_handle_gc_with_max_age() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let daemon = Daemon::new(config);

    let resp = daemon.handle_gc(&GcRequest::explicit_age(24));
    assert!(resp.ok);
    assert_eq!(resp.evicted, Some(0));
    let breakdown = resp.gc.unwrap();
    assert_eq!(breakdown.mode, GcRequestMode::ExplicitAge);
    assert_eq!(breakdown.duplicate.entries_evicted, 0);
    assert_eq!(breakdown.size.entries_evicted, 0);
}

#[test]
fn explicit_age_request_without_hours_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::new(test_config(dir.path()));
    let resp = daemon.handle_gc(&GcRequest {
        max_age_hours: None,
        mode: GcRequestMode::ExplicitAge,
        effective_max_age_hours: None,
    });
    assert!(!resp.ok);
    assert!(
        resp.error
            .as_deref()
            .unwrap()
            .contains("missing max_age_hours")
    );
}

#[test]
fn automatic_request_without_effective_age_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::new(test_config(dir.path()));
    let resp = daemon.handle_gc(&GcRequest {
        max_age_hours: None,
        mode: GcRequestMode::Automatic,
        effective_max_age_hours: None,
    });
    assert!(!resp.ok);
    assert!(
        resp.error
            .as_deref()
            .unwrap()
            .contains("missing effective_max_age_hours")
    );
}

#[test]
fn test_handle_gc_evicts_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());

    // Create a source file outside the store (put() copies it in)
    let src_file = dir.path().join("big.rlib");
    std::fs::write(&src_file, vec![0u8; 200]).unwrap();

    let store = Store::open(&config).unwrap();
    store
        .put(
            "testkey",
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(src_file.clone(), "lib.rlib".into())],
            "",
            "",
        )
        .unwrap();
    store.remove_clone_for_test(&src_file);
    assert!(store.contains("testkey"));
    assert!(store.total_size().unwrap() >= 200);
    // Age past the active-pin grace so eviction can claim it (a just-put
    // entry is "recently accessed" and pinned — kunobi-ninja/kache#326).
    store.set_last_accessed_for_test("testkey", "-1 hour");
    drop(store);

    // Now set max_size below the entry size so eviction triggers
    config.max_size = 100;

    let daemon = Daemon::new(config);
    let stats = daemon
        .run_gc(
            GcPolicy::Automatic {
                max_age_hours: daemon.config.gc_max_age_hours,
            },
            GcDriver::Requested,
        )
        .unwrap();
    assert!(
        stats.total.entries_evicted > 0,
        "should have evicted at least 1 entry"
    );
}

/// kunobi-ninja/kache#1126: the daemon sweep removes stale key locks and
/// records the housekeeping counts.
#[test]
fn automatic_gc_runs_store_housekeeping_and_records_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());

    // Two stale key locks with no entry, one claimed just now.
    std::fs::create_dir_all(config.store_dir()).unwrap();
    let lock_path = |seed: u8| {
        config
            .store_dir()
            .join(format!("{}.lock", blake3::hash(&[seed]).to_hex()))
    };
    for seed in [1, 2, 3] {
        std::fs::write(lock_path(seed), b"1").unwrap();
    }
    for seed in [1, 2] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(lock_path(seed))
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(7200))
            .unwrap();
    }

    let daemon = Daemon::new(config.clone());
    let report = daemon
        .run_gc(GcPolicy::Automatic { max_age_hours: 0 }, GcDriver::Periodic)
        .unwrap();
    assert_eq!(
        report.total.housekeeping,
        Some(crate::store::HousekeepingStats {
            key_locks_removed: 2,
            key_locks_remaining: 1,
            predictions_pruned: 0,
            file_hashes_pruned: 0,
        })
    );
    assert!(!lock_path(1).exists());
    assert!(!lock_path(2).exists());
    assert!(lock_path(3).exists());
    let recorded = crate::report::read_gc_stats(&config.cache_dir).expect("run recorded");
    assert_eq!(recorded.key_locks_removed, Some(2));
    assert_eq!(recorded.key_locks_remaining, Some(1));
    assert_eq!(recorded.predictions_pruned, Some(0));
    assert_eq!(recorded.file_hashes_pruned, Some(0));
}

/// kunobi-ninja/kache#711: automatic GC applies configured age retention
/// even while the store is below its size budget.
#[test]
fn automatic_gc_applies_configured_max_age_even_under_size_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1024 * 1024 * 1024;
    config.gc_max_age_hours = 1;

    let src_file = dir.path().join("stale.rlib");
    std::fs::write(&src_file, vec![0u8; 32]).unwrap();
    let store = Store::open(&config).unwrap();
    store
        .put(
            "stale_key",
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(src_file.clone(), "lib.rlib".into())],
            "",
            "",
        )
        .unwrap();
    store.remove_clone_for_test(&src_file);
    store.set_last_accessed_for_test("stale_key", "-2 hours");
    drop(store);

    let daemon = Daemon::new(config);
    let stats = daemon
        .run_gc(
            GcPolicy::Automatic {
                max_age_hours: daemon.config.gc_max_age_hours,
            },
            GcDriver::Requested,
        )
        .unwrap();
    assert_eq!(stats.total.entries_evicted, 1);
    let store = Store::open(&daemon.config).unwrap();
    assert!(!store.contains("stale_key"));
}

#[test]
fn automatic_gc_skips_age_eviction_when_max_age_hours_is_zero() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1024 * 1024 * 1024;
    config.gc_max_age_hours = 0;

    let src_file = dir.path().join("stale.rlib");
    std::fs::write(&src_file, vec![0u8; 32]).unwrap();
    let store = Store::open(&config).unwrap();
    store
        .put(
            "stale_key",
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(src_file, "lib.rlib".into())],
            "",
            "",
        )
        .unwrap();
    store.set_last_accessed_for_test("stale_key", "-2 hours");
    drop(store);

    let daemon = Daemon::new(config);
    let stats = daemon
        .run_gc(
            GcPolicy::Automatic {
                max_age_hours: daemon.config.gc_max_age_hours,
            },
            GcDriver::Requested,
        )
        .unwrap();
    assert_eq!(stats.total.entries_evicted, 0);
    let store = Store::open(&daemon.config).unwrap();
    assert!(store.contains("stale_key"));
}

#[test]
fn manual_automatic_gc_sends_effective_age_and_runs_age_before_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1_000; // physical 1,200; size target 900
    config.gc_max_age_hours = 0; // daemon startup policy differs from request

    let old_file = dir.path().join("old.rlib");
    let fresh_file = dir.path().join("fresh.rlib");
    std::fs::write(&old_file, vec![b'o'; 400]).unwrap();
    std::fs::write(&fresh_file, vec![b'f'; 800]).unwrap();
    let store = Store::open(&config).unwrap();
    store
        .put(
            "old_valuable",
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(old_file.clone(), "old.rlib".into())],
            "",
            "",
        )
        .unwrap();
    for _ in 0..1_000 {
        assert!(store.get("old_valuable").unwrap().is_some());
    }
    store
        .put(
            "fresh_cheap",
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(fresh_file.clone(), "fresh.rlib".into())],
            "",
            "",
        )
        .unwrap();
    store.remove_clone_for_test(&old_file);
    store.remove_clone_for_test(&fresh_file);
    store.set_last_accessed_for_test("old_valuable", "-2 hours");
    store.set_last_accessed_for_test("fresh_cheap", "-2 minutes");
    drop(store);

    let daemon = Daemon::new(config);
    let resp = daemon.handle_gc(&GcRequest::automatic(1));
    assert!(resp.ok);
    assert_eq!(resp.evicted, Some(1));
    let breakdown = resp.gc.expect("new daemon returns policy breakdown");
    assert_eq!(breakdown.mode, GcRequestMode::Automatic);
    assert_eq!(breakdown.age.entries_evicted, 1);
    assert_eq!(breakdown.size.entries_evicted, 0);

    let store = Store::open(&daemon.config).unwrap();
    assert!(!store.contains("old_valuable"));
    assert!(store.contains("fresh_cheap"));
}

#[test]
fn test_upload_triggered_eviction_respects_gc_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 100;

    let src_file = dir.path().join("big.rlib");
    std::fs::write(&src_file, vec![0u8; 200]).unwrap();

    let store = Store::open(&config).unwrap();
    store
        .put(
            "upload_evict_key",
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(src_file.clone(), "lib.rlib".into())],
            "",
            "",
        )
        .unwrap();
    store.remove_clone_for_test(&src_file);
    // Age past the active-pin grace so eviction can claim it
    // (kunobi-ninja/kache#326).
    store.set_last_accessed_for_test("upload_evict_key", "-1 hour");

    let gc_lock = store.try_gc_lock().unwrap().expect("gc lock");
    let daemon = Daemon::new(config);
    daemon.sweep_under_size_pressure().unwrap();
    assert!(
        store.contains("upload_evict_key"),
        "upload-triggered eviction must skip while gc.lock is held"
    );

    drop(gc_lock);
    daemon.sweep_under_size_pressure().unwrap();
    assert!(
        !store.contains("upload_evict_key"),
        "eviction should run once gc.lock is available"
    );
}

/// Upload-triggered eviction is a GC driver too: without a record, the
/// evictions it makes (and the ones it fails) never reach gc_stats.json.
#[test]
fn upload_triggered_eviction_records_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 100;

    let src_file = dir.path().join("big.rlib");
    std::fs::write(&src_file, vec![0u8; 200]).unwrap();
    let store = Store::open(&config).unwrap();
    store
        .put(
            "upload_evict_key",
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(src_file.clone(), "lib.rlib".into())],
            "",
            "",
        )
        .unwrap();
    store.remove_clone_for_test(&src_file);
    store.set_last_accessed_for_test("upload_evict_key", "-1 hour");

    Daemon::new(config).sweep_under_size_pressure().unwrap();

    let recorded = crate::report::read_gc_stats(dir.path()).expect("gc_stats.json written");
    assert_eq!(recorded.source, "daemon");
    assert_eq!(recorded.entries_evicted, 1);
}

/// Store an idle `size`-byte entry for the upload-eviction tests; with
/// `retained`, a target directory still hardlinks its blob.
fn put_upload_evict_entry(
    store: &Store,
    dir: &std::path::Path,
    key: &str,
    size: usize,
    retained: bool,
) {
    let src_file = dir.join(format!("{key}.rlib"));
    std::fs::write(&src_file, &key.repeat(size)[..size]).unwrap();
    store
        .put(
            key,
            "testcrate",
            &["lib".into()],
            &[],
            "host",
            "dev",
            &[(src_file.clone(), "lib.rlib".into())],
            "",
            "",
        )
        .unwrap();
    store.remove_clone_for_test(&src_file);
    // A retained entry is one a build is using: no sweep may evict it
    // yet, so it keeps the store over budget.
    let last_used = if retained { "+0 seconds" } else { "-1 hour" };
    store.set_last_accessed_for_test(key, last_used);
}

#[test]
#[cfg(unix)]
fn disk_pressure_reaches_main_and_shards_below_budget_and_respects_locks_and_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(&dir.path().join("main"));
    config.max_size = u64::MAX;
    let main = Store::open(&config).unwrap();
    let _volume = kache_fs::testutil::VolumeUsageGuard::new(
        dir.path(),
        Some(kache_fs::VolumeUsage {
            total: 1000,
            free: 10,
        }),
    );
    config.auto_recover_min_free_bytes = 100;
    let (shard_config, shard) = add_budgeted_shard(&mut config, dir.path(), "shard", u64::MAX);
    put_upload_evict_entry(&main, dir.path(), "main_old", 200, false);
    put_upload_evict_entry(&main, dir.path(), "main_recent", 200, true);
    put_upload_evict_entry(&shard, dir.path(), "shard_old", 200, false);
    let held = shard.try_gc_lock().unwrap().unwrap();
    let daemon = Daemon::new(config.clone());
    daemon.sweep_under_size_pressure().unwrap();
    assert!(!main.contains("main_old"));
    assert!(main.contains("main_recent"));
    assert!(shard.contains("shard_old"));
    assert!(!crate::disk_recovery::wanted(&config));
    assert!(crate::disk_recovery::wanted(&shard_config));
    let report = crate::report::read_gc_stats(&config.cache_dir).unwrap();
    assert_eq!(report.entries_evicted, 1);
    assert_eq!(report.entries_pinned, 1);
    drop(held);
    daemon.sweep_under_size_pressure().unwrap();
    assert!(!shard.contains("shard_old"));
    put_upload_evict_entry(&main, dir.path(), "later", 200, false);
    daemon.sweep_under_size_pressure().unwrap();
    assert!(
        main.contains("later"),
        "the persistent recovery interval holds"
    );
}

/// Hardlink `key`'s blob into a stand-in target directory, which then
/// holds its blocks.
fn hold_in_target_dir(store: &Store, dir: &std::path::Path, key: &str) {
    let meta = store.get(key).unwrap().unwrap();
    std::fs::hard_link(
        store.blob_path(&meta.files[0].hash),
        dir.join(format!("{key}-target.rlib")),
    )
    .unwrap();
    store.set_last_accessed_for_test(key, "-1 hour");
}

/// Bytes target directories hold are not pressure: a hint sweeps, finds
/// them held, evicts nothing else, and leaves no backoff to wait out
/// (#1206). It used to evict every other entry and back off.
#[test]
fn a_gc_hint_leaves_bytes_held_by_target_directories_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = Store::open(&config).unwrap();
    for i in 0..6 {
        let key = format!("held_{i}");
        put_upload_evict_entry(&store, dir.path(), &key, 200, false);
        hold_in_target_dir(&store, dir.path(), &key);
    }
    put_upload_evict_entry(&store, dir.path(), "evictable", 50, false);

    let daemon = Daemon::new(config.clone());
    assert!(daemon.handle_request_sync(&Request::GcHint).ok);
    assert!(store.contains("evictable") && store.contains("held_0"));
    let stats = crate::report::read_gc_stats(dir.path()).unwrap();
    assert_eq!(stats.entries_evicted, 0);
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        Some(0),
        "the held bytes are on record, with nothing to back off from"
    );
    assert!(!crate::wrapper::auto_gc_sweep_due(
        &config,
        store.physical_size().unwrap()
    ));
}

/// Every upload used to start another full sweep of a store the last
/// sweep had already failed to bring under budget.
#[test]
fn upload_triggered_eviction_waits_out_the_auto_gc_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = Store::open(&config).unwrap();
    for i in 0..6 {
        put_upload_evict_entry(&store, dir.path(), &format!("retained_{i}"), 200, true);
    }
    put_upload_evict_entry(&store, dir.path(), "evictable", 50, false);

    let daemon = Daemon::new(config.clone());
    daemon.sweep_under_size_pressure().unwrap();
    assert!(!store.contains("evictable"));
    assert!(store.contains("retained_0"));
    assert!(
        dir.path().join("auto-gc-backoff.json").exists(),
        "a sweep that leaves the store over budget records a backoff"
    );

    // Growth inside the slack: nothing a sweep could not already free.
    put_upload_evict_entry(&store, dir.path(), "next", 50, false);
    daemon.sweep_under_size_pressure().unwrap();
    assert!(
        store.contains("next"),
        "the next upload must not sweep again during the backoff"
    );
}

/// The post-upload check used to start at 100% of `max_size` while the
/// wrapper waited for 110%, so the two alternated on a store in between.
#[test]
fn upload_triggered_eviction_starts_one_byte_above_the_shared_trigger() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = Store::open(&config).unwrap();
    put_upload_evict_entry(&store, dir.path(), "at_the_trigger", 1100, false);
    assert_eq!(store.physical_size().unwrap(), 1100);
    assert!(!crate::wrapper::auto_gc_sweep_due(&config, 1100));

    let daemon = Daemon::new(config.clone());
    daemon.sweep_under_size_pressure().unwrap();
    assert!(store.contains("at_the_trigger"));
    assert!(crate::report::read_gc_stats(dir.path()).is_none());

    put_upload_evict_entry(&store, dir.path(), "x", 1, false);
    assert_eq!(store.physical_size().unwrap(), 1101);
    assert!(crate::wrapper::auto_gc_sweep_due(&config, 1101));
    daemon.sweep_under_size_pressure().unwrap();
    assert!(!store.contains("at_the_trigger"));
    assert!(crate::report::read_gc_stats(dir.path()).is_some());
}

/// Add a `[cache.volumes]` shard with its own `budget` to `config`, and
/// open its store, which gives it the index GC looks for.
fn add_budgeted_shard(
    config: &mut Config,
    dir: &std::path::Path,
    name: &str,
    budget: u64,
) -> (Config, Store) {
    let shard = crate::config::VolumeStore {
        volume: format!("/mnt/{name}/"),
        store: dir.join(name),
        max_size: Some(budget),
    };
    let shard_config = config.for_volume_store(&shard, |_| None);
    config.volume_stores.push(shard);
    let store = Store::open(&shard_config).unwrap();
    (shard_config, store)
}

/// kunobi-ninja/kache#974: shards used to grow without a budget, since
/// GC only ever saw the main store.
#[test]
fn requested_and_periodic_gc_sweep_a_shard_over_its_own_budget() {
    for driver in [GcDriver::Periodic, GcDriver::Requested] {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(&dir.path().join("main"));
        config.max_size = 1_000_000;
        let main = Store::open(&config).unwrap();
        put_upload_evict_entry(&main, dir.path(), "main_entry", 1200, false);
        let (over_config, over) = add_budgeted_shard(&mut config, dir.path(), "over", 1000);
        put_upload_evict_entry(&over, dir.path(), "over_a", 600, false);
        put_upload_evict_entry(&over, dir.path(), "over_b", 600, false);
        let (_, under) = add_budgeted_shard(&mut config, dir.path(), "under", 1_000_000);
        put_upload_evict_entry(&under, dir.path(), "under_entry", 1200, false);

        let report = Daemon::new(config)
            .run_gc(GcPolicy::Automatic { max_age_hours: 0 }, driver)
            .unwrap();
        assert_eq!(over.physical_size().unwrap(), 600, "{driver:?}");
        assert!(under.contains("under_entry"), "{driver:?}");
        assert!(main.contains("main_entry"), "{driver:?}");
        assert_eq!(
            report.total.entries_evicted, 0,
            "the report is the main store's"
        );
        let recorded = crate::report::read_gc_stats(&over_config.cache_dir)
            .expect("the shard's run is recorded in the shard");
        assert_eq!(recorded.entries_evicted, 1, "{driver:?}");
    }
}

#[test]
fn explicit_age_gc_reaches_shards_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(&dir.path().join("main"));
    let (_, shard) = add_budgeted_shard(&mut config, dir.path(), "shard", 1_000_000);
    put_upload_evict_entry(&shard, dir.path(), "stale", 10, false);
    shard.set_last_accessed_for_test("stale", "-48 hours");
    put_upload_evict_entry(&shard, dir.path(), "fresh", 10, false);

    Daemon::new(config)
        .run_gc(GcPolicy::ExplicitAge { hours: 24 }, GcDriver::Requested)
        .unwrap();
    assert!(!shard.contains("stale"));
    assert!(shard.contains("fresh"));
}

/// Each store has its own gc.lock: a busy main store does not stop the
/// shards from being swept, and a busy shard is left to its holder.
#[test]
fn gc_takes_each_stores_lock_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(&dir.path().join("main"));
    let (_, shard) = add_budgeted_shard(&mut config, dir.path(), "shard", 1000);
    put_upload_evict_entry(&shard, dir.path(), "shard_entry", 1200, false);
    let main = Store::open(&config).unwrap();
    let _main_lock = main.try_gc_lock().unwrap().expect("gc lock");
    let shard_lock = shard.try_gc_lock().unwrap().expect("gc lock");
    let daemon = Daemon::new(config);
    let policy = GcPolicy::Automatic { max_age_hours: 0 };

    let report = daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert!(report.total.skipped);
    assert!(shard.contains("shard_entry"), "the shard's lock is held");

    drop(shard_lock);
    let report = daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert!(report.total.skipped);
    assert!(!shard.contains("shard_entry"), "only the main lock is held");
}

#[test]
fn gc_skips_a_missing_shard_and_still_sweeps_the_main_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(&dir.path().join("main"));
    config.max_size = 1000;
    let main = Store::open(&config).unwrap();
    let missing = dir.path().join("unmounted");
    config.volume_stores = vec![crate::config::VolumeStore {
        volume: "/mnt/unmounted/".into(),
        store: missing.clone(),
        max_size: None,
    }];
    let daemon = Daemon::new(config);

    put_upload_evict_entry(&main, dir.path(), "main_a", 600, false);
    put_upload_evict_entry(&main, dir.path(), "main_b", 600, false);
    let report = daemon
        .run_gc(
            GcPolicy::Automatic { max_age_hours: 0 },
            GcDriver::Requested,
        )
        .unwrap();
    assert_eq!(report.total.entries_evicted, 1);

    put_upload_evict_entry(&main, dir.path(), "main_c", 600, false);
    daemon.sweep_under_size_pressure().unwrap();
    assert_eq!(main.physical_size().unwrap(), 600);
    assert!(!missing.exists(), "GC must not create a shard it skips");
}

/// The post-upload check and wrapper hints size each shard against its
/// own budget, and a shard's held bytes and backoff stay in that shard.
#[test]
fn size_pressure_sweeps_each_shard_with_its_own_budget_and_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(&dir.path().join("main"));
    config.max_size = 1_000_000;
    let main = Store::open(&config).unwrap();
    put_upload_evict_entry(&main, dir.path(), "main_entry", 1200, false);
    let (_, over) = add_budgeted_shard(&mut config, dir.path(), "over", 1000);
    put_upload_evict_entry(&over, dir.path(), "over_a", 600, false);
    put_upload_evict_entry(&over, dir.path(), "over_b", 600, false);
    let (held_config, held) = add_budgeted_shard(&mut config, dir.path(), "held", 1000);
    for i in 0..6 {
        let key = format!("held_{i}");
        put_upload_evict_entry(&held, dir.path(), &key, 200, false);
        hold_in_target_dir(&held, dir.path(), &key);
    }
    put_upload_evict_entry(&held, dir.path(), "evictable", 50, false);
    let daemon = Daemon::new(config.clone());

    daemon.sweep_under_size_pressure().unwrap();
    assert_eq!(over.physical_size().unwrap(), 600);
    assert!(main.contains("main_entry"));
    assert!(held.contains("evictable") && held.contains("held_0"));
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(&held_config.cache_dir),
        Some(0),
        "the shard's held bytes are on record in the shard"
    );
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(&config.cache_dir),
        None
    );
    assert!(!crate::wrapper::auto_gc_sweep_due(
        &held_config,
        held.physical_size().unwrap()
    ));
}

#[test]
fn size_pressure_leaves_a_shard_whose_gc_lock_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(&dir.path().join("main"));
    let (_, shard) = add_budgeted_shard(&mut config, dir.path(), "shard", 1000);
    put_upload_evict_entry(&shard, dir.path(), "shard_entry", 1200, false);
    let held = shard.try_gc_lock().unwrap().expect("gc lock");

    let daemon = Daemon::new(config);
    daemon.sweep_under_size_pressure().unwrap();
    assert!(shard.contains("shard_entry"));
    drop(held);
    daemon.sweep_under_size_pressure().unwrap();
    assert!(!shard.contains("shard_entry"));
}

#[test]
fn upload_triggered_eviction_clears_the_backoff_once_the_store_fits() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = Store::open(&config).unwrap();
    put_upload_evict_entry(&store, dir.path(), "evictable", 1200, false);
    crate::wrapper::record_auto_gc_outcome(&config, 1200, 0);
    crate::wrapper::expire_auto_gc_backoff_for_test(dir.path());

    Daemon::new(config).sweep_under_size_pressure().unwrap();
    assert!(!store.contains("evictable"));
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        None
    );
}

#[test]
fn gc_hints_coalesce_while_a_sweep_is_pending() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::new(test_config(dir.path()));
    assert!(daemon.claim_gc_hint());
    assert!(!daemon.claim_gc_hint(), "a pending sweep covers this hint");
    daemon.run_hinted_sweep();
    assert!(
        daemon.claim_gc_hint(),
        "a finished sweep releases the claim"
    );
}

#[test]
fn delegated_gc_uses_the_invocation_budget_without_changing_daemon_policy() {
    let dir = tempfile::tempdir().unwrap();
    let mut startup = test_config(dir.path());
    startup.auto_gc = false;
    startup.max_size = 10_000;
    let mut invocation = startup.clone();
    invocation.auto_gc = true;
    invocation.max_size = 1_000;
    let store = Store::open(&invocation).unwrap();
    put_upload_evict_entry(&store, dir.path(), "evictable", 1_200, false);

    let daemon = Daemon::new(startup);
    daemon
        .sweep_invocation_under_size_pressure(&invocation)
        .unwrap();
    assert!(!store.contains("evictable"));
    assert!(store.physical_size().unwrap() <= 1_000);
    assert!(!daemon.config.auto_gc);
    assert_eq!(daemon.config.max_size, 10_000);
}

#[test]
fn a_gc_hint_sweeps_under_size_pressure_and_honours_the_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = Store::open(&config).unwrap();
    for i in 0..6 {
        put_upload_evict_entry(&store, dir.path(), &format!("retained_{i}"), 200, true);
    }
    put_upload_evict_entry(&store, dir.path(), "evictable", 50, false);

    let daemon = Daemon::new(config);
    assert!(daemon.handle_request_sync(&Request::GcHint).ok);
    assert!(!store.contains("evictable"));
    assert_eq!(
        crate::report::read_gc_stats(dir.path()).unwrap().source,
        "daemon"
    );
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        Some(600),
        "a hinted sweep that leaves the store over budget records the backoff"
    );

    put_upload_evict_entry(&store, dir.path(), "next", 50, false);
    assert!(daemon.handle_request_sync(&Request::GcHint).ok);
    assert!(
        store.contains("next"),
        "a hint during the backoff is a no-op"
    );

    crate::wrapper::expire_auto_gc_backoff_for_test(dir.path());
    assert!(daemon.handle_request_sync(&Request::GcHint).ok);
    assert!(!store.contains("next"));
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        Some(1200)
    );
}

#[test]
fn a_gc_hint_does_nothing_at_the_trigger_and_clears_the_backoff_once_the_store_fits() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = Store::open(&config).unwrap();
    put_upload_evict_entry(&store, dir.path(), "at_the_trigger", 1100, false);
    let daemon = Daemon::new(config.clone());
    assert!(daemon.handle_request_sync(&Request::GcHint).ok);
    assert!(store.contains("at_the_trigger"));

    put_upload_evict_entry(&store, dir.path(), "x", 1, false);
    crate::wrapper::record_auto_gc_outcome(&config, 1101, 0);
    crate::wrapper::expire_auto_gc_backoff_for_test(dir.path());
    assert!(daemon.handle_request_sync(&Request::GcHint).ok);
    assert!(!store.contains("at_the_trigger"));
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        None
    );
}

#[test]
fn gc_hint_wire_name_and_ack_timeout_are_pinned() {
    assert_eq!(
        serde_json::to_string(&Request::GcHint).unwrap(),
        "\"gc_hint\""
    );
    assert!(Request::GcHint.is_build_activity());
    assert_eq!(GC_HINT_ACK_TIMEOUT, Duration::from_millis(500));
}

#[test]
fn a_gc_hint_counts_as_accepted_only_on_an_ok_reply() {
    assert!(gc_hint_accepted(Ok("{\"ok\":true}\n".into())));
    // A daemon from before the hint rejects the unknown request.
    assert!(!gc_hint_accepted(Ok(
        "{\"ok\":false,\"error\":\"invalid request\"}\n".into()
    )));
    assert!(!gc_hint_accepted(Ok("not json".into())));
    assert!(!gc_hint_accepted(Err(anyhow::anyhow!("timed out"))));
}

#[test]
fn send_gc_hint_reports_no_daemon_without_starting_one() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    assert!(!send_gc_hint(&config));
    assert!(!crate::transport::is_reachable(&config.socket_path()));
}

/// The daemon acknowledges the hint before it sweeps, then sweeps off the
/// connection.
#[test]
fn send_gc_hint_is_acknowledged_and_the_daemon_sweeps() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path());
        config.max_size = 1000;
        let socket_path = config.socket_path();
        std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();
        let store = Store::open(&config).unwrap();
        put_upload_evict_entry(&store, dir.path(), "evictable", 1200, false);

        let listener = bind_listener(&socket_path).await;
        let daemon = Arc::new(Daemon::new(config.clone()));
        let server = smol::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            handle_connection(stream, &daemon, &Arc::new(Lifecycle::default()))
                .await
                .expect("handle_connection");
        });

        let cfg = config.clone();
        let accepted = smol::unblock(move || send_gc_hint(&cfg)).await;
        assert!(accepted);
        // Bounded: if no hint ever reached the socket, `accept` would wait
        // forever and the test would hang instead of failing.
        smol_opt_timeout(Duration::from_secs(10), server)
            .await
            .expect("the daemon never received the hint");

        let deadline = Instant::now() + Duration::from_secs(10);
        while store.contains("evictable") {
            assert!(Instant::now() < deadline, "the hinted sweep never ran");
            async_io::Timer::after(Duration::from_millis(20)).await;
        }
    });
}

/// Entries for the periodic-sweep tests: a store over budget on retained
/// bytes, one entry past the age policy and one idle entry a size pass
/// would evict.
fn seed_periodic_gc_store(config: &Config, dir: &Path) -> Store {
    let store = Store::open(config).unwrap();
    for i in 0..6 {
        put_upload_evict_entry(&store, dir, &format!("retained_{i}"), 200, true);
    }
    put_upload_evict_entry(&store, dir, "old", 50, false);
    store.set_last_accessed_for_test("old", "-48 hours");
    put_upload_evict_entry(&store, dir, "idle", 50, false);
    store
}

/// Age eviction is retention policy, not size pressure: it stays on the
/// timer while the size pass waits out the shared backoff.
#[test]
fn periodic_gc_skips_its_size_pass_under_the_backoff_and_still_expires_by_age() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = seed_periodic_gc_store(&config, dir.path());
    crate::wrapper::record_auto_gc_outcome(&config, store.physical_size().unwrap(), 0);

    let daemon = Daemon::new(config);
    let policy = GcPolicy::Automatic { max_age_hours: 24 };
    let report = daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert_eq!(report.age.entries_evicted, 1);
    assert_eq!(report.size.entries_evicted, 0);
    assert!(!store.contains("old"));
    assert!(
        store.contains("idle"),
        "the size pass waits out the backoff"
    );

    // `kache gc` ignores the backoff and leaves it as it found it.
    let report = daemon.run_gc(policy, GcDriver::Requested).unwrap();
    assert_eq!(report.size.entries_evicted, 1);
    assert!(!store.contains("idle"));
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        Some(600)
    );
}

#[test]
fn periodic_gc_records_where_its_size_pass_left_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = seed_periodic_gc_store(&config, dir.path());

    let daemon = Daemon::new(config.clone());
    let policy = GcPolicy::Automatic { max_age_hours: 0 };
    let report = daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert_eq!(report.size.entries_evicted, 2);
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        Some(600),
        "the retained entries keep the store over budget"
    );

    crate::wrapper::expire_auto_gc_backoff_for_test(dir.path());
    daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        Some(1200)
    );

    // The builds finish with the retained entries: the next due sweep
    // fits the store.
    for i in 0..6 {
        store.set_last_accessed_for_test(&format!("retained_{i}"), "-1 hour");
    }
    crate::wrapper::expire_auto_gc_backoff_for_test(dir.path());
    daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert!(store.physical_size().unwrap() <= 900);
    assert_eq!(
        crate::wrapper::auto_gc_backoff_interval_for_test(dir.path()),
        None
    );
}

#[test]
fn periodic_gc_size_pass_starts_one_byte_above_the_shared_trigger() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.max_size = 1000;
    let store = Store::open(&config).unwrap();
    put_upload_evict_entry(&store, dir.path(), "at_the_trigger", 1100, false);

    let daemon = Daemon::new(config);
    let policy = GcPolicy::Automatic { max_age_hours: 0 };
    let report = daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert_eq!(report.size.entries_evicted, 0);
    assert!(store.contains("at_the_trigger"));

    put_upload_evict_entry(&store, dir.path(), "x", 1, false);
    let report = daemon.run_gc(policy, GcDriver::Periodic).unwrap();
    assert!(report.size.entries_evicted >= 1);
    assert!(!store.contains("at_the_trigger"));
}

#[test]
fn test_handle_request_sync_dispatches_gc() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let daemon = Daemon::new(config);

    let req = Request::Gc(GcRequest::automatic(daemon.config.gc_max_age_hours));
    let resp = daemon.handle_request_sync(&req);
    assert!(resp.ok);
    assert_eq!(resp.evicted, Some(0));
}

/// #281: the blocking handlers are dispatched through `offload`, which must
/// return the handler's own response unchanged.
#[test]
fn offload_returns_the_handler_response() {
    smol::block_on(async {
        let resp = offload(Response::ok).await;
        assert!(resp.ok);
    });
}

/// #281: a panic inside an offloaded handler must surface as an error
/// response, not unwind and tear down the connection task.
#[test]
fn offload_maps_a_handler_panic_to_an_error_response() {
    smol::block_on(async {
        let resp = offload(|| panic!("handler boom")).await;
        assert!(!resp.ok, "a panicking handler must yield an error response");
        assert!(
            resp.error
                .as_deref()
                .unwrap_or_default()
                .contains("task failed"),
            "error should explain the handler task failed, got {:?}",
            resp.error
        );
    });
}








/// Stats carry whether the daemon skips uploads, so `kache stats` can warn
/// a writable process. A daemon from before the field reads as unknown.
#[test]
fn test_run_gc_returns_count() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let daemon = Daemon::new(config);

    let stats = daemon
        .run_gc(
            GcPolicy::Automatic {
                max_age_hours: daemon.config.gc_max_age_hours,
            },
            GcDriver::Requested,
        )
        .unwrap();
    assert_eq!(stats.total.entries_evicted, 0);
}

#[test]
fn gc_pinned_lower_bound_is_policy_aware() {
    assert_eq!(
        gc_entries_pinned_lower_bound(GcPolicy::ExplicitAge { hours: 24 }, 9, 2, 8),
        2
    );

    for (duplicate, age, size) in [(7, 2, 3), (2, 7, 3), (2, 3, 7)] {
        assert_eq!(
            gc_entries_pinned_lower_bound(
                GcPolicy::Automatic { max_age_hours: 24 },
                duplicate,
                age,
                size,
            ),
            7
        );
    }
    assert_eq!(
        gc_entries_pinned_lower_bound(GcPolicy::Automatic { max_age_hours: 0 }, 0, 0, 0,),
        0
    );
}

#[test]
fn test_run_gc_cleans_registered_incremental_dirs_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    config.clean_incremental = true;
    let incremental_dir = dir.path().join("workspace/target/debug/incremental");
    std::fs::create_dir_all(&incremental_dir).unwrap();
    std::fs::write(incremental_dir.join("junk"), b"tmp").unwrap();

    let store = Store::open(&config).unwrap();
    store.remember_incremental_dir(&incremental_dir).unwrap();
    drop(store);

    let daemon = Daemon::new(config.clone());
    let stats = daemon
        .run_gc(
            GcPolicy::Automatic {
                max_age_hours: daemon.config.gc_max_age_hours,
            },
            GcDriver::Requested,
        )
        .unwrap();
    assert_eq!(stats.total.entries_evicted, 0);
    assert!(!incremental_dir.exists());

    std::fs::create_dir_all(&incremental_dir).unwrap();
    std::fs::write(incremental_dir.join("junk"), b"tmp2").unwrap();

    let stats = daemon
        .run_gc(
            GcPolicy::Automatic {
                max_age_hours: daemon.config.gc_max_age_hours,
            },
            GcDriver::Requested,
        )
        .unwrap();
    assert_eq!(stats.total.entries_evicted, 0);
    assert!(incremental_dir.exists());
}

#[test]
fn clean_tool_version_caches_removes_only_old_tool_version_txt() {
    // Branch: old rustc/linker version-cache file cleanup.
    let dir = tempfile::tempdir().unwrap();
    let old_rustc = dir.path().join("rustc-ver-old.txt");
    let old_linker = dir.path().join("linker-ver-old.txt");
    let fresh_rustc = dir.path().join("rustc-ver-fresh.txt");
    let old_other = dir.path().join("other-ver-old.txt");
    for path in [&old_rustc, &old_linker, &fresh_rustc, &old_other] {
        std::fs::write(path, b"version").unwrap();
    }

    let old = filetime::FileTime::from_system_time(
        std::time::SystemTime::now() - Duration::from_secs(8 * 24 * 3600),
    );
    for path in [&old_rustc, &old_linker, &old_other] {
        filetime::set_file_mtime(path, old).unwrap();
    }

    Daemon::clean_tool_version_caches(dir.path());

    assert!(!old_rustc.exists());
    assert!(!old_linker.exists());
    assert!(fresh_rustc.exists());
    assert!(old_other.exists());
}

// ── Socket integration tests ─────────────────────────────────

#[test]
fn test_socket_gc_roundtrip() {
    smol::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let socket_path = config.socket_path();
        std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();

        let daemon = Arc::new(Daemon::new(config));
        let resp = one_shot_request(
            &daemon,
            &socket_path,
            &Request::Gc(GcRequest::automatic(daemon.config.gc_max_age_hours)),
        )
        .await;

        assert!(resp.ok);
        assert_eq!(resp.evicted, Some(0));
    });
}


#[test]
fn test_send_request_to_nonexistent_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("nonexistent.sock");

    let req = Request::Gc(GcRequest::automatic(0));
    let result = send_request_with_timeout(&socket_path, &req, Duration::from_secs(30));
    assert!(result.is_err());
}


