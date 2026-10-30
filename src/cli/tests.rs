use super::*;
use std::fs;

#[test]
fn doctor_link_layout_uses_the_mapped_store_and_leaves_missing_shards_absent() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::test_support::test_config(dir.path().join("main"));
    let build = dir.path().join("build");
    fs::create_dir(&build).unwrap();
    let volume = format!("{}{}", build.display(), std::path::MAIN_SEPARATOR);
    let mapping = crate::config::VolumeStore {
        volume,
        store: dir.path().join("shard"),
        max_size: Some(1000),
    };
    config.volume_stores.push(mapping.clone());
    let missing = doctor_link_layout(&config, &build);
    assert!(!missing.pass);
    assert!(missing.detail.contains("unavailable; link probe skipped"));
    assert!(!mapping.store.exists());

    Store::open(&config.for_volume_store(&mapping, |_| None)).unwrap();
    let mapped = doctor_link_layout(&config, &build);
    assert!(mapped.pass, "{}", mapped.detail);
    let store = crate::link_probe::shown_path(&mapping.store);
    assert!(mapped.detail.contains(&store), "{}", mapped.detail);
    assert!(mapped.fix.is_none());

    config.volume_stores.clear();
    let unmapped = doctor_link_layout(&config, &build);
    assert!(unmapped.pass, "{}", unmapped.detail);
    assert!(
        unmapped
            .detail
            .contains(&crate::link_probe::shown_path(&config.cache_dir))
    );
    let failed = doctor_link_layout(&config, &dir.path().join("absent-build"));
    assert!(!failed.pass);
    assert!(failed.fix.unwrap().contains("SAME mount"));
}

#[test]
fn detail_fields_render_only_nonempty_values() {
    let mut lines = Vec::new();
    push_nonempty_detail(&mut lines, "  Type:     ", "");
    assert!(lines.is_empty());

    push_nonempty_detail(&mut lines, "  Type:     ", "lib");
    push_nonempty_detail(&mut lines, "  Features: ", "serde, std");
    assert_eq!(
        lines,
        vec![
            "  Type:     lib".to_string(),
            "  Features: serde, std".to_string(),
        ]
    );
}

struct FailOnWrite {
    fail_on_call: usize,
    calls: usize,
}

impl std::io::Write for FailOnWrite {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.calls += 1;
        if self.calls == self.fail_on_call {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "pager exited",
            ))
        } else {
            Ok(buffer.len())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn pager_line_writer_stops_on_content_or_newline_errors() {
    let lines = vec!["first".to_string(), "second".to_string()];
    let mut output = Vec::new();
    assert!(write_pager_lines(&mut output, &lines));
    assert_eq!(output, b"first\nsecond\n");

    let mut content_error = FailOnWrite {
        fail_on_call: 1,
        calls: 0,
    };
    assert!(!write_pager_lines(&mut content_error, &lines));
    assert_eq!(content_error.calls, 1);

    let mut newline_error = FailOnWrite {
        fail_on_call: 2,
        calls: 0,
    };
    assert!(!write_pager_lines(&mut newline_error, &lines));
    assert_eq!(newline_error.calls, 2);
}

#[test]
fn pager_resolution_obeys_tty_disable_and_environment_precedence() {
    assert_eq!(
        resolve_pager_argv(false, true, Some("most -R"), Some("less -S")),
        Some(vec!["most".to_string(), "-R".to_string()])
    );
    assert_eq!(
        resolve_pager_argv(false, true, None, Some("less -S")),
        Some(vec!["less".to_string(), "-S".to_string()])
    );
    assert_eq!(resolve_pager_argv(true, true, Some("less"), None), None);
    assert_eq!(resolve_pager_argv(false, false, Some("less"), None), None);

    // A present but empty KACHE_PAGER wins over PAGER and disables paging.
    assert_eq!(
        resolve_pager_argv(false, true, Some(""), Some("less")),
        None
    );
    assert_eq!(
        resolve_pager_argv(false, true, Some("cat"), Some("less")),
        None
    );
    assert_eq!(resolve_pager_argv(false, true, None, Some("cat")), None);
}

#[test]
fn pager_resolution_uses_platform_defaults() {
    assert_eq!(
        resolve_pager_argv(false, true, None, None),
        Some(vec!["less".to_string(), "-FRX".to_string()])
    );
}

#[test]
fn pager_resolution_groups_quotes_without_shell_evaluation() {
    assert_eq!(
        resolve_pager_argv(
            false,
            true,
            Some(r#""C:\Program Files\Git\usr\bin\less.exe" -FRX"#),
            None
        ),
        Some(vec![
            r"C:\Program Files\Git\usr\bin\less.exe".to_string(),
            "-FRX".to_string()
        ])
    );
    assert_eq!(
        resolve_pager_argv(
            false,
            true,
            Some("less --prompt='literal ; $HOME | text'"),
            None
        ),
        Some(vec![
            "less".to_string(),
            "--prompt=literal ; $HOME | text".to_string()
        ])
    );
    assert_eq!(
        resolve_pager_argv(false, true, Some("less 'unterminated"), None),
        None
    );
}

// ── Doctor check dispositions (kunobi-ninja/kache#443, #626) ──────────

/// Which failing checks are informational: the full truth table for
/// daemon-optional and probe-no-compiler downgrades.
/// `kache stats` names the host file only while it is merged in.
#[test]
fn stats_names_the_host_config_only_when_in_effect() {
    use crate::config::HostConfigStatus;
    let path = std::path::PathBuf::from("/etc/kache/config.toml");
    assert_eq!(
        host_config_in_effect(&HostConfigStatus::Present {
            path: path.clone(),
            keys: Vec::new(),
        })
        .as_deref(),
        Some("/etc/kache/config.toml")
    );
    assert_eq!(host_config_in_effect(&HostConfigStatus::Disabled), None);
    assert_eq!(
        host_config_in_effect(&HostConfigStatus::Absent { path: path.clone() }),
        None
    );
    assert_eq!(
        host_config_in_effect(&HostConfigStatus::Invalid {
            path,
            error: "parsing host config".to_string(),
        }),
        None
    );
}
#[test]
fn doctor_host_config_check_wording() {
    use crate::config::{HostConfigKey, HostConfigStatus, HostKeySource};
    let path = std::path::PathBuf::from("/etc/kache/config.toml");

    assert!(doctor_host_config_check(&HostConfigStatus::Disabled).0);

    let (pass, detail, fix) =
        doctor_host_config_check(&HostConfigStatus::Absent { path: path.clone() });
    assert!(pass);
    assert!(
        detail.contains("none at /etc/kache/config.toml"),
        "{detail}"
    );
    assert!(fix.is_none());

    let (pass, detail, fix) = doctor_host_config_check(&HostConfigStatus::Invalid {
        path: path.clone(),
        error: "parsing host config".to_string(),
    });
    assert!(!pass);
    assert!(detail.contains("is ignored"), "{detail}");
    assert!(fix.unwrap().contains("fix or remove"));

    let (pass, detail, _) = doctor_host_config_check(&HostConfigStatus::Present {
        path,
        keys: vec![
            HostConfigKey {
                key: "cache.input_predictions".to_string(),
                source: HostKeySource::Host,
            },
            HostConfigKey {
                key: "cache.local_max_size".to_string(),
                source: HostKeySource::ChosenFile("/job/kache-action.toml".into()),
            },
            HostConfigKey {
                key: "cache.explain_miss".to_string(),
                source: HostKeySource::Env("KACHE_EXPLAIN_MISS"),
            },
        ],
    });
    assert!(pass);
    assert!(detail.contains("cache.input_predictions, "), "{detail}");
    assert!(
        detail.contains("cache.local_max_size (overridden by /job/kache-action.toml)"),
        "{detail}"
    );
    assert!(
        detail.contains("cache.explain_miss (overridden by KACHE_EXPLAIN_MISS)"),
        "{detail}"
    );
}

#[test]
fn doctor_install_path_check_wording() {
    let mut selection = kache_shims::Selection {
        path: "/home/me/.nix-profile/bin/kache".into(),
        kind: kache_shims::Kind::Nix,
        stability: kache_shims::Stability::InstallerManaged,
        reason:
            "the Nix profile in /home/me/.nix-profile/bin, which each upgrade or rollback repoints"
                .into(),
    };
    let (pass, detail, fix) = doctor_install_path_check(&Ok(selection.clone()));
    assert!(pass);
    assert_eq!(
        detail,
        "/home/me/.nix-profile/bin/kache (Nix, installer-managed): the Nix profile in \
         /home/me/.nix-profile/bin, which each upgrade or rollback repoints"
    );
    assert_eq!(fix, None);

    selection.stability = kache_shims::Stability::Versioned;
    let (pass, _, fix) = doctor_install_path_check(&Ok(selection));
    assert!(!pass);
    assert!(fix.unwrap().contains("kache daemon install"));

    let error = kache_shims::Error::Replaced("/usr/bin/kache".into());
    let (pass, detail, fix) = doctor_install_path_check(&Err(error));
    assert!(!pass);
    assert!(detail.contains("was replaced"), "{detail}");
    assert_eq!(fix, None);
}

#[test]
fn doctor_check_optionality_truth_table() {
    assert!(doctor_check_is_optional("Compiler execution", false, false));
    assert!(doctor_check_is_optional("Compiler execution", true, true));
    assert_eq!(doctor_section("Compiler execution"), "Services");
    // An install path an upgrade may break is information, not an issue.
    assert!(doctor_check_is_optional("Install path", false, false));
    // So is a build that could reuse another checkout with a setting changed.
    assert!(doctor_check_is_optional(OTHER_CHECKOUTS, false, false));
    // Daemon labels downgrade exactly when the daemon is optional.
    assert!(doctor_check_is_optional("Daemon version", true, false));
    assert!(!doctor_check_is_optional("Daemon version", false, false));
    // The compiler probe downgrades exactly when there is no cc at all.
    assert!(doctor_check_is_optional("Compiler probe", false, true));
    assert!(!doctor_check_is_optional("Compiler probe", false, false));
    // probe_no_compiler must not leak onto other labels, nor
    // daemon_optional onto the probe.
    assert!(!doctor_check_is_optional("Binary", true, true));
    assert!(!doctor_check_is_optional("Daemon version", false, true));
    assert!(!doctor_check_is_optional("Compiler probe", true, false));
    // Non-daemon, non-probe labels are never optional, except C/C++
    // shims: PATH masquerade is opt-in and rust-only setups must not
    // fail doctor for skipping it.
    assert!(!doctor_check_is_optional("Remote", true, true));
    assert!(doctor_check_is_optional("C/C++ shims", false, false));
    assert!(doctor_check_is_optional("C/C++ shims", true, true));
}

#[test]
fn daemon_service_is_satisfied_by_install_or_healthy_on_demand_daemon() {
    assert_eq!(daemon_service_check(true, false), (true, None));
    assert_eq!(daemon_service_check(false, true), (true, None));
    assert_eq!(
        daemon_service_check(false, false),
        (false, Some("kache daemon install"))
    );
}

/// The daemon footnote prints only for a FAILING daemon check under an
/// optional daemon — never for passing daemon checks, failing non-daemon
/// checks, or a required daemon.
#[test]
fn daemon_footnote_only_for_downgraded_daemon_failures() {
    assert!(daemon_footnote_needed(
        true,
        &[("Daemon service", false), ("Binary", true)]
    ));
    assert!(!daemon_footnote_needed(false, &[("Daemon service", false)]));
    assert!(!daemon_footnote_needed(
        true,
        &[("Daemon service", true), ("Binary", true)]
    ));
    assert!(!daemon_footnote_needed(
        true,
        &[("Compiler probe", false), ("Binary", false)]
    ));
    assert!(!daemon_footnote_needed(true, &[]));
}

// ── Daemon version reporting (kunobi-ninja/kache#720) ──────────────────

#[test]
fn daemon_version_check_reports_release_order_and_unknown_legacy_versions() {
    for (daemon, client, pass, message, fix) in [
        ("0.13.0", "0.14.0", false, "remains active", true),
        ("0.14.0", "0.13.0", true, "newer than binary", false),
        (
            "0.14.0+build.1",
            "0.14.0+build.2",
            true,
            "v0.14.0+build.1",
            false,
        ),
        ("", "0.14.0", true, "unknown", false),
        ("legacy", "0.14.0", true, "unknown", false),
        ("0.14.0", "development", true, "unknown", false),
    ] {
        let (actual, detail, hint) =
            daemon_version_check(Some(daemon), false, client, "/run/kache/daemon.log");
        assert_eq!(actual, pass, "{detail}");
        assert!(detail.contains(message), "{detail}");
        assert_eq!(hint.is_some(), fix, "{hint:?}");
        if let Some(hint) = hint {
            assert!(hint.contains("doctor --fix"));
        }
        assert!(!detail.contains("shutting down"));
    }
}

#[test]
fn daemon_version_check_reports_pending_start_without_guessing_the_release() {
    let (pass, detail, hint) = daemon_version_check(None, true, "0.14.0", "/run/kache/daemon.log");
    assert!(pass);
    assert!(detail.contains("starting"));
    assert!(!detail.contains("v0.14.0"));
    assert!(hint.is_none());
}

/// The behaviour this whole change exists for: a plain `doctor` run reports
/// a stale daemon, it does not replace it and pay the startup wait. Only
/// `--fix` opts into that.
#[test]
fn only_fix_restarts_a_stale_daemon() {
    assert!(should_restart_stale_daemon(true, true));

    assert!(!should_restart_stale_daemon(false, true), "plain doctor");
    assert!(!should_restart_stale_daemon(true, false), "nothing stale");
    assert!(!should_restart_stale_daemon(false, false), "neither");
}

/// A restart that did not finish must never read as one that is still
/// finishing — that is what turns a failed `--fix` into a silent no-op in the
/// reader's head.
#[test]
fn stale_restart_note_never_claims_a_replacement_that_may_not_exist() {
    assert!(stale_restart_note(&Ok(true)).is_none());

    let timed_out = stale_restart_note(&Ok(false)).unwrap();
    assert!(timed_out.contains("did not bind"), "{timed_out}");
    assert!(!timed_out.contains("background"), "{timed_out}");

    let failed = stale_restart_note(&Err(anyhow::anyhow!("spawn refused"))).unwrap();
    assert!(failed.contains("restart failed"), "{failed}");
    assert!(failed.contains("spawn refused"), "{failed}");
}

/// Nothing answering and nothing coming up keeps the original wording.
#[test]
fn daemon_version_check_reports_an_absent_daemon() {
    let (pass, detail, fix) = daemon_version_check(None, false, "0.14.0", "/run/kache/daemon.log");
    assert!(!pass);
    assert_eq!(detail, "daemon not reachable");
    let fix = fix.unwrap();
    assert!(fix.contains("kache daemon start"), "{fix}");
    assert!(fix.contains("/run/kache/daemon.log"), "{fix}");
}

/// A daemon that answered wins over the coordinator file: a leftover
/// `Starting` record must not relabel a reachable daemon as starting.
#[test]
fn daemon_version_check_prefers_the_daemon_that_answered() {
    let (pass, detail, _) =
        daemon_version_check(Some("0.14.0"), true, "0.14.0", "/run/kache/daemon.log");
    assert!(pass, "{detail}");
    assert_eq!(detail, "v0.14.0");
}

// ── Eviction reporting (kunobi-ninja/kache#509) ────────────────────────

fn gc_stats(evicted: usize, pinned: usize, bytes: u64) -> crate::store::GcStats {
    crate::store::GcStats {
        entries_evicted: evicted,
        bytes_freed: bytes,
        entries_pinned: pinned,
        ..Default::default()
    }
}

#[cfg(unix)]
#[test]
fn stats_reads_the_volume_holding_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::test_support::test_config(dir.path().join("cache"));
    std::fs::create_dir_all(&config.cache_dir).unwrap();
    config.auto_recover_min_free_bytes = 1;
    let volume = volume_view(&config, None).expect("Unix reads volume usage");
    assert!(volume.total_bytes > 0, "{volume:?}");
    assert!(volume.free_bytes <= volume.total_bytes, "{volume:?}");
    assert_eq!(volume.recovery_floor_bytes, 1);
}

#[test]
fn the_volume_row_says_what_is_free_and_whether_recovery_is_on() {
    let gib = 1u64 << 30;
    let usage = |free: u64| kache_fs::VolumeUsage {
        total: 100 * gib,
        free: free * gib,
    };
    assert_eq!(
        row_text(volume_row(&VolumeView::new(usage(2), 0))),
        "Volume | 2.0 GiB free | of 100.0 GiB · 98% used · free-space recovery off"
    );
    let below = VolumeView::new(usage(2), 10 * gib);
    assert!(below.below_floor);
    assert_eq!(
        row_text(volume_row(&below)),
        "Volume | 2.0 GiB free | of 100.0 GiB · 98% used · below the 10.0 GiB recovery floor · cache only"
    );
    let above = VolumeView::new(usage(40), 10 * gib);
    assert!(!above.below_floor);
    assert!(
        row_text(volume_row(&above))
            .ends_with("60% used · recovery keeps 10.0 GiB free · cache only"),
        "{above:?}"
    );
    // An unset floor is never "below", and recovers nothing.
    let off = VolumeView::new(usage(0), 0);
    assert!(!off.below_floor);
}

#[test]
fn cloned_targets_summary_only_appears_for_retained_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let mut disk = crate::machine::disk_view(dir.path(), 0, 1024);
    assert!(cloned_targets_row(&disk).map(row_text).is_none());

    disk.disk_private_bytes = 3;
    disk.cloned_into_targets_bytes = 7;
    let line = cloned_targets_row(&disk)
        .map(row_text)
        .expect("cloned blocks need a summary");
    assert!(line.contains("3 B"), "{line}");
    assert!(
        line.contains("7 B cloned into target/, which the limit leaves out"),
        "{line}"
    );
    assert!(!line.contains("snapshots"), "{line}");

    disk.snapshot_retained_bytes = 5;
    let line = cloned_targets_row(&disk)
        .map(row_text)
        .expect("both retainers are named");
    assert!(line.contains("7 B cloned"), "{line}");
    assert!(
        line.contains("5 B held only by filesystem snapshots"),
        "{line}"
    );

    disk.cloned_into_targets_bytes = 0;
    let line = cloned_targets_row(&disk)
        .map(row_text)
        .expect("snapshot blocks need a summary");
    assert!(!line.contains("cloned"), "{line}");
    assert!(line.contains("5 B held only"), "{line}");
}

#[test]
fn auto_gc_worker_retries_after_pins_or_a_lost_lock() {
    let swept_clean = crate::store::GcStats::default();
    assert!(!auto_gc_retry_wanted(&swept_clean));
    assert!(auto_gc_retry_wanted(&skipped_gc_stats()));
    let pinned = crate::store::GcStats {
        entries_pinned: 1,
        ..crate::store::GcStats::default()
    };
    assert!(auto_gc_retry_wanted(&pinned));
}

#[test]
fn gc_machine_output_boundaries_are_explicit() {
    assert!(human_gc_output(false));
    assert!(!human_gc_output(true));

    let stats = skipped_gc_stats();
    assert!(stats.skipped);
    assert_eq!(stats.entries_evicted, 0);

    assert!(!doctor_has_issues(0));
    assert!(doctor_has_issues(1));
}

#[test]
fn json_list_crate_filter_is_exact() {
    assert!(crate_name_matches("serde", "serde"));
    assert!(!crate_name_matches("serde", "serde_json"));
}

#[test]
fn daemon_gc_breakdown_sums_every_policy_field() {
    fn policy(n: u64) -> crate::daemon::GcPolicyOutcome {
        crate::daemon::GcPolicyOutcome {
            entries_evicted: n as usize,
            bytes_freed: n * 10,
            entries_pinned: (n * 100) as usize,
            disk_bytes_reclaimed: n * 1_000,
            entries_unreclaimable: (n * 10_000) as usize,
            entries_failed: (n * 100_000) as usize,
            entries_locked: (n * 1_000_000) as usize,
            evict_write_ms: n * 10_000_000,
            bytes_held: n * 100_000_000,
        }
    }
    let report = crate::daemon::GcBreakdown {
        mode: crate::daemon::GcRequestMode::Automatic,
        age: policy(1),
        duplicate: policy(2),
        size: policy(3),
    };
    let total = gc_stats_from_breakdown(&report);
    assert_eq!(total.entries_evicted, 6);
    assert_eq!(total.bytes_freed, 60);
    assert_eq!(total.entries_pinned, 600);
    assert_eq!(total.disk_bytes_reclaimed, 6_000);
    assert_eq!(total.entries_unreclaimable, 60_000);
    assert_eq!(total.entries_failed, 600_000);
    assert_eq!(total.entries_locked, 6_000_000);
    assert_eq!(total.evict_write_ms, 60_000_000);
    assert_eq!(
        total.bytes_held, 300_000_000,
        "the largest probe, not a sum"
    );

    let mut accumulated = crate::store::GcStats {
        entries_evicted: 1,
        bytes_freed: 2,
        entries_pinned: 3,
        blobs_removed: 4,
        duration_ms: 7,
        entries_unreclaimable: 5,
        disk_bytes_reclaimed: 6,
        skipped: false,
        entries_failed: 8,
        entries_locked: 9,
        entries_busy_snapshot: 0,
        entries_recent_prefiltered: 0,
        entries_import_pinned: 12,
        evict_write_ms: 11,
        bytes_held: 13,
        // Set once by the driver, never summed over policies.
        housekeeping: None,
    };
    let part = crate::store::GcStats {
        entries_evicted: 10,
        bytes_freed: 20,
        entries_pinned: 30,
        blobs_removed: 40,
        duration_ms: 70,
        entries_unreclaimable: 50,
        disk_bytes_reclaimed: 60,
        skipped: true,
        entries_failed: 80,
        entries_locked: 90,
        entries_busy_snapshot: 0,
        entries_recent_prefiltered: 0,
        entries_import_pinned: 120,
        evict_write_ms: 110,
        bytes_held: 7,
        // Set once by the driver, never summed over policies.
        housekeeping: None,
    };
    add_gc_stats(&mut accumulated, &part);
    assert_eq!(accumulated.bytes_held, 13, "the larger probe, not a sum");
    assert_eq!(accumulated.entries_evicted, 11);
    assert_eq!(accumulated.bytes_freed, 22);
    assert_eq!(accumulated.entries_pinned, 33);
    assert_eq!(accumulated.blobs_removed, 44);
    assert_eq!(accumulated.duration_ms, 77);
    assert_eq!(accumulated.entries_unreclaimable, 55);
    assert_eq!(accumulated.disk_bytes_reclaimed, 66);
    assert_eq!(accumulated.entries_failed, 88);
    assert_eq!(accumulated.entries_locked, 99);
    assert_eq!(accumulated.evict_write_ms, 121);
    assert_eq!(accumulated.entries_import_pinned, 132);
    assert!(accumulated.skipped);
}

