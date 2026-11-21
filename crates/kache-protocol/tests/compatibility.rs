use kache_protocol::{
    CompileFinishedRequest, CompileStartedRequest, DaemonHealth, EffectiveConfig, GcRequest,
    HashFilesRequest, PrestageRequest, Request, StatsRequest, TransferEvent,
};
use deser_value::{from_value, to_value, value};

#[test]
fn legacy_compile_notifications_keep_their_defaults() {
    let started: CompileStartedRequest = from_value(&value!({
        "crate_name": "example", "pid": 42, "started_at_ms": 123
    }))
    .unwrap();
    assert_eq!(
        to_value(&started).unwrap(),
        value!({
            "crate_name": "example", "root": "", "pid": 42,
            "started_at_ms": 123, "typical_ms": null, "client_epoch": 0
        })
    );
    let finished: CompileFinishedRequest = from_value(&value!({"pid": 42})).unwrap();
    assert_eq!(finished.started_at_ms, 0);
    let current = CompileFinishedRequest {
        pid: 42,
        started_at_ms: 123,
    };
    assert_eq!(
        to_value(&current).unwrap(),
        value!({"pid": 42, "started_at_ms": 123})
    );
}

#[test]
fn hash_requests_preserve_fingerprints_and_accept_pre_inode_clients() {
    let request: HashFilesRequest = from_value(&value!({"files": [
        {"path": "/old", "size": 9, "mtime_ns": 10, "ctime_ns": 11},
        {"path": "/new", "size": 12, "mtime_ns": 13, "ctime_ns": 14, "inode": 15}
    ]}))
    .unwrap();
    assert_eq!(
        to_value(&request).unwrap(),
        value!({"files": [
            {"path": "/old", "size": 9, "mtime_ns": 10, "ctime_ns": 11, "inode": 0},
            {"path": "/new", "size": 12, "mtime_ns": 13, "ctime_ns": 14, "inode": 15}
        ]})
    );
}

#[test]
fn legacy_health_and_configuration_remain_distinguishable() {
    let health: DaemonHealth = from_value(&value!({})).unwrap();
    assert_eq!(health.version, "");
    let config: EffectiveConfig = from_value(&value!({
        "max_size": 1024, "cache_dir": "/cache", "runtime_dir": "/run",
        "config_path": "/config", "prefetch_enabled": true,
        "remote_description": null, "local_only": true,
        "socket_path": "/run/daemon.sock", "started_at_ms": 123
    }))
    .unwrap();
    assert_eq!(config.config_fingerprint, None);
}

#[test]
fn old_transfer_records_keep_their_schema_and_direction() {
    let transfer: TransferEvent = from_value(&value!({
        "direction": "upload", "crate_name": "example",
        "compressed_bytes": 42, "elapsed_ms": 7, "ok": true, "timestamp": 123
    }))
    .unwrap();
    assert_eq!(transfer.schema, 5);
}

#[test]
fn complete_envelopes_keep_legacy_gc_shape() {
    use kache_protocol::{GcRequestMode, Request};
    let request: Request = from_value(&value!({"gc": {"max_age_hours": 12}})).unwrap();
    let Request::Gc(gc) = &request else {
        panic!("expected legacy gc")
    };
    assert_eq!(gc.mode, GcRequestMode::Legacy);
    assert_eq!(gc.effective_max_age_hours, None);
    assert!(!request.is_build_activity());
    assert_eq!(
        to_value(&request).unwrap(),
        value!({
            "gc": {"max_age_hours": 12, "mode": "legacy"}
        })
    );
}

#[test]
fn only_build_requests_keep_the_machine_busy_and_only_some_declare_a_release() {
    let stats = |client_version: Option<&str>| StatsRequest {
        include_entries: false,
        include_summaries: false,
        sort_by: None,
        event_hours: None,
        event_secs: None,
        client_epoch: 0,
        client_version: client_version.map(str::to_owned),
    };
    let started: CompileStartedRequest = from_value(&value!({
        "crate_name": "example", "pid": 42, "started_at_ms": 123, "client_version": "2.0.0"
    }))
    .unwrap();
    let gc = || GcRequest::automatic(1);
    let probe = Request::ExecutionStatus {
        client_pid: 7,
        client_version: "2.0.0".into(),
        trusted_pipeline: false,
    };
    let finished = CompileFinishedRequest {
        pid: 42,
        started_at_ms: 123,
    };
    let prestage = PrestageRequest {
        target_dir: "/target".into(),
    };
    let hash_files = HashFilesRequest { files: Vec::new() };

    for (request, busy, release) in [
        (Request::Stats(stats(Some("2.0.0"))), false, Some("2.0.0")),
        (Request::Stats(stats(None)), false, None),
        (Request::CompileStarted(started), true, Some("2.0.0")),
        (Request::CompileFinished(finished), true, None),
        (Request::HashFiles(hash_files), true, None),
        (Request::Prestage(prestage), true, None),
        (Request::GcHint, true, None),
        (Request::Gc(gc()), false, None),
        (Request::GcV2(gc()), false, None),
        (probe, false, None),
        (Request::Health, false, None),
        (Request::Shutdown, false, None),
    ] {
        assert_eq!(request.is_build_activity(), busy, "{request:?}");
        assert_eq!(request.client_version(), release, "{request:?}");
    }
}