/// The auto-GC worker used to throw its outcome away, so a machine where
/// only the worker ever ran GC had no `gc_stats.json` at all.
#[test]
fn local_gc_records_its_run_with_the_driver_that_ran_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());

    run_gc_local(&config, GcMode::Background).unwrap();
    let stats = crate::report::read_gc_stats(&config.cache_dir).expect("worker run recorded");
    assert_eq!(stats.source, "auto");

    run_gc_local(&config, GcMode::Cli).unwrap();
    let stats = crate::report::read_gc_stats(&config.cache_dir).unwrap();
    assert_eq!(stats.source, "manual");
}

/// A sandbox a crashed hermetic attempt left is gone after any GC run.
#[test]
fn local_gc_sweeps_hermetic_build_script_runs() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());
    let leftover = config.cache_dir.join("out-dirs/v2").join("ab".repeat(16));
    std::fs::create_dir_all(leftover.join("debug/build/z-1/out")).unwrap();
    run_gc_local(&config, GcMode::Background).unwrap();
    assert!(!leftover.exists());
}

/// `kache purge` removes every shared build-script run but the ones a
/// target directory still links to.
#[cfg(unix)]
#[test]
fn purge_keeps_only_linked_build_script_runs() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let v2 = config.cache_dir.join("out-dirs/v2");
    let run = |key: &str| {
        let root = v2.join(key.repeat(16));
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join(".kache-sealed"), b"{}").unwrap();
        root
    };
    let (linked, unlinked) = (run("ab"), run("cd"));
    let link = dir.path().join("target-out");
    std::os::unix::fs::symlink(linked.join("out"), &link).unwrap();
    let mut line = link.as_os_str().as_encoded_bytes().to_vec();
    line.push(b'\n');
    std::fs::write(linked.with_extension("refs"), line).unwrap();

    purge(&config, None).unwrap();
    assert!(linked.exists(), "a target directory links to it");
    assert!(!unlinked.exists());
}

/// Every driver records through record_gc_run; the local one shows the
/// history is wired in and stays off until record_sessions asks for it.
#[test]
fn local_gc_appends_to_the_gc_history_only_when_record_sessions_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = save_manifest_config(dir.path().to_path_buf());

    run_gc_local(&config, GcMode::Cli).unwrap();
    assert!(!config.cache_dir.join("telemetry").exists());

    config.record_sessions = true;
    run_gc_local(&config, GcMode::Background).unwrap();
    let log = std::fs::read_to_string(crate::report::gc_runs_log_path(&config.cache_dir)).unwrap();
    let records: Vec<crate::report::GcRunRecord> = log
        .lines()
        .map(|line| deser_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 1, "{log}");
    assert_eq!(records[0].source, "auto");
}

/// kunobi-ninja/kache#1126: the local sweep removes stale key locks and
/// records the housekeeping counts.
#[test]
fn local_gc_runs_store_housekeeping_and_records_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());

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

    let stats = run_gc_local(&config, GcMode::Background).unwrap();
    assert_eq!(
        stats.housekeeping,
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

#[test]
fn stale_schema_gc_records_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());

    gc(&config, None, true, true).unwrap();

    let stats = crate::report::read_gc_stats(&config.cache_dir).expect("stale-schema run recorded");
    assert_eq!(stats.source, "manual");
}

/// `kache gc --max-age` with no reachable daemon evicts locally; that run
/// counts like any other.
#[test]
fn age_gc_without_a_daemon_records_its_run() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());
    let store = Store::open(&config).unwrap();
    let _gc_lock = store.try_gc_lock().unwrap().expect("gc lock");

    evict_older_than_recorded(&store, &config, 24).unwrap();

    let stats = crate::report::read_gc_stats(&config.cache_dir).expect("age run recorded");
    assert_eq!(stats.source, "manual");
}

/// An idle `size`-byte entry the store holds alone.
fn put_idle_entry(store: &Store, dir: &std::path::Path, key: &str, size: usize) {
    let src = dir.join(format!("{key}.o"));
    std::fs::write(&src, &key.as_bytes().repeat(size)[..size]).unwrap();
    store
        .put(
            key,
            "test-crate",
            &[],
            &[],
            "host",
            "dev",
            &[(src.clone(), format!("{key}.o"))],
            "",
            "",
        )
        .unwrap();
    store.remove_clone_for_test(&src);
    store.set_last_accessed_for_test(key, "-1 hour");
}

/// A main config with one `[cache.volumes]` shard of `budget` bytes whose
/// store holds two idle 600-byte entries, and one mapped shard that does
/// not exist.
fn config_with_an_over_budget_shard(dir: &std::path::Path, budget: u64) -> (Config, Config, Store) {
    let mut config = save_manifest_config(dir.join("main"));
    let shard = crate::config::VolumeStore {
        volume: "/mnt/shard/".into(),
        store: dir.join("shard"),
        max_size: Some(budget),
    };
    let shard_config = config.for_volume_store(&shard, |_| None);
    config.volume_stores = vec![
        crate::config::VolumeStore {
            volume: "/mnt/gone/".into(),
            store: dir.join("unmounted"),
            max_size: None,
        },
        shard,
    ];
    let store = Store::open(&shard_config).unwrap();
    put_idle_entry(&store, dir, "shard_a", 600);
    put_idle_entry(&store, dir, "shard_b", 600);
    (config, shard_config, store)
}

/// kunobi-ninja/kache#974: `kache gc` without a daemon sweeps each shard
/// against its own budget, skips a missing one, and records the shard's
/// run in the shard.
#[test]
fn local_gc_sweeps_each_shard_against_its_own_budget() {
    let dir = tempfile::tempdir().unwrap();
    let (config, shard_config, shard) = config_with_an_over_budget_shard(dir.path(), 1000);
    let main = Store::open(&config).unwrap();
    put_idle_entry(&main, dir.path(), "main_entry", 1200);

    let stats = run_gc_local_with_shards(&config, GcMode::Cli).unwrap();
    assert_eq!(stats.entries_evicted, 0, "the stats are the main store's");
    assert!(main.contains("main_entry"));
    assert_eq!(shard.physical_size().unwrap(), 600);
    let recorded = crate::report::read_gc_stats(&shard_config.cache_dir).unwrap();
    assert_eq!(
        (recorded.source.as_str(), recorded.entries_evicted),
        ("manual", 1)
    );
    assert!(!dir.path().join("unmounted").exists());

    let dir = tempfile::tempdir().unwrap();
    let (config, _, shard) = config_with_an_over_budget_shard(dir.path(), 1_000_000);
    run_gc_local_with_shards(&config, GcMode::Background).unwrap();
    assert_eq!(
        shard.physical_size().unwrap(),
        1200,
        "under budget: left alone"
    );
}

/// Each store has its own gc.lock: a busy main store does not stop the
/// shards from being swept, and a busy shard is left to its holder.
#[test]
fn local_gc_takes_each_stores_lock_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let (config, _, shard) = config_with_an_over_budget_shard(dir.path(), 1000);
    let main = Store::open(&config).unwrap();
    let main_lock = main.try_gc_lock().unwrap().expect("gc lock");
    let shard_lock = shard.try_gc_lock().unwrap().expect("gc lock");

    let stats = run_gc_local_with_shards(&config, GcMode::Background).unwrap();
    assert!(stats.skipped);
    assert_eq!(
        shard.physical_size().unwrap(),
        1200,
        "the shard's lock is held"
    );

    drop(shard_lock);
    run_gc_local_with_shards(&config, GcMode::Background).unwrap();
    assert_eq!(
        shard.physical_size().unwrap(),
        600,
        "only the main lock is held"
    );
    drop(main_lock);
}

#[test]
fn only_a_printing_sweep_names_the_shard_first() {
    let shard = std::path::Path::new("/mnt/shard");
    assert_eq!(
        shard_sweep_heading(GcMode::Cli, shard).as_deref(),
        Some("Volume shard /mnt/shard:")
    );
    assert_eq!(shard_sweep_heading(GcMode::Background, shard), None);
}

/// The auto-GC worker a build spawns with no daemon sweeps shards too,
/// each against its own trigger.
#[test]
fn the_auto_gc_worker_sweeps_an_over_budget_shard() {
    let dir = tempfile::tempdir().unwrap();
    let (config, shard_config, shard) = config_with_an_over_budget_shard(dir.path(), 1000);

    run_auto_gc_workers(&config, std::time::Duration::ZERO);
    assert_eq!(shard.physical_size().unwrap(), 600);
    let recorded = crate::report::read_gc_stats(&shard_config.cache_dir).unwrap();
    assert_eq!(recorded.source, "auto");
    assert!(!dir.path().join("unmounted").exists());
}

#[test]
fn age_gc_without_a_daemon_reaches_shards_whose_lock_is_free() {
    let dir = tempfile::tempdir().unwrap();
    let (config, shard_config, shard) = config_with_an_over_budget_shard(dir.path(), 1_000_000);
    shard.set_last_accessed_for_test("shard_a", "-48 hours");

    let held = shard.try_gc_lock().unwrap().expect("gc lock");
    let stats = evict_shards_older_than(&config, 24);
    assert_eq!(stats.len(), 1);
    assert!(stats[0].skipped);
    assert!(shard.contains("shard_a"));
    drop(held);

    let stats = evict_shards_older_than(&config, 24);
    assert_eq!(stats[0].entries_evicted, 1);
    assert!(!shard.contains("shard_a") && shard.contains("shard_b"));
    let recorded = crate::report::read_gc_stats(&shard_config.cache_dir).unwrap();
    assert_eq!(recorded.source, "manual");
}

#[test]
fn the_gc_summary_has_a_line_per_shard_it_swept() {
    let dir = tempfile::tempdir().unwrap();
    let (config, shard_config, shard) = config_with_an_over_budget_shard(dir.path(), 1000);
    assert_eq!(
        shard_store_lines(&config),
        vec![format!(
            "Volume shard {}: {} / {} · 2 entries",
            shard_config.cache_dir.display(),
            kache_core::size::format_bytes(shard.physical_size().unwrap()),
            kache_core::size::format_bytes(1000),
        )]
    );
}

#[test]
fn gc_summary_compares_deduplicated_blob_bytes_with_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let store = Store::open(&config).unwrap();
    let artifact = dir.path().join("shared.o");
    fs::write(&artifact, [42; 60]).unwrap();
    for key in ["first", "second"] {
        store
            .put(
                key,
                key,
                &[],
                &[],
                "",
                "",
                &[(artifact.clone(), "shared.o".into())],
                "",
                "",
            )
            .unwrap();
    }
    assert_eq!(store.total_size().unwrap(), 120);
    assert_eq!(store.physical_size().unwrap(), 60);
    assert_eq!(
        gc_store_usage(&store, 90, None).unwrap(),
        "60 B / 90 B · 2 entries"
    );
    assert_eq!(
        gc_store_usage(&store, 0, None).unwrap(),
        "60 B / 0 B · 2 entries"
    );
}

#[test]
fn machine_snapshot_reads_a_store_without_creating_one() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());

    let empty = machine_snapshot(&config);
    assert!(empty.index_bytes.is_none());
    assert!(empty.wal_bytes.is_none());
    assert!(empty.rowid_high_water.is_empty());
    assert!(empty.gc.is_none());
    assert!(
        !config.index_db_path().exists(),
        "a snapshot must never create the index"
    );

    let store = Store::open(&config).unwrap();
    let artifact = dir.path().join("physical-size.rlib");
    std::fs::write(&artifact, b"payload").unwrap();
    store
        .put(
            "physical-size",
            "physical_size",
            &["lib".to_string()],
            &[],
            "",
            "dev",
            &[(artifact, "libphysical_size.rlib".to_string())],
            "",
            "",
        )
        .unwrap();
    drop(store);
    crate::report::record_gc_run(
        &config,
        "auto",
        crate::store::SweepOrigin::Automatic,
        &crate::store::GcStats::default(),
    )
    .unwrap();
    let snap = machine_snapshot(&config);
    let db_len = std::fs::metadata(config.index_db_path()).unwrap().len();
    let mut wal_path = config.index_db_path().into_os_string();
    wal_path.push("-wal");
    let wal_len = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
    assert_eq!(snap.wal_bytes, Some(wal_len));
    assert_eq!(snap.index_bytes, Some(db_len + wal_len));
    assert_eq!(snap.store_physical_bytes, Some(7));
    assert_eq!(snap.index_free_bytes, Some(0), "a fresh index has no holes");
    assert_eq!(
        snap.blob_drift,
        Some(crate::store::BlobRefcountDrift::default())
    );
    let db = rusqlite::Connection::open(config.index_db_path()).unwrap();
    db.execute_batch(
        "UPDATE blobs SET refcount = 5;
             INSERT INTO blobs (hash, size, refcount) VALUES ('unowned', 4096, 1);",
    )
    .unwrap();
    drop(db);
    assert_eq!(
        machine_snapshot(&config).blob_drift,
        Some(crate::store::BlobRefcountDrift {
            unowned: 1,
            unowned_bytes: 4096,
            too_high: 1,
            too_high_bytes: 7,
            ..Default::default()
        })
    );
    let tables: Vec<_> = snap
        .rowid_high_water
        .iter()
        .map(|(table, _)| *table)
        .collect();
    assert!(
        tables.contains(&"entries") && tables.contains(&"blobs"),
        "{tables:?}"
    );
    assert_eq!(snap.gc.expect("gc_stats.json read").source, "auto");
}

/// The last read-write connection to a WAL database checkpoints on close,
/// copying the WAL into `index.db`: a write, on a 27 GiB file, that a
/// snapshot must never make. A read-only connection leaves both files as
/// it found them.
#[test]
fn machine_snapshot_never_checkpoints_the_wal() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());
    drop(Store::open(&config).unwrap());
    let db_path = config.index_db_path();
    let mut wal_path = db_path.clone().into_os_string();
    wal_path.push("-wal");
    {
        // A writer that exits without checkpointing, as a killed build does.
        let writer = rusqlite::Connection::open(&db_path).unwrap();
        writer
            .set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
                true,
            )
            .unwrap();
        writer
            .execute_batch(
                "PRAGMA wal_autocheckpoint = 0;
                     CREATE TABLE probe (x INTEGER);
                     INSERT INTO probe VALUES (1);",
            )
            .unwrap();
    }
    let db_before = std::fs::read(&db_path).unwrap();
    let wal_before = std::fs::metadata(&wal_path).unwrap().len();
    assert!(wal_before > 0, "the setup leaves frames in the WAL");

    machine_snapshot(&config);

    assert_eq!(
        std::fs::read(&db_path).unwrap(),
        db_before,
        "index.db must not be written"
    );
    assert_eq!(
        std::fs::metadata(&wal_path).map(|m| m.len()).ok(),
        Some(wal_before),
        "the WAL must be left in place"
    );
}

/// One `kache stats` row as plain text, for assertions.
fn row_text(row: StatsRow) -> String {
    format!("{} | {} | {}", row.0, row.1, row.2)
}

/// The GC note grows a suffix only when there is something to report:
/// zero failures and zero write time add nothing, one of each adds both.
/// A record without a driver predates `source`, when only the daemon
/// wrote one. A time that does not parse is shown as it was recorded.
#[test]
fn stats_gc_note_suffixes_start_at_one() {
    let gc_rows = |failed: usize, locked: usize, write_ms: u64, source: &str| {
        machine_rows(&crate::otel::MachineSnapshot {
            gc: Some(crate::report::GcStatsPersisted {
                last_run: "not a time".to_string(),
                source: source.to_string(),
                entries_evicted: 3,
                entries_failed: failed,
                entries_locked: locked,
                evict_write_ms: write_ms,
                ..Default::default()
            }),
            ..Default::default()
        })
        .into_iter()
        .map(row_text)
        .collect::<Vec<_>>()
    };
    assert_eq!(
        gc_rows(0, 0, 0, "manual"),
        vec!["Last GC | not a time | manual, 3 evicted".to_string()]
    );
    assert_eq!(
        gc_rows(1, 1, 1, ""),
        vec![
            "Last GC | not a time | daemon, 3 evicted, 1 failed (1 lost the index write lock), 1 ms in index writes"
                .to_string()
        ]
    );
    let index_only = machine_rows(&crate::otel::MachineSnapshot {
        index_bytes: Some(4096),
        ..Default::default()
    });
    assert_eq!(
        index_only.into_iter().map(row_text).collect::<Vec<_>>(),
        vec!["Index | 4.0 KiB | ".to_string()]
    );
}

#[test]
fn stats_rows_show_the_index_and_a_gc_that_keeps_losing_the_lock() {
    let machine = crate::otel::MachineSnapshot {
        store_physical_bytes: None,
        index_bytes: Some(29_074_419_712),
        wal_bytes: Some(1_073_741_824),
        index_free_bytes: None,
        blob_drift: None,
        rowid_high_water: vec![
            ("entries", 2),
            ("file_hashes", 13_286_285),
            ("cc_preprocess_memos", 874_517),
            ("eviction_tombstones", 5_425_819),
            ("blobs", 5),
        ],
        gc: Some(crate::report::GcStatsPersisted {
            last_run: "2026-09-12T12:11:05+00:00".to_string(),
            source: "auto".to_string(),
            entries_failed: 40,
            entries_locked: 40,
            evict_write_ms: 4200,
            ..Default::default()
        }),
    };
    let rows: Vec<String> = machine_rows(&machine).into_iter().map(row_text).collect();
    assert_eq!(
        rows[0],
        "Index | 27.1 GiB | WAL 1.0 GiB · rowid high-water: file_hashes 13,286,285, eviction_tombstones 5,425,819, cc_preprocess_memos 874,517"
    );
    assert!(rows[1].starts_with("Last GC | "), "{rows:?}");
    assert!(
        !rows[1].contains("2026-09-12T"),
        "shown in local time: {rows:?}"
    );
    assert!(
        rows[1].contains("auto, 0 evicted, 40 failed (40 lost the index write lock)"),
        "{rows:?}"
    );
    assert!(rows[1].ends_with(", 4200 ms in index writes"), "{rows:?}");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(machine_rows(&crate::otel::MachineSnapshot::default()).is_empty());
}

fn lifetime_totals() -> crate::savings::Totals {
    crate::savings::Totals {
        since: kache_core::time::UtcTime::parse_rfc3339("2026-09-12T12:00:00Z").unwrap(),
        hits: 128_402,
        hit_compile_time_ms: 147_600_000,
        zero_copy_bytes: 3 << 30,
        copied_bytes: 1 << 20,
        pruned_automatic_bytes: 2 << 30,
        pruned_requested_bytes: 1 << 30,
    }
}

fn lifetime_text(totals: Option<&crate::savings::Totals>) -> Vec<String> {
    let today = kache_core::time::UtcTime::from_ymd_hms(2026, 10, 1, 0, 0, 0).unwrap().date();
    lifetime_rows(totals, today)
        .into_iter()
        .map(row_text)
        .collect()
}

#[test]
fn lifetime_rows_show_each_recorded_total() {
    assert!(
        lifetime_text(None).is_empty(),
        "nothing recorded: no section"
    );
    let totals = lifetime_totals();
    let rows = lifetime_text(Some(&totals));
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows[0].starts_with("Since | Sep "), "{rows:?}");
    assert!(rows[0].ends_with(" | "), "{rows:?}");
    assert_eq!(
        rows[1..],
        [
            "Time saved | ~41.0 h | compile work avoided over 128,402 hits",
            "No-copy | 3.0 GiB | restored without copying, 1.0 MiB copied",
            "Pruned | 3.0 GiB | 2.0 GiB automatically, 1.0 GiB on request",
        ]
    );
}

/// A total that is still zero gets no row, and all zeros get no section;
/// one of each part is enough for its row.
#[test]
fn lifetime_rows_leave_out_zero_totals() {
    let mut totals = crate::savings::Totals {
        hits: 0,
        hit_compile_time_ms: 0,
        zero_copy_bytes: 0,
        copied_bytes: 0,
        pruned_automatic_bytes: 0,
        pruned_requested_bytes: 0,
        ..lifetime_totals()
    };
    assert!(
        lifetime_text(Some(&totals)).is_empty(),
        "nothing saved yet: no section"
    );

    totals.hits = 1;
    totals.copied_bytes = 5;
    totals.pruned_requested_bytes = 7;
    let rows = lifetime_text(Some(&totals));
    assert_eq!(
        rows[1..],
        [
            "Time saved | 0 ms | compile work avoided over 1 hit",
            "No-copy | 0 B | restored without copying, 5 B copied",
            "Pruned | 7 B | 0 B automatically, 7 B on request",
        ]
    );
}

#[test]
fn since_label_adds_the_year_only_for_another_year() {
    let date = |y, m, d| kache_core::time::UtcTime::from_ymd_hms(y, m, d, 0, 0, 0).unwrap().date();
    assert_eq!(since_label(date(2026, 9, 2), date(2026, 10, 1)), "Sep 02");
    assert_eq!(
        since_label(date(2025, 9, 2), date(2026, 10, 1)),
        "Sep 02 2025"
    );
}

/// The lifetime section sits between the window's build rows and the cache.
#[test]
fn render_stats_places_the_lifetime_section_after_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::test_support::test_config(dir.path().to_path_buf());
    let extras = StatsExtras {
        lifetime: vec![("Since", "Sep 12".to_string(), "lifetime".to_string())],
        ..Default::default()
    };
    let lines = render_stats_with(
        &StatsSnapshot::default(),
        &config,
        SinceWindow::DEFAULT,
        &extras,
    );
    let at = |label: &str| {
        lines
            .iter()
            .position(|line| line.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("{label}: {lines:#?}"))
    };
    assert!(at("Time saved") < at("Since"), "{lines:#?}");
    assert!(at("Since") < at("Cache"), "{lines:#?}");
    assert_eq!(
        lines[at("Since") - 1],
        "Lifetime",
        "its own section: {lines:#?}"
    );
}

/// Dropping a table leaves its pages on the freelist until a compaction.
#[test]
fn index_free_bytes_counts_freelist_pages() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA page_size = 4096; CREATE TABLE keep (v BLOB);")
        .unwrap();
    assert_eq!(index_free_bytes(&db), Some(0));

    db.execute_batch(
        "CREATE TABLE junk (v BLOB);
             INSERT INTO junk VALUES (zeroblob(65536));
             DROP TABLE junk;",
    )
    .unwrap();
    let pages: i64 = db
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .unwrap();
    assert!(
        pages >= 16,
        "a 64 KiB blob spans at least 16 pages: {pages}"
    );
    assert_eq!(index_free_bytes(&db), Some(pages as u64 * 4096));

    db.execute_batch("VACUUM").unwrap();
    assert_eq!(index_free_bytes(&db), Some(0));
}

#[test]
fn free_page_bytes_multiplies_and_rejects_nonsense() {
    assert_eq!(free_page_bytes(3, 4096), Some(12_288));
    assert_eq!(free_page_bytes(0, 4096), Some(0));
    assert_eq!(free_page_bytes(-1, 4096), None);
    assert_eq!(free_page_bytes(1, -4096), None);
    assert_eq!(free_page_bytes(i64::MAX, 4096), None);
}

/// Why the figure is not called rows: `INSERT OR REPLACE` on a TEXT key,
/// the way `file_hashes` and the memo tables are written, deletes the old
/// row and inserts the new one at the next rowid.
#[test]
fn rowid_high_water_counts_replacements_not_rows() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE file_hashes (path TEXT PRIMARY KEY, hash TEXT)")
        .unwrap();
    for hash in ["a", "b", "c"] {
        db.execute(
            "INSERT OR REPLACE INTO file_hashes VALUES ('src/lib.rs', ?1)",
            [hash],
        )
        .unwrap();
    }
    let high_water: i64 = db
        .query_row(&rowid_high_water_sql("file_hashes"), [], |row| row.get(0))
        .unwrap();
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM file_hashes", [], |row| row.get(0))
        .unwrap();
    assert_eq!((high_water, rows), (3, 1));
}

/// The rowid high-water mark on a 27 GiB index must not read the table.
/// The query seeks to the last row (`Last`) and stops, so its step count does
/// not grow with the table; a scan steps once per row, and `COUNT(*)` is a
/// single `Count` step that still reads every page.
#[test]
fn index_row_figures_seek_instead_of_scanning() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE memos (key TEXT PRIMARY KEY, body BLOB);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000)
             INSERT INTO memos SELECT 'k' || i, zeroblob(64) FROM n;",
    )
    .unwrap();

    let mut stmt = db.prepare(&rowid_high_water_sql("memos")).unwrap();
    let top: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
    assert_eq!(top, 5000);
    let steps = stmt.get_status(rusqlite::StatementStatus::VmStep);
    assert!(steps < 100, "{steps} VM steps for one high-water mark");

    let mut explain = db
        .prepare(&format!("EXPLAIN {}", rowid_high_water_sql("memos")))
        .unwrap();
    let opcodes: Vec<String> = explain
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(opcodes.iter().any(|op| op == "Last"), "{opcodes:?}");
    assert!(!opcodes.iter().any(|op| op == "Count"), "{opcodes:?}");
}

/// `kache stats` must not stall behind the index. The store's own busy
/// timeout is 5 s per statement; the snapshot gives up after 25 ms and
/// drops the figures it could not read.
#[test]
fn machine_snapshot_skips_row_figures_on_a_locked_index() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().to_path_buf());
    drop(Store::open(&config).unwrap());
    let holder = rusqlite::Connection::open(config.index_db_path()).unwrap();
    holder
        .execute_batch(
            "PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE; \
                 DELETE FROM entries WHERE 0;",
        )
        .unwrap();

    let started = std::time::Instant::now();
    let snap = machine_snapshot(&config);
    let waited = started.elapsed();

    assert!(snap.index_bytes.is_some(), "file sizes need no lock");
    assert!(
        snap.rowid_high_water.is_empty(),
        "{:?}",
        snap.rowid_high_water
    );
    assert!(
        waited < std::time::Duration::from_secs(4),
        "waited {waited:?} on a locked index"
    );
    holder.execute_batch("COMMIT").unwrap();
}

fn report_run(config: &Config, out: &std::path::Path, record: bool) {
    report(
        config,
        ReportFormat::Json,
        SinceWindow::DEFAULT,
        crate::report::ReportFilter {
            root: Some(std::path::PathBuf::from("/ci/runner-3/_work/secret-repo")),
            last_build: false,
        },
        Some(out.join("report.json")),
        10,
        record,
        false,
    )
    .unwrap();
}

/// Every file under `dir` with its contents.
fn tree_contents(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let body = std::fs::read(&path).unwrap();
                files.push((path, body));
            }
        }
    }
    files.sort();
    files
}

/// `kache stats --last-build` is not the full report, but a host that turned
/// `record_sessions` on still gets the session. `--record` never reaches
/// this function: it selects the full report instead.
#[test]
fn stats_last_build_records_when_record_sessions_is_on() {
    let cache = tempfile::tempdir().unwrap();
    let mut config = save_manifest_config(cache.path().to_path_buf());
    config.record_sessions = true;
    plant_last_build(&config);
    stats_last_build(&config, None, false).unwrap();
    stats_last_build(&config, None, true).unwrap();
    let log = std::fs::read_to_string(session_log_path(&config)).unwrap();
    assert_eq!(log.lines().count(), 2, "{log}");

    let quiet = tempfile::tempdir().unwrap();
    let config = save_manifest_config(quiet.path().to_path_buf());
    plant_last_build(&config);
    stats_last_build(&config, None, true).unwrap();
    assert!(
        !session_log_path(&config).exists(),
        "a last-build summary records only when record_sessions is on"
    );
}

fn plant_last_build(config: &Config) {
    let mut event = build_event(
        "demo",
        crate::events::EventResult::LocalHit,
        4,
        5,
        64,
        "key",
    );
    event.root = "/repo".to_string();
    event.session_id = "build".to_string();
    crate::events::log_event(&config.event_log_path(), &event).unwrap();
}

/// `record_sessions` (env or config) makes a plain `kache report` record
/// exactly as `--record` would, so a host can opt in once for every job.
#[test]
fn report_records_without_the_flag_when_record_sessions_is_on() {
    let cache = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut config = save_manifest_config(cache.path().to_path_buf());
    config.record_sessions = true;

    report_run(&config, out.path(), false);

    let log = std::fs::read_to_string(session_log_path(&config)).unwrap();
    assert_eq!(log.lines().count(), 1, "{log}");
    let record: crate::report::SessionRecord =
        deser_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(record.schema, crate::report::SESSION_RECORD_SCHEMA);
}

/// Recording is opt-in: a plain `kache report` leaves the cache dir
/// exactly as it found it.
#[test]
fn report_without_record_leaves_the_cache_dir_unchanged() {
    let cache = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let config = save_manifest_config(cache.path().to_path_buf());
    drop(Store::open(&config).unwrap());
    let before = tree_contents(cache.path());

    report_run(&config, out.path(), false);

    assert!(out.path().join("report.json").is_file());
    assert!(!cache.path().join("telemetry").exists());
    assert_eq!(tree_contents(cache.path()), before);
}

/// The report reads events from the runtime dir, which CI deletes with the
/// job, so a recorded session has to land in the cache dir, without the
/// path.
#[test]
fn report_record_appends_one_line_to_the_cache_dir_not_the_runtime_dir() {
    let cache = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut config = save_manifest_config(cache.path().to_path_buf());
    config.runtime_dir = runtime.path().to_path_buf();
    let line_count = || {
        std::fs::read_to_string(session_log_path(&config))
            .unwrap()
            .lines()
            .count()
    };

    report_run(&config, out.path(), true);
    assert_eq!(line_count(), 1, "one line per recorded session");
    report_run(&config, out.path(), true);
    assert_eq!(line_count(), 2, "a second record appends, never rewrites");

    assert!(!runtime.path().join("telemetry").exists());
    let log = std::fs::read_to_string(session_log_path(&config)).unwrap();
    let lines: Vec<_> = log.lines().collect();
    assert!(
        !log.contains("secret-repo"),
        "the root is hashed, never written"
    );
    let record: crate::report::SessionRecord = deser_json::from_str(lines[0]).unwrap();
    assert_eq!(record.schema, crate::report::SESSION_RECORD_SCHEMA);
    assert_eq!(record.root_hash.as_deref().map(str::len), Some(16));
    assert_eq!(record.summary.total_crates, 0);
    assert_eq!(record.machine.store_max, config.max_size);
}

/// Recording is a side effect. A cache dir it cannot write (a read-only
/// mount, another owner) costs the session line, never the report.
#[test]
fn report_record_that_cannot_write_still_delivers_the_report() {
    let cache = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let config = save_manifest_config(cache.path().to_path_buf());
    // A file where the telemetry dir goes fails create_dir_all the way a
    // read-only mount does, and root cannot write past it.
    std::fs::write(cache.path().join("telemetry"), b"").unwrap();

    let result = report(
        &config,
        ReportFormat::Json,
        SinceWindow::DEFAULT,
        crate::report::ReportFilter {
            root: None,
            last_build: false,
        },
        Some(out.path().join("report.json")),
        10,
        true,
        false,
    );

    assert!(result.is_ok(), "{result:?}");
    assert!(out.path().join("report.json").is_file());
}

#[test]
fn evicting_nothing_because_everything_is_pinned_explains_itself() {
    // The #509 report: `evicted 0 entries` printed next to a store at 912%.
    // The number is right and the message is useless, so the user concludes
    // GC is broken. The output must name the grace and say what to do.
    let msg = describe_eviction(&gc_stats(0, 24, 0), true);
    assert!(
        msg.contains("24"),
        "must say how many were held back: {msg}"
    );
    assert!(
        msg.contains("within the last ~2 min"),
        "must name the grace period so the wait is bounded and knowable: {msg}"
    );
    assert!(
        msg.contains("Run it again once builds and uploads are idle"),
        "must tell the user what to do next: {msg}"
    );
    assert!(
        msg.contains("durable remote upload"),
        "the shared pin counter must describe upload-backed entries too: {msg}"
    );
}

#[test]
fn evicting_nothing_while_over_limit_still_explains_the_grace() {
    // Nothing pinned and nothing evicted, but still over budget: the user
    // needs to know the idle rule exists, or "0" reads as a broken GC.
    let msg = describe_eviction(&gc_stats(0, 0, 0), true);
    assert!(msg.contains("over its limit"), "{msg}");
    assert!(msg.contains("within the last ~2 min"), "{msg}");
}

#[test]
fn an_empty_store_does_not_imply_something_is_wrong() {
    // Under budget with nothing to do is the normal case and must not
    // inherit the alarming phrasing of the over-limit one.
    let msg = describe_eviction(&gc_stats(0, 0, 0), false);
    assert!(msg.contains("nothing to evict"), "{msg}");
    assert!(
        !msg.contains("over its limit"),
        "a healthy store must not be told it is over budget: {msg}"
    );
}

#[test]
fn a_successful_eviction_reports_bytes_and_still_flags_pinned_entries() {
    let mut stats = gc_stats(12, 3, 5 * 1024 * 1024);
    stats.disk_bytes_reclaimed = 4 * 1024 * 1024;
    stats.entries_unreclaimable = 2;
    let msg = describe_eviction(&stats, false);
    assert!(msg.contains("12 entries"), "{msg}");
    assert!(msg.contains("MiB") || msg.contains("MB"), "{msg}");
    assert!(
        msg.contains('3'),
        "entries left behind matter even on a successful sweep — they are \
            why the store may still be over budget: {msg}"
    );
    assert!(msg.contains("became free on disk"), "{msg}");
    assert!(msg.contains("is still held by clones"), "{msg}");
    assert!(msg.contains("2 entries left in place"), "{msg}");
}

#[test]
fn an_unsuccessful_eviction_still_flags_pinned_entries_when_clones_remain() {
    let mut stats = gc_stats(0, 7, 0);
    stats.entries_unreclaimable = 2;
    let msg = describe_eviction(&stats, false);
    assert!(
        msg.contains("7 more entries"),
        "must report the 7 pinned entries even when clones retain others: {msg}"
    );

    stats.entries_pinned = 0;
    let msg = describe_eviction(&stats, false);
    assert!(!msg.contains(" more "), "{msg}");
}

#[test]
fn a_successful_eviction_without_pinned_entries_has_no_pinned_note() {
    let msg = describe_eviction(&gc_stats(3, 0, 1024), false);
    assert!(!msg.contains(" more "), "{msg}");
}

#[test]
fn fully_retained_eviction_has_a_cleanup_path() {
    let mut stats = gc_stats(0, 0, 0);
    stats.entries_unreclaimable = 1;
    let msg = describe_eviction(&stats, false);
    assert!(msg.contains("1 entry cloned"), "{msg}");
    assert!(msg.contains("Remove unneeded build outputs"), "{msg}");
    assert!(msg.contains("then run it again"), "{msg}");

    stats.bytes_held = 3 * 1024 * 1024 * 1024;
    let msg = describe_eviction(&stats, false);
    assert!(msg.contains("1 entry (3.0 GiB) cloned"), "{msg}");
}

#[test]
fn a_successful_eviction_sizes_the_entries_clones_hold() {
    let mut stats = gc_stats(1, 0, 1024);
    stats.entries_unreclaimable = 2;
    stats.bytes_held = 2048;
    let msg = describe_eviction(&stats, false);
    assert!(msg.contains("2 entries (2.0 KiB) left in place"), "{msg}");
}

#[test]
fn eviction_messages_are_singular_for_one_entry() {
    let mut stats = gc_stats(1, 0, 1024);
    stats.disk_bytes_reclaimed = 1024;
    let msg = describe_eviction(&stats, false);
    assert!(msg.contains("1 entry"), "{msg}");
    assert!(!msg.contains("1 entries"), "{msg}");
    assert!(!msg.contains("is still held by clones"), "{msg}");
    assert!(!msg.contains("more 0 entries"), "{msg}");
    assert!(!msg.contains("0 entries left in place"), "{msg}");
}

#[test]
fn a_clean_sweep_says_nothing_about_entries_left_behind() {
    // Nothing pinned: the grace-period paragraph would be noise at best,
    // and at worst reads as "some entries are stuck" on a sweep that
    // emptied everything it was asked to.
    let msg = describe_eviction(&gc_stats(7, 0, 4096), false);
    assert!(msg.contains("7 entries"), "{msg}");
    assert!(
        !msg.contains("in use within the last"),
        "no entries were held back, so the grace note must not appear: {msg}"
    );
}

#[test]
fn over_limit_is_decided_by_a_strict_comparison_against_the_budget() {
    assert!(store_over_limit(Some(1025), 1024));
    assert!(
        !store_over_limit(Some(1024), 1024),
        "exactly at budget is within it: a store that fits must not be told \
             it is over"
    );
    assert!(!store_over_limit(Some(512), 1024));
    assert!(
        !store_over_limit(None, 1024),
        "an unreadable size must not be reported as over budget — that sends \
             the user chasing an eviction problem they may not have"
    );
}

fn zero_event_stats() -> daemon::EventStatsResponse {
    daemon::EventStatsResponse {
        local_hits: 0,
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
    }
}





#[test]
fn test_optional_failing_check_is_not_an_issue() {
    // A downgraded (optional) daemon check that failed must not count.
    assert!(!is_doctor_issue(false, true));
}

#[test]
fn test_genuine_failing_check_is_an_issue() {
    assert!(is_doctor_issue(false, false));
}

#[test]
fn test_passing_check_is_never_an_issue() {
    assert!(!is_doctor_issue(true, false));
    assert!(!is_doctor_issue(true, true));
}

#[test]
fn test_count_hit_rate_zero_total_is_zero() {
    assert_eq!(count_hit_rate(&zero_event_stats()), 0.0);
}

#[test]
fn test_count_hit_rate_counts_all_hit_kinds() {
    let es = daemon::EventStatsResponse {
        local_hits: 6,
        dups: 0,
        misses: 4,
        ..zero_event_stats()
    };
    // 6 hits / (6+4) total = 60%
    assert!((count_hit_rate(&es) - 60.0).abs() < 1e-9);
}

#[test]
fn test_count_hit_rate_all_hits_is_hundred() {
    let es = daemon::EventStatsResponse {
        local_hits: 5,
        ..zero_event_stats()
    };
    assert!((count_hit_rate(&es) - 100.0).abs() < 1e-9);
}

#[test]
fn test_compile_weighted_hit_rate_none_when_no_compile_time() {
    assert_eq!(compile_weighted_hit_rate(&zero_event_stats()), None);
}

#[test]
fn test_compile_weighted_hit_rate_weights_by_time() {
    let es = daemon::EventStatsResponse {
        hit_compile_time_ms: 750,
        miss_compile_time_ms: 250,
        ..zero_event_stats()
    };
    let r = compile_weighted_hit_rate(&es).unwrap();
    assert!((r - 75.0).abs() < 1e-9);
}

#[test]
fn test_format_relative_time_invalid_passes_through() {
    assert_eq!(format_relative_time("not a date"), "not a date");
}

#[test]
fn test_format_relative_time_buckets() {
    let now = kache_core::time::UtcTime::now();
    let fmt = |dt: kache_core::time::UtcTime| format_relative_time(&dt.format_sqlite());
    assert_eq!(fmt(now - std::time::Duration::from_secs(10)), "just now");
    assert_eq!(fmt(now - std::time::Duration::from_secs(5 * 60)), "5m ago");
    assert_eq!(fmt(now - std::time::Duration::from_secs(3 * 3600)), "3h ago");
    assert_eq!(fmt(now - std::time::Duration::from_secs(2 * 86400)), "2d ago");
    // A future timestamp clamps to "just now" (secs.max(0)).
    assert_eq!(fmt(now + std::time::Duration::from_secs(3600)), "just now");
}

#[test]
fn test_sccache_program_detection_accepts_paths() {
    assert!(is_sccache_program("sccache"));
    assert!(is_sccache_program("/opt/homebrew/bin/sccache"));
    assert!(is_sccache_program("sccache.exe"));
    assert!(!is_sccache_program("kache"));
    assert!(!is_sccache_program("sccache-wrapper"));
}

#[test]
fn test_sccache_rc_detection_ignores_fallback_setting() {
    assert!(!active_sccache_migration_line("# RUSTC_WRAPPER=sccache"));
    assert!(!active_sccache_migration_line(
        "export KACHE_FALLBACK=sccache"
    ));
    assert!(active_sccache_migration_line(
        "export RUSTC_WRAPPER=sccache"
    ));
    assert!(active_sccache_migration_line("rustc-wrapper = \"sccache\""));
}

#[test]
fn test_fallback_is_sccache() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = save_manifest_config(dir.path().to_path_buf());

    // No config / no fallback -> false.
    assert!(!fallback_is_sccache(None));
    assert!(!fallback_is_sccache(Some(&cfg)));

    // Fallback set to an sccache binary (incl. a full path) -> true.
    cfg.fallback = Some("sccache".to_string());
    assert!(fallback_is_sccache(Some(&cfg)));
    cfg.fallback = Some("/usr/local/bin/sccache".to_string());
    assert!(fallback_is_sccache(Some(&cfg)));

    // A non-sccache fallback -> false.
    cfg.fallback = Some("/usr/bin/gcc".to_string());
    assert!(!fallback_is_sccache(Some(&cfg)));
}

#[test]
fn test_dir_size_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(dir_size(dir.path()), 0);
}

#[test]
fn test_dir_size_with_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), vec![0u8; 100]).unwrap();
    fs::write(dir.path().join("b.txt"), vec![0u8; 200]).unwrap();

    let size = dir_size(dir.path());
    assert!(size >= 300, "expected >= 300, got {}", size);
}

#[test]
fn test_dir_size_recursive() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("file.txt"), vec![0u8; 50]).unwrap();

    let size = dir_size(dir.path());
    assert!(size >= 50);
}

#[test]
fn test_dir_size_nonexistent() {
    assert_eq!(dir_size(std::path::Path::new("/nonexistent/path")), 0);
}

#[cfg(unix)]
#[test]
fn init_reports_whether_this_terminal_paces_tests() {
    assert_eq!(test_pacing(Some("kache test-runner")), TestPacing::Active);
    assert_eq!(
        test_pacing(Some("qemu-x86_64")),
        TestPacing::Overridden("qemu-x86_64".into())
    );
    assert_eq!(test_pacing(Some("")), TestPacing::Pending);
    assert_eq!(test_pacing(None), TestPacing::Pending);
}

#[cfg(unix)]
#[test]
fn init_reads_the_host_triple_from_rustc() {
    let host = rustc_host().expect("rustc on the test PATH");
    assert!(host.contains(std::env::consts::ARCH), "{host}");
}

#[test]
fn test_cargo_wrapper_edit_create() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let plan = plan_cargo_wrapper_edit(&path).unwrap();
    assert!(matches!(plan, CargoWrapperPlan::Create));
    let new = apply_cargo_wrapper_edit("", &plan);
    assert_eq!(new, "[build]\nrustc-wrapper = \"kache\"\n");
}

#[test]
fn test_cargo_wrapper_edit_replace() {
    let existing = "[build]\nrustc-wrapper = \"sccache\"\n";
    let plan = CargoWrapperPlan::Replace("sccache".into());
    let new = apply_cargo_wrapper_edit(existing, &plan);
    assert_eq!(new, "[build]\nrustc-wrapper = \"kache\"\n");
}

#[test]
fn test_cargo_wrapper_edit_replace_quote_styles_and_miss() {
    // Replace plan -> single-quoted, compact, and no-match branches.
    let single = apply_cargo_wrapper_edit(
        "[build]\nrustc-wrapper = 'sccache'\n",
        &CargoWrapperPlan::Replace("sccache".into()),
    );
    assert_eq!(single, "[build]\nrustc-wrapper = \"kache\"\n");

    let compact = apply_cargo_wrapper_edit(
        "[build]\nrustc-wrapper=\"sccache\"\n",
        &CargoWrapperPlan::Replace("sccache".into()),
    );
    assert_eq!(compact, "[build]\nrustc-wrapper = \"kache\"\n");

    let unchanged = apply_cargo_wrapper_edit(
        "[build]\nrustc-wrapper = \"other\"\n",
        &CargoWrapperPlan::Replace("sccache".into()),
    );
    assert_eq!(unchanged, "[build]\nrustc-wrapper = \"other\"\n");
}

#[test]
fn doctor_fix_changes_only_an_sccache_wrapper_and_keeps_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let source = "# sccache since 2023\n[build]\nrustc-wrapper = \"/opt/homebrew/bin/sccache\" # mine\njobs = 4\n\n[env]\nSCCACHE_DIR = \"/data/sccache\"\n";
    fs::write(&path, source).unwrap();

    let replaced = migrate_cargo_wrapper(&path).unwrap();

    assert_eq!(replaced.as_deref(), Some("/opt/homebrew/bin/sccache"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        source.replace("\"/opt/homebrew/bin/sccache\"", "\"kache\"")
    );
    let backups: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|backup| backup != &path)
        .map(|backup| fs::read_to_string(backup).unwrap())
        .collect();
    assert_eq!(backups, [source]);
}

#[test]
fn doctor_fix_leaves_other_cargo_configs_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    for source in [
        "[build]\nrustc-wrapper = \"other-wrapper\"\n# sccache was here\n",
        "[build]\njobs = 2\n[env]\nSCCACHE_DIR = \"/data/sccache\"\n",
        "[build\nrustc-wrapper = \"sccache\"\n",
    ] {
        fs::write(&path, source).unwrap();
        assert!(matches!(migrate_cargo_wrapper(&path), Ok(None) | Err(_)));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no backups");
}

#[test]
fn migrate_with_nothing_to_do_points_at_init() {
    let summary = migration_summary(&[], &[], false);
    assert!(summary.contains("kache init"), "{summary}");
    assert!(!summary.contains(".zshrc"), "{summary}");

    let rc = vec![(
        ".bashrc".to_string(),
        vec![(3, "export RUSTC_WRAPPER=sccache".to_string())],
    )];
    let summary = migration_summary(&[], &rc, true);
    assert!(
        summary.contains("+ export RUSTC_WRAPPER=kache"),
        "{summary}"
    );
    assert!(!summary.contains("source ~/.zshrc"), "{summary}");
    assert!(!summary.contains("--purge-sccache"), "{summary}");
    let summary = migration_summary(&[], &rc, false);
    assert!(summary.contains("--purge-sccache"), "{summary}");
}

#[test]
fn doctor_suggests_fix_only_for_failures_fix_repairs() {
    let argv = |next: &[crate::machine::NextAction]| {
        next.iter()
            .map(|action| action.argv.join(" "))
            .collect::<Vec<_>>()
    };
    assert!(doctor_next_actions(&["Remote access", "Config file"], &[], &[]).is_empty());
    assert_eq!(
        argv(&doctor_next_actions(
            &["Cargo config", "Daemon version"],
            &["Cargo config", "Daemon version"],
            &[],
        )),
        ["kache doctor --fix"]
    );
    assert_eq!(
        argv(&doctor_next_actions(
            &["RUSTC_WRAPPER", "cargo doc"],
            &[],
            &["RUSTC_WRAPPER", "cargo doc"]
        )),
        ["kache init"]
    );
    // A repair for a check that passed is not a reason to run anything.
    assert!(doctor_next_actions(&["Remote access"], &["sccache"], &["cargo doc"]).is_empty());
}

#[test]
fn cargo_wrapper_repair_names_fix_for_sccache_and_init_otherwise() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("cargo-home");
    let project = dir.path().join("project/.cargo");
    let in_home = home.join("config.toml");
    assert_eq!(cargo_wrapper_repair("sccache", &in_home, &home), Some(true));
    assert_eq!(
        cargo_wrapper_repair("/usr/local/bin/sccache", &in_home, &home),
        Some(true)
    );
    assert_eq!(cargo_wrapper_repair("other", &in_home, &home), Some(false));
    assert_eq!(cargo_wrapper_repair("", &in_home, &home), Some(false));
    assert_eq!(
        cargo_wrapper_repair("sccache", &project.join("config.toml"), &home),
        None
    );
}

#[test]
fn doctor_config_file_check_fails_on_a_file_that_does_not_parse() {
    let path = std::path::Path::new("/etc/elsewhere/kache.toml");
    let (pass, detail, fix) =
        doctor_config_file_check(path, true, Some("TOML parse error at line 2, column 3: x"));
    assert!(!pass);
    assert!(detail.contains("/etc/elsewhere/kache.toml"), "{detail}");
    assert!(detail.contains("line 2"), "{detail}");
    assert!(fix.unwrap().contains("defaults"));

    let shown = crate::wrapper_config::display_path(path);
    assert_eq!(
        doctor_config_file_check(path, true, None),
        (true, shown.clone(), None)
    );
    assert_eq!(
        doctor_config_file_check(path, false, None),
        (true, format!("{shown} (not created; defaults apply)"), None)
    );
}

#[test]
fn doctor_files_each_wrapper_repair_under_the_right_command() {
    use crate::wrapper_config::WrapperSetting;
    let home = std::path::Path::new("/home/me/.cargo");
    let in_home = |value: &str| WrapperSetting::CargoConfig {
        value: value.into(),
        path: home.join("config.toml"),
    };
    assert_eq!(wrapper_setting_repair(None, home), Some(false));
    assert_eq!(
        wrapper_setting_repair(Some(&in_home("sccache")), home),
        Some(true)
    );
    assert_eq!(
        wrapper_setting_repair(Some(&in_home("other")), home),
        Some(false)
    );
    let env = WrapperSetting::Environment {
        value: "sccache".into(),
    };
    assert_eq!(wrapper_setting_repair(Some(&env), home), None);

    let (mut fix, mut init) = (Vec::new(), Vec::new());
    note_repair("a", Some(true), &mut fix, &mut init);
    note_repair("b", Some(false), &mut fix, &mut init);
    note_repair("c", None, &mut fix, &mut init);
    assert_eq!((fix, init), (vec!["a"], vec!["b"]));
}

#[test]
fn doctor_fix_refuses_a_wrapper_it_cannot_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let source = "[build]\nrustc-wrapper  =  \"sccache\"\n";
    fs::write(&path, source).unwrap();

    let error = migrate_cargo_wrapper(&path).unwrap_err().to_string();

    assert!(error.contains("edit"), "{error}");
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no backup");
    assert!(ensure_cargo_wrapper_is_kache("[build]\nrustc-wrapper = \"kache\"\n", &path).is_ok());
    assert!(ensure_cargo_wrapper_is_kache(source, &path).is_err());
}

#[test]
fn test_cargo_wrapper_edit_add_under_build() {
    let existing = "[build]\njobs = 4\n";
    let plan = CargoWrapperPlan::AddUnderBuild;
    let new = apply_cargo_wrapper_edit(existing, &plan);
    assert!(new.contains("rustc-wrapper = \"kache\""));
    assert!(new.contains("jobs = 4"));
}

#[test]
fn test_cargo_wrapper_edit_append_section() {
    let existing = "[net]\nretry = 3\n";
    let plan = CargoWrapperPlan::AppendSection;
    let new = apply_cargo_wrapper_edit(existing, &plan);
    assert!(new.contains("[net]"));
    assert!(new.trim_end().ends_with("rustc-wrapper = \"kache\""));
}

#[test]
fn test_cargo_wrapper_edit_already_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[build]\nrustc-wrapper = \"kache\"\n").unwrap();
    let plan = plan_cargo_wrapper_edit(&path).unwrap();
    assert!(matches!(plan, CargoWrapperPlan::AlreadySet));
}

// The planner's Replace / AddUnderBuild / AppendSection arms are reached by
// reading a real config file (the apply tests above build those plans by
// hand). Drive each shape through the file-reading path, then apply the
// resulting plan to confirm the round-trip lands kache as the wrapper.
#[test]
fn test_plan_cargo_wrapper_edit_replace_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[build]\nrustc-wrapper = \"sccache\"\n").unwrap();
    let plan = plan_cargo_wrapper_edit(&path).unwrap();
    assert_eq!(plan, CargoWrapperPlan::Replace("sccache".into()));
    let new = apply_cargo_wrapper_edit(&std::fs::read_to_string(&path).unwrap(), &plan);
    assert_eq!(new, "[build]\nrustc-wrapper = \"kache\"\n");
}

#[test]
fn test_plan_cargo_wrapper_edit_add_under_build_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[build]\njobs = 8\n").unwrap();
    let plan = plan_cargo_wrapper_edit(&path).unwrap();
    assert_eq!(plan, CargoWrapperPlan::AddUnderBuild);
    let new = apply_cargo_wrapper_edit(&std::fs::read_to_string(&path).unwrap(), &plan);
    assert!(new.contains("jobs = 8"));
    assert!(new.contains("rustc-wrapper = \"kache\""));
}

#[test]
fn test_plan_cargo_wrapper_edit_append_section_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[net]\nretry = 2\n").unwrap();
    let plan = plan_cargo_wrapper_edit(&path).unwrap();
    assert_eq!(plan, CargoWrapperPlan::AppendSection);
    let new = apply_cargo_wrapper_edit(&std::fs::read_to_string(&path).unwrap(), &plan);
    assert!(new.contains("[net]"));
    assert!(new.contains("[build]"));
    assert!(new.contains("rustc-wrapper = \"kache\""));
}

#[test]
fn test_plan_cargo_wrapper_edit_rejects_malformed_toml() {
    // A file that isn't valid TOML surfaces the parse-error context arm.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "this = = not valid toml\n").unwrap();
    let err = plan_cargo_wrapper_edit(&path).unwrap_err();
    assert!(
        err.to_string().contains("parsing"),
        "expected a parse-context error, got: {err}"
    );
}



// ── backend-neutral remote tests ──────────────────────────────────────────
















fn save_manifest_config(
    cache_dir: std::path::PathBuf,
) -> Config {
    use crate::config::DEFAULT_DAEMON_IDLE_TIMEOUT_SECS;
    Config {
        socket_path_override: None,
        fallback: None,
        key_salt: None,
        cc_extra_allowlist_flags: Vec::new(),
        readonly_store: None,
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

        auto_recover_min_free_bytes: 0,

        scheduler_memory_pressure: true,

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
        runtime_dir: cache_dir.clone(),
        cache_dir,
        max_size: 1024 * 1024,
        disabled: false,
        cache_executables: false,
        cache_cc_links: false,
        trust_codegen_backends: false,
        clean_incremental: true,
        preserve_incremental: false,
        adaptive_incremental: true,
        seed_incremental: true,
        incremental_seed_from: None,
        unstable: Default::default(),
        event_log_max_size: 1024 * 1024,
        event_log_keep_lines: 1000,
        compression_level: 3,
        min_store_compile_ms: crate::config::DEFAULT_MIN_STORE_COMPILE_MS,
        gc_max_age_hours: crate::config::DEFAULT_GC_MAX_AGE_HOURS,
        daemon_idle_timeout_secs: DEFAULT_DAEMON_IDLE_TIMEOUT_SECS,
    }
}

fn put_entry(config: &Config, key: &str, crate_name: &str, dir: &std::path::Path) {
    let store = Store::open(config).unwrap();
    let src = dir.join(format!("{key}.rlib"));
    std::fs::write(&src, format!("artifact bytes for {key}")).unwrap();
    store
        .put(
            key,
            crate_name,
            &["lib".to_string()],
            &[],
            "x86_64-unknown-linux-gnu",
            "debug",
            &[(src, format!("{key}.rlib"))],
            "",
            "",
        )
        .unwrap();
}

fn overwrite_entry_meta(
    config: &Config,
    key: &str,
    crate_name: &str,
    mut meta: crate::store::EntryMeta,
) {
    meta.cache_key = key.to_string();
    meta.crate_name = crate_name.to_string();
    std::fs::write(
        config.store_dir().join(key).join("meta.json"),
        deser_json::to_string(&meta).map(String::into_bytes).unwrap(),
    )
    .unwrap();
}

#[test]
fn why_miss_no_events_prints_tip() {
    // No events for the crate -> the "build it first" tip path.
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    why_miss(&config, "ghost", false).expect("why_miss with no events should succeed");
}

#[test]
fn why_miss_shows_stored_metadata_for_a_missed_key() {
    // A Miss event whose cache_key has a stored entry on disk drives the
    // miss-metadata display (target/profile) + the stored-entries listing.
    // Covers why_miss's metadata branch (cli.rs ~471-490+).
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    // Seed a store entry for "serde" so meta.json (with target/profile) exists.
    put_entry(&config, "serdemisskey", "serde", dir.path());
    // Log a Miss event for that crate + key.
    crate::events::log_event(
        &config.event_log_path(),
        &build_event(
            "serde",
            crate::events::EventResult::Miss,
            1234,
            1300,
            4096,
            "serdemisskey",
        ),
    )
    .unwrap();

    why_miss(&config, "serde", false).expect("why_miss should succeed for a missed key");
}

#[test]
fn why_miss_leads_with_a_store_failure_when_there_was_one() {
    // The store-failure banner replaces guesswork about the key: nothing was
    // stored, so no later build could have matched (kunobi-ninja/kache#629).
    let mut miss = build_event(
        "lint_crate",
        crate::events::EventResult::Miss,
        1000,
        900,
        4096,
        "somekey",
    );
    assert!(
        store_failure_banner(&miss).is_none(),
        "a normal miss gets no banner"
    );

    miss.store_error = "creating blob shard directory: Permission denied (os error 13)".into();
    let banner = store_failure_banner(&miss).expect("failed store should produce a banner");
    assert!(banner.contains("NOT CACHED"));
    assert!(banner.contains("Permission denied (os error 13)"));
    // The stored-entry analysis owns the `Diagnosis:` label; a second one
    // here would read as a competing conclusion.
    assert!(!banner.contains("Diagnosis:"), "banner: {banner}");
}

#[test]
fn why_miss_prioritizes_same_key_lookup_rejection() {
    let mut miss = build_event(
        "foo.c",
        crate::events::EventResult::Miss,
        10,
        20,
        30,
        "same-key",
    );
    assert!(lookup_rejection_banner(&miss, true).is_none());

    miss.lookup_rejection = "matching entry lacks dep-info required by this invocation".to_string();
    let banner = lookup_rejection_banner(&miss, true).unwrap();
    assert!(banner.contains("matching key was found but rejected"));
    assert!(banner.contains("lacks dep-info required"));
    assert!(banner.contains("currently present under the same key"));
    assert!(!banner.contains("key mismatch"), "banner: {banner}");
}

#[test]
fn why_miss_legacy_same_key_repeat_does_not_claim_key_mismatch() {
    let mut miss = build_event(
        "foo.c",
        crate::events::EventResult::Miss,
        10,
        20,
        30,
        "same-key",
    );
    miss.schema = 14;

    assert!(legacy_repeated_same_key_banner(&miss, false).is_none());
    let banner = legacy_repeated_same_key_banner(&miss, true).unwrap();
    assert!(banner.contains("repeated miss for the same cache key"));
    assert!(banner.contains("older event did not record"));
    assert!(banner.contains("not caused by a cache-key change"));
    assert!(
        !banner.contains("Diagnosis: key mismatch"),
        "banner: {banner}"
    );

    miss.schema = 15;
    assert!(legacy_repeated_same_key_banner(&miss, true).is_none());
}

#[test]
fn why_miss_all_hits_reports_no_misses() {
    // Events exist but none are Miss/Dup -> the "all events are hits" path.
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    crate::events::log_event(
        &config.event_log_path(),
        &build_event(
            "tokio",
            crate::events::EventResult::LocalHit,
            0,
            5,
            4096,
            "tokiohitkey",
        ),
    )
    .unwrap();

    why_miss(&config, "tokio", false).expect("why_miss with only hits should succeed");
}

#[test]
fn why_miss_reports_many_stored_diffs() {
    // Stored miss + many other entries -> diff printer cap branch.
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    put_entry(&config, "misskeymanydiffs", "serde", dir.path());
    overwrite_entry_meta(
        &config,
        "misskeymanydiffs",
        "serde",
        diff_meta("wasm32", "release", &[], &["lib"]),
    );
    for i in 0..3 {
        let key = format!("otherkeymanydiffs{i}");
        let feature = format!("feat{i}");
        put_entry(&config, &key, "serde", dir.path());
        overwrite_entry_meta(
            &config,
            &key,
            "serde",
            diff_meta(
                &format!("target{i}"),
                &format!("profile{i}"),
                &[&feature],
                &["bin"],
            ),
        );
    }
    crate::events::log_event(
        &config.event_log_path(),
        &build_event(
            "serde",
            crate::events::EventResult::Miss,
            123,
            130,
            4096,
            "misskeymanydiffs",
        ),
    )
    .unwrap();

    why_miss(&config, "serde", false).expect("why_miss should print capped diffs");
}

#[test]
fn why_miss_diff_messages_cap_feature_and_type_diffs() {
    // Diff helper -> empty miss features, crate-type diffs, and output cap.
    let miss = diff_meta("wasm32", "release", &[], &["lib"]);
    let others: Vec<_> = (0..3)
        .map(|i| {
            (
                format!("other{i}"),
                diff_meta("x86_64", "debug", &[&format!("feat{i}")], &["bin"]),
            )
        })
        .collect();
    let (messages, extra) = why_miss_diff_messages(
        &miss,
        others.iter().map(|(key, meta)| (key.as_str(), meta)),
        5,
    );

    assert_eq!(messages.len(), 5);
    assert!(extra > 0, "expected capped messages, got {messages:?}");
    assert!(messages.iter().any(|m| m.contains("different target")));
    assert!(messages.iter().any(|m| m.contains("different profile")));
    assert!(
        messages.iter().any(|m| m.contains("[(none)] vs [feat0]")),
        "got {messages:?}"
    );
    assert!(messages.iter().any(|m| m.contains("different crate types")));
}

#[test]
fn why_miss_diff_messages_dedupes_same_config_and_empty_other_features() {
    // Diff helper -> empty other features and same-config de-duplication.
    let miss = diff_meta("x86_64", "debug", &["feat"], &["lib"]);
    let others = [
        (
            "empty-features",
            diff_meta("x86_64", "debug", &[], &["lib"]),
        ),
        ("same-a", diff_meta("x86_64", "debug", &["feat"], &["lib"])),
        ("same-b", diff_meta("x86_64", "debug", &["feat"], &["lib"])),
    ];
    let (messages, extra) =
        why_miss_diff_messages(&miss, others.iter().map(|(key, meta)| (*key, meta)), 5);

    assert_eq!(extra, 0);
    assert!(
        messages.iter().any(|m| m.contains("[feat] vs [(none)]")),
        "got {messages:?}"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.contains("likely a source"))
            .count(),
        1
    );
}

#[test]
fn explain_crate_last_miss_rows_name_the_key_its_config_and_what_shifts_it() {
    use crate::events::EventResult;
    let miss = build_event("dep", EventResult::Miss, 10, 20, 30, "abcdef0123456789");
    let mut hit = build_event("dep", EventResult::LocalHit, 0, 1, 30, "fedcba9876543210");
    hit.ts = miss.ts - std::time::Duration::from_secs(5);
    let meta = diff_meta("x86_64", "", &["a", "b"], &["lib"]);
    let rows = last_miss_rows(&miss, Some(&meta), Some(&hit), Some("salt"));
    let labels: Vec<&str> = rows.iter().map(|row| row.0).collect();
    assert_eq!(
        labels,
        [
            "Result", "Key", "Target", "Features", "Last hit", "Key salt"
        ]
    );
    assert_eq!(rows[0].1, "miss");
    assert_eq!(rows[1].1, "abcdef012345");
    assert_eq!(rows[3].1, "a, b");
    assert_eq!(rows[4].1, "fedcba987654");
    assert_eq!(rows[5].1, "\"salt\"");

    // A hit under the same key, or one after the miss, explains nothing.
    let mut same = hit.clone();
    same.cache_key = miss.cache_key.clone();
    let mut later = hit.clone();
    later.ts = miss.ts + std::time::Duration::from_secs(5);
    let mut simultaneous = hit.clone();
    simultaneous.ts = miss.ts;
    for hit in [&same, &later, &simultaneous] {
        let rows = last_miss_rows(&miss, None, Some(hit), Some(""));
        let labels: Vec<&str> = rows.iter().map(|row| row.0).collect();
        assert_eq!(labels, ["Result", "Key"]);
    }
    let mut keyless = miss.clone();
    keyless.cache_key.clear();
    assert_eq!(last_miss_rows(&keyless, None, None, None)[1].1, "none");
}

#[test]
fn explain_crate_diagnosis_lines_lead_with_the_cause() {
    use crate::events::EventResult;
    let miss = build_event("dep", EventResult::Miss, 10, 20, 30, "abcdef0123456789");
    let never = MissDiagnosis::new(&miss, false, 0, false, None, false);
    assert_eq!(
        miss_diagnosis_lines(&miss, None, &[], &never),
        ["  never cached: first build of this crate"]
    );
    assert_eq!(
        extern_chain_lines(&never),
        [
            "  not analyzed: no per-dependency digests recorded",
            "  set explain_miss = true under [cache], then rebuild",
        ]
    );
    let recorded = MissDiagnosis::new(&miss, false, 0, false, None, true);
    assert!(extern_chain_lines(&recorded).is_empty());

    let now_cached = MissDiagnosis::new(&miss, false, 1, true, None, false);
    assert_eq!(
        miss_diagnosis_lines(&miss, None, &[], &now_cached),
        ["  first build with these inputs: the entry is now cached"]
    );
    for (stored, others) in [(2, "1 other stored entry"), (3, "2 other stored entries")] {
        let mismatch = MissDiagnosis::new(&miss, false, stored, true, None, false);
        assert_eq!(
            miss_diagnosis_lines(&miss, None, &[], &mismatch),
            [format!(
                "  key mismatch: key abcdef012345 differs from {others}"
            )]
        );
    }

    let mut failed = miss.clone();
    failed.store_error = "disk full".into();
    let not_cached = MissDiagnosis::new(&failed, false, 0, false, None, false);
    let lines = miss_diagnosis_lines(&failed, None, &[], &not_cached);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("  NOT CACHED: "), "{lines:?}");
    assert!(lines[0].ends_with("reason: disk full"), "{lines:?}");
}

fn stats_entry(key: &str, profile: &str, crate_type: &str) -> crate::daemon::StatsEntry {
    crate::daemon::StatsEntry {
        store_dirs: vec!["/nonexistent-store".into()],
        cache_key: key.to_string(),
        crate_name: "dep".to_string(),
        crate_type: crate_type.to_string(),
        profile: profile.to_string(),
        size: 1024,
        hit_count: 3,
        created_at: String::new(),
        last_accessed: "not a date".to_string(),
        content_hash: None,
    }
}

#[test]
fn explain_crate_finds_the_missed_entry_only_by_its_own_key() {
    let other = stats_entry("other-key", "", "");
    let keyless = stats_entry("", "", "");
    let missed = stats_entry("missed-key", "", "");
    let stored = [&other, &keyless, &missed];
    assert_eq!(
        missed_entry(&stored, "missed-key").map(|e| e.cache_key.as_str()),
        Some("missed-key")
    );
    assert!(
        missed_entry(&stored, "").is_none(),
        "a keyless miss matches nothing"
    );
    assert!(missed_entry(&stored, "absent").is_none());
}

#[test]
fn explain_crate_warns_once_per_unreadable_store() {
    let stores = [
        crate::store_view::StoreSummary {
            path: "/fine".into(),
            ..Default::default()
        },
        crate::store_view::StoreSummary {
            path: "/broken".into(),
            error: Some("index locked".into()),
            ..Default::default()
        },
    ];
    assert_eq!(
        unavailable_store_warnings(&stores),
        ["warning: store /broken is unavailable (index locked)"]
    );
}

#[test]
fn explain_crate_entry_rows_name_what_sets_each_entry_apart() {
    let entry = stats_entry("abcdef0123456789", "debug", "lib");
    let meta = diff_meta("x86_64", "debug", &["a", "b"], &["lib"]);
    let row = stored_entry_row(&entry, Some(&meta), Some("abcdef0123456789"));
    assert_eq!(
        row,
        [
            "abcdef012345",
            "not a date",
            "1.0 KiB",
            "3",
            "/nonexistent-store",
            "profile debug, type lib, target x86_64, features [a, b], the missed key, stored \
             after the compile",
        ]
    );
    let bare = stats_entry("abcdef0123456789", "", "");
    let empty = diff_meta("", "", &[], &[]);
    assert_eq!(stored_entry_row(&bare, Some(&empty), Some("other"))[5], "");
    assert_eq!(stored_entry_row(&bare, None, None)[5], "");

    let entries: Vec<_> = (0..11)
        .map(|i| stats_entry(&format!("key{i:02}"), "", ""))
        .collect();
    let refs: Vec<_> = entries.iter().collect();
    let ten = stored_entry_lines(None, &refs[..10]);
    assert_eq!(ten.len(), 11, "a header and ten rows: {ten:?}");
    let eleven = stored_entry_lines(None, &refs);
    assert_eq!(eleven.len(), 12, "{eleven:?}");
    assert_eq!(eleven[11], "  and 1 older entry");
}

#[test]
fn explain_crate_differences_stop_at_five_and_count_the_rest() {
    let miss = diff_meta("wasm32", "debug", &[], &["lib"]);
    let others = |n: usize| -> Vec<(String, crate::store::EntryMeta)> {
        (0..n)
            .map(|i| {
                (
                    format!("k{i}"),
                    diff_meta(&format!("t{i}"), "debug", &[], &["lib"]),
                )
            })
            .collect()
    };
    let five = difference_lines(&miss, &others(5));
    assert_eq!(five.len(), 5, "{five:?}");
    assert!(
        five[0].starts_with("    - different target vs k0"),
        "{five:?}"
    );
    let seven = difference_lines(&miss, &others(7));
    assert_eq!(seven.len(), 6, "{seven:?}");
    assert_eq!(seven[5], "    and 2 more");
}

#[test]
fn explain_crate_cascade_notes_uncached_compiles_and_unresolved_endpoints() {
    use crate::events::EventResult;
    use crate::miss_chain::{Chain, ChangedDep, PassthroughGroup, Root, RootKind};
    let miss = build_event("app", EventResult::Miss, 10, 20, 30, "abcdef0123456789");
    let chain = |kind: RootKind, passthroughs: Vec<PassthroughGroup>| Chain {
        roots: vec![Root {
            crate_name: "leaf".to_string(),
            unit: None,
            kind,
            passthroughs,
            branches: 1,
            path: Vec::new(),
        }],
        direct: vec![ChangedDep {
            name: "leaf".to_string(),
            from: Some("a".to_string()),
            to: Some("b".to_string()),
            unit: None,
        }],
        truncated: None,
        baseline_root: None,
    };
    let resolved = MissDiagnosis::new(
        &miss,
        false,
        0,
        false,
        Some(chain(
            RootKind::Groups(vec!["sources".to_string()]),
            vec![PassthroughGroup {
                reason: "unsupported flag".to_string(),
                count: 2,
            }],
        )),
        true,
    );
    assert_eq!(
        extern_chain_lines(&resolved),
        [
            "  this miss is downstream of a dependency that changed (leaf)",
            "  root: leaf, its own inputs changed: sources",
            "    2x uncached compile in leaf: unsupported flag",
            "    (uncached compiles make the artifact vary per checkout; attributed by package \
             directory)",
        ]
    );
    let unresolved = MissDiagnosis::new(
        &miss,
        false,
        0,
        false,
        Some(chain(RootKind::NoBaseline, Vec::new())),
        true,
    );
    assert_eq!(
        extern_chain_lines(&unresolved),
        [
            "  this miss is downstream of a dependency that changed (leaf)",
            "  unresolved: leaf, nothing earlier to compare it against",
            "  note: no endpoint could be resolved; the recorded history does not explain this \
             miss",
        ]
    );
}

#[test]
fn explain_crate_sections_skip_empty_bodies_and_tabulate_events() {
    use crate::events::EventResult;
    assert!(section("Hints", Vec::new()).is_empty());
    assert_eq!(
        section("Hints", vec!["  a".to_string()]),
        ["", "Hints", "  a"]
    );
    assert_eq!(stored_entry_lines(None, &[]), ["  none"]);
    let miss = build_event("dep", EventResult::Miss, 10, 20, 2048, "abcdef0123456789");
    let lines = recent_event_lines(&[&miss]);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("  TIME"), "{lines:?}");
    assert!(
        lines[1].ends_with("miss     abcdef012345     20 ms   2.0 KiB"),
        "{lines:?}"
    );
}

#[test]
fn checkout_comparison_lines_separate_path_only_from_input_changes() {
    use crate::miss_chain::{ChangedDep, CheckoutComparison, CheckoutVerdict};
    let dep = |name: &str| ChangedDep {
        name: name.to_string(),
        from: Some("a".to_string()),
        to: Some("b".to_string()),
        unit: None,
    };
    let compared = |verdict, groups: &[&str], dependencies: Vec<ChangedDep>| {
        checkout_comparison_lines(&CheckoutComparison {
            root: "/b".to_string(),
            baseline_root: "/a".to_string(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
            dependencies,
            same_key: verdict == CheckoutVerdict::PathOnly,
            verdict,
        })
    };

    let lines = compared(CheckoutVerdict::PathOnly, &[], vec![]);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("the key is not why this missed"));

    let lines = compared(CheckoutVerdict::OwnInputs, &["env_deps"], vec![]);
    assert_eq!(lines[0], "own inputs differ: env_deps");
    assert_eq!(lines.len(), 2, "no dependency or remap line: {lines:?}");
    assert!(lines[1].contains("real input difference"));

    let lines = compared(
        CheckoutVerdict::OwnInputs,
        &["args", "remap"],
        vec![dep("mid"), dep("util")],
    );
    assert_eq!(lines[0], "own inputs differ: args, remap");
    assert_eq!(lines[1], "dependencies differ: mid, util");
    assert!(lines[2].contains("real input difference"), "{lines:?}");
    assert!(lines[3].starts_with("(remap:"), "{lines:?}");

    // Only `remap` differs: the normalization note does not apply.
    let lines = compared(CheckoutVerdict::OwnInputs, &["remap"], vec![]);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[1].starts_with("(remap:"), "{lines:?}");

    let lines = compared(CheckoutVerdict::Dependencies, &[], vec![dep("mid")]);
    assert_eq!(
        lines,
        vec!["own inputs match after path normalization; dependencies differ: mid"]
    );

    let lines = compared(CheckoutVerdict::Untraced, &[], vec![]);
    assert!(lines[0].contains("key salt or extra inputs"));
}



#[test]
fn telemetry_write_emits_cache_scope_not_bench() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let out = dir.path().join("otlp");
    telemetry_write(&config, &out, Some("bench-firefox"), Some("warm")).expect("telemetry write");
    let body = std::fs::read_to_string(out.join("metrics.otlp.json")).unwrap();
    assert!(
        body.contains("\"kache.cache\""),
        "cache OTLP must use the kache.cache scope"
    );
    assert!(
        body.contains("\"kache.cache.scenario\""),
        "bench dumps must name the scenario"
    );
    assert!(
        body.contains("bench-firefox"),
        "scenario value must match kache.bench.project"
    );
    assert!(
        body.contains("\"kache.cache.phase\""),
        "bench dumps must name the phase"
    );
    assert!(
        body.contains("warm"),
        "phase value must match the bench phase"
    );
    assert!(
        !body.contains("kache.bench."),
        "cache dump must not mix bench gauges"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("schema_version"))
            .unwrap()
            .trim(),
        "2"
    );
}

#[test]
fn purge_skips_corrupt_filtered_entry() {
    // purge(crate) -> corrupt meta removal error is skipped, not fatal.
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    put_entry(&config, "badpurgekey", "bad", dir.path());
    std::fs::write(
        config.store_dir().join("badpurgekey").join("meta.json"),
        b"{ not valid json",
    )
    .unwrap();

    purge(&config, Some("bad")).expect("purge skips corrupt entries");
    assert!(
        config
            .store_dir()
            .join("badpurgekey")
            .join("meta.json")
            .exists(),
        "corrupt entry remains accounted-for after skipped purge"
    );
}

/// Purge is a bulk mutation and must serialize behind the same
/// cross-process lock every GC driver takes; unlocked, a purge can
/// interleave with a live sweep and each observes the other's
/// half-removed state.
#[test]
fn purge_waits_for_the_gc_lock() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    put_entry(&config, "purgelockkey", "locked", dir.path());

    let store = crate::store::Store::open(&config).unwrap();
    let gc_lock = store.try_gc_lock().unwrap().expect("uncontended lock");

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let cfg = config.clone();
    let worker = std::thread::spawn(move || {
        let result = purge(&cfg, None);
        let _ = done_tx.send(());
        result
    });

    // While the "sweep" holds gc.lock the purge must not complete.
    // (A scheduling hiccup can only delay the mutant's completion
    // signal, never produce a false failure for the real code.)
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "purge completed while the GC lock was held"
    );
    drop(gc_lock);
    done_rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("purge proceeds once the GC lock is released");
    worker.join().unwrap().expect("purge succeeds");
    let store = crate::store::Store::open(&config).unwrap();
    assert_eq!(store.entry_count().unwrap(), 0, "store cleared");
}

#[test]
fn verify_reports_valid_entries_on_a_clean_store() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    put_entry(&config, "validkey1", "serde", dir.path());

    // A clean store verifies with checksums and no repair needed.
    verify(&config, true, false).expect("verify of a clean store should succeed");
}

/// The `kache stats` row labelled `label` as `label | value | note`, the
/// same form as [`row_text`].
#[test]
fn doctor_groups_each_label_by_its_component() {
    for label in ["Daemon socket", "Service file", "Remote cache"] {
        assert_eq!(doctor_section(label), "Services", "{label}");
    }
    for label in [
        "Cache directory",
        "Store index",
        "Shard mount",
        "Link layout",
    ] {
        assert_eq!(doctor_section(label), "Storage", "{label}");
    }
    assert_eq!(doctor_section("RUSTC_WRAPPER"), "Setup");
    assert_eq!(doctor_section("Other"), "Setup");
}

#[test]
fn login_service_configuration_covers_existing_and_new_installations() {
    assert!(!login_service_configured(false, false));
    assert!(login_service_configured(true, false));
    assert!(login_service_configured(false, true));
    assert!(login_service_configured(true, true));
}

fn stats_row(lines: &[String], label: &str) -> Option<String> {
    let line = lines.iter().find(|line| {
        line.strip_prefix("  ")
            .and_then(|rest| rest.strip_prefix(label))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with("   "))
    })?;
    let mut cells = line
        .split("   ")
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .skip(1);
    let value = cells.next().unwrap_or_default();
    let note = cells.collect::<Vec<_>>().join("   ");
    Some(format!("{label} | {value} | {note}"))
}


#[test]
fn render_stats_cache_fallback_labels_logical_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let snap = StatsSnapshot {
        total_size: 8192,
        blob_stats: None,
        ..Default::default()
    };
    let lines = render_stats(&snap, &config, SinceWindow::DEFAULT);
    assert_eq!(
        stats_row(&lines, "Cache").unwrap(),
        "Cache | 8.0 KiB | logical (blob size unavailable) · 0 entries"
    );
}

#[test]
fn render_stats_cache_preserves_logical_size_without_registered_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let snap = StatsSnapshot {
        total_size: 8192,
        blob_stats: Some(crate::store::BlobStats {
            total_logical_size: 8192,
            ..Default::default()
        }),
        ..Default::default()
    };
    let lines = render_stats(&snap, &config, SinceWindow::DEFAULT);
    assert_eq!(
        stats_row(&lines, "Cache").unwrap(),
        "Cache | 0 B | blob bytes · 0 entries"
    );
    assert_eq!(
        stats_row(&lines, "Dedup").unwrap(),
        "Dedup | 0 blobs | 8.0 KiB logical, 0.0% saved"
    );
}

#[test]
fn render_stats_cache_shards_use_blob_bytes_and_label_missing_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let snap = StatsSnapshot {
        stores: vec![
            crate::store_view::StoreSummary {
                path: "/cache-one".into(),
                bytes: 8192,
                max_size: 4096,
                entries: 2,
                blob_stats: Some(crate::store::BlobStats {
                    total_blob_size: 1024,
                    ..Default::default()
                }),
                ..Default::default()
            },
            crate::store_view::StoreSummary {
                path: "/cache-two".into(),
                bytes: 2048,
                max_size: 4096,
                entries: 1,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let stores = cache_rows(&snap, &config)
        .into_iter()
        .filter(|row| row.0 == "Store")
        .collect::<Vec<_>>();
    assert_eq!(
        stores,
        vec![
            (
                "Store",
                "1.0 KiB".into(),
                "blob bytes · of 4.0 KiB · 2 entries · /cache-one".into()
            ),
            (
                "Store",
                "2.0 KiB".into(),
                "logical (blob size unavailable) · of 4.0 KiB · 1 entry · /cache-two".into()
            ),
        ]
    );
}



#[test]
fn stats_summary_shows_every_source_even_when_zero() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    for (counts, expected) in [
        ([30, 2, 86], "local 30 · dup 2 · miss 86"),
        ([4, 0, 0], "local 4 · dup 0 · miss 0"),
        ([0, 0, 0], "local 0 · dup 0 · miss 0"),
    ] {
        let mut snap = StatsSnapshot::default();
        [
            snap.event_stats.local_hits,
            snap.event_stats.dups,
            snap.event_stats.misses,
        ] = counts;
        let lines = render_stats(&snap, &config, SinceWindow::DEFAULT);
        assert_eq!(
            stats_row(&lines, "Sources"),
            Some(format!("Sources | {expected} | "))
        );
    }
}

#[test]
fn hit_rate_row_adds_every_outcome() {
    let row = |local, dups, misses| {
        let dir = tempfile::tempdir().unwrap();
        let config = save_manifest_config(dir.path().join("cache"));
        let mut snap = StatsSnapshot::default();
        snap.event_stats.local_hits = local;
        snap.event_stats.dups = dups;
        snap.event_stats.misses = misses;
        let lines = render_stats(&snap, &config, SinceWindow::DEFAULT);
        stats_row(&lines, "Hit rate").unwrap_or_default()
    };
    assert!(
        row(5, 0, 0).contains("5 of 5 crates from cache"),
        "{}",
        row(5, 0, 0)
    );
    assert!(
        row(3, 2, 0).contains("3 of 5 crates from cache"),
        "{}",
        row(3, 2, 0)
    );
}









#[test]
fn render_stats_offline_and_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let snap = StatsSnapshot::default(); // daemon_connected=false
    let out = render_stats(&snap, &config, SinceWindow::DEFAULT);
    assert_eq!(stats_row(&out, "Daemon").unwrap(), "Daemon | offline | ");
    // No blobs -> no Dedup row.
    assert_eq!(stats_row(&out, "Dedup"), None);
}

#[test]
fn cache_limit_note_names_the_disk_share_budget() {
    const GIB: u64 = 1 << 30;
    let share = format!(
        " (5% of {}, floor 5.0 GiB, cap 100.0 GiB)",
        kache_core::size::format_bytes(200 * GIB)
    );
    // 5% of 200 GiB is 10 GiB, inside the floor and the cap.
    assert_eq!(cache_limit_note(0, 10 * GIB, Some(200 * GIB)), share);
    assert_eq!(cache_limit_note(1, 10 * GIB, Some(200 * GIB)), share);
    assert_eq!(cache_limit_note(2, 10 * GIB, Some(200 * GIB)), "");
    assert_eq!(cache_limit_note(0, 7 * GIB, Some(200 * GIB)), "");
    // 5% of 1 GiB rounds to 0, then the floor raises the budget to 5 GiB.
    let floor = cache_limit_note(0, 5 * GIB, Some(GIB));
    assert!(floor.contains("floor"), "{floor}");
    assert!(floor.contains("floor 5.0 GiB"), "{floor}");
    assert!(floor.contains(&kache_core::size::format_bytes(GIB).to_string()), "{floor}");
    // 5% of 4000 GiB is 200 GiB, then the cap lowers the budget to 100 GiB.
    let cap = cache_limit_note(0, 100 * GIB, Some(4000 * GIB));
    assert!(cap.contains("cap"), "{cap}");
    assert!(cap.contains("cap 100.0 GiB"), "{cap}");
    assert_eq!(
        cache_limit_note(0, 50 * GIB, None),
        " (default; disk size unknown)"
    );
    assert_eq!(
        cache_limit_note(1, 50 * GIB, Some(0)),
        " (default; disk size unknown)"
    );
    assert_eq!(cache_limit_note(0, 0, Some(0)), "");
    assert_eq!(cache_limit_note(0, 49 * GIB, None), "");
}

#[test]
fn render_stats_handles_zero_limits_and_zero_logical_dedup() {
    // Zero max/logical sizes -> percentage branches stay finite.
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let snap = StatsSnapshot {
        total_size: 500,
        max_size: 0,
        blob_stats: Some(crate::store::BlobStats {
            total_blobs: 2,
            total_blob_size: 500,
            total_logical_size: 0,
            savings: 0,
        }),
        ..Default::default()
    };

    let window = SinceWindow::parse("15m").unwrap();
    let out = render_stats(&snap, &config, window);
    assert_eq!(
        stats_row(&out, "Cache").unwrap(),
        "Cache | 500 B | blob bytes · 0 entries"
    );
    assert_eq!(
        stats_row(&out, "Dedup").unwrap(),
        "Dedup | 2 blobs | 0 B logical, 0.0% saved"
    );
    // #897: the title names the requested window, not a hardcoded 24h.
    assert_eq!(out[0], "kache · last 15m");
    assert_eq!(
        stats_row(&out, "Time saved").unwrap(),
        "Time saved | none yet | "
    );

    let no_blobs = StatsSnapshot {
        blob_stats: Some(crate::store::BlobStats {
            total_blobs: 0,
            total_blob_size: 0,
            total_logical_size: 0,
            savings: 0,
        }),
        ..Default::default()
    };
    let out = render_stats(&no_blobs, &config, window);
    assert_eq!(
        stats_row(&out, "Dedup"),
        None,
        "an empty blob snapshot must not render a dedup row"
    );
}

#[test]
fn snapshot_from_direct_reads_reflects_store_and_events() {
    // No daemon: the snapshot is built from direct store + event-log reads.
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    put_entry(&config, "serdekey", "serde", dir.path());
    // Log a couple of events so event_stats is populated.
    crate::events::log_event(
        &config.event_log_path(),
        &build_event(
            "serde",
            crate::events::EventResult::LocalHit,
            0,
            5,
            4096,
            "serdekey",
        ),
    )
    .unwrap();
    crate::events::log_event(
        &config.event_log_path(),
        &build_event(
            "tokio",
            crate::events::EventResult::Miss,
            900,
            950,
            8192,
            "tk",
        ),
    )
    .unwrap();

    let snap = snapshot_from_direct_reads(&config, SinceWindow::DEFAULT, false);
    assert!(!snap.daemon_connected, "direct reads report no daemon");
    assert_eq!(snap.entry_count, 1);
    assert_eq!(snap.event_stats.local_hits, 1);
    assert_eq!(snap.event_stats.misses, 1);
    assert_eq!(snap.max_size, config.max_size);
}

/// #897: `--since` must narrow the counters, not just the label. A 15m
/// window excludes the three-hour-old miss that a 24h window includes.
#[test]
fn snapshot_from_direct_reads_honors_a_sub_hour_window() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let now = kache_core::time::UtcTime::now();
    let mut recent = build_event(
        "serde",
        crate::events::EventResult::LocalHit,
        0,
        5,
        4096,
        "a",
    );
    recent.ts = now - std::time::Duration::from_secs(10 * 60);
    let mut old = build_event(
        "tokio",
        crate::events::EventResult::Miss,
        900,
        950,
        8192,
        "b",
    );
    old.ts = now - std::time::Duration::from_secs(3 * 3600);
    crate::events::log_event(&config.event_log_path(), &recent).unwrap();
    crate::events::log_event(&config.event_log_path(), &old).unwrap();

    let narrow = SinceWindow::parse("15m").unwrap();
    let snap = snapshot_from_direct_reads(&config, narrow, false);
    assert_eq!(snap.event_stats.local_hits, 1);
    assert_eq!(snap.event_stats.misses, 0, "a 3h-old miss is outside 15m");

    let wide = SinceWindow::parse("24h").unwrap();
    let snap = snapshot_from_direct_reads(&config, wide, false);
    assert_eq!(snap.event_stats.local_hits, 1);
    assert_eq!(snap.event_stats.misses, 1);
}

#[test]
fn repair_tip_and_reconciliation_message_cover_every_boundary() {
    assert!(!should_print_repair_tip(0, 0, 0, false));
    assert!(should_print_repair_tip(1, 0, 0, false));
    assert!(should_print_repair_tip(0, 1, 0, false));
    assert!(should_print_repair_tip(0, 0, 1, false));
    assert!(!should_print_repair_tip(1, 1, 1, true));

    assert_eq!(
        reconciled_index_message(crate::store::BlobIndexDrift::default()),
        None
    );
    assert_eq!(
        reconciled_index_message(crate::store::BlobIndexDrift {
            entry_mappings: 1,
            blobs: 2,
        })
        .as_deref(),
        Some("Repairing: reconciled 1 entry mappings and 2 blob rows.")
    );
}

#[test]
fn verify_detects_missing_blob_and_missing_meta_then_repairs() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let store = Store::open(&config).unwrap();

    // Entry A: blob deleted -> "missing blob" path.
    put_entry(&config, "missingblobkey", "aaa", dir.path());
    let meta_a = store.get("missingblobkey").unwrap().unwrap();
    let blob_a = store.blob_path(&meta_a.files[0].hash);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            blob_a.parent().unwrap(),
            std::fs::Permissions::from_mode(0o755),
        );
    }
    let _ = std::fs::remove_file(&blob_a);

    // Entry B: meta.json deleted -> "missing meta" path.
    put_entry(&config, "missingmetakey", "bbb", dir.path());
    std::fs::remove_file(config.store_dir().join("missingmetakey").join("meta.json")).unwrap();

    // Entry C: meta.json corrupted -> "invalid meta" path.
    put_entry(&config, "badmetakey", "ccc", dir.path());
    std::fs::write(
        config.store_dir().join("badmetakey").join("meta.json"),
        b"{ not valid json",
    )
    .unwrap();

    // Entry D: a clean valid entry.
    put_entry(&config, "validkey", "ddd", dir.path());

    // repair=true attempts to remove every corrupted entry; the call must
    // succeed regardless of whether each removal is permitted.
    verify(&config, false, true).expect("verify --repair should succeed");

    // The valid entry survives. The missing-blob entry has a *parseable*
    // meta, so repair can (and does) remove it. The missing-meta and
    // corrupt-meta entries are deliberately NOT removed (#276: refusing to
    // orphan blob refcounts), so we don't assert their removal — running
    // verify again must still succeed over the remaining corrupt entries.
    let store2 = Store::open(&config).unwrap();
    assert!(store2.get("validkey").unwrap().is_some());
    assert!(store2.get("missingblobkey").unwrap().is_none());
    verify(&config, false, false).expect("a second verify pass should succeed");
}

#[test]
fn verify_reports_and_repairs_blob_index_drift() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    put_entry(&config, "driftkey", "drift", dir.path());
    let store = Store::open(&config).unwrap();
    let hash = store.get("driftkey").unwrap().unwrap().files[0]
        .hash
        .clone();
    drop(store);

    let db = crate::store::open_index_db(&config.index_db_path()).unwrap();
    db.execute(
        "UPDATE blobs SET refcount = 77 WHERE hash = ?1",
        rusqlite::params![hash],
    )
    .unwrap();
    drop(db);

    let drifted = verify(&config, false, false).unwrap();
    assert_eq!(drifted.index_drift, 1, "{drifted:?}");
    assert_eq!(drifted.unresolved_integrity_findings(), 1, "{drifted:?}");

    let repaired = verify(&config, false, true).unwrap();
    assert_eq!(repaired.index_drift, 0, "{repaired:?}");
    assert_eq!(repaired.unresolved_integrity_findings(), 0, "{repaired:?}");
    let clean = verify(&config, false, false).unwrap();
    assert_eq!(clean.index_drift, 0, "{clean:?}");
    assert_eq!(clean.unresolved_integrity_findings(), 0, "{clean:?}");
}

#[test]
fn verify_accepts_build_script_out_dir_artifact_names() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let store = Store::open(&config).unwrap();
    let output = dir.path().join("asm.s");
    std::fs::write(&output, b"asm bytes").unwrap();

    // What a build-script run commits: OUT_DIR contents under `out/`
    // (`build_script.rs`). `doctor --verify` must read this as valid
    // metadata — flagging it reports corruption no repair can remove.
    store
        .put(
            "buildscriptkey",
            "build_script_run",
            &["build-script".to_string()],
            &[],
            "x86_64-unknown-linux-gnu",
            "debug",
            &[(output, "out/asm.s".to_string())],
            "",
            "",
        )
        .unwrap();
    drop(store);

    let outcome = verify(&config, false, false).expect("verify must succeed");
    assert_eq!(outcome.corrupted_entries, 0, "{outcome:?}");
    assert_eq!(outcome.index_drift, 0, "{outcome:?}");
    assert_eq!(outcome.unresolved_integrity_findings(), 0, "{outcome:?}");
}

#[test]
fn verify_detects_checksum_mismatch_with_checksums_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let store = Store::open(&config).unwrap();
    put_entry(&config, "corruptblobkey", "eee", dir.path());

    // Corrupt the blob in place, same size so only the checksum differs.
    let meta = store.get("corruptblobkey").unwrap().unwrap();
    let blob = store.blob_path(&meta.files[0].hash);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&blob, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    #[cfg(not(unix))]
    {
        let mut p = std::fs::metadata(&blob).unwrap().permissions();
        p.set_readonly(false);
        std::fs::set_permissions(&blob, p).unwrap();
    }
    std::fs::write(&blob, vec![b'X'; meta.files[0].size as usize]).unwrap();

    // checksums=true detects the content mismatch and reports it as an
    // unresolved integrity finding (kunobi-ninja/kache#176).
    let outcome = verify(&config, true, false).expect("verify with checksums should succeed");
    assert_eq!(outcome.checksum_failures, 1, "{outcome:?}");
    assert_eq!(outcome.corrupted_entries, 1, "{outcome:?}");
    assert_eq!(
        outcome.unresolved_integrity_findings(),
        1,
        "without --repair the finding stands: {outcome:?}"
    );

    // --repair removes the entry, so nothing is left unresolved and a CI
    // run gated on this exits zero.
    let repaired = verify(&config, true, true).expect("verify --repair should succeed");
    assert_eq!(repaired.corrupted_removed, repaired.corrupted_entries);
    assert_eq!(repaired.unresolved_integrity_findings(), 0, "{repaired:?}");

    // And the store is clean afterwards.
    let clean = verify(&config, true, false).expect("verify after repair should succeed");
    assert_eq!(clean.corrupted_entries, 0, "{clean:?}");
    assert_eq!(clean.unresolved_integrity_findings(), 0, "{clean:?}");
}

/// kunobi-ninja/kache#176: the scrub hashes each unique blob ONCE, however
/// many entries share it — the dedup that makes the store cheap must not
/// make verification quadratic. Two entries sharing one corrupt blob
/// produce ONE checksum failure while marking BOTH entries corrupt.
#[test]
fn verify_scrubs_each_unique_blob_once_and_marks_every_referencing_entry() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let store = Store::open(&config).unwrap();

    // Two entries, identical content → one shared blob. Each put gets
    // its OWN source path: on Linux the store hardlinks the source into
    // the read-only blob store, so reusing one path would make the
    // second write fail with EACCES (macOS reflinks, and would not).
    for key in ["sharedkey_a", "sharedkey_b"] {
        let src = dir.path().join(format!("{key}.rlib"));
        std::fs::write(&src, b"shared artifact bytes").unwrap();
        store
            .put(
                key,
                "shared",
                &["lib".to_string()],
                &[],
                "",
                "dev",
                &[(src, "lib.rlib".to_string())],
                "",
                "",
            )
            .unwrap();
    }
    let meta = store.get("sharedkey_a").unwrap().unwrap();
    let blob = store.blob_path(&meta.files[0].hash);
    assert_eq!(
        store.get("sharedkey_b").unwrap().unwrap().files[0].hash,
        meta.files[0].hash,
        "the two entries must share one blob for this test to mean anything"
    );

    let mut perms = std::fs::metadata(&blob).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o644);
    }
    #[cfg(not(unix))]
    perms.set_readonly(false);
    std::fs::set_permissions(&blob, perms).unwrap();
    std::fs::write(&blob, vec![b'X'; meta.files[0].size as usize]).unwrap();

    // Without --checksums the scrub does not run, so silent corruption
    // is invisible: that is exactly what the flag buys, and asserting it
    // pins the flag's meaning.
    let unchecked = verify(&config, false, false).expect("verify should succeed");
    assert_eq!(
        unchecked.checksum_failures, 0,
        "checksums=false must not hash anything: {unchecked:?}"
    );
    assert_eq!(
        unchecked.unresolved_integrity_findings(),
        0,
        "same-size corruption is undetectable without --checksums: {unchecked:?}"
    );

    let outcome = verify(&config, true, false).expect("verify should succeed");
    assert_eq!(
        outcome.checksum_failures, 1,
        "one shared blob is one failure, not one per referencing entry: {outcome:?}"
    );
    assert_eq!(
        outcome.corrupted_entries, 2,
        "but both entries referencing it are corrupt: {outcome:?}"
    );
    assert_eq!(outcome.unresolved_integrity_findings(), 2, "{outcome:?}");
}

/// A healthy store reports nothing unresolved, so a CI gate on
/// `doctor --verify` stays green (kunobi-ninja/kache#176).
#[test]
fn verify_reports_no_findings_for_a_clean_store() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    put_entry(&config, "cleanstorekey", "ggg", dir.path());

    let outcome = verify(&config, true, false).expect("verify should succeed");
    assert_eq!(outcome.corrupted_entries, 0, "{outcome:?}");
    assert_eq!(outcome.checksum_failures, 0, "{outcome:?}");
    assert_eq!(outcome.missing_blobs, 0, "{outcome:?}");
    assert_eq!(outcome.unresolved_integrity_findings(), 0, "{outcome:?}");
    assert!(outcome.valid_entries >= 1, "{outcome:?}");
}

#[test]
fn verify_detects_blob_size_mismatch() {
    // Blob metadata length mismatch -> entry is marked corrupt.
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let store = Store::open(&config).unwrap();
    put_entry(&config, "sizemismatchkey", "fff", dir.path());
    let meta = store.get("sizemismatchkey").unwrap().unwrap();
    let blob = store.blob_path(&meta.files[0].hash);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&blob, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    #[cfg(not(unix))]
    {
        let mut p = std::fs::metadata(&blob).unwrap().permissions();
        p.set_readonly(false);
        std::fs::set_permissions(&blob, p).unwrap();
    }
    std::fs::write(&blob, vec![b'Y'; meta.files[0].size as usize + 1]).unwrap();

    verify(&config, false, false).expect("verify with size mismatch should succeed");
}





fn build_event(
    crate_name: &str,
    result: crate::events::EventResult,
    compile_time_ms: u64,
    elapsed_ms: u64,
    size: u64,
    cache_key: &str,
) -> crate::events::BuildEvent {
    crate::events::BuildEvent {
        ts: kache_core::time::UtcTime::now(),
        session_id: String::new(),
        demands: Vec::new(),
        crate_name: crate_name.to_string(),
        version: "0.1.0".to_string(),
        result,
        elapsed_ms,
        compile_time_ms,
        size,
        cache_key: cache_key.to_string(),
        schema: 8,
        key_ms: 0,
        key_hash_hits: 0,
        key_hash_misses: 0,
        key_hash_bytes: 0,
        lookup_ms: 0,
        restore_ms: 0,
        store_ms: 0,
        startup_ms: 0,
        dep_info_ms: 0,
        dep_info_runs: 0,
        prediction_mismatches: 0,
        flight_wait_ms: 0,
        permit_wait_ms: 0,
        store_output_blobs: 0,
        store_duplicate_blobs: 0,
        store_new_blobs: 0,
        compiler_runs: 0,
        preprocessor_runs: 0,
        probe_runs: 0,
        reflinked_bytes: 0,
        hardlinked_bytes: 0,
        copied_bytes: 0,
        store_reflinked_bytes: 0,
        store_hardlinked_bytes: 0,
        store_copied_bytes: 0,
        store_copy_cross_device_bytes: 0,
        store_copy_permission_bytes: 0,
        store_copy_ineligible_bytes: 0,
        store_copy_other_bytes: 0,
        restore_copy_cross_device_bytes: 0,
        restore_copy_permission_bytes: 0,
        restore_copy_exclusive_bytes: 0,
        restore_copy_other_bytes: 0,
        root: String::new(),
        passthrough_reason: String::new(),
        skip_reason: String::new(),
        store_error: String::new(),
        store_handed_off: false,
        daemon_store_ms: 0,
        lookup_rejection: String::new(),
        verify_compare: String::new(),
        fallback: false,
        fallback_attempt: None,
        exit_code: None,
        key_fields: Default::default(),
        key_diff: Vec::new(),
        key_externs: Default::default(),
        key_env_deps: None,
        key_externs_recorded: false,
        unit_id: String::new(),
        extern_units: Default::default(),
        miss_reason: crate::events::MissReason::None,
        object_output: String::new(),
        package: String::new(),
        rebuilt_package: None,
        rebuilt_fingerprint: None,
        rebuilt_path: None,
        rebuilt_paths: Vec::new(),
    }
}

fn diff_meta(
    target: &str,
    profile: &str,
    features: &[&str],
    crate_types: &[&str],
) -> crate::store::EntryMeta {
    crate::store::EntryMeta {
        cache_key: "k".to_string(),
        key_schema: crate::cache_key::CACHE_KEY_VERSION,
        crate_name: "c".to_string(),
        crate_types: crate_types.iter().map(|v| (*v).to_string()).collect(),
        files: Vec::new(),
        stdout: String::new(),
        stderr: String::new(),
        features: features.iter().map(|v| (*v).to_string()).collect(),
        target: target.to_string(),
        profile: profile.to_string(),
        compile_time_ms: 0,
        emit_kinds: Vec::new(),
    }
}
























#[test]
fn a_quiet_gc_the_user_asked_for_is_requested() {
    use crate::store::SweepOrigin;
    assert_eq!(GcMode::Cli.sweep_origin(), SweepOrigin::Requested);
    assert_eq!(GcMode::CliQuiet.sweep_origin(), SweepOrigin::Requested);
    assert_eq!(GcMode::Background.sweep_origin(), SweepOrigin::Automatic);
}
#[test]
fn cache_cleaning_previews_without_removing_and_removes_on_yes() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::test_support::test_config(dir.path().join("cache"));
    put_entry(&config, "k1", "serde", dir.path());
    put_entry(&config, "k2", "serde", dir.path());
    put_entry(&config, "k3", "tokio", dir.path());
    let entries = || Store::open(&config).unwrap().entry_count().unwrap();

    clean_cache(
        &config,
        CacheClean::Crate("serde".into()),
        true,
        false,
        false,
    )
    .unwrap();
    clean_cache(&config, CacheClean::All, true, false, false).unwrap();
    clean_cache(&config, CacheClean::OlderThan(24), true, false, false).unwrap();
    clean_cache(&config, CacheClean::StaleSchema, true, false, false).unwrap();
    assert_eq!(entries(), 3, "a dry run removes nothing");
    assert!(clean_cache(&config, CacheClean::All, false, true, true).is_err());
    assert_eq!(entries(), 3, "JSON is refused before anything is removed");

    clean_cache(
        &config,
        CacheClean::Crate("missing".into()),
        false,
        true,
        false,
    )
    .unwrap();
    clean_cache(
        &config,
        CacheClean::Crate("serde".into()),
        false,
        true,
        false,
    )
    .unwrap();
    assert_eq!(entries(), 1, "only serde's entries go");
    clean_cache(&config, CacheClean::All, false, true, false).unwrap();
    assert_eq!(entries(), 0);
}

#[test]
fn a_cache_preview_says_what_each_clean_would_do() {
    assert_eq!(
        cache_preview(&CacheClean::OlderThan(48), 3, 2048),
        "Cache is 2.0 KiB in 3 entries. This would evict the entries older than 2d."
    );
    assert!(cache_preview(&CacheClean::StaleSchema, 3, 2048).contains("not keyed by schema"));
    assert_eq!(
        cache_preview(&CacheClean::All, 1, 1024),
        "Would remove all 1 cache entry (1.0 KiB)."
    );
    assert_eq!(
        cache_preview(&CacheClean::Crate("serde".into()), 2, 1024),
        "Would remove 2 cache entries for 'serde' (1.0 KiB)."
    );
    assert_eq!(
        cache_preview(&CacheClean::Crate("serde".into()), 0, 0),
        "No cache entries for 'serde'."
    );
}

#[test]
fn crate_entries_counts_only_that_crate() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::test_support::test_config(dir.path().join("cache"));
    put_entry(&config, "k1", "serde", dir.path());
    put_entry(&config, "k2", "serde", dir.path());
    put_entry(&config, "k3", "tokio", dir.path());
    let store = Store::open(&config).unwrap();
    let (serde, serde_bytes) = crate_entries(&store, "serde").unwrap();
    assert_eq!(serde, 2);
    assert!(serde_bytes > 0);
    assert_eq!(crate_entries(&store, "tokio").unwrap().0, 1);
    assert_eq!(crate_entries(&store, "missing").unwrap(), (0, 0));
}

#[test]
fn command_lines_quote_for_the_posix_shell() {
    let argv = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    assert_eq!(
        command_line(&argv(&["kache", "clean", "/tmp/my dir"])),
        "kache clean '/tmp/my dir'"
    );
    assert_eq!(command_line(&argv(&["kache", ""])), "kache ''");
}

#[test]
fn no_terminal_names_the_same_command_with_yes() {
    let args = ["clean".to_string(), "/tmp/my dir".to_string()];
    let quoted = "'/tmp/my dir'";
    assert_eq!(
        no_terminal("nothing removed", &args, true).to_string(),
        format!(
            "nothing removed: there is no terminal to ask. Run `kache clean {quoted} --yes` \
             to go ahead without asking, or --dry-run to preview."
        )
    );
    assert_eq!(
        no_terminal("init changed nothing", &["init".to_string()], false).to_string(),
        "init changed nothing: there is no terminal to ask. Run `kache init --yes` to go ahead \
         without asking."
    );
}

#[test]
fn removal_asks_only_at_a_terminal() {
    let never = |_: &str| -> Result<bool> { panic!("asked without a terminal") };
    assert!(confirm_removal_with("Remove?", true, false, never).unwrap());
    let refused = confirm_removal_with("Remove?", false, false, never).unwrap_err();
    assert!(refused.to_string().contains("--yes"), "{refused}");
    assert!(confirm_removal_with("Remove?", false, true, |_| Ok(true)).unwrap());
    assert!(!confirm_removal_with("Remove?", false, true, |_| Ok(false)).unwrap());
}

#[test]
fn a_cache_clean_decides_from_what_it_counted() {
    let serde = || CacheClean::Crate("serde".into());
    assert_eq!(
        cache_action(serde(), 2, 1024, true, false).unwrap(),
        CacheAction::Say("Would remove 2 cache entries for 'serde' (1.0 KiB).".into())
    );
    assert_eq!(
        cache_action(serde(), 0, 0, false, false).unwrap(),
        CacheAction::Say("No cache entries for 'serde'.".into())
    );
    assert_eq!(
        cache_action(serde(), 2, 1024, false, false).unwrap(),
        CacheAction::Purge {
            crate_name: Some("serde".into()),
            question: "Remove 2 cache entries for 'serde' (1.0 KiB)?".into(),
        }
    );
    assert_eq!(
        cache_action(CacheClean::All, 3, 2048, false, false).unwrap(),
        CacheAction::Purge {
            crate_name: None,
            question: "Remove all 3 cache entries (2.0 KiB)?".into(),
        }
    );
    assert!(cache_action(CacheClean::All, 3, 2048, false, true).is_err());
    assert!(cache_action(serde(), 2, 1024, false, true).is_err());
    assert_eq!(
        cache_action(CacheClean::OlderThan(24), 3, 0, false, true).unwrap(),
        CacheAction::Gc {
            max_age_hours: Some(24),
            stale_schema: false
        }
    );
    assert_eq!(
        cache_action(CacheClean::StaleSchema, 3, 0, false, false).unwrap(),
        CacheAction::Gc {
            max_age_hours: None,
            stale_schema: true
        }
    );
}

#[test]
fn cache_counts_cover_one_crate_or_the_whole_cache() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::test_support::test_config(dir.path().join("cache"));
    put_entry(&config, "k1", "serde", dir.path());
    put_entry(&config, "k2", "tokio", dir.path());
    let store = Store::open(&config).unwrap();
    assert_eq!(
        cache_counts(&store, &CacheClean::Crate("serde".into()))
            .unwrap()
            .0,
        1
    );
    assert_eq!(cache_counts(&store, &CacheClean::All).unwrap().0, 2);
    assert!(cache_counts(&store, &CacheClean::All).unwrap().1 > 0);
}

/// The "Other checkouts" line names the count, both trees and where to look,
/// in the singular and the plural, and says nothing when nothing traced there.
#[test]
fn checkout_check_wording() {
    let clean = checkout_check("/w/b", 0, None);
    assert!(clean.pass);
    assert_eq!(clean.fix, None);

    let one = checkout_check("/w/b", 1, Some("/w/a"));
    assert!(!one.pass);
    assert_eq!(one.label, OTHER_CHECKOUTS);
    assert_eq!(one.detail, "1 miss in /w/b differs from the build in /w/a");
    assert!(one.fix.as_deref().unwrap().contains("kache explain"));

    let many = checkout_check("/w/b", 3, None);
    assert_eq!(
        many.detail,
        "3 misses in /w/b differ from the build in other checkouts"
    );
}

/// Read from a real event log: a second checkout whose first build of a crate
/// differs from the first checkout's is reported; no log reports nothing.
#[test]
fn doctor_reads_cross_checkout_misses_from_the_event_log() {
    use crate::events::{BuildEvent, EventResult};
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("events.jsonl");
    assert!(doctor_checkout_check(&log).is_none());

    let built = |root: &str, session: &str, sources: &str| {
        let mut e = BuildEvent::new_for_test("one", EventResult::Miss);
        e.root = root.to_string();
        e.session_id = session.to_string();
        e.unit_id = "uone".to_string();
        e.key_fields = [("sources".to_string(), sources.to_string())]
            .into_iter()
            .collect();
        e.key_externs_recorded = true;
        e
    };
    for event in [built("/first", "a", "1111"), built("/second", "b", "2222")] {
        crate::events::log_event(&log, &event).unwrap();
    }
    let check = doctor_checkout_check(&log).unwrap();
    assert!(!check.pass);
    assert_eq!(
        check.detail,
        "1 miss in /second differs from the build in /first"
    );
}

#[test]
fn doctor_and_full_clean_report_quarantined_indexes_without_deleting_them() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::test_support::test_config(dir.path().join("cache"));
    let absent = doctor_index_quarantines(&config);
    assert!(absent.pass);
    assert_eq!(absent.detail, "no retained corrupt index files");
    assert!(absent.fix.is_none());
    assert!(
        cache_quarantine_notice(&config, &CacheClean::All)
            .unwrap()
            .is_none()
    );
    fs::create_dir_all(&config.cache_dir).unwrap();
    for (name, bytes) in [
        ("index.db.corrupt-123-456", 1000),
        ("index.db.corrupt-123-456-wal", 2000),
    ] {
        fs::write(config.cache_dir.join(name), vec![0; bytes]).unwrap();
    }
    let check = doctor_index_quarantines(&config);
    assert!(!check.pass);
    assert_eq!(check.label, "Store quarantine");
    assert_eq!(doctor_section(check.label), "Storage");
    assert_eq!(
        check.detail,
        format!(
            "2 files ({}) retained in {} after index corruption",
            kache_core::size::format_bytes(3000),
            term::home_path(&config.cache_dir)
        )
    );
    assert!(check.fix.unwrap().contains("`kache clean --cache`"));
    let notice = cache_quarantine_notice(&config, &CacheClean::All)
        .unwrap()
        .unwrap();
    assert_eq!(
        notice,
        format!(
            "Full cache removal also deletes 2 quarantined index files ({}). Save these copies first if needed for investigation.",
            term::bytes(3000)
        )
    );
    for what in [
        CacheClean::Crate("serde".to_string()),
        CacheClean::OlderThan(24),
        CacheClean::StaleSchema,
    ] {
        assert!(cache_quarantine_notice(&config, &what).unwrap().is_none());
    }
    assert_eq!(
        crate::store::quarantined_index_files(&config.index_db_path())
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn doctor_quarantine_inspection_reports_unreadable_cache_directory() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::test_support::test_config(dir.path().join("cache"));
    fs::write(&config.cache_dir, b"not a directory").unwrap();
    let check = doctor_index_quarantines(&config);
    assert!(!check.pass);
    assert!(
        check
            .detail
            .starts_with("could not inspect retained corrupt index files: ")
    );
    assert_eq!(
        check.fix,
        Some(format!("ensure {} is readable", config.cache_dir.display()))
    );
    assert!(cache_quarantine_notice(&config, &CacheClean::All).is_err());
}

fn effective_config_like(config: &Config) -> crate::daemon::EffectiveConfig {
    crate::daemon::EffectiveConfig {
        process: None,
        max_size: config.max_size,
        cache_dir: config.cache_dir.display().to_string(),
        runtime_dir: config.runtime_dir.display().to_string(),
        config_path: "/daemon-home/.config/kache/config.toml".to_string(),
        config_fingerprint: Some("daemon-fingerprint".to_string()),
        socket_path: config.socket_path().display().to_string(),
        started_at_ms: 1_700_000_000_000,
        auto_recover_min_free_bytes: None,

    }
}

#[test]
fn stats_shows_the_recovery_the_daemon_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::test_support::test_config(dir.path().to_path_buf());
    config.auto_recover_min_free_bytes = 1;
    assert_eq!(recovery_in_effect(&config, None), 1);
    let mut daemon = effective_config_like(&config);
    daemon.auto_recover_min_free_bytes = Some(7);
    assert_eq!(recovery_in_effect(&config, Some(&daemon)), 7);
    // A daemon from before the fields: this process's settings.
    daemon.auto_recover_min_free_bytes = None;
    assert_eq!(recovery_in_effect(&config, Some(&daemon)), 1);
}

#[test]
fn doctor_checks_the_cargo_wrapper_once_and_names_init() {
    use crate::wrapper_config::WrapperSetting;
    let config = std::path::Path::new("/home/me/.cargo/config.toml");
    let (pass, detail, fix) = wrapper_check(None, Some(false), config);
    assert!(!pass);
    assert_eq!(detail, "not set");
    let fix = fix.unwrap();
    assert!(fix.contains("kache init"), "{fix}");
    assert!(fix.contains("config.toml"), "{fix}");

    let env = |value: &str| WrapperSetting::Environment {
        value: value.into(),
    };
    assert_eq!(
        wrapper_check(Some(&env("/bin/kache")), None, config),
        (true, "kache via env".to_string(), None)
    );
    let (pass, detail, fix) = wrapper_check(Some(&env("sccache")), None, config);
    assert!(!pass);
    assert_eq!(detail, "sccache (sccache)");
    assert!(fix.unwrap().contains("export RUSTC_WRAPPER=kache"));
    let (_, detail, _) = wrapper_check(Some(&env("other")), None, config);
    assert_eq!(detail, "other (not kache)");

    let file = |value: &str| WrapperSetting::CargoConfig {
        value: value.into(),
        path: config.to_path_buf(),
    };
    let (pass, detail, fix) = wrapper_check(Some(&file("kache")), Some(false), config);
    assert!(pass, "{detail}");
    assert!(detail.starts_with("kache via "), "{detail}");
    assert!(fix.is_none());
    let fix_for = |repair| {
        wrapper_check(Some(&file("sccache")), repair, config)
            .2
            .unwrap()
    };
    assert!(fix_for(Some(true)).contains("kache doctor --fix"));
    assert!(fix_for(Some(true)).contains("kache init"));
    assert!(!fix_for(Some(false)).contains("--fix"));
    assert!(fix_for(Some(false)).contains("kache init"));
    assert!(fix_for(None).starts_with("replace `rustc-wrapper = \"sccache\"`"));
}

#[test]
fn doctor_suggests_the_running_binary_directory_for_path() {
    let dir = std::path::Path::new("/opt/kache/bin");
    assert_eq!(binary_fix(true, Some(dir)), None);
    assert_eq!(
        binary_fix(false, Some(dir)).as_deref(),
        Some("add /opt/kache/bin to PATH")
    );
    let fallback = binary_fix(false, None).unwrap();
    assert!(!fallback.contains("cargo install"), "{fallback}");
}

#[test]
fn doctor_lists_volume_stores_under_storage_and_only_live_daemon_labels() {
    assert_eq!(doctor_section("Volume store"), "Storage");
    for label in DAEMON_CHECK_LABELS {
        assert_eq!(doctor_section(label), "Services", "{label}");
    }
}

#[test]
fn init_preview_ends_with_the_flags_given_or_nothing_to_change() {
    assert_eq!(
        preview_end(true, false, false, false),
        "Preview only. Run kache init to apply."
    );
    assert_eq!(
        preview_end(true, true, true, true),
        "Preview only. Run kache init --no-service --no-shell --yes to apply."
    );
    assert_eq!(
        preview_end(true, false, true, false),
        "Preview only. Run kache init --no-shell to apply."
    );
    assert_eq!(preview_end(false, true, false, false), "Nothing to change.");
}

#[test]
fn init_steps_follow_the_plans() {
    assert_eq!(edit_action(false, false), "keep");
    assert_eq!(edit_action(true, false), "configure");
    assert_eq!(edit_action(false, true), "configure");
    for action in ["configure", "update"] {
        assert!(step_changes(action), "{action}");
    }
    for action in [
        "keep",
        "unsupported",
        "unavailable",
        "manual",
        "ensure_running",
        "create",
    ] {
        assert!(!step_changes(action), "{action}");
    }
    assert_eq!(service_action(true, true, true, true), None);
    assert_eq!(
        service_action(false, false, true, true),
        Some("unavailable")
    );
    assert_eq!(service_action(false, true, true, true), Some("update"));
    assert_eq!(service_action(false, true, true, false), Some("keep"));
    assert_eq!(service_action(false, true, false, false), Some("configure"));
}

#[test]
fn init_never_backs_up_a_file_it_created() {
    let mut created = Vec::new();
    let config = std::path::Path::new("/home/me/.cargo/config.toml");
    let rc = std::path::Path::new("/home/me/.bashrc");
    // The run creates the Cargo config, which counts as created.
    assert!(backup_step(&mut created, config, false).is_none());
    assert!(backup_step(&mut created, config, true).is_none());
    assert!(backup_step(&mut created, config, false).is_none());
    assert_eq!(created, [config]);
    // An existing startup file is backed up before each edit.
    for _ in 0..2 {
        let step = backup_step(&mut created, rc, true).unwrap();
        assert_eq!((step.component, step.action), ("backup", "create"));
        assert_eq!(step.paths, [rc]);
    }
    assert_eq!(created, [config]);
}

#[test]
fn init_steps_list_each_edit_with_the_backup_before_it() {
    use std::path::PathBuf;
    let cargo = PathBuf::from("/home/me/.cargo/config.toml");
    let rc = PathBuf::from("/home/me/.bashrc");
    let facts = |cargo_ready: bool, cargo_exists: bool, rc_exists: bool| SetupFacts {
        cargo_path: cargo.clone(),
        cargo: if cargo_ready { "keep" } else { "configure" },
        cargo_exists,
        shell: Some(SetupStep::new("shell", "configure", vec![rc.clone()])),
        shell_changes: vec![(rc.clone(), rc_exists)],
        test_runner: Some(SetupStep::new("test_runner", "configure", vec![rc.clone()])),
        test_changes: vec![(rc.clone(), rc_exists)],


        service: Some("keep"),
        service_path: Some(PathBuf::from("/svc")),
    };
    let summary = |steps: Vec<SetupStep>| {
        steps
            .into_iter()
            .map(|step| {
                let path = step
                    .paths
                    .first()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default();
                format!("{} {} {path}", step.component, step.action)
            })
            .collect::<Vec<_>>()
    };
    // A fresh machine: init creates every file it edits, so no backups.
    assert_eq!(
        summary(setup_steps(&facts(false, false, false))),
        [
            "cargo configure /home/me/.cargo/config.toml",
            "shell configure /home/me/.bashrc",
            "test_runner configure /home/me/.bashrc",

            "login_service keep /svc",
            "daemon ensure_running ",
        ]
    );
    // Existing files are backed up before each edit.
    assert_eq!(
        summary(setup_steps(&facts(false, true, true))),
        [
            "cargo configure /home/me/.cargo/config.toml",
            "backup create /home/me/.cargo/config.toml",
            "shell configure /home/me/.bashrc",
            "backup create /home/me/.bashrc",
            "test_runner configure /home/me/.bashrc",
            "backup create /home/me/.bashrc",

            "login_service keep /svc",
            "daemon ensure_running ",
        ]
    );
    // A configured machine keeps Cargo; nothing else is listed when skipped.
    let kept = SetupFacts {
        shell: None,
        shell_changes: Vec::new(),
        test_runner: None,
        test_changes: Vec::new(),

        service: None,
        ..facts(true, true, true)
    };
    assert_eq!(
        summary(setup_steps(&kept)),
        [
            "cargo keep /home/me/.cargo/config.toml",

            "daemon ensure_running ",
        ]
    );
}

fn explain_event(result: events::EventResult, secs: i64, key: &str) -> events::BuildEvent {
    let mut event = events::BuildEvent::new_for_test("app", result);
    event.ts = kache_core::time::UtcTime::from_timestamp(1_760_454_000 + secs, 0).unwrap();
    event.cache_key = key.to_string();
    event.elapsed_ms = 1_989;
    event
}

#[test]
fn the_newest_compile_decides_whether_a_crate_is_cached() {
    use events::EventResult;
    let miss = explain_event(EventResult::Miss, 1, "missmissmissmiss");
    let mut seed = explain_event(EventResult::Passthrough, 2, "seedseedseedseed");
    seed.passthrough_reason = "adaptive seed".to_string();
    let mut probe = explain_event(EventResult::Passthrough, 3, "");
    probe.passthrough_reason = "not-a-compile|--print cfg".to_string();
    let error = explain_event(EventResult::Error, 4, "");
    let skipped = explain_event(EventResult::Skipped, 5, "");
    let hit = explain_event(EventResult::LocalHit, 6, "hithithithithit");

    let latest = |events: &[&events::BuildEvent]| {
        latest_uncached_compile(events).map(|event| event.cache_key.clone())
    };
    // A probe, an error or a skip after the seed does not hide it.
    assert_eq!(
        latest(&[&miss, &seed, &probe, &error, &skipped]).as_deref(),
        Some("seedseedseedseed")
    );
    // A lookup after the seed decides instead.
    assert_eq!(latest(&[&miss, &seed, &hit]), None);
    assert_eq!(latest(&[&seed, &miss]), None);
    assert_eq!(latest(&[&probe]), None);
    assert_eq!(latest(&[]), None);

    let lines = uncached_compile_lines("app", &seed, Some(&miss), &[&miss, &seed]);
    assert_eq!(lines[0], "kache · explain · app", "{lines:?}");
    assert_eq!(lines[2], "`app` was compiled, not cached.", "{lines:?}");
    assert_eq!(lines[4], "Latest compile", "{lines:?}");
    assert!(
        lines.contains(&format!("  When      {}", term::local_datetime(seed.ts))),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"  Key       seedseedseed".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(
            &"  Why       edit rebuild: rustc rebuilt the edited crate incrementally (adaptive seed)"
                .to_string()
        ),
        "{lines:?}"
    );
    assert!(
        lines.contains(&format!(
            "  Earlier   miss   {}, key missmissmiss",
            term::local_datetime(miss.ts)
        )),
        "{lines:?}"
    );
    let joined = lines.join("\n");
    assert!(!joined.contains("now cached"), "{joined}");
    let alone = uncached_compile_lines("app", &seed, None, &[&seed]).join("\n");
    assert!(!alone.contains("Earlier"), "{alone}");
    assert!(alone.contains("\nRecent events\n"), "{alone}");
}

#[test]
fn a_crate_without_misses_is_not_called_all_hits_unless_it_is() {
    use events::EventResult;
    let hit = explain_event(EventResult::LocalHit, 1, "k");
    let remote = explain_event(EventResult::LocalHit, 2, "k");
    let mut pass = explain_event(EventResult::Passthrough, 3, "");
    pass.passthrough_reason = "unsupported|cc link mode".to_string();
    assert_eq!(
        no_miss_headline("app", &[&hit, &remote]),
        "No misses for `app`: all events are hits."
    );
    assert_eq!(
        no_miss_headline("app", &[&pass, &hit]),
        "No misses for `app`: its latest lookup hit the cache."
    );
    assert_eq!(
        no_miss_headline("app", &[&pass]),
        "No misses or hits for `app`."
    );
    let lines = no_miss_lines("app", &[], &[&hit]);
    assert_eq!(lines[0], "kache · explain · app");
    assert_eq!(lines[2], "No misses for `app`: all events are hits.");
    assert!(
        lines.contains(&"Stored entries (0)".to_string()),
        "{lines:?}"
    );
    assert!(lines.contains(&"Recent events".to_string()), "{lines:?}");
}

#[test]
fn a_run_reports_declined_and_failed_steps_as_not_done() {
    use std::path::PathBuf;
    let cargo = PathBuf::from("/home/me/.cargo/config.toml");
    let plan = SetupFacts {
        cargo_path: cargo.clone(),
        cargo: "configure",
        cargo_exists: true,
        shell: None,
        shell_changes: Vec::new(),
        test_runner: None,
        test_changes: Vec::new(),


        service: Some("configure"),
        service_path: Some(PathBuf::from("/svc")),
    };
    let components = |facts: &SetupFacts| {
        setup_steps(facts)
            .into_iter()
            .map(|step| format!("{} {}", step.component, step.action))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        components(&plan),
        [
            "cargo configure",
            "backup create",

            "login_service configure",
            "daemon ensure_running",
        ]
    );
    let declined = run_facts(
        &plan,
        RunOutcome {
            cargo_applied: false,
            test_runner: None,
            test_changes: Vec::new(),

            service_installed_now: false,
        },
    );
    assert_eq!(
        components(&declined),
        [
            "cargo skip",

            "login_service failed",
            "daemon ensure_running",
        ]
    );
    let done = run_facts(
        &plan,
        RunOutcome {
            cargo_applied: true,
            test_runner: None,
            test_changes: Vec::new(),

            service_installed_now: true,
        },
    );
    assert_eq!(
        components(&done),
        [
            "cargo configure",
            "backup create",

            "login_service configure",
            "daemon ensure_running",
        ]
    );
    // A kept Cargo config stays kept.
    let kept = run_facts(
        &SetupFacts {
            cargo: "keep",
            service: Some("keep"),
            ..plan.clone()
        },
        RunOutcome {
            cargo_applied: false,
            test_runner: None,
            test_changes: Vec::new(),

            service_installed_now: false,
        },
    );
    assert_eq!(
        components(&kept)[..2],
        [
            "cargo keep",

            "login_service keep"
        ]
    );
}

#[test]
fn a_run_reports_the_test_runner_it_planned_after_the_shell_edits() {
    use std::path::PathBuf;
    let rc = PathBuf::from("/home/me/.bashrc");
    let plan = SetupFacts {
        cargo_path: PathBuf::from("/home/me/.cargo/config.toml"),
        cargo: "keep",
        cargo_exists: true,
        shell: Some(SetupStep::new("shell", "configure", vec![rc.clone()])),
        shell_changes: vec![(rc.clone(), true)],
        test_runner: Some(SetupStep::new("test_runner", "unsupported", Vec::new())),
        test_changes: Vec::new(),


        service: None,
        service_path: None,
    };
    let run = run_facts(
        &plan,
        RunOutcome {
            cargo_applied: false,
            test_runner: Some(SetupStep::new("test_runner", "configure", vec![rc.clone()])),
            test_changes: vec![(rc.clone(), true)],

            service_installed_now: false,
        },
    );
    let steps: Vec<_> = setup_steps(&run)
        .into_iter()
        .map(|step| format!("{} {}", step.component, step.action))
        .collect();
    assert_eq!(
        steps,
        [
            "cargo keep",
            "shell configure",
            "backup create",
            "test_runner configure",
            "backup create",

            "daemon ensure_running",
        ]
    );
}

#[test]
fn recent_events_are_the_newest_five_in_local_time() {
    use events::EventResult;
    let events: Vec<events::BuildEvent> = (0..7)
        .map(|i| explain_event(EventResult::LocalHit, i, &format!("key{i}-0123456789")))
        .collect();
    let refs: Vec<&events::BuildEvent> = events.iter().collect();
    let lines = recent_event_lines(&refs);
    assert_eq!(lines.len(), 6, "a header and five events: {lines:?}");
    assert!(lines[0].starts_with("  TIME"), "{lines:?}");
    assert!(
        lines[1].starts_with(&format!(
            "  {}   local_hit",
            term::local_clock(events[2].ts)
        )),
        "{lines:?}"
    );
    assert!(lines[1].contains("key2-0123456"), "{lines:?}");
    assert!(
        lines[1].contains(&term::duration_ms(events[2].elapsed_ms)),
        "{lines:?}"
    );
    assert!(lines[5].contains("key6-0123456"), "{lines:?}");
    assert_eq!(recent_event_lines(&refs[..2]).len(), 3);
}

#[test]
fn list_details_use_the_short_key_and_local_time() {
    let entry = daemon::StatsEntry {
        store_dirs: vec![std::path::PathBuf::from("/s")],
        cache_key: "96b68103bea22193ffeeddcc".to_string(),
        crate_name: "app".to_string(),
        crate_type: String::new(),
        profile: "debug".to_string(),
        size: 1536,
        hit_count: 3,
        created_at: "2026-10-14 15:18:56".to_string(),
        last_accessed: "not a time".to_string(),
        content_hash: None,
    };
    let lines = entry_detail_lines(&entry);
    assert_eq!(lines[0], "Cache key: 96b68103bea2");
    assert!(
        !lines.iter().any(|line| line.contains("Type:")),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"  Profile:  debug".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&format!(
            "  Created:  {}",
            term::local_store_time("2026-10-14 15:18:56")
        )),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"  Accessed: not a time".to_string()),
        "{lines:?}"
    );
    assert_eq!(
        local_store_date("2026-10-14 15:18:56"),
        term::local_store_time("2026-10-14 15:18:56")[..10]
    );
}


#[test]
fn a_declined_store_is_the_latest_compile_and_a_fallback_run_is_not() {
    use events::EventResult;
    let miss = explain_event(EventResult::Miss, 1, "missmissmissmiss");
    let mut seed = explain_event(EventResult::Passthrough, 2, "seedseedseedseed");
    seed.passthrough_reason = "adaptive seed".to_string();
    let mut declined = explain_event(EventResult::Skipped, 3, "skipskipskipskip");
    declined.skip_reason = "admission-threshold".to_string();
    let mut fallback = explain_event(EventResult::Passthrough, 4, "fbfbfbfbfbfbfbfb");
    fallback.passthrough_reason = "unsupported|assembly".to_string();
    fallback.fallback = true;
    let legacy_skip = explain_event(EventResult::Skipped, 5, "oldskipoldskip00");

    let latest = |events: &[&events::BuildEvent]| {
        latest_uncached_compile(events).map(|event| event.cache_key.clone())
    };
    assert_eq!(
        latest(&[&miss, &seed, &declined, &fallback, &legacy_skip]).as_deref(),
        Some("skipskipskipskip")
    );
    assert_eq!(latest(&[&miss, &fallback]), None);

    let lines = uncached_compile_lines("app", &declined, Some(&miss), &[&miss, &declined]);
    assert!(
        lines.contains(&"  Why       compiled, not stored: admission-threshold".to_string()),
        "{lines:?}"
    );
}

#[test]
fn compile_time_backfill_message_does_not_claim_completion_or_hide_errors() {
    assert_eq!(
        compile_time_backfill_message(Ok(0)),
        " no positive costs recovered this pass."
    );
    assert_eq!(compile_time_backfill_message(Ok(1)), " 1 entry updated.");
    assert_eq!(compile_time_backfill_message(Ok(2)), " 2 entries updated.");
    assert_eq!(
        compile_time_backfill_message(Err(anyhow::anyhow!("index busy"))),
        " deferred: index busy"
    );
}

/// A Cargo config naming the given `[build]` settings.
///
/// The values are written as TOML strings, so a path with a backslash in it
/// stays a valid value instead of breaking the file: a Windows path is
/// `C:\Users\...`, and an unescaped one makes the whole config unreadable.
fn cargo_build_config(settings: &[(&str, &std::path::Path)]) -> String {
    let mut build = deser_value::Map::new();
    for (key, value) in settings {
        build.insert(
            (*key).to_string(),
            deser_value::Value::from(value.to_string_lossy().into_owned()),
        );
    }
    let mut table = deser_value::Map::new();
    table.insert("build".to_string(), deser_value::Value::from(build));
    deser_toml::to_string(&table).unwrap()
}

/// Write a Cargo config naming `target_dir`, creating the `.cargo` directory
/// Cargo resolves a relative value against and the target dir itself. Creating
/// the target dir keeps a test asserting *which* config won, rather than how far
/// [`closest_existing`] had to walk back up; one test below covers that walk.
fn cargo_config(root: &std::path::Path, target_dir: &std::path::Path) {
    let cargo = root.join(".cargo");
    fs::create_dir_all(&cargo).unwrap();
    fs::create_dir_all(if target_dir.is_absolute() {
        target_dir.to_path_buf()
    } else {
        root.join(target_dir)
    })
    .unwrap();
    fs::write(
        cargo.join("config.toml"),
        cargo_build_config(&[("target-dir", target_dir)]),
    )
    .unwrap();
}

fn os(path: &std::path::Path) -> Option<std::ffi::OsString> {
    Some(path.as_os_str().to_owned())
}

/// The environment a resolver test passes in.
///
/// The home directory is a path that does not exist, so the search cannot
/// reach the developer's own cargo home and read a target dir from it.
fn environment() -> CargoEnvironment {
    CargoEnvironment {
        home: std::path::PathBuf::from("/nonexistent-cargo-home"),
        ..Default::default()
    }
}

#[test]
fn the_target_dir_environment_beats_every_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let from_config = dir.path().join("from-config");
    let from_env = dir.path().join("from-env");
    cargo_config(&project, &from_config);
    fs::create_dir_all(&from_env).unwrap();
    let env = CargoEnvironment {
        target_dir: os(&from_env),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        from_env,
        "Cargo gives the environment precedence over every config file"
    );
}

#[test]
fn the_build_target_dir_environment_is_read_when_the_target_dir_one_is_unset() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let from_config = dir.path().join("from-config");
    let from_env = dir.path().join("from-env");
    cargo_config(&project, &from_config);
    fs::create_dir_all(&from_env).unwrap();
    let env = CargoEnvironment {
        build_target_dir: os(&from_env),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        from_env,
        "CARGO_BUILD_TARGET_DIR names the target dir as well"
    );
}

#[test]
fn the_target_dir_environment_wins_over_the_build_target_dir_one() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let from_target_dir = dir.path().join("from-target-dir");
    let from_build_target_dir = dir.path().join("from-build-target-dir");
    fs::create_dir_all(&from_target_dir).unwrap();
    fs::create_dir_all(&from_build_target_dir).unwrap();
    let env = CargoEnvironment {
        target_dir: os(&from_target_dir),
        build_target_dir: os(&from_build_target_dir),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        from_target_dir,
        "CARGO_TARGET_DIR is the older name for the same setting"
    );
}

#[test]
fn an_empty_target_dir_environment_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let target = dir.path().join("shared-target");
    cargo_config(&project, &target);
    let env = CargoEnvironment {
        target_dir: os(std::path::Path::new("")),
        ..environment()
    };
    assert_eq!(cargo_output_dir_from(&project, &env), None);
}

#[test]
fn a_relative_target_dir_environment_resolves_against_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("target")).unwrap();
    let env = CargoEnvironment {
        target_dir: os(std::path::Path::new("target")),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        project.join("target"),
        "an environment path is relative to the working directory"
    );
}

#[test]
fn the_nearest_config_wins_over_an_outer_one() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().join("outer");
    let project = outer.join("crates/inner");
    fs::create_dir_all(&project).unwrap();
    let outer_target = dir.path().join("outer-target");
    let nearest_target = dir.path().join("nearest-target");
    cargo_config(&outer, &outer_target);
    cargo_config(&outer.join("crates"), &nearest_target);
    assert_eq!(
        cargo_output_dir_from(&project, &environment()).unwrap(),
        nearest_target,
        "Cargo merges ancestor configs nearest first"
    );
}

#[test]
fn the_cargo_home_config_is_read_when_no_ancestor_names_one() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let home = dir.path().join("cargo-home");
    let target = dir.path().join("cargo-home-target");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(
        home.join("config.toml"),
        cargo_build_config(&[("target-dir", &target)]),
    )
    .unwrap();
    let env = CargoEnvironment {
        cargo_home: os(&home),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        target,
        "Cargo reads $CARGO_HOME/config.toml after the per-directory ones"
    );
}

#[test]
fn cargo_home_defaults_to_the_home_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let home = dir.path().join("home");
    let target = dir.path().join("home-target");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    fs::create_dir_all(home.join(".cargo")).unwrap();
    fs::create_dir_all(&target).unwrap();
    // The project's own config names no target dir, so the search has to reach
    // the one under $HOME, which is where Cargo looks when CARGO_HOME is unset.
    fs::write(project.join(".cargo/config.toml"), "[build]\njobs = 2\n").unwrap();
    fs::write(
        home.join(".cargo/config.toml"),
        cargo_build_config(&[("target-dir", &target)]),
    )
    .unwrap();
    let env = CargoEnvironment {
        home: home.clone(),
        ..environment()
    };
    assert_eq!(cargo_output_dir_from(&project, &env).unwrap(), target);
}

#[test]
fn an_empty_cargo_home_falls_back_to_the_home_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let home = dir.path().join("home");
    let target = dir.path().join("home-target");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(home.join(".cargo")).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(
        home.join(".cargo/config.toml"),
        cargo_build_config(&[("target-dir", &target)]),
    )
    .unwrap();
    let env = CargoEnvironment {
        cargo_home: os(std::path::Path::new("")),
        home: home.clone(),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        target,
        "an empty CARGO_HOME is no setting at all"
    );
}

#[test]
fn a_relative_target_dir_resolves_against_the_config_that_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    cargo_config(&project, std::path::Path::new("build/output"));
    assert_eq!(
        cargo_output_dir_from(&project, &environment()).unwrap(),
        project.join("build/output"),
        "Cargo resolves a relative target-dir against the parent of .cargo"
    );
}

#[test]
fn a_target_dir_no_build_has_created_yet_probes_its_nearest_existing_ancestor() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    // Named but not created, which is how a target dir looks before the first
    // build, so this writes the config rather than going through `cargo_config`.
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[("target-dir", &project.join("target/debug"))]),
    )
    .unwrap();
    assert_eq!(
        cargo_output_dir_from(&project, &environment()).unwrap(),
        project,
        "the probe needs a directory it can write a temp file into"
    );
}

#[test]
fn a_project_with_nothing_configured_probes_its_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join("target")).unwrap();
    assert_eq!(
        cargo_output_dir_from(&project, &environment()).unwrap(),
        project.join("target"),
        "without a setting Cargo uses target below the working directory"
    );
}

#[test]
fn a_workspace_member_probes_the_workspace_target_dir() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let member = workspace.join("crates/member");
    fs::create_dir_all(&member).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/member\"]\n",
    )
    .unwrap();
    fs::write(
        member.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(workspace.join("target")).unwrap();
    assert_eq!(
        cargo_output_dir_from(&member, &environment()).unwrap(),
        workspace.join("target"),
        "Cargo puts the default target dir below the workspace root"
    );
}

#[test]
fn the_legacy_config_file_wins_over_the_modern_one() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    let legacy = dir.path().join("legacy-target");
    let modern = dir.path().join("modern-target");
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&modern).unwrap();
    // Cargo keeps the extensionless name for compatibility, reads it in
    // preference to `config.toml`, and warns that both exist.
    fs::write(
        project.join(".cargo/config"),
        cargo_build_config(&[("target-dir", &legacy)]),
    )
    .unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[("target-dir", &modern)]),
    )
    .unwrap();
    assert_eq!(
        cargo_output_dir_from(&project, &environment()).unwrap(),
        legacy,
        "Cargo reads the extensionless config in preference to config.toml"
    );
}

#[test]
fn the_build_dir_environment_selects_the_probe_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let target = dir.path().join("target");
    let build = dir.path().join("intermediate");
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(&build).unwrap();
    let env = CargoEnvironment {
        target_dir: os(&target),
        build_dir: os(&build),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        build,
        "the store hardlinks the intermediates, which live in the build dir"
    );
}

#[test]
fn the_build_dir_config_selects_the_probe_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    let target = dir.path().join("target");
    let build = dir.path().join("intermediate");
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(&build).unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[("target-dir", &target), ("build-dir", &build)]),
    )
    .unwrap();
    assert_eq!(
        cargo_output_dir_from(&project, &environment()).unwrap(),
        build,
        "a split build dir holds the intermediates the store hardlinks"
    );
}

#[test]
fn a_build_dir_template_expands_to_the_workspace_root() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let member = workspace.join("crates/member");
    fs::create_dir_all(member.join(".cargo")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/member\"]\n",
    )
    .unwrap();
    fs::write(
        member.join(".cargo/config.toml"),
        cargo_build_config(&[(
            "build-dir",
            std::path::Path::new("{workspace-root}/intermediate"),
        )]),
    )
    .unwrap();
    fs::create_dir_all(workspace.join("intermediate")).unwrap();
    assert_eq!(
        cargo_output_dir_from(&member, &environment()).unwrap(),
        workspace.join("intermediate"),
        "Cargo expands the workspace root in a build dir"
    );
}

#[test]
fn an_unknown_build_dir_template_leaves_the_probe_unresolved() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    let target = dir.path().join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[
            ("target-dir", &target),
            ("build-dir", std::path::Path::new("{nowhere}/intermediate")),
        ]),
    )
    .unwrap();
    assert_eq!(cargo_output_dir_from(&project, &environment()), None);
}

#[test]
fn an_empty_build_dir_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    fs::create_dir_all(project.join("target")).unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[("build-dir", std::path::Path::new(""))]),
    )
    .unwrap();
    assert_eq!(cargo_output_dir_from(&project, &environment()), None);
}

#[test]
fn an_empty_build_dir_environment_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    let build = dir.path().join("intermediate");
    fs::create_dir_all(&build).unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[("build-dir", &build)]),
    )
    .unwrap();
    let env = CargoEnvironment {
        build_dir: os(std::path::Path::new("")),
        ..environment()
    };
    assert_eq!(cargo_output_dir_from(&project, &env), None);
}

#[test]
fn an_empty_target_dir_in_a_config_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    fs::create_dir_all(project.join("target")).unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[("target-dir", std::path::Path::new(""))]),
    )
    .unwrap();
    assert_eq!(cargo_output_dir_from(&project, &environment()), None);
}

#[test]
fn a_config_without_a_build_table_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    fs::create_dir_all(project.join("target")).unwrap();
    fs::write(project.join(".cargo/config.toml"), "[net]\nretry = 2\n").unwrap();
    assert_eq!(
        cargo_output_dir_from(&project, &environment()).unwrap(),
        project.join("target"),
        "a config that names no target dir leaves the default in place"
    );
}

#[test]
fn a_malformed_config_leaves_the_probe_unresolved() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    fs::create_dir_all(project.join("target")).unwrap();
    fs::write(project.join(".cargo/config.toml"), "[build\n").unwrap();
    assert_eq!(cargo_output_dir_from(&project, &environment()), None);
}

#[test]
fn a_package_outside_a_workspace_probes_its_own_target_dir() {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("package");
    let source = package.join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        package.join("Cargo.toml"),
        "[package]\nname = \"package\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(package.join("target")).unwrap();
    assert_eq!(
        cargo_output_dir_from(&source, &environment()).unwrap(),
        package.join("target"),
        "Cargo puts the default target dir below the package root"
    );
}

#[test]
fn a_relative_cargo_home_resolves_against_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let cargo_home = project.join("cargo-home");
    let target = dir.path().join("cargo-home-target");
    fs::create_dir_all(&cargo_home).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(
        cargo_home.join("config.toml"),
        cargo_build_config(&[("target-dir", &target)]),
    )
    .unwrap();
    let env = CargoEnvironment {
        cargo_home: os(std::path::Path::new("cargo-home")),
        ..environment()
    };
    assert_eq!(
        cargo_output_dir_from(&project, &env).unwrap(),
        target,
        "a relative cargo home is relative to the working directory"
    );
}

#[test]
fn a_configured_build_dir_survives_an_environment_target_override() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let target = dir.path().join("environment-target");
    let build = project.join("intermediate");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(&build).unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        cargo_build_config(&[("build-dir", std::path::Path::new("intermediate"))]),
    )
    .unwrap();
    for env in [
        CargoEnvironment {
            target_dir: os(&target),
            ..environment()
        },
        CargoEnvironment {
            build_target_dir: os(&target),
            ..environment()
        },
    ] {
        assert_eq!(cargo_output_dir_from(&project, &env), Some(build.clone()));
    }
}

#[test]
fn a_member_can_name_a_workspace_outside_its_ancestors() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let member = dir.path().join("member");
    fs::create_dir_all(workspace.join("target")).unwrap();
    fs::create_dir_all(member.join("src")).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[workspace]\nmembers = [\"../member\"]\n",
    )
    .unwrap();
    fs::write(
        member.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\nworkspace = \"../workspace\"\n",
    )
    .unwrap();
    fs::write(member.join("src/lib.rs"), "").unwrap();
    assert_eq!(
        cargo_output_dir_from(&member, &environment()),
        Some(workspace.canonicalize().unwrap().join("target"))
    );
}

#[test]
fn an_unresolved_include_or_file_target_never_probes_a_different_directory() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(project.join(".cargo")).unwrap();
    fs::write(
        project.join(".cargo/config.toml"),
        "include = [\"other.toml\"]\n",
    )
    .unwrap();
    assert_eq!(cargo_output_dir_from(&project, &environment()), None);
    fs::remove_file(project.join(".cargo/config.toml")).unwrap();
    let target = project.join("file-target");
    fs::write(&target, b"keep this file").unwrap();
    let env = CargoEnvironment {
        target_dir: os(&target),
        ..environment()
    };
    assert_eq!(cargo_output_dir_from(&project, &env), None);
    assert_eq!(fs::read(target).unwrap(), b"keep this file");
}

#[test]
fn an_explicit_workspace_must_have_a_valid_workspace_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let member = dir.path().join("member");
    let workspace = dir.path().join("workspace");
    fs::create_dir_all(&member).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::write(
        member.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\nworkspace = \"../workspace\"\n",
    )
    .unwrap();
    for manifest in [
        "[package]\nname = \"other\"\nversion = \"0.1.0\"\n",
        "not = [valid",
        "workspace = \"not a table\"\n",
    ] {
        fs::write(workspace.join("Cargo.toml"), manifest).unwrap();
        assert_eq!(
            cargo_output_dir_from(&member, &environment()),
            None,
            "{manifest}"
        );
    }
    fs::remove_file(workspace.join("Cargo.toml")).unwrap();
    assert_eq!(cargo_output_dir_from(&member, &environment()), None);
    fs::write(member.join("Cargo.toml"), "[package").unwrap();
    assert_eq!(cargo_output_dir_from(&member, &environment()), None);
}

#[test]
fn direct_stats_include_rotated_events() {
    let dir = tempfile::tempdir().unwrap();
    let config = save_manifest_config(dir.path().join("cache"));
    let log = config.event_log_path();
    let mut hit =
        crate::events::BuildEvent::new_for_test("archived", crate::events::EventResult::LocalHit);
    hit.ts = kache_core::time::UtcTime::now() - std::time::Duration::from_secs(10 * 60);
    for _ in 0..12 {
        crate::events::log_event(&log, &hit).unwrap();
    }
    let max_size = std::fs::metadata(&log).unwrap().len() / 2;
    crate::events::rotate_events_if_needed(&log, max_size, 1, &config.cache_dir).unwrap();
    assert!(crate::events::read_events(&log).unwrap().len() < 12);
    let miss = crate::events::BuildEvent::new_for_test("live", crate::events::EventResult::Miss);
    crate::events::log_event(&log, &miss).unwrap();

    let snap = snapshot_from_direct_reads(&config, SinceWindow::DEFAULT, false);
    assert_eq!(snap.event_stats.local_hits, 12);
    assert_eq!(snap.event_stats.misses, 1);
}

#[test]
fn doctor_shows_the_native_linker_only_on_linux_with_a_compiler() {
    assert!(doctor_shows_native_linker(true, false));
    assert!(!doctor_shows_native_linker(true, true));
    assert!(!doctor_shows_native_linker(false, false));
    assert!(!doctor_shows_native_linker(false, true));
}

#[cfg(unix)]
#[test]
fn native_linker_check_reports_the_linker_cc_runs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    kache_fs::testutil::write_executable(
        &bin.join("cc"),
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in -print-prog-name=ld) echo {bin}/ld ;; -print-prog-name=*) echo \"${{arg#-print-prog-name=}}\" ;; esac; done\n",
            bin = bin.display()
        ),
    );
    kache_fs::testutil::write_executable(&bin.join("ld"), "#!/bin/sh\necho 'GNU ld (test) 2.45'\n");
    let env = |rustflags: &str| {
        crate::invocation::InvocationEnv::from_parts(
            [
                ("PATH", bin.as_os_str()),
                ("RUSTFLAGS", std::ffi::OsStr::new(rustflags)),
            ],
            Some(root.clone()),
            None,
            root.clone(),
        )
    };

    let check = native_linker_check(&env(""));
    assert!(check.pass, "{}", check.detail);
    assert_eq!(
        check.detail,
        format!("{} (GNU ld (test) 2.45)", bin.join("ld").display())
    );

    let check = native_linker_check(&env("-C link-arg=-fuse-ld=mold"));
    assert!(!check.pass);
    assert!(check.detail.contains("`ld.mold`"), "{}", check.detail);
    assert!(check.fix.is_some());

    // `-C linker` in RUSTFLAGS names the driver to ask.
    kache_fs::testutil::write_executable(
        &bin.join("clang"),
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do case \"$arg\" in -print-prog-name=ld) echo {bin}/ld.alt ;; esac; done\n",
            bin = bin.display()
        ),
    );
    kache_fs::testutil::write_executable(&bin.join("ld.alt"), "#!/bin/sh\necho 'LLD 22.1.8'\n");
    let check = native_linker_check(&env("-C linker=clang"));
    assert!(check.pass, "{}", check.detail);
    assert_eq!(
        check.detail,
        format!("{} (LLD 22.1.8)", bin.join("ld.alt").display())
    );
}
