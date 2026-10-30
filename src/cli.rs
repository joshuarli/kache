use crate::term;
use crate::term::human_println as println;
use anyhow::{Context, Result};
use std::io::IsTerminal;

use crate::config::Config;
use crate::daemon;
use crate::events;
use crate::since::SinceWindow;
use crate::store::{STAGING_SWEEP_GRACE, Store};
use kache_core::time::UtcTime;

mod doctor_daemon;
mod doctor_probe;
mod doctor_storage;
mod miss_diagnosis;
use miss_diagnosis::{Cause, MissDiagnosis};

// ── Stats snapshot (daemon-first, fallback to direct) ──────────────────────

/// Cached store + event stats, refreshed periodically.
/// Used by `kache stats`.
pub(crate) struct StatsSnapshot {
    pub stores: Vec<crate::store_view::StoreSummary>,
    pub total_size: u64,
    pub max_size: u64,
    pub entry_count: usize,
    pub event_stats: daemon::EventStatsResponse,
    pub daemon_connected: bool,
    pub daemon_version: String,
    pub daemon_build_epoch: u64,
    pub blob_stats: Option<crate::store::BlobStats>,
    /// The daemon's effective config from the stats response
    /// (kunobi-ninja/kache#689); `None` when the daemon is unreachable or
    /// predates effective-config reporting.
    pub daemon_effective_config: Option<daemon::EffectiveConfig>,
}

impl Default for StatsSnapshot {
    fn default() -> Self {
        Self {
            stores: Vec::new(),
            total_size: 0,
            max_size: 0,
            entry_count: 0,
            event_stats: daemon::EventStatsResponse {
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
            },
            daemon_connected: false,
            daemon_version: String::new(),
            daemon_build_epoch: 0,
            blob_stats: None,
            daemon_effective_config: None,
        }
    }
}

pub fn count_hit_rate(es: &daemon::EventStatsResponse) -> f64 {
    let total = es.local_hits + es.dups + es.misses;
    if total > 0 {
        (es.local_hits as f64 / total as f64) * 100.0
    } else {
        0.0
    }
}

pub fn compile_weighted_hit_rate(es: &daemon::EventStatsResponse) -> Option<f64> {
    let total = es.hit_compile_time_ms + es.miss_compile_time_ms;
    if total > 0 {
        Some((es.hit_compile_time_ms as f64 / total as f64) * 100.0)
    } else {
        None
    }
}

/// Try daemon first, fall back to direct reads.
///
/// With `announce_auto_start` set, the daemon-unreachable path says on stderr
/// that it is starting a daemon inheriting this process's environment
/// (kunobi-ninja/kache#689) — otherwise the same command silently flips from
/// "daemon's config" to "my config" depending on daemon liveness.
pub(crate) fn fetch_stats_snapshot(
    config: &Config,
    window: SinceWindow,
    announce_auto_start: bool,
    include_summaries: bool,
) -> StatsSnapshot {
    // Try daemon
    if let Ok(resp) = daemon::send_stats_request_options(
        config,
        false,
        include_summaries,
        None,
        Some(window),
    ) {
        return StatsSnapshot {
            stores: resp.stores,
            total_size: resp.total_size,
            max_size: resp.max_size,
            entry_count: resp.entry_count,
            event_stats: resp.events,
            daemon_connected: true,
            daemon_version: resp.version,
            daemon_build_epoch: resp.build_epoch,
            blob_stats: resp.blob_stats,
            daemon_effective_config: resp.effective_config,
        };
    }

    // Daemon unreachable or stale socket: best-effort auto-start for stats UX.
    // This path is not used by compile-time hot operations.
    if announce_auto_start {
        eprintln!(
            "kache: no daemon reachable at {}; starting one inheriting this process's environment",
            config.socket_path().display()
        );
    }
    if daemon::start_daemon_background().unwrap_or(false)
        && let Ok(resp) = daemon::send_stats_request_options(
            config,
            false,
            include_summaries,
            None,
            Some(window),
        )
    {
        return StatsSnapshot {
            stores: resp.stores,
            total_size: resp.total_size,
            max_size: resp.max_size,
            entry_count: resp.entry_count,
            event_stats: resp.events,
            daemon_connected: true,
            daemon_version: resp.version,
            daemon_build_epoch: resp.build_epoch,
            blob_stats: resp.blob_stats,
            daemon_effective_config: resp.effective_config,
        };
    }

    // Fallback: direct reads (no daemon reachable).
    snapshot_from_direct_reads(config, window, include_summaries)
}

/// Build a [`StatsSnapshot`] by reading the store and event log directly, with no
/// daemon. Split out from [`fetch_stats_snapshot`]'s fallback so it is unit-
/// testable against a seeded cache without a running (or auto-started) daemon.
pub(crate) fn snapshot_from_direct_reads(
    config: &Config,
    window: SinceWindow,
    include_summaries: bool,
) -> StatsSnapshot {
    let inventory = crate::store_view::read(config, false, "size").unwrap_or_default();

    let since = window.cutoff(UtcTime::now());
    let es = events::event_stats_since(&config.event_log_path(), since).unwrap_or_default();
    let _ = include_summaries;

    StatsSnapshot {
        total_size: inventory.total_size,
        max_size: inventory.max_size,
        entry_count: inventory.entry_count,
        stores: inventory.stores,
        event_stats: daemon::EventStatsResponse {
            local_hits: es.local_hits,
            dups: es.dups,
            misses: es.misses,
            errors: es.errors,
            total_elapsed_ms: es.total_elapsed_ms,
            hit_elapsed_ms: es.hit_elapsed_ms,
            miss_elapsed_ms: es.miss_elapsed_ms,
            hit_compile_time_ms: es.hit_compile_time_ms,
            miss_compile_time_ms: es.miss_compile_time_ms,
            store_output_blobs: es.store_output_blobs,
            store_duplicate_blobs: es.store_duplicate_blobs,
            store_new_blobs: es.store_new_blobs,
        },
        daemon_connected: false,
        daemon_version: String::new(),
        daemon_build_epoch: 0,
        blob_stats: Some(inventory.blob_stats),
        daemon_effective_config: None,
    }
}

/// Index tables whose size explains a store: the entry and blob maps that grow
/// with the cache, and the key-side caches and tombstones beside them.
const MACHINE_INDEX_TABLES: [&str; 7] = [
    "entries",
    "blobs",
    "entry_blobs",
    "file_hashes",
    "cc_preprocess_memos",
    "input_predictions",
    "eviction_tombstones",
];

/// Read the shared cache without getting in a build's way: a read-only
/// connection (`query_only`, 25 ms busy timeout), no schema work and no
/// daemon, so a figure the index is too busy to answer is left out rather
/// than waited for. Read-only also means closing the connection can never
/// checkpoint the WAL into `index.db`.
pub(crate) fn machine_snapshot(config: &Config) -> crate::otel::MachineSnapshot {
    let db_path = config.index_db_path();
    let index_bytes = index_file_bytes(config);
    let mut snap = crate::otel::MachineSnapshot {
        index_bytes,
        wal_bytes: index_wal_bytes(config),
        gc: crate::report::read_gc_stats(&config.cache_dir),
        ..Default::default()
    };
    if index_bytes.is_none() {
        return snap;
    }
    let Ok(db) = crate::store::open_index_db_readonly(&db_path) else {
        return snap;
    };
    snap.store_physical_bytes = db
        .query_row("SELECT COALESCE(SUM(size), 0) FROM blobs", [], |row| {
            row.get::<_, i64>(0)
        })
        .ok()
        .map(|bytes| bytes.max(0) as u64);
    snap.index_free_bytes = index_free_bytes(&db);
    snap.blob_drift = crate::store::blob_refcount_drift(&db).ok();
    for table in MACHINE_INDEX_TABLES {
        let top: rusqlite::Result<Option<i64>> =
            db.query_row(&rowid_high_water_sql(table), [], |row| row.get(0));
        if let Ok(top) = top {
            snap.rowid_high_water
                .push((table, top.unwrap_or(0).max(0) as u64));
        }
    }
    snap
}

/// Bytes `index.db` holds in free pages: both pragmas read the database
/// header, so this costs no table scan.
fn index_free_bytes(db: &rusqlite::Connection) -> Option<u64> {
    let pragma = |sql: &str| db.query_row(sql, [], |row| row.get::<_, i64>(0)).ok();
    free_page_bytes(
        pragma("PRAGMA freelist_count")?,
        pragma("PRAGMA page_size")?,
    )
}

fn free_page_bytes(pages: i64, page_size: i64) -> Option<u64> {
    u64::try_from(pages)
        .ok()?
        .checked_mul(u64::try_from(page_size).ok()?)
}

/// A table's rowid high-water mark: one seek to the last leaf of its b-tree,
/// where `COUNT(*)` would read every page of a table that can hold gigabytes.
fn rowid_high_water_sql(table: &str) -> String {
    format!("SELECT MAX(rowid) FROM {table}")
}

/// `index.db` plus its `-wal`, or `None` when there is no index yet.
fn index_file_bytes(config: &Config) -> Option<u64> {
    let db_path = config.index_db_path();
    let mut wal = db_path.clone().into_os_string();
    wal.push("-wal");
    std::fs::metadata(&db_path)
        .ok()
        .map(|db| db.len() + std::fs::metadata(&wal).map(|w| w.len()).unwrap_or(0))
}

/// The index's `-wal` file alone: 0 when it is absent, `None` when there is
/// no index. A WAL that stays large means checkpoints are not keeping up
/// with the writes.
fn index_wal_bytes(config: &Config) -> Option<u64> {
    let db_path = config.index_db_path();
    std::fs::metadata(&db_path).ok()?;
    let mut wal = db_path.into_os_string();
    wal.push("-wal");
    Some(std::fs::metadata(&wal).map(|w| w.len()).unwrap_or(0))
}

/// The machine-level session log. In the cache dir, which every job on the
/// host shares, not the runtime dir a CI job deletes when it ends.
pub(crate) fn session_log_path(config: &Config) -> std::path::PathBuf {
    config.cache_dir.join("telemetry").join("sessions.jsonl")
}

/// Append `report` to the machine-level session log (`kache report
/// --record`): its summary and timing breakdown, plus what the host looked
/// like at that moment. Meant for the end of a CI job, before its runtime dir
/// and the events in it are deleted.
pub(crate) fn record_session(config: &Config, report: &crate::report::BuildReport) -> Result<()> {
    let mut loads = [0.0; 3];
    let written = crate::otel::sample_load_averages(&mut loads);
    let machine = crate::report::SessionMachine {
        load_1m: crate::otel::one_minute_load(written, loads[0]),
        cpus: std::thread::available_parallelism()
            .ok()
            .and_then(|n| u32::try_from(n.get()).ok()),
        index_bytes: index_file_bytes(config),
        store_max: config.max_size,
    };
    let record = crate::report::SessionRecord::from_report(report, machine);
    let path = session_log_path(config);
    crate::events::append_json_line(&path, &record)?;
    crate::events::rotate_if_needed(
        &path,
        config.event_log_max_size,
        config.event_log_keep_lines,
    )
}

/// Write cache counters as OTLP JSON for Kartero (`metrics.otlp.json` +
/// `schema_version`). Uses the running daemon when reachable; otherwise the
/// local store. Does not auto-start a daemon, so a finished bench dumps what
/// is already on disk instead of an empty new process.
pub fn telemetry_write(
    config: &Config,
    dir: &std::path::Path,
    scenario: Option<&str>,
    phase: Option<&str>,
) -> Result<()> {
    let snap = match daemon::send_stats_request_options(
        config,
        false,
        false,
        None,
        Some(SinceWindow::DEFAULT),
    ) {
        Ok(resp) => StatsSnapshot {
            stores: resp.stores,
            total_size: resp.total_size,
            max_size: resp.max_size,
            entry_count: resp.entry_count,
            event_stats: resp.events,
            daemon_connected: true,
            daemon_version: resp.version,
            daemon_build_epoch: resp.build_epoch,
            blob_stats: resp.blob_stats,
            daemon_effective_config: resp.effective_config,
        },
        Err(_) => snapshot_from_direct_reads(config, SinceWindow::DEFAULT, false),
    };
    crate::otel::write_otlp(
        dir,
        &otel_snapshot_from_stats(config, &snap),
        &machine_snapshot(config),
        crate::VERSION,
        scenario,
        phase,
    )?;
    eprintln!(
        "wrote {} and {}",
        dir.join(crate::otel::METRICS_FILE).display(),
        dir.join(crate::otel::SCHEMA_VERSION_FILE).display()
    );
    Ok(())
}

fn otel_snapshot_from_stats(config: &Config, snap: &StatsSnapshot) -> crate::otel::OtelSnapshot {
    let _ = config;
    crate::otel::OtelSnapshot {
        store_max: snap.max_size,
        store_size: Some(snap.total_size),
        store_entries: Some(snap.entry_count as u64),
    }
}

// ── kache stats ────────────────────────────────────────────────────────────

fn cloned_targets_row(disk: &crate::machine::DiskView) -> Option<StatsRow> {
    let cloned = disk.cloned_into_targets_bytes;
    let snapshot = disk.snapshot_retained_bytes;
    if cloned == 0 && snapshot == 0 {
        return None;
    }
    let mut notes = vec!["private".to_string()];
    if cloned > 0 {
        // Evicting a blob a target still holds frees nothing, so a size
        // sweep leaves these out, and the Cache size can stay above Limit.
        notes.push(format!(
            "{} cloned into target/, which the limit leaves out",
            term::bytes(cloned)
        ));
    }
    if snapshot > 0 {
        notes.push(format!(
            "{} held only by filesystem snapshots",
            term::bytes(snapshot)
        ));
    }
    Some((
        "On disk",
        term::bytes(disk.disk_private_bytes),
        notes.join(", "),
    ))
}

/// The volume holding the cache: its size, what is free, and the free-space
/// floor below which kache recovers space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, deser::Serialize)]
pub(crate) struct VolumeView {
    total_bytes: u64,
    free_bytes: u64,
    /// `cache.auto_recover_min_free_bytes`; `0` means recovery is off.
    recovery_floor_bytes: u64,
    /// Free space is below a recovery floor that is set.
    below_floor: bool,
}

impl VolumeView {
    fn new(usage: kache_fs::VolumeUsage, floor: u64) -> Self {
        Self {
            total_bytes: usage.total,
            free_bytes: usage.free,
            recovery_floor_bytes: floor,
            below_floor: floor > 0 && usage.free < floor,
        }
    }
}

/// The cache volume, `None` where its usage cannot be read (not Unix).
fn volume_view(config: &Config, daemon: Option<&daemon::EffectiveConfig>) -> Option<VolumeView> {
    let floor = recovery_in_effect(config, daemon);
    kache_fs::volume_usage(&config.cache_dir).map(|usage| VolumeView::new(usage, floor))
}

fn recovery_in_effect(config: &Config, daemon: Option<&daemon::EffectiveConfig>) -> u64 {
    daemon
        .and_then(|daemon| daemon.auto_recover_min_free_bytes)
        .unwrap_or(config.auto_recover_min_free_bytes)
}

fn volume_row(volume: &VolumeView) -> StatsRow {
    let used = volume.total_bytes.saturating_sub(volume.free_bytes);
    let percent = if volume.total_bytes == 0 {
        0
    } else {
        (used as u128 * 100 / volume.total_bytes as u128) as u64
    };
    let scope = "cache only";
    let recovery = if volume.recovery_floor_bytes == 0 {
        "free-space recovery off".to_string()
    } else if volume.below_floor {
        format!(
            "below the {} recovery floor · {scope}",
            term::bytes(volume.recovery_floor_bytes)
        )
    } else {
        format!(
            "recovery keeps {} free · {scope}",
            term::bytes(volume.recovery_floor_bytes)
        )
    };
    (
        "Volume",
        format!("{} free", term::bytes(volume.free_bytes)),
        format!(
            "of {} · {percent}% used · {recovery}",
            term::bytes(volume.total_bytes)
        ),
    )
}

/// Print a one-shot stats summary to stdout.
pub fn stats(
    config: &Config,
    provenance: &crate::config::ConfigFileProvenance,
    window: SinceWindow,
    json: bool,
) -> Result<()> {
    let snap = fetch_stats_snapshot(config, window, true, true);

    // #689: the numbers below are daemon-first, so before rendering them say
    // when the daemon's effective config disagrees with this invocation's —
    // otherwise a config edit that has not reached the daemon masquerades as
    // "kache ignores config files".
    if let Some(eff) = &snap.daemon_effective_config {
        for warning in config_mismatch_warnings(config, provenance, eff) {
            eprintln!("{warning}");
        }
    }

    let main_store = snap.stores.first();
    let store_bytes = main_store
        .and_then(|s| s.blob_stats.as_ref())
        .or(snap.blob_stats.as_ref())
        .map(|s| s.total_blob_size)
        .unwrap_or(snap.total_size);
    let disk = crate::machine::disk_view(
        &config.store_dir(),
        store_bytes,
        main_store.map_or(snap.max_size, |s| s.max_size),
    );
    let volume = volume_view(config, snap.daemon_effective_config.as_ref());
    let host_config = host_config_in_effect(&crate::config::host_config_status());
    let machine = machine_snapshot(config);
    let lifetime = crate::savings::lifetime(config);

    if json {
        #[derive(deser::Serialize)]
        struct Body<'a> {
            disk: crate::machine::DiskView,
            /// The volume holding the cache. Absent where its usage cannot
            /// be read.
            #[deser(skip_serializing_if = Option::is_none)]
            volume: Option<VolumeView>,
            stores: &'a [crate::store_view::StoreSummary],
            /// This machine's shared index (`index.db` plus `-wal`) and each
            /// table's largest rowid, as `kache telemetry write` reports them.
            /// The rowid grows with every insert and replacement; it is not
            /// a row count.
            #[deser(skip_serializing_if = Option::is_none)]
            index_bytes: Option<u64>,
            /// The `-wal` file alone, also counted in `index_bytes`.
            #[deser(skip_serializing_if = Option::is_none)]
            index_wal_bytes: Option<u64>,
            #[deser(skip_serializing_if = std::collections::BTreeMap::is_empty)]
            index_rowid_high_water: std::collections::BTreeMap<&'static str, u64>,
            /// The last GC run, from `gc_stats.json`.
            #[deser(skip_serializing_if = Option::is_none)]
            gc: Option<crate::report::GcStatsPersisted>,
            /// Totals since counting began: the savings ledger plus the
            /// event log. Absent while nothing is recorded.
            #[deser(skip_serializing_if = Option::is_none)]
            lifetime: Option<crate::savings::Totals>,
            entries: usize,
            hit_rate_pct: f64,
            weighted_hit_rate_pct: Option<f64>,
            time_saved_ms: u64,
            hit_elapsed_ms: u64,
            miss_elapsed_ms: u64,
            total_elapsed_ms: u64,
            local_hits: usize,
            dups: usize,
            misses: usize,
            daemon_connected: bool,
            daemon_version: String,
            daemon_epoch: u64,
            /// Whole hours, rounded down (0 for a sub-hour window). Kept for
            /// consumers that predate `since_secs`.
            hours: u64,
            since_secs: u64,
            /// The window as requested: `15m`, `2h`, `24h`.
            since: String,
            /// The host config file merged under the chosen one, when there is
            /// one in effect.
            #[deser(skip_serializing_if = Option::is_none)]
            host_config: Option<&'a str>,
        }
        let hit_rate = count_hit_rate(&snap.event_stats);
        return crate::machine::emit(
            "stats",
            Body {
                disk: disk.clone(),
                volume,
                stores: &snap.stores,
                index_bytes: machine.index_bytes,
                index_wal_bytes: machine.wal_bytes,
                index_rowid_high_water: machine.rowid_high_water.iter().copied().collect(),
                gc: machine.gc.clone(),
                lifetime,
                entries: snap.entry_count,
                hit_rate_pct: hit_rate,
                weighted_hit_rate_pct: compile_weighted_hit_rate(&snap.event_stats),
                time_saved_ms: snap.event_stats.hit_compile_time_ms,
                hit_elapsed_ms: snap.event_stats.hit_elapsed_ms,
                miss_elapsed_ms: snap.event_stats.miss_elapsed_ms,
                total_elapsed_ms: snap.event_stats.total_elapsed_ms,
                local_hits: snap.event_stats.local_hits,
                dups: snap.event_stats.dups,
                misses: snap.event_stats.misses,
                daemon_connected: snap.daemon_connected,
                daemon_version: snap.daemon_version.clone(),
                daemon_epoch: snap.daemon_build_epoch,
                hours: window.hours(),
                since_secs: window.secs(),
                since: window.label(),
                host_config: host_config.as_deref(),
            },
            crate::machine::next_for_clones(&disk),
        );
    }

    let mut extras = StatsExtras {
        lifetime: lifetime_rows(
            lifetime.as_ref(),
            kache_core::time::today_local(),
        ),
        ..Default::default()
    };
    extras.cache.extend(cloned_targets_row(&disk));
    extras.cache.extend(volume.as_ref().map(volume_row));
    let mut compact_machine = machine.clone();
    compact_machine.rowid_high_water.clear();
    extras.cache.extend(machine_rows(&compact_machine));
    if let Some(path) = &host_config {
        extras
            .service
            .push(("Host config", path.clone(), String::new()));
    }
    for line in render_stats_with(&snap, config, window, &extras) {
        println!("{line}");
    }

    println!();
    println!("Details: `kache stats --full`");
    Ok(())
}

/// `kache stats` rows for the machine's shared index and GC record: what the
/// cache costs the host beyond its artifacts, and whether GC keeps up. Pure,
/// like [`render_stats`].
fn machine_rows(machine: &crate::otel::MachineSnapshot) -> Vec<StatsRow> {
    let mut rows = Vec::new();
    if let Some(bytes) = machine.index_bytes {
        let mut notes = Vec::new();
        if let Some(wal) = machine.wal_bytes {
            notes.push(format!("WAL {}", term::bytes(wal)));
        }
        // Each table's largest rowid: it grows with every insert and
        // replacement, so it shows where the index churns, not a row count.
        let mut tables = machine.rowid_high_water.clone();
        tables.sort_by_key(|&(_, rows)| std::cmp::Reverse(rows));
        let top: Vec<String> = tables
            .iter()
            .take(3)
            .map(|(table, rows)| format!("{table} {}", term::count(*rows)))
            .collect();
        if !top.is_empty() {
            notes.push(format!("rowid high-water: {}", top.join(", ")));
        }
        rows.push(("Index", term::bytes(bytes), notes.join(" · ")));
    }
    if let Some(gc) = &machine.gc {
        let source = if gc.source.is_empty() {
            "daemon"
        } else {
            gc.source.as_str()
        };
        let when = UtcTime::parse_rfc3339(&gc.last_run)
            .map(|at| kache_core::time::local_parts(at).month_name_day_clock())
            .unwrap_or_else(|| gc.last_run.clone());
        let mut note = format!(
            "{source}, {} evicted",
            term::count(gc.entries_evicted as u64)
        );
        if gc.entries_failed > 0 {
            note.push_str(&format!(
                ", {} failed ({} lost the index write lock)",
                gc.entries_failed, gc.entries_locked
            ));
        }
        if gc.evict_write_ms > 0 {
            note.push_str(&format!(", {} ms in index writes", gc.evict_write_ms));
        }
        rows.push(("Last GC", when, note));
    }
    rows
}

/// One `label   value   note` row of `kache stats`.
pub(crate) type StatsRow = term::Row<'static>;

/// Rows `kache stats` adds to the snapshot's sections from its own reads.
#[derive(Default)]
pub(crate) struct StatsExtras {
    pub(crate) lifetime: Vec<StatsRow>,
    pub(crate) cache: Vec<StatsRow>,
    pub(crate) service: Vec<StatsRow>,
}

/// [`render_stats_with`] without extra rows.
#[cfg(test)]
pub(crate) fn render_stats(
    snap: &StatsSnapshot,
    config: &Config,
    window: SinceWindow,
) -> Vec<String> {
    render_stats_with(snap, config, window, &StatsExtras::default())
}

/// The `kache stats` summary: a title, then the build, cache and service
/// sections on shared columns.
pub(crate) fn render_stats_with(
    snap: &StatsSnapshot,
    config: &Config,
    window: SinceWindow,
    extras: &StatsExtras,
) -> Vec<String> {
    let mut cache = cache_rows(snap, config);
    cache.extend(extras.cache.iter().cloned());
    let mut service = service_rows(snap);
    service.extend(extras.service.iter().cloned());
    let mut lines = vec![
        term::heading(format!("kache · last {window}")),
        String::new(),
    ];
    lines.extend(term::named_sections(&[
        ("Builds", build_rows(snap)),
        ("Lifetime", extras.lifetime.clone()),
        ("Storage", cache),
        ("Services", service),
    ]));
    lines
}

/// The `kache stats` section on lifetime totals, after the window's rows.
/// A total that is still zero gets no row, and the section is empty when
/// every total is. Dates are local, like the `Last GC` row.
fn lifetime_rows(
    totals: Option<&crate::savings::Totals>,
    today: kache_core::time::CivilDate,
) -> Vec<StatsRow> {
    let Some(totals) = totals else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    if totals.hits > 0 {
        rows.push((
            "Time saved",
            term::duration_ms(totals.hit_compile_time_ms),
            format!(
                "compile work avoided over {} {}",
                term::count(totals.hits),
                if totals.hits == 1 { "hit" } else { "hits" }
            ),
        ));
    }
    if totals.zero_copy_bytes + totals.copied_bytes > 0 {
        rows.push((
            "No-copy",
            term::bytes(totals.zero_copy_bytes),
            format!(
                "restored without copying, {} copied",
                term::bytes(totals.copied_bytes)
            ),
        ));
    }
    if totals.pruned_bytes() > 0 {
        rows.push((
            "Pruned",
            term::bytes(totals.pruned_bytes()),
            format!(
                "{} automatically, {} on request",
                term::bytes(totals.pruned_automatic_bytes),
                term::bytes(totals.pruned_requested_bytes)
            ),
        ));
    }
    if rows.is_empty() {
        return rows;
    }
    let since = kache_core::time::local_date(totals.since);
    rows.insert(0, ("Since", since_label(since, today), String::new()));
    rows
}

/// `Sep 12`, with the year when it is not this year's.
fn since_label(since: kache_core::time::CivilDate, today: kache_core::time::CivilDate) -> String {
    since.label_since(today)
}

/// Hit rate, source counts, weighted hit rate, time saved and miss time.
fn build_rows(snap: &StatsSnapshot) -> Vec<StatsRow> {
    let es = &snap.event_stats;
    let hits = es.local_hits;
    let total = hits + es.dups + es.misses;
    let from_cache = if total == 0 {
        "no builds in this window".to_string()
    } else {
        format!(
            "{} of {} from cache",
            term::count(hits as u64),
            term::counted(total as u64, "crate", "crates")
        )
    };
    let mut rows = vec![
        ("Hit rate", term::percent(count_hit_rate(es)), from_cache),
        (
            "Sources",
            format!(
                "local {} · dup {} · miss {}",
                term::count(es.local_hits as u64),
                term::count(es.dups as u64),
                term::count(es.misses as u64),
            ),
            String::new(),
        ),
    ];
    if let Some(weighted) = compile_weighted_hit_rate(es) {
        rows.push((
            "By time",
            term::percent(weighted),
            "compile time served from cache".to_string(),
        ));
    }
    rows.push(if es.hit_compile_time_ms > 0 {
        (
            "Time saved",
            term::duration_ms(es.hit_compile_time_ms),
            "compile work avoided".to_string(),
        )
    } else {
        ("Time saved", "none yet".to_string(), String::new())
    });
    if es.total_elapsed_ms > 0 {
        let miss_share = (es.miss_elapsed_ms as f64 / es.total_elapsed_ms as f64) * 100.0;
        rows.push((
            "Miss time",
            term::duration_ms(es.miss_elapsed_ms),
            format!("{} of wrapper time", term::percent(miss_share)),
        ));
    }
    rows
}

/// Parenthetical for a cap that is the disk-share budget, matching
/// [`crate::config::describe_max_size`].
///
/// One store whose cap equals that budget says `5% of <disk>, floor 5.0 GiB,
/// cap 100.0 GiB`, or `default; disk size unknown` when the probe failed. Any
/// other cap, and a summary that covers more than one store, stays unlabeled.
fn cache_limit_note(store_count: usize, max_size: u64, filesystem_bytes: Option<u64>) -> String {
    if store_count > 1 {
        return String::new();
    }
    let derived = crate::config::disk_share_budget(filesystem_bytes);
    if max_size != derived {
        return String::new();
    }
    match filesystem_bytes.filter(|&bytes| bytes > 0) {
        Some(total) => format!(" ({})", crate::config::disk_share_note(total)),
        None => " (default; disk size unknown)".to_string(),
    }
}

/// The store, each extra store, and content dedup.
fn cache_rows(snap: &StatsSnapshot, config: &Config) -> Vec<StatsRow> {
    let note = cache_limit_note(
        snap.stores.len(),
        snap.max_size,
        crate::cache_fs::probe(&config.cache_dir).total_bytes,
    );
    let mut rows = vec![
        (
            "Cache",
            term::bytes(
                snap.blob_stats
                    .as_ref()
                    .map_or(snap.total_size, |stats| stats.total_blob_size),
            ),
            format!(
                "{} · {}",
                if snap.blob_stats.is_some() {
                    "blob bytes"
                } else {
                    "logical (blob size unavailable)"
                },
                term::counted(snap.entry_count as u64, "entry", "entries")
            ),
        ),
        (
            "Limit",
            term::bytes(snap.max_size),
            format!("registered blob budget{note}"),
        ),
    ];
    for store in &snap.stores {
        if let Some(error) = &store.error {
            rows.push((
                "Store",
                "unavailable".to_string(),
                format!("{} ({error})", term::home_path(&store.path)),
            ));
        } else if snap.stores.len() > 1 {
            rows.push((
                "Store",
                term::bytes(
                    store
                        .blob_stats
                        .as_ref()
                        .map_or(store.bytes, |stats| stats.total_blob_size),
                ),
                format!(
                    "{} · of {} · {} · {}",
                    if store.blob_stats.is_some() {
                        "blob bytes"
                    } else {
                        "logical (blob size unavailable)"
                    },
                    term::bytes(store.max_size),
                    term::counted(store.entries as u64, "entry", "entries"),
                    term::home_path(&store.path)
                ),
            ));
        }
    }
    if let Some(blob_stats) = snap
        .blob_stats
        .as_ref()
        .filter(|stats| stats.total_blobs > 0 || stats.total_logical_size > 0)
    {
        let savings_pct = if blob_stats.total_logical_size > 0 {
            blob_stats.savings as f64 / blob_stats.total_logical_size as f64 * 100.0
        } else {
            0.0
        };
        rows.push((
            "Dedup",
            term::counted(blob_stats.total_blobs as u64, "blob", "blobs"),
            format!(
                "{} logical, {savings_pct:.1}% saved",
                term::bytes(blob_stats.total_logical_size)
            ),
        ));
    }
    rows
}

/// The daemon, the remote, and remote traffic when there is any.
fn service_rows(snap: &StatsSnapshot) -> Vec<StatsRow> {
    let mut rows: Vec<StatsRow> = Vec::new();
    // Daemon status
    if snap.daemon_connected {
        let mismatch =
            if crate::daemon::client_version_is_newer(crate::VERSION, &snap.daemon_version) {
                " · older release; upgrade available"
            } else {
                ""
            };
        // Name the config file the daemon loaded (#689): the store cap and
        // policy lines describe THAT file, which need not be the one this
        // invocation resolved.
        let config_note = snap
            .daemon_effective_config
            .as_ref()
            .map(|eff| {
                format!(
                    " · {}",
                    term::home_path(std::path::Path::new(&eff.config_path))
                )
            })
            .unwrap_or_default();
        rows.push((
            "Daemon",
            if snap.daemon_version.is_empty() {
                "release unknown".into()
            } else {
                format!("v{}", snap.daemon_version)
            },
            format!("running{config_note}{mismatch}"),
        ));
    } else {
        rows.push(("Daemon", "offline".to_string(), String::new()));
    }

    rows
}

/// One warning line per rendered stats field where this process's resolved
/// config disagrees with the daemon's effective config
/// (kunobi-ninja/kache#689). Each line names both values and both sources, so
/// "the daemon shows 50 GiB after I set 117 GiB" reads as the config-delivery
/// problem it is, never as "kache ignores config files". Empty when the two
/// agree. Config provenance is itself meaningful: different resolved paths
/// warn even when their current rendered values happen to match.
/// Pure (no I/O) so every divergence branch is unit-testable without a daemon.
pub(crate) fn config_mismatch_warnings(
    config: &Config,
    provenance: &crate::config::ConfigFileProvenance,
    eff: &daemon::EffectiveConfig,
) -> Vec<String> {
    let daemon_side = format!(
        "daemon (started {}, config {})",
        format_epoch_ms_utc(eff.started_at_ms),
        eff.config_path
    );
    let client_side = format!("this process's config ({})", provenance.path.display());
    let remedy = format!(
        "the daemon's value is in effect; edit its watched config ({}) and let it restart; \
         environment overrides require restarting it from an environment it inherits",
        eff.config_path
    );

    let mut warnings = Vec::new();
    if eff.config_path != provenance.path.display().to_string() {
        warnings.push(format!(
            "warning: daemon loaded config {}; this process resolved {} — values may diverge; \
             edit the daemon's watched config to apply persistent changes",
            eff.config_path,
            provenance.path.display(),
        ));
    } else if eff
        .config_fingerprint
        .as_deref()
        .is_some_and(|fingerprint| fingerprint != provenance.fingerprint)
    {
        warnings.push(format!(
            "warning: daemon and this process read different snapshots of config {} — the \
             daemon's loaded values remain in effect until its watched-file restart completes",
            eff.config_path,
        ));
    }
    if eff.max_size != config.max_size {
        warnings.push(format!(
            "warning: {daemon_side} has local_max_size={}; {client_side} says {} — {remedy}",
            kache_core::size::format_bytes(eff.max_size),
            kache_core::size::format_bytes(config.max_size),
        ));
    }
    if eff.cache_dir != config.cache_dir.display().to_string() {
        warnings.push(format!(
            "warning: {daemon_side} has local_store={}; {client_side} says {} — the daemon's \
             numbers describe ITS store; {remedy}",
            eff.cache_dir,
            config.cache_dir.display(),
        ));
    }
    if !eff.runtime_dir.is_empty() && eff.runtime_dir != config.runtime_dir.display().to_string() {
        warnings.push(format!(
            "warning: {daemon_side} has runtime_dir={}; {client_side} says {} — {remedy}",
            eff.runtime_dir,
            config.runtime_dir.display(),
        ));
    }
    if let Some(floor) = eff.auto_recover_min_free_bytes
        && floor != config.auto_recover_min_free_bytes
    {
        warnings.push(format!(
            "warning: {daemon_side} has auto_recover_min_free_bytes={floor}; {client_side} says {} — {remedy}",
            config.auto_recover_min_free_bytes
        ));
    }
    warnings
}

/// Format a Unix-millisecond timestamp as a short UTC datetime for the
/// mismatch warnings; `0` (an old daemon's serde default) stays honest as
/// "unknown time" instead of claiming 1970.
fn format_epoch_ms_utc(ms: u64) -> String {
    if ms == 0 {
        return "unknown time".to_string();
    }
    UtcTime::from_timestamp_millis(ms as i64)
        .map(kache_core::time::format_utc_ymd_hm)
        .unwrap_or_else(|| "unknown time".to_string())
}

// ── kache report ──────────────────────────────────────────────────────────

pub fn stats_last_build(
    config: &Config,
    root: Option<std::path::PathBuf>,
    json: bool,
) -> Result<()> {
    let filter = crate::report::ReportFilter {
        root,
        last_build: true,
    };
    let report =
        crate::report::generate_report_with_filter(config, SinceWindow::DEFAULT, 10, &filter)?;
    // `--record` takes the full-report path. This summary is what
    // `kache stats --last-build` prints, and a host that set
    // `record_sessions` still needs the session written.
    let recorded = if config.record_sessions {
        record_session(config, &report)
    } else {
        Ok(())
    };
    let printed = if json {
        #[derive(deser::Serialize)]
        struct Body {
            report: crate::report::BuildReport,
        }
        crate::machine::emit("stats", Body { report }, Vec::new())
    } else {
        println!("{}", crate::report::format_text(&report));
        Ok(())
    };
    if let Err(error) = recorded {
        eprintln!("warning: this session was not recorded: {error:#}");
    }
    printed
}

/// The full build report's output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Text,
    Json,
    Markdown,
    Github,
    /// Perfetto / Chrome trace JSON
    Trace,
}

impl ReportFormat {
    /// Spellings the `--format` flag accepts, with the short and
    /// tool-specific aliases.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "text" => ReportFormat::Text,
            "json" => ReportFormat::Json,
            "markdown" | "md" => ReportFormat::Markdown,
            "github" | "gh" => ReportFormat::Github,
            "trace" | "perfetto" | "chrome-trace" => ReportFormat::Trace,
            _ => return None,
        })
    }

    /// Every spelling `--format` accepts, for help and error messages.
    pub(crate) fn spellings() -> &'static [&'static str] {
        &[
            "text",
            "json",
            "markdown",
            "md",
            "github",
            "gh",
            "trace",
            "perfetto",
            "chrome-trace",
        ]
    }
}

impl ReportFormat {
    pub fn name(self) -> &'static str {
        match self {
            ReportFormat::Text => "text",
            ReportFormat::Json => "json",
            ReportFormat::Markdown => "markdown",
            ReportFormat::Github => "github",
            ReportFormat::Trace => "trace",
        }
    }
}

pub fn report(
    config: &Config,
    format: ReportFormat,
    window: SinceWindow,
    filter: crate::report::ReportFilter,
    output: Option<std::path::PathBuf>,
    top: usize,
    record: bool,
    redact: bool,
) -> Result<()> {
    let mut report = if filter.root.is_some() || filter.last_build {
        crate::report::generate_report_with_filter(config, window, top, &filter)?
    } else {
        crate::report::generate_report(config, window, top)?
    };

    // Record the unredacted report. Redaction is for the copy being shared.
    let recorded = if record || config.record_sessions {
        record_session(config, &report)
    } else {
        Ok(())
    };
    let terms = if redact {
        let home = kache_fs::home_dir().unwrap_or_default();
        let terms = crate::report::redaction_terms(&report, &[&config.cache_dir, &home]);
        crate::report::redact_report(&mut report);
        terms
    } else {
        Vec::new()
    };

    let text = match format {
        ReportFormat::Json => crate::report::format_json(&report)?,
        ReportFormat::Trace => crate::report::format_trace_json(&report)?,
        ReportFormat::Markdown => crate::report::format_markdown(&report),
        ReportFormat::Github => crate::report::format_github(&report),
        ReportFormat::Text => {
            let mut text = crate::report::format_text(&report);
            let mut machine = machine_snapshot(config);
            machine.gc = None;
            let diagnostics =
                term::named_sections(&[("Index diagnostics", machine_rows(&machine))]);
            if !diagnostics.is_empty() {
                text.push_str("\n\n");
                text.push_str(&diagnostics.join("\n"));
            }
            text
        }
    };
    let text = crate::report::scrub(&text, &terms);

    if let Some(path) = output {
        std::fs::write(&path, term::strip_sgr(&text))
            .with_context(|| format!("writing report to {}", path.display()))?;
        eprintln!("Report written to {}", path.display());
        if crate::machine::is_json() {
            #[derive(deser::Serialize)]
            struct Written {
                output: std::path::PathBuf,
                format: String,
            }
            crate::machine::emit(
                crate::machine::command(),
                Written {
                    output: path,
                    format: format.name().into(),
                },
                Vec::new(),
            )?;
        }
    } else if crate::machine::is_json() {
        // The standalone report already has schema_version. Keep all its
        // fields while adding the common command and success metadata.
        let mut body = deser_value::to_value(&report)?;
        body.as_map_mut()
            .context("report is not an object")?
            .remove("schema_version");
        crate::report::scrub_json(&mut body, &terms);
        crate::machine::emit(crate::machine::command(), body, Vec::new())?;
    } else {
        println!("{text}");
    }

    if let Err(e) = recorded {
        eprintln!("warning: this session was not recorded: {e:#}");
    }

    Ok(())
}

// ── kache why-miss ─────────────────────────────────────────────────────────

/// Format a SQLite datetime string (e.g. "2024-03-12 10:30:00") as a
/// human-readable relative time like "2h ago", "3d ago", etc.
fn format_relative_time(sqlite_dt: &str) -> String {
    let parsed = UtcTime::parse_sqlite(sqlite_dt);

    match parsed {
        Some(dt) => {
            let secs = UtcTime::now().saturating_duration_since(dt).as_secs();
            if secs < 60 {
                "just now".to_string()
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            }
        }
        None => sqlite_dt.to_string(),
    }
}

/// Diagnose cache misses for a specific crate by inspecting the event log
/// and the local store.
/// The store-failure banner for [`why_miss`], or `None` when the compile was
/// stored normally (kunobi-ninja/kache#629).
///
/// A compile that ran and could not be stored answers the question outright, and
/// it outranks key comparisons: nothing was written for a later build to match.
/// The terminal keeps the established `NOT CACHED` heading for this failure.
fn store_failure_banner(miss: &crate::events::BuildEvent) -> Option<String> {
    if miss.store_error.is_empty() {
        return None;
    }
    Some(format!(
        "  NOT CACHED: this compile ran and its outputs failed to store, so the crate\n  \
         misses on every build until the cause is fixed.\n    \
         reason: {}",
        miss.store_error
    ))
}

/// The exact-key lookup-rejection diagnosis for [`why_miss`].
///
/// Without this persisted pre-compile reason, a replacement entry written
/// under the same key makes the current store look like a successful cold
/// population. With other entries present, the old fallback was worse: it
/// claimed a key mismatch even though lookup found the exact key (#655).
fn lookup_rejection_banner(
    miss: &crate::events::BuildEvent,
    same_key_present: bool,
) -> Option<String> {
    if miss.lookup_rejection.is_empty() {
        return None;
    }
    let replacement = if same_key_present {
        "currently present under the same key"
    } else {
        "not currently present in the local store"
    };
    Some(format!(
        "  matching key was found but rejected before restore\n    \
         reason: {}\n    \
         replacement: {replacement}",
        miss.lookup_rejection
    ))
}

/// Conservative fallback for events written before lookup rejections were
/// persisted. Seeing the same non-empty key earlier proves that the miss was
/// not caused by different inputs, but old logs cannot distinguish cleanup,
/// eviction, corruption, or rejection.
fn legacy_repeated_same_key_banner(
    miss: &crate::events::BuildEvent,
    prior_same_key_miss: bool,
) -> Option<&'static str> {
    if miss.schema >= 15 || !miss.lookup_rejection.is_empty() || !prior_same_key_miss {
        return None;
    }
    Some(
        "  repeated miss for the same cache key\n    \
         this older event did not record whether the entry was absent, evicted, invalid, or rejected\n    \
         the miss was not caused by a cache-key change",
    )
}

pub fn why_miss(config: &Config, crate_name: &str, json: bool) -> Result<()> {
    let all_events = events::read_events(&config.event_log_path())?;
    let crate_events: Vec<_> = all_events
        .iter()
        .filter(|e| e.crate_name == crate_name)
        .collect();

    if crate_events.is_empty() {
        if json {
            #[derive(deser::Serialize)]
            struct Body<'a> {
                crate_name: &'a str,
                diagnosis: &'static str,
            }
            return crate::machine::emit(
                "explain",
                Body {
                    crate_name,
                    diagnosis: "no_events",
                },
                vec![crate::machine::NextAction {
                    argv: vec![
                        "cargo".into(),
                        "build".into(),
                        "-p".into(),
                        crate_name.into(),
                    ],
                    why: "no wrapper events yet; build the crate first".into(),
                }],
            );
        }
        println!("No events for `{crate_name}` yet. Build it first: `cargo build -p {crate_name}`");
        return Ok(());
    }

    let inventory = crate::store_view::read(config, true, "name")?;
    let all_entries = inventory.entries;
    let stored: Vec<_> = all_entries
        .iter()
        .filter(|e| e.crate_name == crate_name)
        .collect();

    // ── Find last entry miss ───────────────────────────────────────────
    let last_miss = crate_events.iter().rev().find(|e| {
        matches!(
            e.result,
            events::EventResult::Dup | events::EventResult::Miss
        )
    });
    // The newest event decides: a compile kache passed through after the
    // last miss means the crate is not cached now, whatever that miss stored.
    let latest_uncached = latest_uncached_compile(&crate_events);

    if last_miss.is_none() {
        if json {
            #[derive(deser::Serialize)]
            struct Body<'a> {
                crate_name: &'a str,
                diagnosis: &'static str,
                stored_entries: usize,
                store_dirs: Vec<&'a std::path::Path>,
                stores: &'a [crate::store_view::StoreSummary],
                latest_uncached_compile: Option<UncachedCompile<'a>>,
            }
            return crate::machine::emit(
                "explain",
                Body {
                    crate_name,
                    diagnosis: if latest_uncached.is_some() {
                        "not_cached"
                    } else if crate_events.iter().any(|event| {
                        matches!(event.result, events::EventResult::LocalHit)
                    }) {
                        "all_hits"
                    } else {
                        "no_cache_lookups"
                    },
                    stored_entries: stored.len(),
                    store_dirs: stored
                        .iter()
                        .flat_map(|e| e.store_dirs.iter().map(std::path::PathBuf::as_path))
                        .collect(),
                    stores: &inventory.stores,
                    latest_uncached_compile: latest_uncached.map(UncachedCompile::of),
                },
                Vec::new(),
            );
        }
        for warning in unavailable_store_warnings(&inventory.stores) {
            eprintln!("{warning}");
        }
        let lines = match latest_uncached {
            Some(latest) => uncached_compile_lines(crate_name, latest, None, &crate_events),
            None => no_miss_lines(crate_name, &stored, &crate_events),
        };
        for line in lines {
            println!("{line}");
        }
        return Ok(());
    }

    let miss = last_miss.unwrap();
    let last_miss_index = crate_events
        .iter()
        .position(|event| std::ptr::eq(*event, *miss))
        .expect("last miss came from crate_events");
    let prior_same_key_miss = last_miss_index > 0 && !miss.cache_key.is_empty() && {
        let previous = crate_events[last_miss_index - 1];
        previous.cache_key == miss.cache_key
            && matches!(
                previous.result,
                events::EventResult::Dup | events::EventResult::Miss
            )
    };
    let same_key_present = stored.iter().any(|e| e.cache_key == miss.cache_key);
    let miss_index = all_events
        .iter()
        .position(|event| std::ptr::eq(event, *miss));
    let chain = miss_index.and_then(|index| crate::miss_chain::analyze(&all_events, index));
    let mut diagnosis = MissDiagnosis::new(
        miss,
        prior_same_key_miss,
        stored.len(),
        same_key_present,
        chain,
        config.explain_miss,
    );
    diagnosis.checkout =
        miss_index.and_then(|index| crate::miss_chain::compare_checkout(&all_events, index));
    if let Some(index) = miss_index {
        diagnosis.hints =
            miss_diagnosis::env_hints(&crate::miss_chain::changed_env_inputs(&all_events, index));
    }
    if json {
        return why_miss_json(
            crate_name,
            miss,
            &stored,
            &inventory.stores,
            &diagnosis,
            latest_uncached,
        );
    }
    if let Some(latest) = latest_uncached {
        for line in uncached_compile_lines(crate_name, latest, Some(miss), &crate_events) {
            println!("{line}");
        }
        return Ok(());
    }

    for warning in unavailable_store_warnings(&inventory.stores) {
        eprintln!("{warning}");
    }

    let last_hit = crate_events.iter().rev().find(|e| {
        matches!(e.result, events::EventResult::LocalHit)
    });
    // Metadata of the missed key, when the compile was stored after it.
    let miss_meta = missed_entry(&stored, &miss.cache_key).and_then(read_entry_meta);

    let mut lines = vec![
        term::heading(format!("kache · explain · {crate_name}")),
        String::new(),
    ];
    lines.extend(term::named_sections(&[(
        "Last miss",
        last_miss_rows(
            miss,
            miss_meta.as_ref(),
            last_hit.copied(),
            config.key_salt.as_deref(),
        ),
    )]));
    lines.extend(section(
        "Diagnosis",
        miss_diagnosis_lines(miss, miss_meta.as_ref(), &stored, &diagnosis),
    ));
    lines.extend(section("Other build tree", checkout_lines(&diagnosis)));
    lines.extend(section(
        "Dependency cascade",
        extern_chain_lines(&diagnosis),
    ));
    lines.extend(section(
        "Hints",
        diagnosis
            .hints
            .iter()
            .map(|hint| format!("  {hint}"))
            .collect(),
    ));
    lines.extend(section(
        &format!("Stored entries ({})", term::count(stored.len() as u64)),
        stored_entry_lines(Some(&miss.cache_key), &stored),
    ));
    lines.extend(section("Recent events", recent_event_lines(&crate_events)));
    lines.push(String::new());
    lines.push(format!(
        "Every key input: `KACHE_LOG=trace cargo build -p {crate_name} 2>&1 | grep '\\[key:{crate_name}\\]'`"
    ));
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// A heading and its lines after a blank line, or nothing when `body` is
/// empty: the sections of `kache explain <crate>` after the first.
fn section(name: &str, body: Vec<String>) -> Vec<String> {
    if body.is_empty() {
        return body;
    }
    let mut lines = vec![String::new(), term::heading(name)];
    lines.extend(body);
    lines
}

/// One warning per store that could not be read, for stderr, so the report
/// still prints.
fn unavailable_store_warnings(stores: &[crate::store_view::StoreSummary]) -> Vec<String> {
    stores
        .iter()
        .filter_map(|store| {
            let error = store.error.as_ref()?;
            Some(format!(
                "warning: store {} is unavailable ({error})",
                term::home_path(&store.path)
            ))
        })
        .collect()
}

/// The stored entry under the missed key. A miss without a key matches none.
fn missed_entry<'a>(
    stored: &[&'a daemon::StatsEntry],
    key: &str,
) -> Option<&'a daemon::StatsEntry> {
    stored
        .iter()
        .copied()
        .find(|entry| !key.is_empty() && entry.cache_key == key)
}

fn read_entry_meta(entry: &daemon::StatsEntry) -> Option<crate::store::EntryMeta> {
    let content = std::fs::read_to_string(crate::store_view::entry_meta_path(entry)?).ok()?;
    deser_json::from_str(&content).ok()
}

/// The "Last miss" rows of `kache explain <crate>`: the miss, its key and
/// configuration, and the two things that change a key wholesale (a last
/// hit under another key, the key salt).
fn last_miss_rows(
    miss: &events::BuildEvent,
    meta: Option<&crate::store::EntryMeta>,
    last_hit: Option<&events::BuildEvent>,
    key_salt: Option<&str>,
) -> Vec<term::Row<'static>> {
    let mut rows = vec![
        (
            "Result",
            miss.result.to_string(),
            term::local_datetime(miss.ts),
        ),
        (
            "Key",
            if miss.cache_key.is_empty() {
                "none".to_string()
            } else {
                term::short_key(&miss.cache_key).to_string()
            },
            String::new(),
        ),
    ];
    if let Some(meta) = meta {
        for (label, value) in [
            ("Target", meta.target.clone()),
            ("Profile", meta.profile.clone()),
            ("Features", meta.features.join(", ")),
        ] {
            if !value.is_empty() {
                rows.push((label, value, String::new()));
            }
        }
    }
    if let Some(hit) = last_hit.filter(|hit| hit.cache_key != miss.cache_key && miss.ts > hit.ts) {
        rows.push((
            "Last hit",
            term::short_key(&hit.cache_key).to_string(),
            "under a different key".to_string(),
        ));
    }
    // The salt is folded into every key but isn't recorded per entry, so a
    // salt change can't be diffed against a stored entry: it shifts the key
    // wholesale and looks like a clean miss. A stray machine-global
    // `KACHE_KEY_SALT`, or a rotated salt, alone explains every miss here.
    if let Some(salt) = key_salt.filter(|salt| !salt.is_empty()) {
        rows.push((
            "Key salt",
            format!("{salt:?}"),
            "folded into every key; a change since the last hit alone explains the miss"
                .to_string(),
        ));
    }
    rows
}

/// The stored entries of the crate, newest first and at most ten. `missed`
/// marks the entry stored under the missed key.
fn stored_entry_lines(missed: Option<&str>, stored: &[&daemon::StatsEntry]) -> Vec<String> {
    if stored.is_empty() {
        return vec!["  none".to_string()];
    }
    let body: Vec<Vec<String>> = stored
        .iter()
        .rev()
        .take(10)
        .map(|entry| stored_entry_row(entry, read_entry_meta(entry).as_ref(), missed))
        .collect();
    let mut lines = term::table(
        &["KEY", "ACCESSED", "SIZE", "HITS", "STORE", "DETAILS"],
        &[
            term::Align::Left,
            term::Align::Left,
            term::Align::Right,
            term::Align::Right,
            term::Align::Left,
            term::Align::Left,
        ],
        &body,
    );
    let hidden = stored.len().saturating_sub(10);
    if hidden > 0 {
        lines.push(format!(
            "  and {}",
            term::counted(hidden as u64, "older entry", "older entries")
        ));
    }
    lines
}

/// One row of [`stored_entry_lines`]: key, last access, size, hits, store and
/// what sets the entry apart. Pure; `meta` is the entry's `meta.json`.
fn stored_entry_row(
    entry: &daemon::StatsEntry,
    meta: Option<&crate::store::EntryMeta>,
    missed: Option<&str>,
) -> Vec<String> {
    let mut details = Vec::new();
    if !entry.profile.is_empty() {
        details.push(format!("profile {}", entry.profile));
    }
    if !entry.crate_type.is_empty() {
        details.push(format!("type {}", entry.crate_type));
    }
    if let Some(meta) = meta {
        if !meta.target.is_empty() {
            details.push(format!("target {}", meta.target));
        }
        if !meta.features.is_empty() {
            details.push(format!("features [{}]", meta.features.join(", ")));
        }
    }
    if missed == Some(entry.cache_key.as_str()) {
        details.push("the missed key, stored after the compile".to_string());
    }
    vec![
        term::short_key(&entry.cache_key).to_string(),
        format_relative_time(&entry.last_accessed),
        term::bytes(entry.size),
        term::count(entry.hit_count),
        crate::store_view::entry_store_locations(entry),
        details.join(", "),
    ]
}

/// The crate's newest five events, oldest first, in local time.
fn recent_event_lines(crate_events: &[&events::BuildEvent]) -> Vec<String> {
    let body: Vec<Vec<String>> = crate_events
        .iter()
        .rev()
        .take(5)
        .rev()
        .map(|event| {
            vec![
                term::local_clock(event.ts),
                event.result.to_string(),
                term::short_key(&event.cache_key).to_string(),
                term::duration_ms(event.elapsed_ms),
                term::bytes(event.size),
            ]
        })
        .collect();
    term::table(
        &["TIME", "RESULT", "KEY", "ELAPSED", "SIZE"],
        &[
            term::Align::Left,
            term::Align::Left,
            term::Align::Left,
            term::Align::Right,
            term::Align::Right,
        ],
        &body,
    )
}

/// The comparison with another checkout, when the miss's own build tree had
/// no earlier build of the crate to compare with.
fn checkout_lines(diagnosis: &MissDiagnosis) -> Vec<String> {
    let Some(checkout) = &diagnosis.checkout else {
        return Vec::new();
    };
    // "Build tree", not "checkout": a registry crate built with the same
    // features and profile has one unit id in unrelated projects too.
    let mut lines = vec![
        format!("  no earlier build of this crate in {}", checkout.root),
        format!(
            "  compared with the same unit built in: {}",
            checkout.baseline_root
        ),
    ];
    lines.extend(
        checkout_comparison_lines(checkout)
            .into_iter()
            .map(|line| format!("  {line}")),
    );
    lines
}

fn checkout_comparison_lines(checkout: &crate::miss_chain::CheckoutComparison) -> Vec<String> {
    use crate::miss_chain::CheckoutVerdict;
    let dependencies = checkout
        .dependencies
        .iter()
        .map(|d| d.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = Vec::new();
    match checkout.verdict {
        CheckoutVerdict::PathOnly => lines.push(
            "same cache key in both build trees: no key input differs, so the key is not why \
             this missed (the entry was evicted, not stored yet, or failed to store)"
                .to_string(),
        ),
        CheckoutVerdict::OwnInputs => {
            lines.push(format!("own inputs differ: {}", checkout.groups.join(", ")));
            if !checkout.dependencies.is_empty() {
                lines.push(format!("dependencies differ: {dependencies}"));
            }
            // `remap` is the one group that holds raw checkout paths on
            // purpose, so the normalization note would be wrong on its own.
            if checkout.groups.iter().any(|g| g != "remap") {
                lines.push(
                    "(input groups hash path-normalized inputs, so this is a real input \
                     difference or a path that escaped normalization)"
                        .to_string(),
                );
            }
            if checkout.groups.iter().any(|g| g == "remap") {
                lines.push(
                    "(remap: path remapping differs between the builds, or it is off and the key \
                     holds the checkout path)"
                        .to_string(),
                );
            }
        }
        CheckoutVerdict::Dependencies => lines.push(format!(
            "own inputs match after path normalization; dependencies differ: {dependencies}"
        )),
        CheckoutVerdict::Untraced => lines.push(
            "no input group or dependency differs, but the keys do (key salt or extra inputs?)"
                .to_string(),
        ),
    }
    lines
}

/// The `extern:` cascade for a miss, when one is recorded (#609).
///
/// Empty when the miss is not downstream of a dependency change. When the
/// digests were never recorded, says how to turn them on rather than staying
/// silent about a diagnosis it could have given.
fn extern_chain_lines(diagnosis: &MissDiagnosis) -> Vec<String> {
    let Some(chain) = &diagnosis.dependency_chain else {
        if diagnosis.dependency_recording_missing {
            return vec![
                "  not analyzed: no per-dependency digests recorded".to_string(),
                "  set explain_miss = true under [cache], then rebuild".to_string(),
            ];
        }
        return Vec::new();
    };

    let direct: Vec<&str> = chain.direct.iter().map(|d| d.name.as_str()).collect();
    let mut lines = vec![format!(
        "  this miss is downstream of {} that changed ({})",
        if direct.len() == 1 {
            "a dependency".to_string()
        } else {
            format!("{} dependencies", direct.len())
        },
        direct.join(", ")
    )];

    for root in &chain.roots {
        let via = if root.branches > 1 {
            format!(" (reached by {} branches)", root.branches)
        } else {
            String::new()
        };
        let name = &root.crate_name;
        lines.push(match &root.kind {
            crate::miss_chain::RootKind::Groups(groups) => format!(
                "  root: {name}{via}, its own inputs changed: {}",
                groups.join(", ")
            ),
            crate::miss_chain::RootKind::PathOnly => format!(
                "  root: {name}{via}, same key in both build trees but its artifact differs \
                 (its output is not reproducible across build trees)"
            ),
            crate::miss_chain::RootKind::NothingRecorded => format!(
                "  root: {name}{via}, dependencies stable and no traced input group changed \
                 (key salt or extra inputs?)"
            ),
            // Everything below is an unresolved endpoint, not a cause. Worded
            // so it can't be read as "this crate is why you missed".
            crate::miss_chain::RootKind::NoMissRecorded => format!(
                "  unresolved: {name}{via}, artifact differs but it has no compile recorded in \
                 this event window"
            ),
            crate::miss_chain::RootKind::NoBaseline => {
                format!("  unresolved: {name}{via}, nothing earlier to compare it against")
            }
            crate::miss_chain::RootKind::NoDiffableHistory => format!(
                "  unresolved: {name}{via}, its own dependency history is not comparable, so the \
                 cascade may continue below it"
            ),
            crate::miss_chain::RootKind::LimitReached => format!(
                "  unresolved: {name}{via}, still descending when the walk limit was reached"
            ),
        });
        if root.path.len() > 1 {
            let mut path: Vec<&str> = root.path.iter().map(|h| h.crate_name.as_str()).collect();
            path.push(name.as_str());
            lines.push(format!("    via: {}", path.join(" <- ")));
        }
        for group in &root.passthroughs {
            lines.push(format!(
                "    {}x uncached compile in {name}: {}",
                group.count, group.reason
            ));
        }
        if !root.passthroughs.is_empty() {
            lines.push(
                "    (uncached compiles make the artifact vary per checkout; attributed by \
                 package directory)"
                    .to_string(),
            );
        }
    }

    if !chain.has_resolved_root() {
        lines.push(
            "  note: no endpoint could be resolved; the recorded history does not explain \
             this miss"
                .to_string(),
        );
    }
    if let Some(reason) = chain.truncated {
        lines.push(format!("  note: the walk stopped early: {reason}"));
    }
    lines
}

/// The "Diagnosis" lines of `kache explain <crate>`. A failed store leads:
/// nothing was written for a later build to match, so it outranks any key
/// comparison (kunobi-ninja/kache#629).
fn miss_diagnosis_lines(
    miss: &events::BuildEvent,
    miss_meta: Option<&crate::store::EntryMeta>,
    stored: &[&daemon::StatsEntry],
    diagnosis: &MissDiagnosis,
) -> Vec<String> {
    let mut lines: Vec<String> = store_failure_banner(miss).into_iter().collect();
    match diagnosis.cause {
        Cause::NotCached => {}
        Cause::LookupRejected => {
            lines.extend(lookup_rejection_banner(miss, diagnosis.same_key_present));
        }
        Cause::RepeatedSameKey => {
            lines.extend(legacy_repeated_same_key_banner(miss, true).map(str::to_string));
        }
        Cause::NeverCached => lines.push("  never cached: first build of this crate".to_string()),
        Cause::FirstBuildNowCached => {
            lines.push("  first build with these inputs: the entry is now cached".to_string());
        }
        Cause::KeyMismatch => {
            lines.push(format!(
                "  key mismatch: key {} differs from {}",
                term::short_key(&miss.cache_key),
                term::counted(
                    diagnosis.other_entries as u64,
                    "other stored entry",
                    "other stored entries"
                ),
            ));
            if let Some(miss_meta) = miss_meta {
                let others: Vec<(String, crate::store::EntryMeta)> = stored
                    .iter()
                    .filter(|e| e.cache_key != miss.cache_key)
                    .filter_map(|e| {
                        Some((
                            term::short_key(&e.cache_key).to_string(),
                            read_entry_meta(e)?,
                        ))
                    })
                    .collect();
                lines.extend(difference_lines(miss_meta, &others));
            }
        }
    }
    lines
}

/// What differs between the missed key's configuration and the other stored
/// entries': at most five lines, then how many more there were. Pure.
fn difference_lines(
    miss_meta: &crate::store::EntryMeta,
    others: &[(String, crate::store::EntryMeta)],
) -> Vec<String> {
    let (diffs, extra) = why_miss_diff_messages(
        miss_meta,
        others.iter().map(|(key, meta)| (key.as_str(), meta)),
        5,
    );
    let mut lines: Vec<String> = diffs.iter().map(|diff| format!("    - {diff}")).collect();
    if extra > 0 {
        lines.push(format!("    and {extra} more"));
    }
    lines
}

fn why_miss_json(
    crate_name: &str,
    miss: &events::BuildEvent,
    stored: &[&daemon::StatsEntry],
    stores: &[crate::store_view::StoreSummary],
    diagnosis: &MissDiagnosis,
    latest_uncached: Option<&events::BuildEvent>,
) -> Result<()> {
    #[derive(deser::Serialize)]
    struct Body<'a> {
        crate_name: &'a str,
        diagnosis: Cause,
        last_result: String,
        cache_key: &'a str,
        store_error: &'a str,
        lookup_rejection: &'a str,
        stored_entries: usize,
        store_dirs: Vec<&'a std::path::Path>,
        stores: &'a [crate::store_view::StoreSummary],
        dependency_chain: &'a Option<crate::miss_chain::Chain>,
        dependency_recording_missing: bool,
        checkout_comparison: &'a Option<crate::miss_chain::CheckoutComparison>,
        hints: &'a [String],
        latest_uncached_compile: Option<UncachedCompile<'a>>,
    }

    crate::machine::emit(
        "explain",
        Body {
            crate_name,
            diagnosis: diagnosis.cause,
            last_result: miss.result.to_string(),
            cache_key: &miss.cache_key,
            store_error: &miss.store_error,
            lookup_rejection: &miss.lookup_rejection,
            stored_entries: stored.len(),
            store_dirs: stored
                .iter()
                .filter(|e| e.cache_key == miss.cache_key)
                .flat_map(|e| e.store_dirs.iter().map(std::path::PathBuf::as_path))
                .collect(),
            stores,
            dependency_chain: &diagnosis.dependency_chain,
            dependency_recording_missing: diagnosis.dependency_recording_missing,
            checkout_comparison: &diagnosis.checkout,
            hints: &diagnosis.hints,
            latest_uncached_compile: latest_uncached.map(UncachedCompile::of),
        },
        Vec::new(),
    )
}

/// A compile kache ran without caching, in `kache --json explain <crate>`.
#[derive(Debug, deser::Serialize)]
struct UncachedCompile<'a> {
    ts: String,
    /// `passthrough`, or `skipped` for a compile whose store kache declined.
    result: String,
    cache_key: &'a str,
    reason: &'a str,
    why: String,
}

impl<'a> UncachedCompile<'a> {
    fn of(event: &'a events::BuildEvent) -> Self {
        let reason = if matches!(event.result, events::EventResult::Skipped) {
            &event.skip_reason
        } else {
            &event.passthrough_reason
        };
        Self {
            ts: event.ts.to_rfc3339(),
            result: event.result.to_string(),
            cache_key: &event.cache_key,
            reason,
            why: crate::report::uncached_compile_words(event),
        }
    }
}

/// The crate's newest compile when kache ran it without storing it and no
/// lookup came after it. Probes, errors, fallback-wrapper runs and skips
/// without a recorded compile are not known compiles, so they never decide.
fn latest_uncached_compile<'a>(
    crate_events: &[&'a events::BuildEvent],
) -> Option<&'a events::BuildEvent> {
    use events::EventResult;
    crate_events
        .iter()
        .rev()
        .copied()
        .find(|event| {
            crate::report::ran_compiler_uncached(event)
                || matches!(
                    event.result,
                    EventResult::LocalHit | EventResult::Miss | EventResult::Dup
                )
        })
        .filter(|event| crate::report::ran_compiler_uncached(event))
}

/// The first line of `kache explain <crate>` for a crate with no miss and no
/// uncached compile since its last lookup.
fn no_miss_headline(crate_name: &str, crate_events: &[&events::BuildEvent]) -> String {
    use events::EventResult;
    let hit = |event: &&events::BuildEvent| {
        matches!(event.result, EventResult::LocalHit)
    };
    if crate_events.iter().all(hit) {
        format!("No misses for `{crate_name}`: all events are hits.")
    } else if crate_events.iter().any(hit) {
        format!("No misses for `{crate_name}`: its latest lookup hit the cache.")
    } else {
        format!("No misses or hits for `{crate_name}`.")
    }
}

/// `kache explain <crate>` for a crate with no miss and no uncached compile
/// since its last lookup.
fn no_miss_lines(
    crate_name: &str,
    stored: &[&daemon::StatsEntry],
    crate_events: &[&events::BuildEvent],
) -> Vec<String> {
    let mut lines = vec![
        term::heading(format!("kache · explain · {crate_name}")),
        String::new(),
        no_miss_headline(crate_name, crate_events),
    ];
    lines.extend(section(
        &format!("Stored entries ({})", term::count(stored.len() as u64)),
        stored_entry_lines(None, stored),
    ));
    lines.extend(section("Recent events", recent_event_lines(crate_events)));
    lines
}

/// `kache explain <crate>` when the crate's newest compile was not cached:
/// that compile, why it was not cached, and the miss before it if any.
fn uncached_compile_lines(
    crate_name: &str,
    latest: &events::BuildEvent,
    earlier_miss: Option<&events::BuildEvent>,
    crate_events: &[&events::BuildEvent],
) -> Vec<String> {
    let mut rows = vec![
        ("When", term::local_datetime(latest.ts), String::new()),
        (
            "Key",
            term::short_key(&latest.cache_key).to_string(),
            String::new(),
        ),
        (
            "Why",
            crate::report::uncached_compile_words(latest),
            String::new(),
        ),
    ];
    if let Some(miss) = earlier_miss {
        rows.push((
            "Earlier",
            miss.result.to_string(),
            format!(
                "{}, key {}",
                term::local_datetime(miss.ts),
                term::short_key(&miss.cache_key)
            ),
        ));
    }
    let mut lines = vec![
        term::heading(format!("kache · explain · {crate_name}")),
        String::new(),
        format!("`{crate_name}` was compiled, not cached."),
        String::new(),
    ];
    lines.extend(term::named_sections(&[("Latest compile", rows)]));
    lines.extend(section("Recent events", recent_event_lines(crate_events)));
    lines
}

/// What differs between the missed key's configuration and each other
/// stored entry's (target, profile, features, crate types), at most `limit`
/// lines and how many more there were.
fn why_miss_diff_messages<'a, I>(
    miss_meta: &crate::store::EntryMeta,
    other_entries: I,
    limit: usize,
) -> (Vec<String>, usize)
where
    I: IntoIterator<Item = (&'a str, &'a crate::store::EntryMeta)>,
{
    // Every same-config entry says the same thing, so they share one line.
    let mut diffs: Vec<(Option<&'static str>, String)> = Vec::new();
    let mut push = |same_config: Option<&'static str>, message: String| {
        diffs.push((same_config, message));
    };
    for (ek, other) in other_entries {
        if miss_meta.target != other.target {
            push(
                None,
                format!(
                    "different target vs {ek}: \"{}\" vs \"{}\"",
                    miss_meta.target, other.target
                ),
            );
        }
        if miss_meta.profile != other.profile {
            push(
                None,
                format!(
                    "different profile vs {ek}: \"{}\" vs \"{}\"",
                    miss_meta.profile, other.profile
                ),
            );
        }
        if miss_meta.features != other.features {
            let miss_feats = if miss_meta.features.is_empty() {
                "(none)".to_string()
            } else {
                miss_meta.features.join(", ")
            };
            let other_feats = if other.features.is_empty() {
                "(none)".to_string()
            } else {
                other.features.join(", ")
            };
            push(
                None,
                format!("different features vs {ek}: [{miss_feats}] vs [{other_feats}]"),
            );
        }
        if miss_meta.crate_types != other.crate_types {
            push(
                None,
                format!(
                    "different crate types vs {ek}: {:?} vs {:?}",
                    miss_meta.crate_types, other.crate_types
                ),
            );
        }

        if miss_meta.target == other.target
            && miss_meta.profile == other.profile
            && miss_meta.features == other.features
            && miss_meta.crate_types == other.crate_types
        {
            push(
                Some("same config"),
                format!("same config as {ek}: likely a source, dependency or rustc version change"),
            );
        }
    }

    let mut unique_diffs: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (same_config, diff) in diffs {
        if seen.insert(same_config.map_or_else(|| diff.clone(), str::to_string)) {
            unique_diffs.push(diff);
        }
    }

    let extra = unique_diffs.len().saturating_sub(limit);
    (unique_diffs.into_iter().take(limit).collect(), extra)
}

/// ` (<size>)` after a count of held entries, when the sweep measured it.
fn held_bytes_note(bytes_held: u64) -> String {
    if bytes_held == 0 {
        String::new()
    } else {
        format!(" ({})", kache_core::size::format_bytes(bytes_held))
    }
}

/// How an eviction sweep went, phrased so "0" is never left unexplained.
///
/// `kache gc` used to print a bare `evicted 0 entries` next to a store sitting
/// at 912% of its limit. That is correct behaviour reported terribly: every
/// candidate was within the idle grace, so the sweep deliberately left them. A
/// user who reads "0" beside "912%" concludes GC is broken and stops trying —
/// which is plausibly how #497 became a 113 GB bug report rather than a
/// self-service fix (kunobi-ninja/kache#509).
///
/// Pure so the phrasing is unit-testable without running a sweep.
pub(crate) fn describe_eviction(stats: &crate::store::GcStats, over_limit: bool) -> String {
    let grace = term::duration_ms(crate::store::EVICTION_IDLE_GRACE.as_millis() as u64);
    let plural = |n: usize| if n == 1 { "entry" } else { "entries" };
    let pinned_note = |n: usize| {
        format!(
            "\n  {n} more {} accessed within the last {grace} or awaiting a durable \
             remote upload and left in place; run it again once builds and uploads \
             are idle.",
            plural(n),
        )
    };

    if stats.entries_evicted > 0 {
        let mut msg = format!(
            "dropped {} {} from the store ({}).",
            stats.entries_evicted,
            plural(stats.entries_evicted),
            kache_core::size::format_bytes(stats.bytes_freed)
        );
        msg.push_str(&format!(
            "\n  {} became free on disk.",
            kache_core::size::format_bytes(stats.disk_bytes_reclaimed)
        ));
        let leftover = stats.bytes_freed.saturating_sub(stats.disk_bytes_reclaimed);
        if leftover > 0 {
            msg.push_str(&format!(
                "\n  {} is still held by clones in build outputs or by filesystem snapshots.",
                kache_core::size::format_bytes(leftover)
            ));
        }
        if stats.entries_pinned > 0 {
            msg.push_str(&pinned_note(stats.entries_pinned));
        }
        if stats.entries_unreclaimable > 0 {
            msg.push_str(&format!(
                "\n  {} {}{} left in place because clones still hold their blocks. \
                 Remove unneeded build outputs, then run it again.",
                stats.entries_unreclaimable,
                plural(stats.entries_unreclaimable),
                held_bytes_note(stats.bytes_held),
            ));
        }
        return msg;
    }

    if stats.entries_unreclaimable > 0 {
        let mut msg = format!(
            "nothing reclaimable on disk.\n  {} {}{} cloned into build outputs \
             (same bytes as target/, not extra).\n  Remove unneeded build outputs, then run it again.",
            stats.entries_unreclaimable,
            plural(stats.entries_unreclaimable),
            held_bytes_note(stats.bytes_held),
        );
        if stats.entries_pinned > 0 {
            msg.push_str(&pinned_note(stats.entries_pinned));
        }
        return msg;
    }

    // Nothing evicted. Say why, because this is the case that reads as a bug.
    if stats.entries_pinned > 0 {
        return format!(
            "evicted 0 entries.\n  {} {} were selected but accessed within the last \
             {grace} or are awaiting a durable remote upload, so they were left in \
             place. Run it again once builds and uploads are idle.",
            stats.entries_pinned,
            plural(stats.entries_pinned),
        );
    }
    if over_limit {
        return format!(
            "evicted 0 entries.\n  The store is still over its limit but nothing was \
             eligible. Entries accessed within the last {grace} are never evicted; \
             if this persists with no builds running, `kache doctor --verify` will \
             report entries that cannot be removed."
        );
    }
    "evicted 0 entries (nothing to evict).".to_string()
}

/// Is the store over its configured budget after a sweep?
///
/// Pure, and split out of the two `gc` paths for the same reason
/// [`describe_eviction`] is: it decides which of the two "evicted 0" messages a
/// user sees, and that decision is worth testing without running a sweep. A
/// store whose size cannot be read is reported as within budget — the same
/// direction the callers already took, since claiming "over limit" on missing
/// data would send the user chasing an eviction problem they may not have.
pub(crate) fn store_over_limit(total_size: Option<u64>, max_size: u64) -> bool {
    total_size.is_some_and(|size| size > max_size)
}

// ── Project stats ──────────────────────────────────────────────────────────

/// List all cached entries, or show details for a specific crate.
pub fn list(
    config: &Config,
    crate_name: Option<&str>,
    sort_by: &str,
    no_pager: bool,
    json: bool,
) -> Result<()> {
    let inventory = crate::store_view::read(config, true, sort_by)?;

    if json {
        #[derive(deser::Serialize)]
        struct EntryBody<'a> {
            cache_key: &'a str,
            store_dirs: &'a [std::path::PathBuf],
            crate_name: &'a str,
            crate_type: &'a str,
            profile: &'a str,
            size: u64,
            hits: u64,
            created_at: &'a str,
            last_accessed: &'a str,
        }
        let entries = inventory.entries;
        let matching: Vec<&daemon::StatsEntry> = if let Some(name) = crate_name {
            entries
                .iter()
                .filter(|e| crate_name_matches(name, &e.crate_name))
                .collect()
        } else {
            entries.iter().collect()
        };
        let body: Vec<EntryBody> = matching
            .iter()
            .map(|e| EntryBody {
                cache_key: &e.cache_key,
                store_dirs: &e.store_dirs,
                crate_name: &e.crate_name,
                crate_type: &e.crate_type,
                profile: &e.profile,
                size: e.size,
                hits: e.hit_count,
                created_at: &e.created_at,
                last_accessed: &e.last_accessed,
            })
            .collect();
        #[derive(deser::Serialize)]
        struct Body<T> {
            entries: Vec<T>,
            stores: Vec<crate::store_view::StoreSummary>,
        }
        return crate::machine::emit(
            "list",
            Body {
                entries: body,
                stores: inventory.stores,
            },
            Vec::new(),
        );
    }

    for store in &inventory.stores {
        if let Some(error) = &store.error {
            eprintln!("Store {}: unavailable ({error})", store.path.display());
        }
    }
    if let Some(name) = crate_name {
        // Detail view for a specific crate
        let entries = inventory.entries;
        let matching: Vec<_> = entries.iter().filter(|e| e.crate_name == name).collect();

        if matching.is_empty() {
            println!("No cached entries for '{name}'.");
            return Ok(());
        }

        let mut lines = Vec::new();
        for entry in &matching {
            lines.extend(entry_detail_lines(entry));

            if let Some(meta_path) = crate::store_view::entry_meta_path(entry)
                && let Ok(content) = std::fs::read_to_string(&meta_path)
                && let Ok(meta) = deser_json::from_str::<crate::store::EntryMeta>(&content)
            {
                push_nonempty_detail(&mut lines, "  Features: ", &meta.features.join(", "));
                push_nonempty_detail(&mut lines, "  Target:   ", &meta.target);
                lines.push("  Files:".to_string());
                for file in &meta.files {
                    lines.push(format!("    {} ({})", file.name, kache_core::size::format_bytes(file.size)));
                }
            }
            lines.push(String::new());
        }
        write_paged(&lines, no_pager);
    } else {
        // Summary view of all entries
        let entries = inventory.entries;

        if entries.is_empty() {
            println!("No cached entries.");
            return Ok(());
        }

        let mut lines = vec![term::heading("kache · entries"), String::new()];
        let mut body = Vec::new();
        for entry in &entries {
            let crate_type = if entry.crate_type.is_empty() {
                "-"
            } else {
                &entry.crate_type
            };
            let profile = if entry.profile.is_empty() {
                "-"
            } else {
                &entry.profile
            };
            body.push(vec![
                entry.crate_name.clone(),
                crate_type.into(),
                profile.into(),
                term::bytes(entry.size),
                term::count(entry.hit_count),
                local_store_date(&entry.created_at),
                local_store_date(&entry.last_accessed),
                crate::store_view::entry_store_locations(entry),
            ]);
        }
        use term::Align::{Left, Right};
        lines.extend(term::table(
            &[
                "CRATE", "TYPE", "PROFILE", "SIZE", "HITS", "CREATED", "ACCESSED", "STORES",
            ],
            &[Left, Left, Left, Right, Right, Left, Left, Left],
            &body,
        ));
        lines.push(String::new());
        lines.push(term::paint(
            term::counted(entries.len() as u64, "entry", "entries"),
            term::Style::Muted,
        ));
        write_paged(&lines, no_pager);
    }

    Ok(())
}

fn crate_name_matches(requested: &str, actual: &str) -> bool {
    requested == actual
}

/// The head of `kache list <crate>` for one entry, times in local time.
fn entry_detail_lines(entry: &daemon::StatsEntry) -> Vec<String> {
    let mut lines = vec![
        format!("Cache key: {}", term::short_key(&entry.cache_key)),
        format!("  Crate:    {}", entry.crate_name),
        format!(
            "  Stores:   {}",
            crate::store_view::entry_store_locations(entry)
        ),
    ];
    push_nonempty_detail(&mut lines, "  Type:     ", &entry.crate_type);
    push_nonempty_detail(&mut lines, "  Profile:  ", &entry.profile);
    lines.push(format!("  Size:     {}", kache_core::size::format_bytes(entry.size)));
    lines.push(format!("  Hits:     {}", entry.hit_count));
    lines.push(format!(
        "  Created:  {}",
        term::local_store_time(&entry.created_at)
    ));
    lines.push(format!(
        "  Accessed: {}",
        term::local_store_time(&entry.last_accessed)
    ));
    lines
}

/// The local date of a store timestamp, `2026-10-14`.
fn local_store_date(utc: &str) -> String {
    term::local_store_time(utc).chars().take(10).collect()
}

fn push_nonempty_detail(lines: &mut Vec<String>, prefix: &str, value: &str) {
    if !value.is_empty() {
        lines.push(format!("{prefix}{value}"));
    }
}

/// Resolve the pager to a direct process argv. Quotes group whitespace but are
/// not shell syntax: there is no expansion, operator handling, or interpolation.
fn resolve_pager_argv(
    no_pager: bool,
    stdout_is_terminal: bool,
    kache_pager: Option<&str>,
    pager: Option<&str>,
) -> Option<Vec<String>> {
    if no_pager || !stdout_is_terminal {
        return None;
    }

    let command = match kache_pager {
        Some(command) => command,
        None => pager.unwrap_or("less -FRX"),
    };
    let argv = parse_pager_argv(command)?;
    if argv.first().is_none_or(|program| program.is_empty())
        || (argv.len() == 1 && argv[0] == "cat")
    {
        None
    } else {
        Some(argv)
    }
}

/// Split a pager command into argv without invoking a shell. Single and double
/// quotes may group whitespace and are removed; every other character remains
/// literal. An unmatched quote makes the command invalid.
fn parse_pager_argv(command: &str) -> Option<Vec<String>> {
    let mut argv = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut word_started = false;

    for character in command.chars() {
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                word.push(character);
            }
            continue;
        }

        match character {
            '\'' | '"' => {
                quote = Some(character);
                word_started = true;
            }
            character if character.is_whitespace() => {
                if word_started {
                    argv.push(std::mem::take(&mut word));
                    word_started = false;
                }
            }
            character => {
                word.push(character);
                word_started = true;
            }
        }
    }

    if quote.is_some() {
        return None;
    }
    if word_started {
        argv.push(word);
    }
    Some(argv)
}

fn write_pager_lines<W: std::io::Write>(writer: &mut W, lines: &[String]) -> bool {
    for line in lines {
        if writer.write_all(line.as_bytes()).is_err() {
            return false;
        }
        if writer.write_all(b"\n").is_err() {
            return false;
        }
    }
    true
}

/// Write output lines to a pager when stdout is a terminal, else plain stdout.
/// `KACHE_PAGER` > `$PAGER` > platform default; `cat` or empty disables. Invalid
/// commands and spawn failures fall back to plain output. An early pager exit
/// stops further delivery without failing the command or reprinting the listing.
fn write_paged(lines: &[String], no_pager: bool) {
    let plain = || {
        for line in lines {
            println!("{line}");
        }
    };

    let kache_pager = std::env::var_os("KACHE_PAGER");
    let pager = std::env::var_os("PAGER");
    let Some(argv) = resolve_pager_argv(
        no_pager,
        std::io::stdout().is_terminal(),
        kache_pager
            .as_deref()
            .map(|value| value.to_str().unwrap_or("")),
        pager.as_deref().map(|value| value.to_str().unwrap_or("")),
    ) else {
        plain();
        return;
    };

    let mut argv = argv.into_iter();
    let Some(program) = argv.next() else {
        plain();
        return;
    };
    let mut child = match std::process::Command::new(program)
        .args(argv)
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            plain();
            return;
        }
    };

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = write_pager_lines(stdin, lines);
    }
    drop(child.stdin.take());
    let _ = child.wait();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcMode {
    Cli,
    /// A `kache gc --json` the daemon could not take: requested, but quiet.
    CliQuiet,
    Background,
}

impl GcMode {
    /// The detached worker a build spawned is automatic; a `kache gc` the
    /// user typed is not, and may evict what the remote just delivered.
    pub fn sweep_origin(self) -> crate::store::SweepOrigin {
        match self {
            GcMode::Cli | GcMode::CliQuiet => crate::store::SweepOrigin::Requested,
            GcMode::Background => crate::store::SweepOrigin::Automatic,
        }
    }

    pub fn from_env() -> Self {
        if std::env::var_os("KACHE_AUTO_GC_WORKER").is_some() {
            GcMode::Background
        } else {
            GcMode::Cli
        }
    }
}

/// How many entries one durability sweep flushes before yielding. The daemon
/// sweeps again two seconds later, so a long backlog drains in batches rather
/// than in one hold of the store.
pub(crate) const DURABILITY_FLUSH_BATCH: usize = 64;

/// Run garbage collection locally under `gc.lock`.
pub fn run_gc_local(config: &Config, mode: GcMode) -> Result<crate::store::GcStats> {
    let verbose = mode == GcMode::Cli;
    let store = Store::open(config)?;
    let _gc_lock = match store.try_gc_lock()? {
        Some(lock) => lock,
        None => {
            if verbose {
                println!("Another GC is already running; skipping.");
            }
            return Ok(skipped_gc_stats());
        }
    };
    // Entries stored without an fsync reach disk before anything is judged
    // by age or size; a flusher that never ran is caught up here.
    match store.flush_durability(usize::MAX) {
        Ok(0) => {}
        Ok(flushed) => log::info!("flushed {flushed} entries pending durability"),
        Err(error) => log::warn!("durability flush before gc failed: {error:#}"),
    }
    let mut combined = crate::store::GcStats::default();
    let started = std::time::Instant::now();

    if verbose {
        print!("Backfilling content hashes...");
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }
    crate::wrapper::prune_session_markers(
        config,
        crate::wrapper::SESSION_MARKER_RETENTION,
        std::time::SystemTime::now(),
    );
    crate::build_script::sweep_hermetic_out_dirs_for_gc(&config.cache_dir);
    let backfilled = store.backfill_content_hashes().unwrap_or(0);
    if verbose {
        if backfilled > 0 {
            println!(
                " {} updated.",
                term::counted(backfilled as u64, "entry", "entries")
            );
        } else {
            println!(" up to date.");
        }
    }

    // Rebuild cost for pre-#594 entries; same sweep, same convergence.
    if verbose {
        print!("Backfilling compile times...");
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }
    let costs = store.backfill_compile_times();
    if verbose {
        println!("{}", compile_time_backfill_message(costs));
    }

    // Entry→blob rows for pre-#608 entries; same sweep, same convergence.
    if verbose {
        print!("Backfilling entry blob maps...");
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }
    let mapped = store.backfill_entry_blobs().unwrap_or(0);
    if verbose {
        if mapped > 0 {
            println!(
                " {} updated.",
                term::counted(mapped as u64, "entry", "entries")
            );
        } else {
            println!(" up to date.");
        }
    }

    // Automatic age retention runs first so later pressure policies observe
    // the reduced physical store. `0` keeps it disabled.
    if config.gc_max_age_hours > 0 {
        if verbose {
            print!(
                "Evicting entries older than {}h...",
                config.gc_max_age_hours
            );
            std::io::Write::flush(&mut std::io::stdout()).ok();
        }
        let age_stats = store.evict_older_than(config.gc_max_age_hours)?;
        add_gc_stats(&mut combined, &age_stats);
        if verbose {
            if age_stats.entries_evicted > 0 {
                println!(
                    " dropped {} from the store ({}); {} became free on disk.",
                    term::counted(age_stats.entries_evicted as u64, "entry", "entries"),
                    crate::report::format_bytes(age_stats.bytes_freed),
                    crate::report::format_bytes(age_stats.disk_bytes_reclaimed),
                );
            } else {
                println!(" none old enough.");
            }
        }
    }

    if verbose {
        print!("Deduplicating entries...");
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }
    let dedup_stats = store
        .evict_duplicate_entries_for(mode.sweep_origin())
        .unwrap_or_default();
    add_gc_stats(&mut combined, &dedup_stats);
    if verbose {
        if dedup_stats.entries_evicted > 0 {
            println!(
                " removed {}.",
                term::counted(
                    dedup_stats.entries_evicted as u64,
                    "duplicate",
                    "duplicates"
                )
            );
        } else {
            println!(" no duplicates found.");
        }
    }

    if verbose {
        print!("Running eviction...");
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }
    let evict_stats = store.evict_for(mode.sweep_origin())?;
    add_gc_stats(&mut combined, &evict_stats);
    if verbose {
        let over_limit = store_over_limit(
            store
                .physical_size()
                .ok()
                .map(|size| size.saturating_sub(evict_stats.bytes_held)),
            config.max_size,
        );
        println!(" {}", describe_eviction(&evict_stats, over_limit));
    }

    // Key lock files and input predictions grow with every distinct key and
    // eviction removes neither (#1126).
    let housekeeping = store.sweep_housekeeping();
    combined.housekeeping = Some(housekeeping);
    if verbose {
        println!(
            "Housekeeping: removed {} ({} remain), {}, {}.",
            term::counted(
                housekeeping.key_locks_removed as u64,
                "stale key lock",
                "stale key locks"
            ),
            housekeeping.key_locks_remaining,
            term::counted(
                housekeeping.predictions_pruned as u64,
                "unused input prediction",
                "unused input predictions"
            ),
            term::counted(
                housekeeping.file_hashes_pruned as u64,
                "old file hash",
                "old file hashes"
            ),
        );
    }

    combined.duration_ms = started.elapsed().as_millis() as u64;
    // Still under gc.lock, so the record cannot race another driver.
    // The auto-GC worker used to discard this outcome entirely. A failed write
    // must not fail the sweep it describes.
    let source = if mode == GcMode::Background {
        "auto"
    } else {
        "manual"
    };
    if let Err(e) = crate::report::record_gc_run(config, source, mode.sweep_origin(), &combined) {
        log::debug!(
            "gc: could not record {}: {e:#}",
            crate::report::GC_STATS_FILE
        );
    }

    Ok(combined)
}

/// [`run_gc_local`] on the main store, then on each `[cache.volumes]` shard
/// under the shard's own `gc.lock` and budget (kunobi-ninja/kache#974).
/// Returns the main store's stats; a shard that fails is logged and skipped.
/// Each store's lock is its own, so a GC that finds the main store busy
/// still sweeps the shards that are not.
pub fn run_gc_local_with_shards(config: &Config, mode: GcMode) -> Result<crate::store::GcStats> {
    let main = run_gc_local(config, mode)?;
    crate::volume_gc::run_on_volume_shards(config, |shard| {
        if let Some(heading) = shard_sweep_heading(mode, &shard.cache_dir) {
            println!("{heading}");
        }
        run_gc_local(shard, mode)
    });
    Ok(main)
}

/// What `kache gc` prints before it sweeps a shard: nothing unless the sweep
/// itself prints its progress.
fn shard_sweep_heading(mode: GcMode, shard: &std::path::Path) -> Option<String> {
    (mode == GcMode::Cli).then(|| format!("Volume shard {}:", shard.display()))
}

/// The auto-GC worker for the main store, then for each `[cache.volumes]`
/// shard. Each store has its own trigger, budget, and backoff.
pub fn run_auto_gc_workers(config: &Config, retry_delay: std::time::Duration) {
    run_auto_gc_worker(config, retry_delay);
    crate::volume_gc::run_on_volume_shards(config, |shard| {
        run_auto_gc_worker(shard, retry_delay);
        Ok(())
    });
}

/// `kache gc --max-age` on each `[cache.volumes]` shard when the daemon could
/// not take it. A shard whose `gc.lock` is held is left to that GC.
fn evict_shards_older_than(config: &Config, hours: u64) -> Vec<crate::store::GcStats> {
    crate::volume_gc::run_on_volume_shards(config, |shard| {
        let store = Store::open(shard)?;
        let Some(_gc_lock) = store.try_gc_lock()? else {
            return Ok(skipped_gc_stats());
        };
        evict_older_than_recorded(&store, shard, hours)
    })
    .into_iter()
    .map(|(_, stats)| stats)
    .collect()
}

/// One line per `[cache.volumes]` shard for the summary `kache gc` prints.
/// Shards without an index are left out, as GC leaves them.
fn shard_store_lines(config: &Config) -> Vec<String> {
    crate::volume_gc::run_on_volume_shards(config, |shard| {
        let store = Store::open(shard)?;
        Ok(format!(
            "Volume shard {}: {}",
            shard.cache_dir.display(),
            gc_store_usage(
                &store,
                shard.max_size,
                crate::volume_gc::filesystem_bytes(&shard.cache_dir),
            )?
        ))
    })
    .into_iter()
    .map(|(_, line)| line)
    .collect()
}

/// The detached worker the wrapper spawns under size pressure when no daemon
/// takes its hint: sweep, and if live builds pinned entries (or another
/// driver held `gc.lock`), wait `retry_delay` for them to age out and sweep
/// again. The worker exits afterwards, so this is its only later chance. It
/// then records where the store ended so every automatic driver backs off
/// while it stays over budget. A worker whose sweeps both lost `gc.lock`
/// leaves the backoff to the driver that held it.
pub fn run_auto_gc_worker(config: &Config, retry_delay: std::time::Duration) {
    let first = auto_gc_worker_sweep(config);
    let second = first
        .as_ref()
        .is_some_and(auto_gc_retry_wanted)
        .then(|| {
            std::thread::sleep(retry_delay);
            auto_gc_worker_sweep(config)
        })
        .flatten();
    // The later sweep's probe is the fresher account of what is held.
    let Some(last) = [second, first]
        .into_iter()
        .flatten()
        .find(|stats| !stats.skipped)
    else {
        return;
    };
    match Store::open(config).and_then(|store| store.physical_size()) {
        Ok(size) => crate::wrapper::record_auto_gc_outcome(config, size, last.bytes_held),
        Err(e) => log::debug!("auto-gc: store size after the sweep unknown: {e:#}"),
    }
}

/// One worker sweep, if the shared trigger still calls for it. The wrapper
/// checked before spawning, but a daemon that acknowledged the hint too late
/// may have swept since, and the first sweep may have cleared the pressure.
fn auto_gc_worker_sweep(config: &Config) -> Option<crate::store::GcStats> {
    crate::disk_recovery::run(config);
    let size = Store::open(config)
        .and_then(|store| store.physical_size())
        .ok()?;
    if !crate::wrapper::auto_gc_sweep_due(config, size) {
        return None;
    }
    run_gc_local(config, GcMode::Background).ok()
}

/// Whether a second worker sweep can do better than the first: only when the
/// first left entries a live build pinned, or never got `gc.lock`.
fn auto_gc_retry_wanted(first: &crate::store::GcStats) -> bool {
    first.skipped || first.entries_pinned > 0
}

fn skipped_gc_stats() -> crate::store::GcStats {
    crate::store::GcStats {
        skipped: true,
        ..crate::store::GcStats::default()
    }
}

fn add_gc_stats(total: &mut crate::store::GcStats, part: &crate::store::GcStats) {
    total.entries_evicted = total.entries_evicted.saturating_add(part.entries_evicted);
    total.bytes_freed = total.bytes_freed.saturating_add(part.bytes_freed);
    total.blobs_removed = total.blobs_removed.saturating_add(part.blobs_removed);
    total.duration_ms = total.duration_ms.saturating_add(part.duration_ms);
    total.entries_pinned = total.entries_pinned.saturating_add(part.entries_pinned);
    total.entries_unreclaimable = total
        .entries_unreclaimable
        .saturating_add(part.entries_unreclaimable);
    total.disk_bytes_reclaimed = total
        .disk_bytes_reclaimed
        .saturating_add(part.disk_bytes_reclaimed);
    total.entries_failed = total.entries_failed.saturating_add(part.entries_failed);
    total.entries_locked = total.entries_locked.saturating_add(part.entries_locked);
    total.entries_busy_snapshot = total
        .entries_busy_snapshot
        .saturating_add(part.entries_busy_snapshot);
    total.entries_recent_prefiltered = total
        .entries_recent_prefiltered
        .saturating_add(part.entries_recent_prefiltered);
    total.entries_import_pinned = total
        .entries_import_pinned
        .saturating_add(part.entries_import_pinned);
    total.evict_write_ms = total.evict_write_ms.saturating_add(part.evict_write_ms);
    // A measurement of the store, not work done: two sweeps probing the same
    // held bytes must not add up to twice as many.
    total.bytes_held = total.bytes_held.max(part.bytes_held);
    total.skipped |= part.skipped;
}

fn compile_time_backfill_message(result: Result<usize>) -> String {
    match result {
        Ok(0) => " no positive costs recovered this pass.".to_owned(),
        Ok(count) => format!(
            " {} updated.",
            term::counted(count as u64, "entry", "entries")
        ),
        Err(error) => format!(" deferred: {error}"),
    }
}

fn gc_stats_from_breakdown(report: &crate::daemon::GcBreakdown) -> crate::store::GcStats {
    let mut total = crate::store::GcStats::default();
    for part in [&report.age, &report.duplicate, &report.size] {
        total.entries_evicted = total.entries_evicted.saturating_add(part.entries_evicted);
        total.bytes_freed = total.bytes_freed.saturating_add(part.bytes_freed);
        total.entries_pinned = total.entries_pinned.saturating_add(part.entries_pinned);
        total.disk_bytes_reclaimed = total
            .disk_bytes_reclaimed
            .saturating_add(part.disk_bytes_reclaimed);
        total.entries_unreclaimable = total
            .entries_unreclaimable
            .saturating_add(part.entries_unreclaimable);
        total.entries_failed = total.entries_failed.saturating_add(part.entries_failed);
        total.entries_locked = total.entries_locked.saturating_add(part.entries_locked);
        total.evict_write_ms = total.evict_write_ms.saturating_add(part.evict_write_ms);
        total.bytes_held = total.bytes_held.max(part.bytes_held);
    }
    total
}

fn human_gc_output(json: bool) -> bool {
    !json
}

fn gc_store_usage(store: &Store, max_size: u64, filesystem_bytes: Option<u64>) -> Result<String> {
    Ok(format!(
        "{} / {} · {}",
        kache_core::size::format_bytes(store.physical_size()?),
        crate::config::describe_max_size(max_size, filesystem_bytes),
        term::counted(store.entry_count()? as u64, "entry", "entries")
    ))
}

fn emit_gc_json(config: &Config, skipped: bool, stats: &crate::store::GcStats) -> Result<()> {
    let store = Store::open(config)?;
    let store_bytes = store.physical_size().unwrap_or(0);
    let disk = crate::machine::disk_view(&config.store_dir(), store_bytes, config.max_size);
    #[derive(deser::Serialize)]
    struct Body {
        skipped: bool,
        disk: crate::machine::DiskView,
        entries: usize,
        entries_dropped: usize,
        store_bytes_removed: u64,
        disk_bytes_reclaimed: u64,
        entries_pinned: usize,
        entries_unreclaimable: usize,
        entries_failed: usize,
    }
    let next = crate::machine::next_after_gc(
        &disk,
        stats.entries_unreclaimable,
        stats.disk_bytes_reclaimed,
        stats.bytes_freed,
    );
    // `kache clean --cache --stale` runs this too, and says `clean`.
    crate::machine::emit(
        crate::machine::command(),
        Body {
            skipped,
            disk,
            entries: store.entry_count().unwrap_or(0),
            entries_dropped: stats.entries_evicted,
            store_bytes_removed: stats.bytes_freed,
            disk_bytes_reclaimed: stats.disk_bytes_reclaimed,
            entries_pinned: stats.entries_pinned,
            entries_unreclaimable: stats.entries_unreclaimable,
            entries_failed: stats.entries_failed,
        },
        next,
    )
}

/// Record a GC run `kache gc` made itself. A failed write costs the record,
/// never the GC.
fn record_manual_gc_run(config: &Config, stats: &crate::store::GcStats) {
    if let Err(e) = crate::report::record_gc_run(
        config,
        "manual",
        crate::store::SweepOrigin::Requested,
        stats,
    ) {
        log::warn!("recording GC run: {e:#}");
    }
}

/// `kache gc --max-age` run locally because the daemon could not take it. The
/// caller holds gc.lock, so the recorded totals cannot race another driver.
fn evict_older_than_recorded(
    store: &Store,
    config: &Config,
    hours: u64,
) -> Result<crate::store::GcStats> {
    let started = std::time::Instant::now();
    let mut stats = store.evict_older_than(hours)?;
    stats.duration_ms = started.elapsed().as_millis() as u64;
    record_manual_gc_run(config, &stats);
    Ok(stats)
}

/// Run garbage collection via the daemon.
pub fn gc(
    config: &Config,
    max_age_hours: Option<u64>,
    stale_schema: bool,
    json: bool,
) -> Result<()> {
    if stale_schema {
        let store = Store::open(config)?;
        let _gc_lock = match store.try_gc_lock()? {
            Some(lock) => lock,
            None => {
                if json {
                    return emit_gc_json(config, true, &crate::store::GcStats::default());
                }
                println!("Another GC is already running; skipping.");
                return Ok(());
            }
        };
        let started = std::time::Instant::now();
        let mut stats = store.evict_stale_key_schemas(crate::cache_key::CACHE_KEY_VERSION)?;
        stats.duration_ms = started.elapsed().as_millis() as u64;
        record_manual_gc_run(config, &stats);
        if json {
            return emit_gc_json(config, false, &stats);
        }
        println!(
            "Stale-schema GC: {}\nCurrent key schema: {}.",
            describe_eviction(&stats, false),
            crate::cache_key::CACHE_KEY_VERSION,
        );
        let total_size = store.physical_size()?;
        let entry_count = store.entry_count()?;
        println!(
            "Store: {} · {}",
            kache_core::size::format_bytes(total_size),
            term::counted(entry_count as u64, "entry", "entries")
        );
        return Ok(());
    }

    let mode = GcMode::from_env();
    if mode == GcMode::Background {
        let sleep_secs = std::env::var("KACHE_AUTO_GC_RETRY_DELAY_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(121);

        run_auto_gc_workers(config, std::time::Duration::from_secs(sleep_secs));
        return Ok(());
    }

    let mut combined = crate::store::GcStats::default();
    let request = {
        let _ticker = term::Ticker::start("Running GC");
        crate::daemon::send_gc_request(config, max_age_hours)
    };
    match request {
        Ok(outcome) if outcome.skipped => {
            if json {
                return emit_gc_json(config, true, &combined);
            }
            println!("Another GC is already running; skipping.");
        }
        Ok(outcome) => {
            if let Some(report) = outcome.breakdown.as_ref() {
                combined = gc_stats_from_breakdown(report);
                if human_gc_output(json) {
                    if let Some(hours) = max_age_hours {
                        println!("Age GC ({hours}h):");
                    } else {
                        println!(
                            "GC complete: age {}, duplicates {}, size {}.",
                            report.age.entries_evicted,
                            report.duplicate.entries_evicted,
                            report.size.entries_evicted,
                        );
                    }
                    let store = Store::open(config)?;
                    let over_limit = store_over_limit(
                        store
                            .physical_size()
                            .ok()
                            .map(|size| size.saturating_sub(combined.bytes_held)),
                        config.max_size,
                    );
                    println!("Eviction: {}", describe_eviction(&combined, over_limit));
                }
            } else if human_gc_output(json) {
                if let Some(hours) = max_age_hours {
                    println!(
                        "Evicted {} older than {hours}h.",
                        term::counted(outcome.evicted.unwrap_or(0) as u64, "entry", "entries")
                    );
                } else {
                    println!(
                        "Evicted {} (daemon returned no per-policy breakdown).",
                        term::counted(outcome.evicted.unwrap_or(0) as u64, "entry", "entries")
                    );
                }
                combined.entries_evicted = outcome.evicted.unwrap_or(0);
            }
        }
        Err(e) => {
            if human_gc_output(json) {
                println!("Daemon GC failed ({e}), running locally...");
            }
            if let Some(hours) = max_age_hours {
                let store = Store::open(config)?;
                let _gc_lock = match store.try_gc_lock()? {
                    Some(lock) => lock,
                    None => {
                        if json {
                            return emit_gc_json(config, true, &combined);
                        }
                        println!("Another GC is already running; skipping.");
                        return Ok(());
                    }
                };
                if human_gc_output(json) {
                    print!("Running eviction...");
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
                let evict_stats = evict_older_than_recorded(&store, config, hours)?;
                drop(_gc_lock);
                evict_shards_older_than(config, hours);
                combined = evict_stats.clone();
                if human_gc_output(json) {
                    let over_limit = store_over_limit(
                        store
                            .physical_size()
                            .ok()
                            .map(|size| size.saturating_sub(evict_stats.bytes_held)),
                        config.max_size,
                    );
                    println!(" {}", describe_eviction(&evict_stats, over_limit));
                }
            } else {
                combined = run_gc_local_with_shards(
                    config,
                    if json { GcMode::CliQuiet } else { GcMode::Cli },
                )?;
            }
        }
    }

    if json {
        return emit_gc_json(config, false, &combined);
    }

    let store = Store::open(config)?;
    println!(
        "Store: {}",
        gc_store_usage(
            &store,
            config.max_size,
            crate::cache_fs::probe(&config.cache_dir).total_bytes,
        )?
    );
    for line in shard_store_lines(config) {
        println!("{line}");
    }

    Ok(())
}

/// Wipe the entire cache or entries for a specific crate.
pub fn purge(config: &Config, crate_filter: Option<&str>) -> Result<()> {
    let store = Store::open(config)?;

    // Purge is a bulk mutation like a GC sweep, so it holds the same
    // cross-process lock every GC driver takes: a sweep either finishes
    // before the purge starts or sees the store only after the purge is
    // done, never a half-wiped one. Per-blob safety against concurrent
    // builds still comes from the delete-row-first transactional gates.
    let _gc_lock = store.acquire_gc_lock().context("locking GC for purge")?;

    if let Some(name) = crate_filter {
        let entries = store.list_entries("name")?;
        let mut removed = 0;
        let mut skipped = 0;
        for entry in &entries {
            if entry.crate_name == name {
                // A corrupt entry (unloadable meta.json) refuses removal to
                // avoid leaking blob refcounts (#276); report it and keep going.
                if let Err(e) = store.remove_entry(&entry.cache_key) {
                    eprintln!("  skipped {}: {e:#}", entry.cache_key);
                    skipped += 1;
                    continue;
                }
                removed += 1;
            }
        }
        println!(
            "Removed {} for '{name}'.",
            term::counted(removed as u64, "entry", "entries")
        );
        if skipped > 0 {
            println!(
                "Skipped {skipped} corrupt entr{} (see warnings above).",
                if skipped == 1 { "y" } else { "ies" }
            );
        }
    } else {
        store.clear()?;
        println!("Cleared entire local store.");
        // Runs a target directory still links to stay: removing them would
        // break its next build.
        let sweep = crate::build_script::sweep_hermetic_out_dirs(
            &config.cache_dir,
            std::time::Duration::ZERO,
        )?;
        println!(
            "Removed {} build-script runs; kept {} that target directories link to.",
            sweep.removed, sweep.kept
        );
    }

    Ok(())
}

/// What `kache clean --cache` or `--crate` removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CacheClean {
    /// Entries older than this many hours.
    OlderThan(u64),
    /// Entries from old or unrecorded cache-key schemas.
    StaleSchema,
    /// Every entry.
    All,
    /// One crate's entries.
    Crate(String),
}

/// `kache clean --cache` and its hidden forms. Eviction by age runs at once,
/// as the daemon would. Removing everything, or a whole crate, asks first
/// unless `yes` is set.
pub fn clean_cache(
    config: &Config,
    what: CacheClean,
    dry_run: bool,
    yes: bool,
    json: bool,
) -> Result<()> {
    let (entries, bytes) = cache_counts(&Store::open(config)?, &what)?;
    let quarantine_notice = cache_quarantine_notice(config, &what)?;
    let action = cache_action(what, entries, bytes, dry_run, json)?;
    if let Some(notice) = quarantine_notice.as_ref().filter(|_| !json) {
        println!("{notice}");
    }
    match action {
        CacheAction::Say(message) if json => {
            #[derive(deser::Serialize)]
            struct Body {
                preview: bool,
                entries: usize,
                bytes: u64,
                message: String,
                #[deser(skip_serializing_if = Option::is_none)]
                notice: Option<String>,
            }
            crate::machine::emit(
                "clean",
                Body {
                    preview: dry_run,
                    entries,
                    bytes,
                    message,
                    notice: quarantine_notice,
                },
                Vec::new(),
            )
        }
        CacheAction::Say(text) => {
            println!("{text}");
            Ok(())
        }
        CacheAction::Gc {
            max_age_hours,
            stale_schema,
        } => gc(config, max_age_hours, stale_schema, json),
        CacheAction::Purge {
            crate_name,
            question,
        } => {
            let terminal = std::io::stdin().is_terminal();
            if !confirm_removal_with(&question, yes, terminal, |question| {
                prompt_yes_no(question, false, false)
            })? {
                println!("Nothing removed.");
                return Ok(());
            }
            purge(config, crate_name.as_deref())
        }
    }
}

/// Full cache removal includes forensic copies; show this before confirmation.
fn cache_quarantine_notice(config: &Config, what: &CacheClean) -> Result<Option<String>> {
    if !matches!(what, CacheClean::All) {
        return Ok(None);
    }
    let files = crate::store::quarantined_index_files(&config.index_db_path())?;
    if files.is_empty() {
        return Ok(None);
    }
    let bytes: u64 = files.iter().map(|file| file.bytes).sum();
    Ok(Some(format!(
        "Full cache removal also deletes {} quarantined index files ({}). Save these copies first if needed for investigation.",
        files.len(),
        term::bytes(bytes)
    )))
}

/// The entries a cache clean is about, and their bytes: one crate's, or
/// the whole cache's.
fn cache_counts(store: &Store, what: &CacheClean) -> Result<(usize, u64)> {
    match what {
        CacheClean::Crate(name) => crate_entries(store, name),
        _ => Ok((store.entry_count()?, store.physical_size()?)),
    }
}

/// What a cache clean does.
#[derive(Debug, PartialEq, Eq)]
enum CacheAction {
    /// Print this and stop: a preview, or nothing to remove.
    Say(String),
    Gc {
        max_age_hours: Option<u64>,
        stale_schema: bool,
    },
    /// Ask `question`, then remove one crate's entries or all of them.
    Purge {
        crate_name: Option<String>,
        question: String,
    },
}

/// Decide a cache clean from what it is about. Pure.
fn cache_action(
    what: CacheClean,
    entries: usize,
    bytes: u64,
    dry_run: bool,
    json: bool,
) -> Result<CacheAction> {
    if dry_run {
        return Ok(CacheAction::Say(cache_preview(&what, entries, bytes)));
    }
    Ok(match what {
        CacheClean::OlderThan(hours) => CacheAction::Gc {
            max_age_hours: Some(hours),
            stale_schema: false,
        },
        CacheClean::StaleSchema => CacheAction::Gc {
            max_age_hours: None,
            stale_schema: true,
        },
        CacheClean::All | CacheClean::Crate(_) if json => anyhow::bail!(
            "removing cache entries this way has no JSON form yet; run it without `--json`."
        ),
        CacheClean::Crate(name) if entries == 0 => {
            CacheAction::Say(format!("No cache entries for '{name}'."))
        }
        what => {
            let question = format!("Remove {}?", removal_words(&what, entries, bytes));
            CacheAction::Purge {
                crate_name: match what {
                    CacheClean::Crate(name) => Some(name),
                    _ => None,
                },
                question,
            }
        }
    })
}

/// How many of the cache's entries belong to `name`, and their bytes.
fn crate_entries(store: &Store, name: &str) -> Result<(usize, u64)> {
    let matching: Vec<_> = store
        .list_entries("name")?
        .into_iter()
        .filter(|entry| entry.crate_name == name)
        .collect();
    Ok((
        matching.len(),
        matching.iter().map(|entry| entry.size).sum(),
    ))
}

/// `all 4 cache entries (6.8 MiB)`, `2 cache entries for 'serde' (957 KiB)`.
fn removal_words(what: &CacheClean, entries: usize, bytes: u64) -> String {
    let plural = if entries == 1 { "y" } else { "ies" };
    match what {
        CacheClean::Crate(name) => format!(
            "{} cache entr{plural} for '{name}' ({})",
            term::count(entries as u64),
            term::bytes(bytes)
        ),
        _ => format!(
            "all {} cache entr{plural} ({})",
            term::count(entries as u64),
            term::bytes(bytes)
        ),
    }
}

/// `14d`, `36h`: a window in whole days when it is one.
fn window_label(hours: u64) -> String {
    if hours > 0 && hours.is_multiple_of(24) {
        format!("{}d", hours / 24)
    } else {
        format!("{hours}h")
    }
}

/// What a cache clean would do, for `--dry-run`. Pure.
fn cache_preview(what: &CacheClean, entries: usize, bytes: u64) -> String {
    match what {
        CacheClean::OlderThan(hours) => format!(
            "Cache is {} in {}. This would evict the entries older than {}.",
            term::bytes(bytes),
            term::counted(entries as u64, "entry", "entries"),
            window_label(*hours)
        ),
        CacheClean::StaleSchema => format!(
            "This would remove the entries not keyed by schema {}.",
            crate::cache_key::CACHE_KEY_VERSION
        ),
        CacheClean::Crate(name) if entries == 0 => format!("No cache entries for '{name}'."),
        what => format!("Would remove {}.", removal_words(what, entries, bytes)),
    }
}

/// `--yes`, or a yes at `prompt` when `terminal`. Without a terminal and
/// without `--yes` nothing is removed and the command fails with
/// [`no_terminal`].
fn confirm_removal_with(
    question: &str,
    yes: bool,
    terminal: bool,
    prompt: impl FnOnce(&str) -> Result<bool>,
) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !terminal {
        return Err(no_terminal("nothing removed", &command_args(), true));
    }
    prompt(question)
}

fn shell_line(argv: &[String]) -> String {
    let plain = |word: &str| {
        !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c))
    };
    argv.iter()
        .map(|word| {
            if plain(word) {
                word.clone()
            } else {
                format!("'{}'", word.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `argv` as one line a POSIX shell reads back as the same words.
fn command_line(argv: &[String]) -> String {
    shell_line(argv)
}

/// The arguments this process was started with, after the program name.
pub(crate) fn command_args() -> Vec<String> {
    kache_client::engine_entry(std::env::args_os().collect(), crate::VERSION)
        .map(|entry| {
            entry
                .args
                .into_iter()
                .skip(1)
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The error a command that asks first gives when there is no terminal and
/// no `--yes`: `outcome`, then the same command line with `--yes`, and
/// `--dry-run` when `preview` says the command has one.
pub(crate) fn no_terminal(outcome: &str, args: &[String], preview: bool) -> anyhow::Error {
    retry_with_yes(outcome, "there is no terminal to ask", args, preview)
}

/// [`no_terminal`] with another reason nobody answered.
fn retry_with_yes(outcome: &str, why: &str, args: &[String], preview: bool) -> anyhow::Error {
    let mut argv = vec!["kache".to_string()];
    argv.extend(args.iter().cloned());
    argv.push("--yes".to_string());
    let preview = if preview {
        ", or --dry-run to preview"
    } else {
        ""
    };
    anyhow::anyhow!(
        "{outcome}: {why}. Run `{}` to go ahead without asking{preview}.",
        command_line(&argv)
    )
}

fn fallback_is_sccache(config: Option<&crate::config::Config>) -> bool {
    config
        .and_then(|cfg| cfg.fallback.as_deref())
        .is_some_and(is_sccache_program)
}

fn is_sccache_program(value: &str) -> bool {
    let name = std::path::Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value);
    name.eq_ignore_ascii_case("sccache") || name.eq_ignore_ascii_case("sccache.exe")
}

fn active_sccache_migration_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    !trimmed.starts_with('#') && trimmed.contains("sccache") && !trimmed.contains("KACHE_FALLBACK")
}

/// Check environment for sccache and configuration issues.
fn daemon_service_check(
    installed: bool,
    healthy_daemon_reachable: bool,
) -> (bool, Option<&'static str>) {
    let pass = installed || healthy_daemon_reachable;
    (pass, (!pass).then_some("kache daemon install"))
}

/// Whether a `doctor` check counts toward the "N issue(s) found" total. Checks
/// downgraded to informational (`optional`) never count, even when they fail.
fn is_doctor_issue(pass: bool, optional: bool) -> bool {
    !pass && !optional
}

/// Who repairs a Cargo `rustc-wrapper` that is not kache: `Some(true)` for
/// `doctor --fix` (an sccache wrapper in $CARGO_HOME), `Some(false)` for
/// `kache init` (unset or another wrapper there), `None` for a project
/// config, which neither edits.
fn cargo_wrapper_repair(
    value: &str,
    path: &std::path::Path,
    cargo_home: &std::path::Path,
) -> Option<bool> {
    let dir = path.parent()?;
    let same = dir == cargo_home
        || matches!(
            (dir.canonicalize(), cargo_home.canonicalize()),
            (Ok(dir), Ok(home)) if dir == home
        );
    same.then(|| is_sccache_program(value))
}

/// [`cargo_wrapper_repair`] for the resolved wrapper: `kache init` sets an
/// unset one, and neither command edits `RUSTC_WRAPPER` in the environment.
fn wrapper_setting_repair(
    setting: Option<&crate::wrapper_config::WrapperSetting>,
    cargo_home: &std::path::Path,
) -> Option<bool> {
    match setting {
        Some(crate::wrapper_config::WrapperSetting::Environment { .. }) => None,
        Some(crate::wrapper_config::WrapperSetting::CargoConfig { value, path }) => {
            cargo_wrapper_repair(value, path, cargo_home)
        }
        None => Some(false),
    }
}

/// Doctor's one check of the wrapper Cargo uses, as `(pass, detail, fix)`.
/// `repair` is [`wrapper_setting_repair`]'s answer; `cargo_config` is the
/// file `kache init` would edit.
fn wrapper_check(
    setting: Option<&crate::wrapper_config::WrapperSetting>,
    repair: Option<bool>,
    cargo_config: &std::path::Path,
) -> (bool, String, Option<String>) {
    use crate::wrapper_config::{WrapperSetting, display_path};
    match setting {
        Some(WrapperSetting::Environment { value }) if value.contains("kache") => {
            (true, "kache via env".into(), None)
        }
        Some(WrapperSetting::Environment { value }) => (
            false,
            if value.contains("sccache") {
                format!("sccache ({value})")
            } else {
                format!("{value} (not kache)")
            },
            Some("export RUSTC_WRAPPER=kache, or unset it and run `kache init`".into()),
        ),
        Some(WrapperSetting::CargoConfig { value, path }) if value.contains("kache") => {
            (true, format!("kache via {}", display_path(path)), None)
        }
        Some(WrapperSetting::CargoConfig { value, path }) => (
            false,
            format!("{value} in {}", display_path(path)),
            Some(match repair {
                Some(true) => {
                    "run `kache doctor --fix` or `kache init` to replace it with kache".to_string()
                }
                Some(false) => "run `kache init` to replace it with kache".to_string(),
                None => format!(
                    "replace `rustc-wrapper = \"{value}\"` with `rustc-wrapper = \"kache\"` in {}",
                    path.display()
                ),
            }),
        ),
        None => (
            false,
            "not set".into(),
            Some(format!(
                "run `kache init` to set it in {}, or export RUSTC_WRAPPER=kache",
                display_path(cargo_config)
            )),
        ),
    }
}

/// Doctor's fix when `kache` is not on PATH: add the directory of the
/// binary that is running.
fn binary_fix(on_path: bool, running_dir: Option<&std::path::Path>) -> Option<String> {
    if on_path {
        return None;
    }
    Some(match running_dir {
        Some(dir) => format!("add {} to PATH", dir.display()),
        None => "add the directory that holds kache to PATH".to_string(),
    })
}

/// File `label` under the command that repairs it, if any.
fn note_repair(
    label: &'static str,
    by_fix: Option<bool>,
    fix_repairs: &mut Vec<&'static str>,
    init_repairs: &mut Vec<&'static str>,
) {
    match by_fix {
        Some(true) => fix_repairs.push(label),
        Some(false) => init_repairs.push(label),
        None => {}
    }
}

/// The JSON `next` for doctor: `doctor --fix` only when it repairs one of the
/// failing checks, `kache init` when init does.
fn doctor_next_actions(
    failed: &[&str],
    fix_repairs: &[&str],
    init_repairs: &[&str],
) -> Vec<crate::machine::NextAction> {
    let repaired_by = |repairs: &[&str]| {
        failed
            .iter()
            .copied()
            .filter(|label| repairs.contains(label))
            .collect::<Vec<_>>()
    };
    let mut next = Vec::new();
    let by_fix = repaired_by(fix_repairs);
    if !by_fix.is_empty() {
        next.push(crate::machine::NextAction {
            argv: vec!["kache".into(), "doctor".into(), "--fix".into()],
            why: format!("repairs: {}", by_fix.join(", ")),
        });
    }
    let by_init = repaired_by(init_repairs);
    if !by_init.is_empty() {
        next.push(crate::machine::NextAction {
            argv: vec!["kache".into(), "init".into()],
            why: format!("sets up: {}", by_init.join(", ")),
        });
    }
    next
}

/// Wording for the "Config file" check, as `(pass, detail, fix hint)`.
fn doctor_config_file_check(
    path: &std::path::Path,
    exists: bool,
    problem: Option<&str>,
) -> (bool, String, Option<String>) {
    let shown = crate::wrapper_config::display_path(path);
    match problem {
        Some(reason) => (
            false,
            format!("{shown} is ignored: {reason}"),
            Some("fix the file; until then every kache command uses the defaults".into()),
        ),
        None if exists => (true, shown, None),
        None => (true, format!("{shown} (not created; defaults apply)"), None),
    }
}

/// Labels of the daemon-related checks that become informational when the
/// daemon is optional. Kept in sync with the check constructions in
/// [`doctor`]; module-level so the disposition logic below is unit-testable.
const DAEMON_CHECK_LABELS: [&str; 4] = [
    "Daemon version",
    "Daemon service",
    "Service exe",
    "Daemon config",
];

/// The host config path `kache stats` names, only when the host file is
/// actually merged in: absent, disabled or unusable files are not in effect.
fn host_config_in_effect(status: &crate::config::HostConfigStatus) -> Option<String> {
    match status {
        crate::config::HostConfigStatus::Present { path, .. } => Some(path.display().to_string()),
        _ => None,
    }
}

/// Wording for the "Host config" check, as `(pass, detail, fix hint)`. Pure,
/// so each state is testable without a host file on the machine.
fn doctor_host_config_check(
    status: &crate::config::HostConfigStatus,
) -> (bool, String, Option<String>) {
    use crate::config::{HostConfigStatus, HostKeySource};
    match status {
        HostConfigStatus::Disabled => (true, "off (KACHE_HOST_CONFIG is empty)".to_string(), None),
        HostConfigStatus::Absent { path } => (true, format!("none at {}", path.display()), None),
        HostConfigStatus::Invalid { path, error } => (
            false,
            format!("{} is ignored: {error}", path.display()),
            Some(format!("fix or remove {}", path.display())),
        ),
        HostConfigStatus::Present { path, keys } => {
            let keys = if keys.is_empty() {
                "sets nothing".to_string()
            } else {
                keys.iter()
                    .map(|entry| match &entry.source {
                        HostKeySource::Host => entry.key.clone(),
                        HostKeySource::ChosenFile(file) => {
                            format!("{} (overridden by {})", entry.key, file.display())
                        }
                        HostKeySource::Env(var) => format!("{} (overridden by {var})", entry.key),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            (true, format!("{}: {keys}", path.display()), None)
        }
    }
}

/// Whether a failing doctor check is informational rather than an issue:
/// daemon checks (the daemon is optional, #443), the
/// compiler probe when there is no `cc` at all to diagnose (#626),
/// C/C++ shims (PATH masquerade is opt-in), and an install path that an
/// upgrade may break, which only matters once something records it.
fn doctor_check_is_optional(label: &str, daemon_optional: bool, probe_no_compiler: bool) -> bool {
    (daemon_optional && DAEMON_CHECK_LABELS.contains(&label))
        || (label == "Compiler probe" && probe_no_compiler)
        || label == "C/C++ shims"
        || label == "Install path"
        || label == "Compiler execution"
        || label == OTHER_CHECKOUTS
        || label == "Store build warning"
        || label == "Store file limits"
        || label == "Store volume probe"
        || label == "Daemon process"
}

const OTHER_CHECKOUTS: &str = "Other checkouts";

/// Only native Linux links key their linker, and without a `cc` there is no
/// driver to ask.
fn doctor_shows_native_linker(linux: bool, no_compiler: bool) -> bool {
    linux && !no_compiler
}

/// Doctor's "Native linker" check: the linker a native Linux link runs with
/// this shell's `RUSTFLAGS`, placed the way its cache key places it.
fn linker_banner(
    linker: &std::path::Path,
    env: &crate::invocation::InvocationEnv,
) -> Option<String> {
    use crate::invocation::process::CommandExt;
    let output = env
        .command(linker)
        .ok()?
        .arg("--version")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .invocation_output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

fn native_linker_check(env: &crate::invocation::InvocationEnv) -> Check {
    let mut argv: Vec<String> = ["rustc", "--crate-type", "bin", "main.rs"]
        .into_iter()
        .map(str::to_string)
        .collect();
    argv.extend(
        env.var("RUSTFLAGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string),
    );
    let placed = crate::args::RustcArgs::parse_for(&argv, env).and_then(|args| {
        let rustc_version =
            crate::cache_key::get_rustc_version_for(&args.rustc, env).unwrap_or_default();
        let driver = std::path::Path::new(args.get_codegen_opt("linker").unwrap_or("cc"));
        crate::cache_key::native_linker_for(&args, &rustc_version, driver, None, env)
    });
    match placed {
        Ok(linker) => Check {
            label: "Native linker",
            pass: true,
            detail: match linker_banner(&linker, env) {
                Some(banner) => format!("{} ({banner})", linker.display()),
                None => linker.display().to_string(),
            },
            fix: None,
        },
        Err(error) => Check {
            label: "Native linker",
            pass: false,
            detail: format!("{error:#}"),
            fix: Some(
                "install the linker that `cc` and RUSTFLAGS select; native Linux links \
                 are not cached until it resolves"
                    .into(),
            ),
        },
    }
}

/// Whether the latest recorded build missed crates that another checkout of
/// the project had built, for a reason `kache explain` can name. `None` when
/// there is no recorded build to look at.
fn doctor_checkout_check(event_log: &std::path::Path) -> Option<Check> {
    let events = crate::events::read_events(event_log).ok()?;
    let sessions = crate::sessions::group(
        &events,
        std::time::Duration::from_secs(crate::wrapper::BUILD_SESSION_SECS),
    );
    let builds = crate::explain::latest_builds(&sessions, None)?;
    let (count, baseline) =
        crate::sessions::other_checkout_misses(&events, &builds.latest.events);
    Some(checkout_check(
        &builds.latest.root,
        count,
        baseline.as_deref(),
    ))
}

/// Wording for the "Other checkouts" check. Informational: the misses are
/// real, but whether to normalize paths is the user's call.
fn checkout_check(root: &str, count: usize, baseline: Option<&str>) -> Check {
    if count == 0 {
        return Check {
            label: OTHER_CHECKOUTS,
            pass: true,
            detail: "no miss in the latest build traces to another checkout".into(),
            fix: None,
        };
    }
    let other = baseline.map_or_else(
        || "other checkouts".to_string(),
        |path| crate::term::home_path(std::path::Path::new(path)),
    );
    Check {
        label: OTHER_CHECKOUTS,
        pass: false,
        detail: format!(
            "{} {} in {} differ{} from the build in {other}",
            count,
            if count == 1 { "miss" } else { "misses" },
            crate::term::home_path(std::path::Path::new(root)),
            if count == 1 { "s" } else { "" },
        ),
        fix: Some(
            "`kache explain` says what to set, often `KACHE_BASE_DIR` or C/C++ prefix maps".into(),
        ),
    }
}

/// Wording for the "Install path" check, as `(pass, detail, fix hint)`:
/// the path shims and the service record, the installer, and whether the
/// path survives an upgrade.
fn doctor_install_path_check(
    selection: &Result<kache_shims::Selection, kache_shims::Error>,
) -> (bool, String, Option<String>) {
    match selection {
        Ok(selection) => {
            let survives = selection.stability.survives_upgrade();
            let detail = format!(
                "{} ({}, {}): {}",
                selection.path.display(),
                selection.kind,
                selection.stability,
                selection.reason
            );
            let fix = (!survives).then(|| {
                "rerun kache init --shims and kache daemon install after each upgrade".to_string()
            });
            (survives, detail, fix)
        }
        Err(error) => (false, error.to_string(), None),
    }
}

/// The daemon footnote prints when at least one daemon check failed but was
/// downgraded to informational; other downgrades (the compiler probe) carry
/// self-explanatory detail lines and get no footnote.
fn daemon_footnote_needed(daemon_optional: bool, results: &[(&str, bool)]) -> bool {
    daemon_optional
        && results
            .iter()
            .any(|(label, pass)| !pass && DAEMON_CHECK_LABELS.contains(label))
}

/// Only doctor --fix may replace an older daemon release.
fn should_restart_stale_daemon(fix_requested: bool, daemon_is_stale: bool) -> bool {
    fix_requested && daemon_is_stale
}

/// What `doctor --fix` prints after trying to replace a stale daemon, or `None`
/// when it came up and the check below will say so.
///
/// A restart that did not finish must not be reported as one that is still
/// finishing: `Ok(false)` means the replacement was spawned but had not bound its
/// socket in time, while `Err` means the handoff failed outright and there may be
/// no replacement at all. Claiming "still coming up in the background" for both
/// invites the reader to ignore a real failure.
fn stale_restart_note(outcome: &anyhow::Result<bool>) -> Option<String> {
    match outcome {
        Ok(true) => None,
        Ok(false) => Some(
            "\x1b[2mthe replacement did not bind its socket within the startup \
             timeout\x1b[0m"
                .into(),
        ),
        Err(error) => Some(format!("\x1b[31mrestart failed: {error:#}\x1b[0m")),
    }
}

/// Report the daemon release without inferring order from build timestamps.
fn daemon_version_check(
    daemon: Option<&str>,
    starting: bool,
    my_version: &str,
    startup_log: &str,
) -> (bool, String, Option<String>) {
    use std::cmp::Ordering;
    match daemon {
        Some(version) => match crate::daemon::compare_daemon_version(my_version, version) {
            Some(Ordering::Equal) => (true, format!("v{version}"), None),
            Some(Ordering::Greater) => (
                false,
                format!("daemon v{version} predates binary v{my_version}; it remains active"),
                Some("`kache doctor --fix` or `kache daemon start` to upgrade".into()),
            ),
            Some(Ordering::Less) => (
                true,
                format!("daemon v{version} is newer than binary v{my_version}"),
                None,
            ),
            None => (
                true,
                format!("daemon release unknown ({version}); automatic upgrade disabled"),
                None,
            ),
        },
        None if starting => (
            true,
            "a daemon is starting; release not yet reported".into(),
            None,
        ),
        None => (
            false,
            "daemon not reachable".into(),
            Some(format!(
                "start daemon with `kache daemon start` or `kache daemon install`; if it does not start, the reason is in {startup_log}"
            )),
        ),
    }
}

pub(crate) struct Check {
    label: &'static str,
    pub(crate) pass: bool,
    pub(crate) detail: String,
    fix: Option<String>,
}


pub(crate) fn doctor_shards(config: &Config) -> Vec<Check> {
    crate::store_view::shard_configs(config)
        .iter()
        .map(|shard| {
            let path = shard.cache_dir.display();
            if !crate::volume_gc::shard_has_index(&shard.cache_dir) {
                return Check {
                    label: "Volume store",
                    pass: false,
                    detail: format!("{path}: no index (not initialized or volume unavailable)"),
                    fix: Some(
                        "check the volume is mounted and the configured shard path is correct"
                            .into(),
                    ),
                };
            }
            let result = Store::open(shard).and_then(|store| {
                Ok((
                    store.entry_count()?,
                    store.total_size()?,
                    store.blob_stats()?,
                ))
            });
            match result {
                Ok((entries, bytes, blobs)) => {
                    let fs = crate::cache_fs::advisory_for(
                        &crate::cache_fs::probe(&shard.cache_dir),
                        &shard.cache_dir,
                    );
                    let drift = blobs.total_blob_size > blobs.total_logical_size;
                    Check {
                        label: "Volume store",
                        pass: fs.is_none() && !drift,
                        detail: format!(
                            "{path}: {}, {} / {}{}",
                            term::counted(entries as u64, "entry", "entries"),
                            kache_core::size::format_bytes(bytes),
                            kache_core::size::format_bytes(shard.max_size),
                            if drift {
                                "; blob accounting needs repair"
                            } else {
                                ""
                            }
                        ),
                        fix: fs.or_else(|| {
                            drift.then(|| "run `kache doctor --verify --repair`".into())
                        }),
                    }
                }
                Err(error) => Check {
                    label: "Volume store",
                    pass: false,
                    detail: format!("{path}: {error}"),
                    fix: Some("check the shard's storage and permissions".into()),
                },
            }
        })
        .collect()
}

/// The directory a build of the current working directory writes its artifacts
/// to: the target dir Cargo would choose for it, or the build dir when Cargo
/// splits the two.
///
/// `kache doctor` probes this rather than the working directory. What the store
/// hardlinks into is the build tree, never the source tree, so probing the
/// working directory reports EXDEV for a project whose `build.target-dir` sits
/// on the store's mount, and a pass for one whose artifacts get copied.
fn cargo_output_dir() -> Option<std::path::PathBuf> {
    cargo_output_dir_from(
        &std::env::current_dir().ok()?,
        &CargoEnvironment {
            target_dir: std::env::var_os("CARGO_TARGET_DIR"),
            build_target_dir: std::env::var_os("CARGO_BUILD_TARGET_DIR"),
            build_dir: std::env::var_os("CARGO_BUILD_BUILD_DIR"),
            cargo_home: std::env::var_os("CARGO_HOME"),
            home: kache_fs::home_dir().unwrap_or_default(),
        },
    )
}

/// The environment Cargo resolves the output directory from, passed in so the
/// search can be tested without mutating the one the suite shares.
#[derive(Default)]
struct CargoEnvironment {
    /// `CARGO_TARGET_DIR`, which names the target dir directly.
    target_dir: Option<std::ffi::OsString>,
    /// `CARGO_BUILD_TARGET_DIR`, the environment name of `build.target-dir`.
    build_target_dir: Option<std::ffi::OsString>,
    /// `CARGO_BUILD_BUILD_DIR`, the environment name of `build.build-dir`.
    build_dir: Option<std::ffi::OsString>,
    /// `CARGO_HOME`.
    cargo_home: Option<std::ffi::OsString>,
    /// The platform's home directory, empty when the platform has none.
    home: std::path::PathBuf,
}

/// [`cargo_output_dir`] with the environment passed in, so the search can be
/// tested without mutating the one the suite shares.
fn cargo_output_dir_from(
    cwd: &std::path::Path,
    env: &CargoEnvironment,
) -> Option<std::path::PathBuf> {
    let workspace = cargo_workspace_root(cwd)?;
    let cargo_home = cargo_home_dir_from(env, cwd);
    let output = cargo_output_resolve(
        cwd,
        &workspace,
        &cargo_home,
        env.target_dir
            .as_deref()
            .or(env.build_target_dir.as_deref()),
        env.build_dir.as_deref(),
    )?;
    closest_existing(output)
}

/// `$CARGO_HOME`, falling back to the platform's home directory (cargo's
/// documented default), the way [`cargo_home_dir`] does.
fn cargo_home_dir_from(env: &CargoEnvironment, cwd: &std::path::Path) -> std::path::PathBuf {
    match env.cargo_home.as_deref().filter(|value| !value.is_empty()) {
        Some(cargo_home) => {
            let cargo_home = std::path::PathBuf::from(cargo_home);
            if cargo_home.is_absolute() {
                cargo_home
            } else {
                cwd.join(cargo_home)
            }
        }
        None => env.home.join(".cargo"),
    }
}

/// The directory Cargo would put the default `target` dir under: the
/// workspace root, or the package root for a package outside a workspace.
///
/// Cargo reads the workspace from the manifest rather than from the lockfile,
/// so a workspace that has not been built yet still resolves.
fn cargo_workspace_root(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut package = None;
    for dir in cwd.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let table = deser_toml::from_str::<deser_value::Map>(
            &std::fs::read_to_string(&manifest).ok()?,
        )
        .ok()?;
        if let Some(workspace) = table
            .get("package")
            .and_then(|value| value.as_map())
            .and_then(|package| package.get("workspace"))
        {
            let root = dir.join(workspace.as_str()?).canonicalize().ok()?;
            return cargo_manifest_is_workspace(&root.join("Cargo.toml")).then_some(root);
        }
        package.get_or_insert(dir);
        if table.get("workspace").is_some_and(|value| value.is_map()) {
            return Some(dir.to_path_buf());
        }
    }
    Some(package.map_or_else(|| cwd.to_path_buf(), std::path::Path::to_path_buf))
}

/// Whether the manifest at `path` declares a workspace.
fn cargo_manifest_is_workspace(manifest: &std::path::Path) -> bool {
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return false;
    };
    let Ok(table) = deser_toml::from_str::<deser_value::Map>(&text) else {
        return false;
    };
    table.get("workspace").is_some_and(|value| value.is_map())
}

/// Probe the first existing directory on the output path. An existing file
/// cannot hold build artifacts, and a missing volume has no directory to probe.
fn closest_existing(path: std::path::PathBuf) -> Option<std::path::PathBuf> {
    let existing = path.ancestors().find(|candidate| candidate.exists())?;
    existing.is_dir().then(|| existing.to_path_buf())
}

/// Resolve Cargo's output directory from config files and environment.
/// File paths are relative to the directory above `.cargo`; unknown templates
/// and includes resolve to nothing.
fn cargo_output_resolve(
    cwd: &std::path::Path,
    workspace: &std::path::Path,
    cargo_home: &std::path::Path,
    target_env: Option<&std::ffi::OsStr>,
    build_env: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    let mut target = workspace.join("target");
    let mut build = None;
    let mut directories = vec![cargo_home.to_path_buf()];
    directories.extend(
        cwd.ancestors()
            .map(|path| path.join(".cargo"))
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    for directory in directories {
        let legacy = directory.join("config");
        let path = if legacy.is_file() {
            legacy
        } else {
            directory.join("config.toml")
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        let table: deser_value::Map = deser_toml::from_str(&text).ok()?;
        if table.contains_key("include") {
            return None;
        }
        let Some(settings) = table.get("build") else {
            continue;
        };
        let settings = settings.as_map()?;
        let base = directory.parent()?;
        if let Some(value) = settings.get("target-dir") {
            target = base.join(value.as_str().filter(|value| !value.is_empty())?);
        }
        if let Some(value) = settings.get("build-dir") {
            build = Some(cargo_output_expand(base, value.as_str()?, workspace, cargo_home)?);
        }
    }
    if let Some(value) = target_env {
        if value.is_empty() {
            return None;
        }
        target = cwd.join(value);
    }
    if let Some(value) = build_env {
        build = Some(cargo_output_expand(cwd, value.to_str()?, workspace, cargo_home)?);
    }
    Some(build.unwrap_or(target))
}

fn cargo_output_expand(
    base: &std::path::Path,
    value: &str,
    workspace: &std::path::Path,
    cargo_home: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if value.is_empty() {
        return None;
    }
    let value = value
        .replace("{workspace-root}", workspace.to_str()?)
        .replace("{cargo-cache-home}", cargo_home.to_str()?);
    if value.contains(['{', '}']) {
        return None;
    }
    Some(base.join(value))
}


fn doctor_link_layout(config: &Config, build_dir: &std::path::Path) -> Check {
    let routed = config.routed_for_path(build_dir);
    let staging = routed.store_dir().join("staging");
    if routed.cache_dir != config.cache_dir && !crate::volume_gc::shard_has_index(&routed.cache_dir)
    {
        return Check {
            label: "Link layout",
            pass: false,
            detail: format!(
                "mapped store {} is unavailable; link probe skipped",
                routed.cache_dir.display()
            ),
            fix: None,
        };
    }
    let probe = crate::link_probe::probe_link_layout(build_dir, &staging);
    Check {
        label: "Link layout", pass: probe.hardlink_supported,
        detail: format!("{} (build: {}, staging: {})", crate::link_probe::format_probe_detail(&probe), crate::link_probe::shown_path(build_dir), crate::link_probe::shown_path(&staging)),
        fix: (!probe.hardlink_supported).then(|| "put the cache and build tree on the SAME mount for zero-copy sharing; if this layout is intentional, expect copies (see `kache report` copy reasons)".into()),
    }
}

fn doctor_index_quarantines(config: &Config) -> Check {
    match crate::store::quarantined_index_files(&config.index_db_path()) {
        Ok(files) if files.is_empty() => Check {
            label: "Store quarantine",
            pass: true,
            detail: "no retained corrupt index files".to_string(),
            fix: None,
        },
        Ok(files) => Check {
            label: "Store quarantine",
            pass: false,
            detail: format!(
                "{} ({}) retained in {} after index corruption",
                term::counted(files.len() as u64, "file", "files"),
                kache_core::size::format_bytes(files.iter().map(|file| file.bytes).sum::<u64>()),
                term::home_path(&config.cache_dir)
            ),
            fix: Some("save the index.db.corrupt-* files if needed for investigation; `kache clean --cache` removes them along with all cached artifacts".to_string()),
        },
        Err(error) => Check {
            label: "Store quarantine",
            pass: false,
            detail: format!("could not inspect retained corrupt index files: {error}"),
            fix: Some(format!("ensure {} is readable", config.cache_dir.display())),
        },
    }
}

fn doctor_section(label: &str) -> &'static str {
    if label.starts_with("Daemon")
        || label.starts_with("Service")
        || label.starts_with("Remote")
        || label == "Compiler execution"
    {
        "Services"
    } else if label.starts_with("Cache")
        || label.starts_with("Store")
        || label.starts_with("Shard")
        || label.starts_with("Link")
        || label.starts_with("Volume")
    {
        "Storage"
    } else {
        "Setup"
    }
}

pub fn doctor(
    fix: bool,
    purge_sccache: bool,
    verify: bool,
    checksums: bool,
    repair: bool,
    probe: bool,
    json: bool,
) -> Result<()> {
    let home = kache_fs::home_dir().unwrap_or_default();
    let (config, provenance) = crate::config::Config::load_with_provenance()
        .map_or((None, None), |(config, provenance)| {
            (Some(config), Some(provenance))
        });
    let sccache_is_fallback = fallback_is_sccache(config.as_ref());

    // The daemon is optional (see README): `kache doctor` shows its checks
    // for diagnostics but never counts them as issues. See #443.
    let daemon_optional = true;

    // Live compiler probe (#626): a toolchain whose `cc -###` resolves no
    // compile line makes every probe-keyed C/C++ flag refuse to cache —
    // builds stay correct but silently lose caching, with zero signal. Run
    // here so the check below reports the live compiler, and so a host with
    // no `cc` at all downgrades to informational instead of failing doctor.
    let probe_diag = crate::probe::live_probe_diagnostic();
    let probe_no_compiler = matches!(probe_diag, crate::probe::LiveProbeDiagnostic::NoCompiler);

    let check_is_optional =
        |label: &str| doctor_check_is_optional(label, daemon_optional, probe_no_compiler);

    let mut checks: Vec<Check> = Vec::new();

    // 1. Binary on PATH
    let which_cmd = "which";
    let (bin_pass, bin_detail) = if let Ok(output) =
        std::process::Command::new(which_cmd).arg("kache").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        (true, path)
    } else {
        (false, "not found".into())
    };
    let running = crate::platform::frontend_executable().ok();
    checks.push(Check {
        label: "Binary",
        pass: bin_pass,
        detail: bin_detail,
        fix: binary_fix(
            bin_pass,
            running.as_deref().and_then(std::path::Path::parent),
        ),
    });

    // Checks `doctor --fix` and `kache init` repair when they fail, for the
    // JSON `next` suggestion. A passing check listed here suggests nothing.
    let mut fix_repairs: Vec<&'static str> = Vec::new();
    let mut init_repairs: Vec<&'static str> = Vec::new();
    let cargo_home = cargo_home_dir();

    // 2. RUSTC_WRAPPER
    let wrapper_setting = crate::wrapper_config::resolve_wrapper_setting();
    note_repair(
        "RUSTC_WRAPPER",
        wrapper_setting_repair(wrapper_setting.as_ref(), &cargo_home),
        &mut fix_repairs,
        &mut init_repairs,
    );
    let (wrapper_pass, wrapper_detail, wrapper_fix) = wrapper_check(
        wrapper_setting.as_ref(),
        wrapper_setting_repair(wrapper_setting.as_ref(), &cargo_home),
        &cargo_config_target_path(),
    );
    checks.push(Check {
        label: "RUSTC_WRAPPER",
        pass: wrapper_pass,
        detail: wrapper_detail,
        fix: wrapper_fix,
    });

    // 3. The kache config file. One that does not parse is ignored
    // everywhere, so every command runs on defaults.
    let config_path = crate::config::resolve_config_path();
    let (config_pass, config_detail, config_fix) = doctor_config_file_check(
        &config_path,
        config_path.exists(),
        crate::config::unusable_config_file()
            .map(|(_, reason)| reason)
            .as_deref(),
    );
    checks.push(Check {
        label: "Config file",
        pass: config_pass,
        detail: config_detail,
        fix: config_fix,
    });

    // 4. Host config layer
    let (host_pass, host_detail, host_fix) =
        doctor_host_config_check(&crate::config::host_config_status());
    checks.push(Check {
        label: "Host config",
        pass: host_pass,
        detail: host_detail,
        fix: host_fix,
    });

    // 5. Cache directory
    if let Some(ref cfg) = config {
        let exists = cfg.cache_dir.exists();
        checks.push(Check {
            label: "Cache dir",
            pass: true,
            detail: if exists {
                cfg.cache_dir.display().to_string()
            } else {
                format!(
                    "{} (will be created on first build)",
                    cfg.cache_dir.display()
                )
            },
            fix: None,
        });

        // Is that directory on storage the WAL index can actually live on?
        // A shared or network mount is the #412 corruption case, and `doctor` is
        // where a user looks when they suspect their setup — so report it here
        // as a real issue rather than only as a build-time advisory (#415).
        let fs_probe = crate::cache_fs::probe(&cfg.cache_dir);
        checks.push(match crate::cache_fs::classify(&fs_probe) {
            crate::cache_fs::CacheFsVerdict::NotLocal { name } => Check {
                label: "Cache FS",
                pass: false,
                detail: format!("{name} — not host-local storage"),
                fix: Some(
                    "the cache index is a WAL SQLite database and can be corrupted on a \
                     shared or network mount: set `cache.local_store`/`KACHE_CACHE_DIR` to a \
                     local, single-machine path"
                        .to_string(),
                ),
            },
            // Local, or unrecognised. Informational either way — worth printing
            // because it is the first thing to ask about in a corruption report.
            verdict => Check {
                label: "Cache FS",
                pass: true,
                detail: match (&fs_probe.name, &verdict) {
                    (Some(name), crate::cache_fs::CacheFsVerdict::Local) => {
                        format!("{name} (local)")
                    }
                    (Some(name), _) => format!("{name} (locality unknown)"),
                    (None, _) => "could not determine filesystem".to_string(),
                },
                fix: None,
            },
        });

        match Store::open(cfg) {
            Ok(_) => checks.push(Check {
                label: "Store DB",
                pass: true,
                detail: cfg.index_db_path().display().to_string(),
                fix: None,
            }),
            Err(e) => checks.push(Check {
                label: "Store DB",
                pass: false,
                detail: format!("{} ({e:#})", cfg.index_db_path().display()),
                fix: Some(format!(
                    "ensure {} is writable; if builds run in a sandboxed or ephemeral env, move `cache.local_store`/`KACHE_CACHE_DIR` to a stable local directory",
                    cfg.cache_dir.display()
                )),
            }),
        }

        checks.extend(doctor_storage::checks(cfg));
        checks.extend(doctor_storage::disk_checks(
            cfg,
            cargo_output_dir().as_deref(),
            &std::env::temp_dir(),
        ));
        checks.push(doctor_index_quarantines(cfg));

        if let Ok(view) = crate::store_view::read(cfg, false, "name") {
            checks.push(Check {
                label: "Store totals",
                pass: view.stores.iter().all(|s| s.error.is_none()),
                detail: format!(
                    "{}, {} stored across {}",
                    term::counted(view.entry_count as u64, "unique entry", "unique entries"),
                    kache_core::size::format_bytes(view.total_size),
                    term::counted(
                        view.stores.len() as u64,
                        "configured store",
                        "configured stores"
                    )
                ),
                fix: None,
            });
        }
        checks.extend(doctor_shards(cfg));
        checks.push(match cargo_output_dir() {
            Some(build_dir) => doctor_link_layout(cfg, &build_dir),
            None => Check {
                label: "Link layout",
                pass: false,
                detail: "Cargo output directory could not be resolved; link probe skipped".into(),
                fix: Some(
                    "check Cargo's output-directory settings from an existing project directory"
                        .into(),
                ),
            },
        });
    }

    // 6. Shell rc sccache remnants
    let mut rc_issues = Vec::new();
    for rc in [".zshrc", ".bashrc", ".bash_profile", ".profile"] {
        let rc_path = home.join(rc);
        if let Ok(content) = std::fs::read_to_string(&rc_path)
            && content.contains("sccache")
        {
            let has_active = content.lines().any(active_sccache_migration_line);
            if has_active {
                rc_issues.push(format!("~/{rc}"));
            }
        }
    }
    if !rc_issues.is_empty() {
        checks.push(Check {
            label: "Shell config",
            pass: false,
            detail: format!("sccache references in {}", rc_issues.join(", ")),
            fix: Some("update or remove those lines; `kache doctor --fix` lists them".into()),
        });
    }

    // 7. sccache daemon running
    if let Ok(output) = std::process::Command::new("pgrep")
        .args(["-x", "sccache"])
        .output()
        && output.status.success()
    {
        if sccache_is_fallback {
            checks.push(Check {
                label: "sccache",
                pass: true,
                detail: "daemon is running as fallback wrapper".into(),
                fix: None,
            });
        } else {
            fix_repairs.push("sccache");
            checks.push(Check {
                label: "sccache",
                pass: false,
                detail: "daemon is running".into(),
                fix: Some("sccache --stop-server".into()),
            });
        }
    }

    // 8. Daemon version match
    //
    // `send_stats_request_without_restart` skips the client-side stale-daemon
    // restart that `send_stats_request` performs, so an upgrade left half-applied
    // is described in the report instead of stalling it for the 8s the
    // replacement takes to bind its socket (kunobi-ninja/kache#720). `--fix` opts
    // into that wait below.
    let my_version = crate::VERSION;
    let mut healthy_daemon_reachable = false;
    if let Some(ref cfg) = config {
        let mut stats = crate::daemon::send_stats_request_without_restart(cfg, false).ok();

        let is_stale = |stats: &Option<crate::daemon::StatsResponse>| {
            stats
                .as_ref()
                .is_some_and(|s| crate::daemon::client_version_is_newer(my_version, &s.version))
        };

        if should_restart_stale_daemon(fix, is_stale(&stats)) {
            // Attributed rather than silent: this is where doctor's runtime goes
            // when it is slow, so say what it is waiting for.
            println!("  Restarting daemon left over from before the upgrade...");
            let outcome = crate::daemon::restart_daemon_for_stale_client(cfg);
            if let Some(note) = stale_restart_note(&outcome) {
                println!("  {note}");
            }
            // Re-read either way: the outgoing daemon is gone now, so the report
            // should describe what is there, not what answered a moment ago.
            stats = crate::daemon::send_stats_request_without_restart(cfg, false).ok();
        }

        healthy_daemon_reachable = stats.is_some();
        if let Some(stats) = &stats {
            checks.extend(doctor_daemon::checks(
                cfg,
                provenance.as_ref().expect("loaded config has provenance"),
                stats.effective_config.as_ref(),
            ));
        }

        let (pass, detail, fix_hint) = daemon_version_check(
            stats.as_ref().map(|s| s.version.as_str()),
            crate::daemon::starting_daemon_epoch(cfg).is_some(),
            my_version,
            &crate::service::startup_log(cfg).to_string(),
        );
        if is_stale(&stats) {
            fix_repairs.push("Daemon version");
        }
        checks.push(Check {
            label: "Daemon version",
            pass,
            detail,
            fix: fix_hint,
        });
        if healthy_daemon_reachable {
            checks.push(match crate::daemon::compiler_execution_status(cfg) {
                Ok(status) => Check {
                    label: "Compiler execution",
                    pass: status.eligible,
                    detail: status.detail,
                    fix: None,
                },
                Err(error) => Check {
                    label: "Compiler execution",
                    pass: false,
                    detail: error.to_string(),
                    fix: Some("kache daemon restart".into()),
                },
            });
        }
    }

    // 10. Daemon service installed
    if let Some(service_path) = crate::service::service_file_path() {
        let installed = service_path.exists();
        let (pass, fix) = daemon_service_check(installed, healthy_daemon_reachable);
        checks.push(Check {
            label: "Daemon service",
            pass,
            detail: if installed {
                term::home_path(&service_path)
            } else if healthy_daemon_reachable {
                "not installed; healthy on-demand daemon is reachable".into()
            } else {
                "not installed".into()
            },
            fix: fix.map(str::to_string),
        });
    }

    // Multiple cache instances may each own a daemon. Readiness above checks
    // this instance; a machine-wide process count cannot diagnose its health.

    // Startup and ownership locks are persistent. Never unlink their inodes,
    // even while idle: another process may already have opened the same file.

    // 11. The binary the service runs. After an upgrade removes the version
    //     it recorded, launchd or systemd has nothing to start.
    if let Some(service_path) = crate::service::service_file_path()
        && service_path.exists()
        && let Some(problem) = crate::service::service_exe_problem(&service_path)
    {
        checks.push(Check {
            label: "Service exe",
            pass: false,
            detail: problem.detail(),
            fix: Some("kache daemon install".into()),
        });
    }

    // Informational: the path shims and the service record for this binary,
    // and whether it outlives an upgrade.
    let (pass, detail, hint) = doctor_install_path_check(&crate::platform::frontend_selection());
    checks.push(Check {
        label: "Install path",
        pass,
        detail,
        fix: hint,
    });

    // Informational: rust-only setups skip the farm, so a miss here is not
    // an issue. Failures tell Make/PKGBUILD users why gcc is not kache.
    let shim_status = crate::compiler::shim::live_shim_status();
    checks.push(Check {
        label: "C/C++ shims",
        pass: shim_status.is_active(),
        detail: shim_status.detail(&|path| term::home_path(path)),
        fix: shim_status.fix(
            crate::compiler::shim::default_shim_dir().as_deref(),
            &|path| term::shell_home_path(path),
        ),
    });
    #[cfg(unix)]
    if let Some(dir) = crate::compiler::shim::default_shim_dir()
        && let Some(check) = stale_shims_check(&dir, &std::env::var_os("PATH").unwrap_or_default())
    {
        checks.push(check);
    }

    if let Some(ref cfg) = config
        && let Some(check) = doctor_checkout_check(&cfg.event_log_path())
    {
        checks.push(check);
    }

    // Compiler probe (#626): reported from the live toolchain, bypassing the
    // probe cache, so a stale stored "unresolved" record can't mask a fixed
    // toolchain — or a fresh breakage.
    checks.push(match probe_diag {
        crate::probe::LiveProbeDiagnostic::Resolved { version_line } => Check {
            label: "Compiler probe",
            pass: true,
            detail: format!("cc -### resolves ({version_line})"),
            fix: None,
        },
        crate::probe::LiveProbeDiagnostic::NoCompiler => Check {
            label: "Compiler probe",
            pass: false,
            detail: "no `cc` on PATH; configured or cross compilers were not checked".into(),
            fix: None,
        },
        crate::probe::LiveProbeDiagnostic::ProbeError { detail } => Check {
            label: "Compiler probe",
            pass: false,
            detail,
            fix: Some("fix the compiler diagnostic failure and rerun `kache doctor`".into()),
        },
        crate::probe::LiveProbeDiagnostic::Unresolved {
            version_line,
            stderr_head,
        } => Check {
            label: "Compiler probe",
            pass: false,
            detail: format!("`cc -###` resolved no compile line ({version_line})"),
            fix: Some(format!(
                "probe-keyed C/C++ flags will refuse to cache on this toolchain; \
                 report the `-###` output below\n{}",
                if stderr_head.is_empty() {
                    "(no -### stderr)"
                } else {
                    &stderr_head
                }
            )),
        },
    });

    // Native linker (#1663): a native Linux link keys the linker its driver
    // runs, so show the one this shell's `cc` and RUSTFLAGS select.
    if doctor_shows_native_linker(cfg!(target_os = "linux"), probe_no_compiler) {
        checks.push(native_linker_check(
            &crate::invocation::InvocationEnv::capture(),
        ));
    }

    if probe {
        checks.push(doctor_probe::check());
    }

    // Print
    let version = crate::VERSION;
    let rustc_version = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let issues = checks
        .iter()
        .filter(|c| is_doctor_issue(c.pass, check_is_optional(c.label)))
        .count();

    if json {
        #[derive(deser::Serialize)]
        struct CheckBody {
            label: &'static str,
            pass: bool,
            optional: bool,
            detail: String,
            fix: Option<String>,
        }
        #[derive(deser::Serialize)]
        struct Body {
            version: &'static str,
            rustc: String,
            issues: usize,
            checks: Vec<CheckBody>,
        }
        let failed: Vec<&str> = checks
            .iter()
            .filter(|c| is_doctor_issue(c.pass, check_is_optional(c.label)))
            .map(|c| c.label)
            .collect();
        let next = doctor_next_actions(&failed, &fix_repairs, &init_repairs);
        crate::machine::emit(
            "doctor",
            Body {
                version,
                rustc: rustc_version,
                issues,
                checks: checks
                    .iter()
                    .map(|c| CheckBody {
                        label: c.label,
                        pass: c.pass,
                        optional: check_is_optional(c.label),
                        detail: c.detail.clone(),
                        fix: c.fix.clone(),
                    })
                    .collect(),
            },
            next,
        )?;
        if fix {
            migrate(purge_sccache)?;
        }
        if verify && let Some(ref cfg) = config {
            let outcome = self::verify(cfg, checksums, repair)?;
            if doctor_has_issues(outcome.unresolved_integrity_findings()) {
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    println!("{}", term::heading("kache · doctor"));
    println!(
        "{}\n",
        term::paint(
            format!("  v{version} · {rustc_version}"),
            term::Style::Muted
        )
    );

    let check_results: Vec<(&str, bool)> = checks.iter().map(|c| (c.label, c.pass)).collect();
    let downgraded_daemon = daemon_footnote_needed(daemon_optional, &check_results);
    for section in ["Setup", "Storage", "Services"] {
        let mut rows = Vec::new();
        for check in checks
            .iter()
            .filter(|check| doctor_section(check.label) == section)
        {
            let (icon, style) = if check.pass {
                ("✓", term::Style::Success)
            } else if check_is_optional(check.label) {
                ("•", term::Style::Muted)
            } else {
                ("✗", term::Style::Error)
            };
            rows.push((check.label, term::paint(icon, style), check.detail.clone()));
            if let Some(fix) = &check.fix {
                rows.push(("", term::paint("→", term::Style::Warning), fix.clone()));
            }
        }
        for line in term::named_sections(&[(section, rows)]) {
            println!("{line}");
        }
        println!();
    }

    println!();
    if issues == 0 {
        println!("  \x1b[32mAll checks passed.\x1b[0m");
    } else {
        println!(
            "  \x1b[31m{} found.\x1b[0m",
            term::counted(issues as u64, "issue", "issues")
        );
    }
    if downgraded_daemon {
        println!(
            "  \x1b[2mDaemon checks are informational: the daemon is optional.\x1b[0m"
        );
    }
    println!();

    if fix {
        println!("Running migration...\n");
        migrate(purge_sccache)?;
    }

    // Cache integrity verification. Unresolved integrity findings make the
    // process exit non-zero so a scheduled `kache doctor --verify` can gate
    // a CI job (kunobi-ninja/kache#176). Orphan blobs are deliberately NOT
    // part of that condition: they are reclaimable space, not wrong bytes,
    // and GC clears them without anyone's intervention.
    if verify {
        if let Some(ref cfg) = config {
            println!();
            let outcome = self::verify(cfg, checksums, repair)?;
            let unresolved = outcome.unresolved_integrity_findings();
            if unresolved > 0 {
                anyhow::bail!(
                    "cache integrity check failed: {unresolved} corrupted \
                     {} remain{} (missing blobs: {}, checksum failures: {}){}",
                    if unresolved == 1 { "entry" } else { "entries" },
                    if unresolved == 1 { "s" } else { "" },
                    outcome.missing_blobs,
                    outcome.checksum_failures,
                    if repair {
                        " — `--repair` could not remove them"
                    } else {
                        " — rerun with `--repair` to remove them"
                    },
                );
            }
        } else {
            println!("  Cannot verify: no valid config found");
        }
    }

    Ok(())
}


fn doctor_has_issues(issues: usize) -> bool {
    issues > 0
}

/// Replace an sccache `build.rustc-wrapper` in the Cargo config at `path`
/// with kache, as `kache init` writes it, after backing the file up. Nothing
/// else in the file changes. Returns the wrapper it replaced.
fn migrate_cargo_wrapper(path: &std::path::Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let plan = plan_cargo_wrapper_edit(path)?;
    let CargoWrapperPlan::Replace(old) = &plan else {
        return Ok(None);
    };
    if !is_sccache_program(old) {
        return Ok(None);
    }
    let existing = std::fs::read_to_string(path)?;
    let updated = apply_cargo_wrapper_edit(&existing, &plan);
    ensure_cargo_wrapper_is_kache(&updated, path)?;
    back_up_cargo_config(path, &existing)?;
    std::fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(Some(old.clone()))
}

/// Migrate from sccache to kache (called by `doctor --fix`).
fn migrate(purge_sccache: bool) -> Result<()> {
    let home = kache_fs::home_dir().unwrap_or_default();
    let mut actions: Vec<String> = Vec::new();

    // 1. Stop sccache daemon if running
    if let Ok(output) = std::process::Command::new("pgrep")
        .args(["-x", "sccache"])
        .output()
        && output.status.success()
    {
        println!("Stopping sccache daemon...");
        let _ = std::process::Command::new("sccache")
            .arg("--stop-server")
            .status();
        actions.push("Stopped sccache daemon".into());
    }

    // 2. Point an sccache `build.rustc-wrapper` in $CARGO_HOME at kache.
    let cargo_dir = cargo_home_dir();
    for name in ["config.toml", "config"] {
        let cargo_config = cargo_dir.join(name);
        match migrate_cargo_wrapper(&cargo_config) {
            Ok(Some(old)) => actions.push(format!(
                "Replaced rustc-wrapper {old} with kache in {}",
                cargo_config.display()
            )),
            Ok(None) => {}
            Err(error) => println!("  Could not update {}: {error:#}", cargo_config.display()),
        }
    }

    // 3. Show what to change in shell rc
    let mut rc_changes: Vec<(String, Vec<(usize, String)>)> = Vec::new();
    for rc in [".zshrc", ".bashrc", ".bash_profile", ".profile"] {
        let rc_path = home.join(rc);
        if let Ok(content) = std::fs::read_to_string(&rc_path) {
            let sccache_lines: Vec<_> = content
                .lines()
                .enumerate()
                .filter(|(_, l)| l.contains("sccache") && !l.trim_start().starts_with('#'))
                .map(|(n, l)| (n + 1, l.to_string()))
                .collect();
            if !sccache_lines.is_empty() {
                rc_changes.push((rc.to_string(), sccache_lines));
            }
        }
    }

    // 4. Purge sccache cache and binary if requested
    if purge_sccache {
        // Remove sccache local cache
        let sccache_cache_dirs = [
            home.join("Library/Caches/Mozilla.sccache"), // macOS
            home.join(".cache/sccache"),                 // Linux
        ];
        for cache_dir in &sccache_cache_dirs {
            if cache_dir.exists() {
                let size = dir_size(cache_dir);
                std::fs::remove_dir_all(cache_dir)?;
                actions.push(format!(
                    "Removed sccache cache {} ({})",
                    cache_dir.display(),
                    kache_core::size::format_bytes(size)
                ));
            }
        }

        // Uninstall sccache binary if cargo-installed
        if let Ok(output) = std::process::Command::new("which").arg("sccache").output()
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let sccache_path = std::path::PathBuf::from(&path);
            let cargo_bin = cargo_dir.join("bin");
            let resolved_sccache = sccache_path.canonicalize().unwrap_or(sccache_path);
            let resolved_cargo_bin = cargo_bin.canonicalize().unwrap_or(cargo_bin);

            if resolved_sccache.starts_with(resolved_cargo_bin) {
                println!("Uninstalling sccache via cargo...");
                let status = std::process::Command::new("cargo")
                    .args(["uninstall", "sccache"])
                    .status();
                if status.map(|s| s.success()).unwrap_or(false) {
                    actions.push("Uninstalled sccache (cargo uninstall)".into());
                }
            } else {
                actions.push(format!(
                    "sccache at {path} not cargo-installed — remove manually if desired"
                ));
            }
        }
    }

    print!(
        "{}",
        migration_summary(&actions, &rc_changes, purge_sccache)
    );
    Ok(())
}

/// What `doctor --fix` did and what is left for the user, as printed text.
fn migration_summary(
    actions: &[String],
    rc_changes: &[(String, Vec<(usize, String)>)],
    purge_sccache: bool,
) -> String {
    use std::fmt::Write;
    let mut out = String::from("\nMigration summary:\n");
    if actions.is_empty() && rc_changes.is_empty() {
        out.push_str("  No sccache configuration found. Nothing to migrate.\n");
        out.push_str("\n  If Cargo is not using kache yet, run `kache init`.\n");
        return out;
    }

    for action in actions {
        let _ = writeln!(out, "  ✓ {action}");
    }

    if !rc_changes.is_empty() {
        out.push_str("\n  Manual changes needed in shell rc files:\n");
        for (rc, lines) in rc_changes {
            let _ = writeln!(out, "\n  ~/{rc}:");
            for (line_num, line) in lines {
                let trimmed = line.trim();
                if trimmed.starts_with("export RUSTC_WRAPPER") {
                    // RUSTC_WRAPPER line → replace with kache
                    let _ = writeln!(out, "    line {line_num}:\n      - {line}");
                    out.push_str("      + export RUSTC_WRAPPER=kache\n");
                } else if trimmed.starts_with("export SCCACHE_") {
                    // SCCACHE_* env vars → remove (not relevant to kache)
                    let _ = writeln!(out, "    line {line_num}: (remove)\n      - {line}");
                } else {
                    // Other sccache references → flag for manual review
                    let _ = writeln!(out, "    line {line_num}: (review)\n      {line}");
                }
            }
        }
        out.push_str("\n  After editing, open a new terminal so the changes apply.\n");
    }

    if !purge_sccache {
        out.push_str(
            "\n  To also remove the sccache cache and binary, run `kache doctor --fix --purge-sccache`.\n",
        );
    }

    out.push_str("\n  Then verify with `kache doctor`.\n");
    out
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut size = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                size += dir_size(&p);
            } else if let Ok(meta) = p.metadata() {
                size += meta.len();
            }
        }
    }
    size
}

/// Verify cache integrity: check all entries and blobs for consistency.
/// Outcome of a store integrity verification pass (kunobi-ninja/kache#176).
///
/// Separates **integrity** findings — a store that can serve broken or lost
/// data — from **reclaimable** ones (orphan blobs are wasted space, never
/// wrong bytes), because only the former should fail a CI run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub total_entries: usize,
    pub valid_entries: usize,
    /// Entries whose metadata or blobs did not check out.
    pub corrupted_entries: usize,
    /// Referenced blob files absent from the store.
    pub missing_blobs: usize,
    /// Blobs whose bytes no longer match their content address.
    pub checksum_failures: usize,
    /// On-disk blobs no entry references — space, not corruption.
    pub orphaned_blobs: usize,
    /// Corrupted entries `--repair` actually removed.
    pub corrupted_removed: usize,
    /// Derived blob-index rows that disagree with committed entry metadata.
    pub index_drift: usize,
}

impl VerifyOutcome {
    /// Integrity problems still present when the pass finished — what a CI
    /// run must fail on. Repair removes corrupted entries, so anything it
    /// could not remove still counts (kunobi-ninja/kache#176).
    pub fn unresolved_integrity_findings(&self) -> usize {
        self.corrupted_entries - self.corrupted_removed + self.index_drift
    }
}

fn reconciled_index_message(drift: crate::store::BlobIndexDrift) -> Option<String> {
    (drift.total() != 0).then(|| {
        format!(
            "Repairing: reconciled {} entry mappings and {} blob rows.",
            drift.entry_mappings, drift.blobs
        )
    })
}

fn should_print_repair_tip(
    corrupted_entries: usize,
    orphaned_blobs: usize,
    index_drift: usize,
    repair: bool,
) -> bool {
    let findings = corrupted_entries
        .saturating_add(orphaned_blobs)
        .saturating_add(index_drift);
    !repair && findings != 0
}

/// Hash each unique blob **once**, streaming, across a bounded worker pool,
/// and return the hashes whose bytes no longer match their content address
/// (kunobi-ninja/kache#176).
///
/// The previous scrub read every blob with `fs::read` once per *referencing
/// entry*: a blob shared by N entries was fully buffered and hashed N times,
/// serially — the dedup that makes the store cheap made verification
/// quadratic in exactly the stores that need it most. Streaming keeps RSS
/// flat on multi-hundred-MB rlibs, and the work is embarrassingly parallel.
///
/// Unreadable blobs are reported as failures: a blob that cannot be read is
/// as unusable as one that hashes wrong.
fn scrub_blob_checksums(
    blobs: &[(String, std::path::PathBuf)],
) -> std::collections::HashSet<String> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    if blobs.is_empty() {
        return std::collections::HashSet::new();
    }
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
        .min(blobs.len());
    let cursor = AtomicUsize::new(0);
    let failures = Mutex::new(std::collections::HashSet::new());

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let idx = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some((hash, path)) = blobs.get(idx) else {
                        return;
                    };
                    let computed = std::fs::File::open(path).and_then(|file| {
                        let mut hasher = blake3::Hasher::new();
                        hasher.update_reader(file)?;
                        Ok(hasher.finalize().to_hex().to_string())
                    });
                    match computed {
                        Ok(computed) if &computed == hash => {}
                        Ok(computed) => {
                            log::warn!(
                                "blob {} checksum mismatch (computed {})",
                                &hash[..16.min(hash.len())],
                                &computed[..16.min(computed.len())]
                            );
                            failures.lock().expect("scrub mutex").insert(hash.clone());
                        }
                        Err(e) => {
                            log::warn!("blob {} unreadable: {e}", &hash[..16.min(hash.len())]);
                            failures.lock().expect("scrub mutex").insert(hash.clone());
                        }
                    }
                }
            });
        }
    });

    failures.into_inner().expect("scrub mutex")
}

pub fn verify(config: &Config, checksums: bool, repair: bool) -> Result<VerifyOutcome> {
    let mut total = verify_store(config, checksums, repair)?;
    for shard in crate::store_view::shard_configs(config) {
        anyhow::ensure!(
            crate::volume_gc::shard_has_index(&shard.cache_dir),
            "cannot verify volume store {}: no index (not initialized or volume unavailable)",
            shard.cache_dir.display()
        );
        println!("\nVolume store: {}", shard.cache_dir.display());
        let outcome = verify_store(&shard, checksums, repair)?;
        total.total_entries += outcome.total_entries;
        total.valid_entries += outcome.valid_entries;
        total.corrupted_entries += outcome.corrupted_entries;
        total.missing_blobs += outcome.missing_blobs;
        total.checksum_failures += outcome.checksum_failures;
        total.orphaned_blobs += outcome.orphaned_blobs;
        total.corrupted_removed += outcome.corrupted_removed;
        total.index_drift += outcome.index_drift;
    }
    Ok(total)
}

fn verify_store(config: &Config, checksums: bool, repair: bool) -> Result<VerifyOutcome> {
    let store = Store::open(config)?;

    // Adopt entries the index doesn't know about before verifying it (#415).
    // `verify` walks the *index*, so an index that lost its rows (quarantined
    // after corruption, or deleted) looks empty and clean while a full store of
    // artifacts sits on disk unreferenced. Rebuilding first is what makes
    // `--repair` able to fix the corruption case rather than just describe it.
    if repair {
        match store.rebuild_index_from_store() {
            Ok(stats) if stats.entries_rebuilt > 0 => println!(
                "Adopted {} unreferenced cache {} from the store ({} blob references).",
                stats.entries_rebuilt,
                if stats.entries_rebuilt == 1 {
                    "entry"
                } else {
                    "entries"
                },
                stats.blobs_registered,
            ),
            Ok(_) => {}
            Err(e) => eprintln!("warning: could not rebuild index rows from the store: {e:#}"),
        }
    }

    let entries = store.list_entries("name")?;
    let store_dir = config.store_dir();
    let blobs_dir = store_dir.join("blobs");

    let mut total_entries: usize = 0;
    let mut valid_entries: usize = 0;
    let mut corrupted_entries: usize = 0;
    let mut missing_blobs: usize = 0;
    let mut corrupted_keys: Vec<String> = Vec::new();
    // Unique blobs to checksum, and which entries reference each — collected
    // during the walk so the scrub hashes every blob ONCE regardless of how
    // many entries share it (kunobi-ninja/kache#176).
    let mut blobs_to_scrub: std::collections::HashMap<String, std::path::PathBuf> =
        std::collections::HashMap::new();
    let mut entries_by_blob: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    // Per-entry state, resolved after the scrub.
    let mut entry_keys: Vec<String> = Vec::new();
    let mut entry_ok_flags: Vec<bool> = Vec::new();

    // Track all blob hashes referenced by valid entries
    let mut referenced_blobs: std::collections::HashSet<String> = std::collections::HashSet::new();

    println!("Verifying {} cache entries...", entries.len());

    for (entry_index, entry) in entries.iter().enumerate() {
        total_entries += 1;
        entry_keys.push(entry.cache_key.clone());
        entry_ok_flags.push(true);

        let entry_dir = store_dir.join(&entry.cache_key);
        let meta_path = entry_dir.join("meta.json");

        // Check metadata file exists and parses
        let meta = match std::fs::read_to_string(&meta_path) {
            Ok(content) => match deser_json::from_str::<crate::store::EntryMeta>(&content) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!(
                        "entry {} has invalid meta.json: {e}",
                        &entry.cache_key[..16.min(entry.cache_key.len())]
                    );
                    corrupted_entries += 1;
                    corrupted_keys.push(entry.cache_key.clone());
                    continue;
                }
            },
            Err(e) => {
                log::warn!(
                    "entry {} missing meta.json: {e}",
                    &entry.cache_key[..16.min(entry.cache_key.len())]
                );
                corrupted_entries += 1;
                corrupted_keys.push(entry.cache_key.clone());
                continue;
            }
        };

        // Check all referenced blob files exist and optionally verify checksums
        let mut entry_ok = true;
        for cached_file in &meta.files {
            let blob_path = store.blob_path(&cached_file.hash);

            if !blob_path.is_file() {
                log::warn!(
                    "entry {} missing blob {} (file: {})",
                    &entry.cache_key[..16.min(entry.cache_key.len())],
                    &cached_file.hash[..16.min(cached_file.hash.len())],
                    cached_file.name
                );
                missing_blobs += 1;
                entry_ok = false;
                continue;
            }

            // Size check
            if let Ok(file_meta) = std::fs::metadata(&blob_path)
                && file_meta.len() != cached_file.size
            {
                log::warn!(
                    "entry {} blob {} size mismatch (expected {}, got {})",
                    &entry.cache_key[..16.min(entry.cache_key.len())],
                    &cached_file.hash[..16.min(cached_file.hash.len())],
                    cached_file.size,
                    file_meta.len()
                );
                entry_ok = false;
                continue;
            }

            // Checksums are deferred to one deduplicated, parallel,
            // streaming pass after the walk (kunobi-ninja/kache#176) — here
            // we only record which unique blobs to scrub and who references
            // them.
            if checksums {
                blobs_to_scrub
                    .entry(cached_file.hash.clone())
                    .or_insert_with(|| blob_path.clone());
                entries_by_blob
                    .entry(cached_file.hash.clone())
                    .or_default()
                    .push(entry_index);
            }

            referenced_blobs.insert(cached_file.hash.clone());
        }

        if !entry_ok {
            entry_ok_flags[entry_index] = false;
        }
    }

    // One deduplicated, streaming, bounded-parallel scrub of every unique
    // blob, then attribute each failure back to the entries referencing it
    // (kunobi-ninja/kache#176).
    // Scrub whatever the walk collected. Deliberately UNGUARDED: the walk
    // only collects when `--checksums` asked for it, so a second `if
    // checksums` here would be redundant — and two guards on one condition
    // make each individually unobservable, which is how a mutation gate
    // reports an "equivalent" mutant that is really a missing test.
    let work: Vec<(String, std::path::PathBuf)> = blobs_to_scrub.into_iter().collect();
    let blobs_scrubbed = work.len();
    let failed = scrub_blob_checksums(&work);
    let checksum_failures = failed.len();
    for hash in &failed {
        for idx in entries_by_blob.get(hash).into_iter().flatten() {
            entry_ok_flags[*idx] = false;
        }
    }

    for (idx, ok) in entry_ok_flags.iter().enumerate() {
        if *ok {
            valid_entries += 1;
        } else {
            corrupted_entries += 1;
            corrupted_keys.push(entry_keys[idx].clone());
        }
    }

    // Scan for orphaned blobs (on-disk blobs not referenced by any entry)
    let mut total_blobs_on_disk: usize = 0;
    let mut orphaned_blobs: usize = 0;

    if blobs_dir.exists()
        && let Ok(prefix_dirs) = std::fs::read_dir(&blobs_dir)
    {
        for prefix_entry in prefix_dirs.flatten() {
            if !prefix_entry.path().is_dir() {
                continue;
            }
            if let Ok(blob_files) = std::fs::read_dir(prefix_entry.path()) {
                for blob_entry in blob_files.flatten() {
                    let path = blob_entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    total_blobs_on_disk += 1;
                    if let Some(name) = path.file_name().and_then(|n| n.to_str())
                        && !referenced_blobs.contains(name)
                    {
                        orphaned_blobs += 1;
                    }
                }
            }
        }
    }

    // Repair: remove corrupted entries. Count what actually went: an entry
    // repair could not remove is still an unresolved finding, and the exit
    // status must say so (kunobi-ninja/kache#176).
    let mut corrupted_removed: usize = 0;
    if repair && !corrupted_keys.is_empty() {
        println!(
            "Repairing: removing {} corrupted entries...",
            corrupted_keys.len()
        );
        for key in &corrupted_keys {
            match store.remove_entry(key) {
                Ok(()) => corrupted_removed += 1,
                Err(e) => log::warn!(
                    "failed to remove corrupted entry {}: {e}",
                    &key[..16.min(key.len())]
                ),
            }
        }
    }

    // `entries` plus each committed meta.json are authoritative; `blobs` and
    // `entry_blobs` are derived acceleration structures. Compare them under
    // the store write lock so a concurrent publisher/remover cannot create a
    // transient mismatch, and rebuild them atomically when requested (#819).
    let index_drift = if repair {
        // Dropping, not strict: an entry whose meta.json cannot be parsed is
        // one `remove_entry` above refuses to touch (#276), and a strict
        // rebuild would then refuse the whole store because of it.
        match store.reconcile_blob_index_dropping_unverifiable() {
            Ok((drift, dropped)) => {
                // A corrupted entry `remove_entry` refused is gone now too.
                corrupted_removed += dropped
                    .iter()
                    .filter(|key| corrupted_keys.contains(key))
                    .count();
                if !dropped.is_empty() {
                    println!(
                        "Repairing: removed {} that failed verification.",
                        term::counted(dropped.len() as u64, "entry", "entries")
                    );
                }
                if let Some(message) = reconciled_index_message(drift) {
                    println!("{message}");
                }
                0
            }
            Err(e) => {
                log::warn!("blob index reconciliation failed: {e:#}");
                1
            }
        }
    } else if corrupted_entries == 0 {
        match store.blob_index_drift() {
            Ok(drift) => drift.total(),
            Err(e) => {
                log::warn!("blob index verification failed: {e:#}");
                1
            }
        }
    } else {
        // Corrupt metadata is already an unresolved integrity finding and
        // cannot safely serve as the source of truth for a graph comparison.
        0
    };

    // Repair: reclaim orphaned blob files (counted above). These are never
    // reclaimed by normal GC, so without this they leak invisibly to
    // size-based eviction. A small grace leaves any blob a concurrent build
    // is materializing untouched.
    if repair && orphaned_blobs > 0 {
        match store.sweep_orphan_blobs(std::time::Duration::from_secs(60)) {
            Ok(swept) => println!(
                "Repairing: reclaimed {} orphan blobs ({})",
                swept.removed,
                kache_core::size::format_bytes(swept.bytes_reclaimed)
            ),
            Err(e) => log::warn!("orphan-blob sweep failed: {e}"),
        }
    }

    // Repair: reclaim put-phase staging snapshots abandoned by a crash
    // between staging and publish (review finding #3). Reported even when
    // empty so the repair narrative always accounts for every reclaim pass.
    //
    // The grace matches the daemon's GC sweep deliberately: a staging file
    // belonging to a put running in ANOTHER process is indistinguishable
    // from a crash leftover, and unlinking one fails that put at publish
    // time. An hour is long enough that only a dead process's snapshot is
    // ever old enough to reclaim.
    if repair {
        let swept_staging = store.sweep_stale_staging(STAGING_SWEEP_GRACE);
        println!(
            "Repairing: reclaimed {} stale staging files ({})",
            swept_staging.removed,
            kache_core::size::format_bytes(swept_staging.bytes_reclaimed)
        );
        let handoffs = crate::daemon_publish::sweep_orphaned_handoffs(config, STAGING_SWEEP_GRACE);
        println!(
            "Repairing: reclaimed {} abandoned cc handoffs ({})",
            handoffs.removed,
            kache_core::size::format_bytes(handoffs.bytes_reclaimed)
        );
        let memos = store.file_hash_cache();
        match memos.prune_cc_preprocess_memos() {
            Ok((removed, inputs)) if removed + inputs > 0 => println!(
                "Repairing: removed {removed} stale C/C++ memos and {inputs} unreferenced inputs"
            ),
            Ok(_) => {}
            Err(error) => eprintln!("warning: could not prune C/C++ memos: {error}"),
        }
        match memos.prune_input_predictions() {
            Ok(removed) => println!("Repairing: removed {removed} unused input predictions"),
            Err(error) => eprintln!("warning: could not prune input predictions: {error}"),
        }
        match memos.prune_file_hashes() {
            Ok(removed) => println!("Repairing: removed {removed} old file hashes"),
            Err(error) => eprintln!("warning: could not prune file hashes: {error}"),
        }
        match memos.compact_sparse_index() {
            Ok(Some((before, after))) => println!(
                "Repairing: compacted index file from {} to {}",
                kache_core::size::format_bytes(before),
                kache_core::size::format_bytes(after)
            ),
            Ok(None) => {}
            Err(error) => eprintln!("warning: index compaction deferred: {error:#}"),
        }
    }

    // Compute store size
    let store_size = store.total_size().unwrap_or(0);

    println!();
    println!("Cache verification complete");
    println!(
        "  Entries: {} total, {} valid, {} corrupted",
        total_entries, valid_entries, corrupted_entries
    );
    println!(
        "  Blobs: {} total, {} orphaned, {} missing, {} scrubbed, {} checksum failures",
        total_blobs_on_disk, orphaned_blobs, missing_blobs, blobs_scrubbed, checksum_failures
    );
    println!("  Blob index drift: {index_drift}");
    println!("  Store size: {}", kache_core::size::format_bytes(store_size));

    if should_print_repair_tip(corrupted_entries, orphaned_blobs, index_drift, repair) {
        println!();
        println!(
            "Run `kache doctor --repair` to remove corrupted entries, reconcile the blob index, and reclaim orphaned blobs."
        );
    }

    Ok(VerifyOutcome {
        total_entries,
        valid_entries,
        corrupted_entries,
        missing_blobs,
        checksum_failures,
        orphaned_blobs,
        corrupted_removed,
        index_drift,
    })
}

#[cfg(test)]
mod tests;

// ── Init ──────────────────────────────────────────────────────────────────
//
// Interactive setup that resolves the common doctor issues:
//   1. Writes `build.rustc-wrapper = "kache"` to $CARGO_HOME/config.toml
//      (fallback to ~/.cargo/config.toml)
//   1b. Adds HOST_CC / HOST_CXX / CC_KNOWN_WRAPPER_CUSTOM under `[env]`
//       when those keys are absent. Never sets CC or CXX.
//   1c. Unix: offers compiler-name shims in ~/.local/lib/kache/shims.
//       Saves shell PATH setup after confirmation; activation needs a new terminal.
//   1d. Unix: offers CARGO_TARGET_<HOST>_RUNNER="kache test-runner" in the
//       same startup files, so test binaries share the scheduler's slots.
//   2. Installs the daemon as a login service (launchd/systemd)
//   3. Starts the daemon
//
// Each step is skipped if already satisfied, so re-running is safe.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CargoWrapperPlan {
    /// File doesn't exist — create it with a fresh `[build]` section.
    Create,
    /// File exists but has a different wrapper (e.g. sccache) — replace the value.
    Replace(String),
    /// File has a `[build]` section but no `rustc-wrapper` — insert the key.
    AddUnderBuild,
    /// File exists with no `[build]` section — append one.
    AppendSection,
    /// Already set to kache.
    AlreadySet,
}

pub(crate) fn plan_cargo_wrapper_edit(path: &std::path::Path) -> Result<CargoWrapperPlan> {
    if !path.exists() {
        return Ok(CargoWrapperPlan::Create);
    }
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed: deser_value::Value =
        deser_toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    let current = parsed
        .get("build")
        .and_then(|b| b.get("rustc-wrapper"))
        .and_then(|v| v.as_str());
    match current {
        Some("kache") => Ok(CargoWrapperPlan::AlreadySet),
        Some(other) => Ok(CargoWrapperPlan::Replace(other.to_string())),
        None if parsed.get("build").is_some() => Ok(CargoWrapperPlan::AddUnderBuild),
        None => Ok(CargoWrapperPlan::AppendSection),
    }
}

pub(crate) fn apply_cargo_wrapper_edit(existing: &str, plan: &CargoWrapperPlan) -> String {
    match plan {
        CargoWrapperPlan::AlreadySet => existing.to_string(),
        CargoWrapperPlan::Create => "[build]\nrustc-wrapper = \"kache\"\n".into(),
        CargoWrapperPlan::Replace(old) => {
            // Try each quoting style; fall back to just single-line textual replace.
            let candidates = [
                format!("rustc-wrapper = \"{old}\""),
                format!("rustc-wrapper = '{old}'"),
                format!("rustc-wrapper=\"{old}\""),
            ];
            for cand in &candidates {
                if existing.contains(cand) {
                    return existing.replacen(cand, "rustc-wrapper = \"kache\"", 1);
                }
            }
            existing.to_string()
        }
        CargoWrapperPlan::AddUnderBuild => {
            let mut out = String::with_capacity(existing.len() + 32);
            let mut inserted = false;
            for line in existing.lines() {
                out.push_str(line);
                out.push('\n');
                if !inserted && line.trim() == "[build]" {
                    out.push_str("rustc-wrapper = \"kache\"\n");
                    inserted = true;
                }
            }
            if !inserted {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("\n[build]\nrustc-wrapper = \"kache\"\n");
            }
            out
        }
        CargoWrapperPlan::AppendSection => {
            let mut out = existing.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("[build]\nrustc-wrapper = \"kache\"\n");
            out
        }
    }
}

/// Install the login service without failing init. A machine that cannot
/// register one (Task Scheduler without admin rights on Windows, a Linux host
/// whose systemd refuses the unit) still gets the daemon started by the next
/// init step (#1080).
fn install_login_service() -> bool {
    match crate::service::install() {
        Ok(()) => true,
        Err(error) => {
            println!("  • Login service: not installed ({error:#})");
            false
        }
    }
}

/// Set when a prompt found stdin closed, so `init` can tell "declined" apart
/// from "nobody was there to answer" (#1080).
static PROMPT_HAD_NO_INPUT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn prompt_yes_no(question: &str, default_yes: bool, auto_yes: bool) -> Result<bool> {
    use std::io::{BufRead, Write};

    if auto_yes {
        return Ok(true);
    }
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("  {} {suffix} ", term::paint(question, term::Style::Value));
    std::io::stdout().flush().ok();

    let stdin = std::io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line)? == 0 {
        println!("no answer");
        PROMPT_HAD_NO_INPUT.store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(false);
    }
    let trimmed = line.trim().to_ascii_lowercase();
    if !stdin.is_terminal() {
        println!();
    }
    if trimmed.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(trimmed.as_str(), "y" | "yes"))
}

/// `$CARGO_HOME`, falling back to `~/.cargo` (cargo's documented default).
pub(crate) fn cargo_home_dir() -> std::path::PathBuf {
    if let Some(cargo_home) = std::env::var_os("CARGO_HOME").filter(|value| !value.is_empty()) {
        let cargo_home = std::path::PathBuf::from(cargo_home);
        if cargo_home.is_absolute() {
            cargo_home
        } else {
            std::env::current_dir().unwrap_or_default().join(cargo_home)
        }
    } else {
        kache_fs::home_dir().unwrap_or_default().join(".cargo")
    }
}

fn cargo_config_target_path() -> std::path::PathBuf {
    let cargo_dir = cargo_home_dir();
    let with_ext = cargo_dir.join("config.toml");
    let legacy = cargo_dir.join("config");
    // Prefer the file that already exists; fall back to the canonical name.
    if legacy.exists() && !with_ext.exists() {
        legacy
    } else {
        with_ext
    }
}

/// Refuse an edit that did not leave `build.rustc-wrapper = "kache"`: the
/// formatting-preserving editor cannot replace every way of writing the old
/// value.
fn ensure_cargo_wrapper_is_kache(updated: &str, cargo_path: &std::path::Path) -> Result<()> {
    let parsed: deser_value::Value = deser_toml::from_str(updated)?;
    anyhow::ensure!(
        parsed
            .get("build")
            .and_then(|v| v.get("rustc-wrapper"))
            .and_then(|value| value.as_str())
            == Some("kache"),
        "could not update Cargo's existing wrapper; edit {} manually",
        cargo_path.display()
    );
    Ok(())
}

/// Keep a copy of the Cargo config beside it before init edits it.
fn back_up_cargo_config(cargo_path: &std::path::Path, existing: &str) -> Result<()> {
    use std::io::Write;
    let mut backup = tempfile::Builder::new()
        .prefix(".kache-cargo-backup-")
        .tempfile_in(cargo_path.parent().context("Cargo config has no parent")?)?;
    backup.write_all(existing.as_bytes())?;
    backup.as_file().sync_all()?;
    let (_, backup) = backup.keep()?;
    println!(
        "    Backup: {}",
        crate::wrapper_config::display_path(&backup)
    );
    Ok(())
}

#[derive(Debug, Clone, deser::Serialize)]
struct SetupStep {
    component: &'static str,
    action: &'static str,
    paths: Vec<std::path::PathBuf>,
}

impl SetupStep {
    fn new(component: &'static str, action: &'static str, paths: Vec<std::path::PathBuf>) -> Self {
        Self {
            component,
            action,
            paths,
        }
    }
}

#[derive(deser::Serialize)]
struct SetupResult {
    preview: bool,
    steps: Vec<SetupStep>,
    cargo_configured: bool,
    shell_activation_required: bool,
    test_activation_required: bool,
    service_installed: bool,
    daemon_running: Option<bool>,
    #[deser(skip_serializing_if = Option::is_none)]
    compiler_execution: Option<kache_protocol::execution::ExecutionStatus>,
}

/// What `kache init` found before it changes anything, or what a run did,
/// for [`setup_steps`].
#[derive(Clone)]
struct SetupFacts {
    cargo_path: std::path::PathBuf,
    /// `keep`, `configure`, or `skip` when the replacement was declined.
    cargo: &'static str,
    cargo_exists: bool,
    /// The `shell` step, and the startup files it changes with whether each
    /// exists now.
    shell: Option<SetupStep>,
    shell_changes: Vec<(std::path::PathBuf, bool)>,
    test_runner: Option<SetupStep>,
    test_changes: Vec<(std::path::PathBuf, bool)>,
    /// The `login_service` step's action; `None` with `--no-service`.
    service: Option<&'static str>,
    service_path: Option<std::path::PathBuf>,
}

/// The JSON `steps` of `kache init`, in the order init takes them, each
/// edit of an existing file after its backup step. Pure.
fn setup_steps(facts: &SetupFacts) -> Vec<SetupStep> {
    let facts = facts.clone();
    let mut created = Vec::new();
    let mut steps = vec![SetupStep::new(
        "cargo",
        facts.cargo,
        vec![facts.cargo_path.clone()],
    )];
    if step_changes(facts.cargo) {
        steps.extend(backup_step(
            &mut created,
            &facts.cargo_path,
            facts.cargo_exists,
        ));
    }
    steps.extend(facts.shell);
    for (path, exists) in &facts.shell_changes {
        steps.extend(backup_step(&mut created, path, *exists));
    }
    steps.extend(facts.test_runner);
    for (path, exists) in &facts.test_changes {
        steps.extend(backup_step(&mut created, path, *exists));
    }
    if let Some(action) = facts.service {
        steps.push(SetupStep::new(
            "login_service",
            action,
            facts.service_path.into_iter().collect(),
        ));
    }
    steps.push(SetupStep::new("daemon", "ensure_running", Vec::new()));
    steps
}

/// What a run did, against the plan it started from.
struct RunOutcome {
    /// Whether the Cargo edit was made; false when it was declined.
    cargo_applied: bool,
    /// The test-runner step as planned after the compiler setup, which may
    /// have written the same startup files.
    test_runner: Option<SetupStep>,
    test_changes: Vec<(std::path::PathBuf, bool)>,
    /// Whether the login service was installed or updated now.
    service_installed_now: bool,
}

/// [`SetupFacts`] for what a run did: a declined or failed step is not
/// reported as done. Pure.
fn run_facts(plan: &SetupFacts, outcome: RunOutcome) -> SetupFacts {
    let cargo = match plan.cargo {
        "configure" if !outcome.cargo_applied => "skip",
        planned => planned,
    };
    let service = match plan.service {
        Some("configure" | "update") if !outcome.service_installed_now => Some("failed"),
        planned => planned,
    };
    SetupFacts {
        cargo,
        test_runner: outcome.test_runner,
        test_changes: outcome.test_changes,
        service,
        ..plan.clone()
    }
}

/// Whether a step with this action changes files or services.
fn step_changes(action: &str) -> bool {
    matches!(action, "configure" | "update")
}

/// `configure` when an edit would change a file or `extra_change` is set,
/// such as missing shims; else `keep`.
#[cfg(any(test, unix))]
fn edit_action(changes_a_file: bool, extra_change: bool) -> &'static str {
    if extra_change || changes_a_file {
        "configure"
    } else {
        "keep"
    }
}

/// `kache init` with the flags that change what it sets up, for the line
/// that ends a preview.
fn init_command(no_service: bool, no_shell: bool, yes: bool) -> String {
    let mut argv = vec!["kache", "init"];
    for (set, flag) in [
        (no_service, "--no-service"),
        (no_shell, "--no-shell"),
        (yes, "--yes"),
    ] {
        if set {
            argv.push(flag);
        }
    }
    argv.join(" ")
}

/// What the login-service step does: `None` with `--no-service`.
fn service_action(
    no_service: bool,
    available: bool,
    installed: bool,
    runs_another_binary: bool,
) -> Option<&'static str> {
    if no_service {
        None
    } else if !available {
        Some("unavailable")
    } else if runs_another_binary {
        Some("update")
    } else if installed {
        Some("keep")
    } else {
        Some("configure")
    }
}

pub fn init(yes: bool, no_service: bool, no_shell: bool, check: bool) -> Result<()> {
    println!("{}\n", term::heading("kache · setup"));

    let cargo_path = cargo_config_target_path();
    let plan = plan_cargo_wrapper_edit(&cargo_path)?;
    let existing = if plan == CargoWrapperPlan::Create {
        String::new()
    } else {
        std::fs::read_to_string(&cargo_path).context("read Cargo configuration")?
    };
    let env_missing = crate::cargo_env::missing_assignments_from_path(&cargo_path)?;
    let mut cargo_ready = plan == CargoWrapperPlan::AlreadySet && env_missing.is_empty();

    // Plan every step before asking, so the plan, the preview and the JSON
    // steps all describe what this run does.
    let existing_paths = |paths: Vec<std::path::PathBuf>| {
        paths
            .into_iter()
            .map(|path| {
                let exists = path.exists();
                (path, exists)
            })
            .collect::<Vec<_>>()
    };
    #[cfg(unix)]
    let compiler = plan_compiler_setup(no_shell)?;
    #[cfg(unix)]
    let tests = plan_test_runner(no_shell)?;
    #[cfg(unix)]
    let shell_facts = (
        compiler.step(),
        existing_paths(compiler.changed_paths()),
        tests.step(),
        existing_paths(tests.changed_paths()),
    );
    #[cfg(not(unix))]
    let shell_facts = {
        let _ = (no_shell, existing_paths);
        (None, Vec::new(), None, Vec::new())
    };
    let (shell, shell_changes, test_runner, test_changes) = shell_facts;
    // Asking runs the service manager, which --no-service must not touch.
    let service_available = !no_service && crate::service::login_service_available();
    let service_path = crate::service::service_file_path();
    let service_installed = service_path.as_ref().is_some_and(|p| p.exists());
    let service_problem = service_path
        .as_deref()
        .filter(|p| p.exists())
        .and_then(crate::service::service_exe_problem);
    let plan_facts = SetupFacts {
        cargo: if cargo_ready { "keep" } else { "configure" },
        cargo_exists: cargo_path.exists(),
        cargo_path: cargo_path.clone(),
        shell,
        shell_changes,
        test_runner,
        test_changes,
        service: service_action(
            no_service,
            service_available,
            service_installed,
            service_problem.is_some(),
        ),
        service_path: service_path.clone(),
    };
    let steps = setup_steps(&plan_facts);
    #[cfg(unix)]
    let (shell_supported, shell_activation, test_activation) = (
        compiler.supported(),
        compiler.activation_required(),
        tests.activation_required(),
    );
    #[cfg(not(unix))]
    let (shell_supported, shell_activation, test_activation) = (false, false, false);
    let changes = steps.iter().any(|step| step_changes(step.action));

    if !check {
        let mut plan_rows = vec![
            (
                "Cargo",
                "caching".into(),
                "Rust and native dependencies".into(),
            ),
            ("Config", term::home_path(&cargo_path), String::new()),
        ];
        if !no_shell && shell_supported {
            plan_rows.push((
                "Shell",
                "new terminals".into(),
                "C/C++ caching and test pacing".into(),
            ));
            for step in steps.iter().filter(|step| step.component == "shell") {
                for path in &step.paths {
                    plan_rows.push(("Shell file", term::home_path(path), String::new()));
                }
            }
        }
        if !no_service {
            plan_rows.push((
                "Service",
                "login".into(),
                "start kache at login when a login service is available".into(),
            ));
        }
        plan_rows.push((
            "Daemon",
            "background".into(),
            "start or restart the cache if needed".into(),
        ));
        for line in term::named_sections(&[("Setup plan", plan_rows)]) {
            println!("{line}");
        }
        println!();
        if !prompt_yes_no("Apply this setup?", true, yes)? {
            if PROMPT_HAD_NO_INPUT.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(retry_with_yes(
                    "init changed nothing",
                    "stdin had no answers",
                    &command_args(),
                    true,
                ));
            }
            println!("  Setup cancelled. No changes made.");
            return Ok(());
        }
    }
    println!(
        "{}",
        term::heading(if check && changes {
            "Proposed changes"
        } else {
            "Setup"
        })
    );
    // Files this run created: no backup of those.
    let mut created = Vec::new();
    let mut cargo_applied = false;
    if cargo_ready {
        println!("  ✓ Cargo caching: configured");
    } else {
        println!("  Cargo caching: Rust and native dependencies");
        println!(
            "    Config: {}",
            crate::wrapper_config::display_path(&cargo_path)
        );
        if check {
            println!("    Would configure Cargo. Existing compiler choices are preserved.");
            if let CargoWrapperPlan::Replace(old) = &plan {
                println!("    Would replace {old} with kache for Cargo builds.");
            }
        } else if match &plan {
            CargoWrapperPlan::Replace(old) => prompt_yes_no(
                &format!("Replace {old} with kache for Cargo builds?"),
                true,
                yes,
            )?,
            _ => true,
        } {
            let wrapped = apply_cargo_wrapper_edit(&existing, &plan);
            let updated = crate::cargo_env::apply_cargo_env_edit(&wrapped, &env_missing);
            ensure_cargo_wrapper_is_kache(&updated, &cargo_path)?;
            if let Some(parent) = cargo_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if backup_step(&mut created, &cargo_path, cargo_path.exists()).is_some() {
                back_up_cargo_config(&cargo_path, &existing)?;
            }
            std::fs::write(&cargo_path, updated).context("save Cargo configuration")?;
            cargo_ready = true;
            cargo_applied = true;
            println!("  ✓ Cargo caching: configured");
        } else {
            println!("  • Cargo caching: skipped");
        }
    }

    #[cfg(unix)]
    let shell_pending = init_compiler_setup(compiler, check, &mut created)?;
    #[cfg(unix)]
    let (tests_pending, tests_done, tests_changes) = {
        // The compiler setup may have just written the same startup files.
        let tests = if check {
            tests
        } else {
            plan_test_runner(no_shell)?
        };
        let step = tests.step();
        let changes = existing_paths(tests.changed_paths());
        (init_test_runner(tests, check, &mut created)?, step, changes)
    };
    #[cfg(not(unix))]
    let (shell_pending, tests_pending, tests_done, tests_changes) =
        (false, false, None, Vec::new());

    // ── Step 2: daemon service ───────────────────────────────────
    let mut service_action_taken = false;

    if no_service {
        println!("  \x1b[33m→\x1b[0m Login service: skipped (--no-service)");
    } else if !service_available {
        // Containers and CI runners have no systemd user manager. Installing
        // the unit would fail, so start the daemon directly below (#1080).
        println!("  • Login service: unavailable (no systemd user session)");
    } else if let Some(problem) = service_problem {
        println!("  \x1b[33m→\x1b[0m Background service: update to this kache binary");
        println!("    {}", problem.detail());
        if !check {
            service_action_taken = install_login_service();
        }
    } else if service_installed {
        println!(
            "  \x1b[32m✓\x1b[0m Login service: configured ({})",
            service_path.as_ref().unwrap().display()
        );
    } else {
        println!("  \x1b[33m→\x1b[0m Background service: start kache when you log in");
        if !check {
            service_action_taken = install_login_service();
        }
    }

    // ── Step 3: daemon running ───────────────────────────────────
    // service::install() on macOS/Linux also starts the daemon, so skip the
    // manual start if we just installed it.
    if check {
        println!("  Background cache: would check and start if needed.");
        println!("\n  {}\n", preview_end(changes, no_service, no_shell, yes));
        if crate::machine::is_json() {
            crate::machine::emit(
                "init",
                SetupResult {
                    preview: true,
                    steps,
                    cargo_configured: cargo_ready,
                    shell_activation_required: shell_activation,
                    test_activation_required: test_activation,
                    service_installed,
                    daemon_running: None,
                    compiler_execution: None,
                },
                Vec::new(),
            )?;
        }
        return Ok(());
    }
    let config = crate::config::Config::load().ok();
    let is_daemon_reachable = |cfg: &Option<crate::config::Config>| {
        cfg.as_ref()
            .is_some_and(|c| crate::daemon::send_health_request(c).is_ok())
    };

    let mut daemon_step_failed = false;

    if is_daemon_reachable(&config) {
        println!("  \x1b[32m✓\x1b[0m Background cache: running");
    } else if service_action_taken {
        // Registration is not readiness. Wait through the same coordinator
        // used by `daemon start`, including a service still opening its store.
        let cfg = config.as_ref().context("loading daemon configuration")?;
        if crate::daemon::start(cfg)?.is_ready() {
            println!("  \x1b[32m✓\x1b[0m Background cache: started");
        } else {
            println!("  \x1b[31m✗\x1b[0m daemon did not start within timeout");
            daemon_step_failed = true;
        }
    } else if service_installed {
        // Service is installed (from a previous run) but daemon isn't reachable.
        // The shared coordinator drains this instance and uses its installed
        // manager only when that manager owns the configured runtime path.
        println!("  \x1b[33m→\x1b[0m Background cache: needs restart");
        if let Some(ref cfg) = config {
            match crate::daemon::restart(cfg)? {
                true => println!("    \x1b[32m✓\x1b[0m Background cache: running"),
                false => {
                    println!("    \x1b[31m✗\x1b[0m daemon did not restart — see `kache doctor`");
                    daemon_step_failed = true;
                }
            }
        }
    } else {
        println!("  \x1b[33m→\x1b[0m Background cache: not running");
        match crate::daemon::start_daemon_background()? {
            true => println!("    \x1b[32m✓\x1b[0m Background cache: running"),
            false => {
                println!("    \x1b[31m✗\x1b[0m daemon did not start within timeout");
                daemon_step_failed = true;
            }
        }
    }

    println!();
    if daemon_step_failed {
        println!("  Background cache setup failed. Run kache doctor for details.\n");
        anyhow::bail!("init did not complete: daemon not reachable");
    }
    let compiler_execution = config
        .as_ref()
        .and_then(|config| crate::daemon::compiler_execution_status(config).ok());

    if let Some(status) = &compiler_execution {
        println!("  Compiler execution: {}", status.detail);
    }
    if cargo_ready {
        println!("  Ready for Cargo builds. Use cargo as usual.");
    }
    if shell_pending {
        println!("  Open a new terminal to activate C/C++ caching.");
    }
    if tests_pending {
        println!("  Open a new terminal to pace test binaries.");
    }
    println!("  Run kache doctor to check this terminal.\n");
    if crate::machine::is_json() {
        crate::machine::emit(
            "init",
            SetupResult {
                preview: false,
                steps: setup_steps(&run_facts(
                    &plan_facts,
                    RunOutcome {
                        cargo_applied,
                        test_runner: tests_done,
                        test_changes: tests_changes,
                        service_installed_now: service_action_taken,
                    },
                )),
                cargo_configured: cargo_ready,
                shell_activation_required: shell_pending,
                test_activation_required: tests_pending,
                service_installed: login_service_configured(
                    service_installed,
                    service_action_taken,
                ),
                daemon_running: Some(is_daemon_reachable(&config)),
                compiler_execution,
            },
            Vec::new(),
        )?;
    }
    Ok(())
}

fn login_service_configured(already_installed: bool, installed_now: bool) -> bool {
    already_installed || installed_now
}

/// The line that ends `init --check`: the command that applies the preview,
/// with the flags given, or that there is nothing to apply.
fn preview_end(changes: bool, no_service: bool, no_shell: bool, yes: bool) -> String {
    if changes {
        format!(
            "Preview only. Run {} to apply.",
            init_command(no_service, no_shell, yes)
        )
    } else {
        "Nothing to change.".to_string()
    }
}

/// The backup a run makes before it changes `path`, which `exists` before
/// the change: none for a file in `created`, which the run made itself. A
/// file that does not exist yet is created by the change and joins `created`.
fn backup_step(
    created: &mut Vec<std::path::PathBuf>,
    path: &std::path::Path,
    exists: bool,
) -> Option<SetupStep> {
    let made_here = created.iter().any(|made| made == path);
    if !exists && !made_here {
        created.push(path.to_owned());
    }
    (exists && !made_here).then(|| SetupStep::new("backup", "create", vec![path.to_owned()]))
}

/// What `kache init` plans for C/C++ caching:
/// the compiler shims and the shell block that puts them first on PATH.
#[cfg(unix)]
enum CompilerPlan {
    /// `--no-shell`.
    Skipped,
    NoHome,
    /// A login shell init cannot set up; the shim directory to use instead.
    Unsupported(std::path::PathBuf),
    /// A startup file init must not edit, and why.
    Manual(String),
    Ready {
        shell: crate::init_shell::Shell,
        shim_dir: std::path::PathBuf,
        edits: Vec<crate::init_shell::Edit>,
        extra: Vec<String>,
        shims_ready: bool,
    },
}

#[cfg(unix)]
impl CompilerPlan {
    /// The `shell` step; none with `--no-shell`.
    fn step(&self) -> Option<SetupStep> {
        let (action, paths) = match self {
            Self::Skipped => return None,
            Self::NoHome => ("unavailable", Vec::new()),
            Self::Unsupported(_) => ("unsupported", Vec::new()),
            Self::Manual(_) => ("manual", Vec::new()),
            Self::Ready {
                edits, shims_ready, ..
            } => (
                edit_action(edits.iter().any(|edit| edit.changed()), !shims_ready),
                edits.iter().map(|edit| edit.path.clone()).collect(),
            ),
        };
        Some(SetupStep::new("shell", action, paths))
    }

    /// The startup files a run changes.
    fn changed_paths(&self) -> Vec<std::path::PathBuf> {
        match self {
            Self::Ready { edits, .. } => edits
                .iter()
                .filter(|edit| edit.changed())
                .map(|edit| edit.path.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether init knows this shell's startup files.
    fn supported(&self) -> bool {
        matches!(self, Self::Manual(_) | Self::Ready { .. })
    }

    /// Whether the shims exist once the plan is applied.
    fn installs_shims(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Whether this terminal needs replacing once the plan is applied.
    fn activation_required(&self) -> bool {
        self.installs_shims() && !crate::compiler::shim::live_shim_status().is_active()
    }
}

#[cfg(unix)]
fn plan_compiler_setup(no_shell: bool) -> Result<CompilerPlan> {
    use crate::init_shell::Edit;
    if no_shell {
        return Ok(CompilerPlan::Skipped);
    }
    let Some(shim_dir) = crate::compiler::shim::default_shim_dir() else {
        return Ok(CompilerPlan::NoHome);
    };
    let Some((shell, paths)) = shell_startup_files()? else {
        return Ok(CompilerPlan::Unsupported(shim_dir));
    };
    let activation = shell.activation(&shim_dir)?;
    let edits = paths
        .into_iter()
        .map(|path| Edit::plan(path, &activation))
        .collect::<Result<Vec<_>>>();
    let edits = match edits {
        Ok(edits) => edits,
        Err(error) => return Ok(CompilerPlan::Manual(error.to_string())),
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let extra = crate::compiler::shim::names_on_path(&path, false);
    let shims_ready = shim_dir_is_ready(&shim_dir, &extra);
    Ok(CompilerPlan::Ready {
        shell,
        shim_dir,
        edits,
        extra,
        shims_ready,
    })
}

/// Apply `plan`, or describe it when `check`. Startup files this run
/// creates go into `created`. True when a new terminal is needed.
#[cfg(unix)]
fn init_compiler_setup(
    plan: CompilerPlan,
    check: bool,
    created: &mut Vec<std::path::PathBuf>,
) -> Result<bool> {
    let (shell, shim_dir, edits, extra, shims_ready) = match plan {
        CompilerPlan::Skipped => {
            println!("  • C/C++ caching: skipped (--no-shell)");
            return Ok(false);
        }
        CompilerPlan::NoHome => {
            println!("  • C/C++ caching: skipped (no home directory)");
            return Ok(false);
        }
        CompilerPlan::Unsupported(shim_dir) => {
            println!(
                "  • C/C++ caching: shell not supported for automatic setup"
            );
            println!(
                "    Use kache init --shims, then add {} to your shell's PATH.",
                shim_dir.display()
            );
            return Ok(false);
        }
        CompilerPlan::Manual(error) => {
            println!(
                "  • C/C++ caching: shell config needs manual attention"
            );
            println!("    {error}");
            println!("    No shell files were changed.");
            return Ok(false);
        }
        CompilerPlan::Ready {
            shell,
            shim_dir,
            edits,
            extra,
            shims_ready,
        } => (shell, shim_dir, edits, extra, shims_ready),
    };
    let needs_edit = edits.iter().any(|edit| edit.changed());
    if !shims_ready || needs_edit {
        println!("  C/C++ caching in new terminals");
        for edit in edits.iter().filter(|edit| edit.changed()) {
            println!(
                "    Shell config: {}",
                crate::wrapper_config::display_path(&edit.path)
            );
        }
        if check {
            println!("    Would install Cargo and compiler links and save the shell setup.");
            return Ok(false);
        }
        if !shims_ready {
            install_shims_named_with_output(&shim_dir, false, &extra, false)?;
            anyhow::ensure!(
                shim_dir_is_ready(&shim_dir, &extra),
                "existing files in {} prevent compiler setup; inspect them before using kache init --shims --force",
                shim_dir.display()
            );
        }
        apply_shell_edits(&edits, created)?;
    }
    if crate::compiler::shim::live_shim_status().is_active() {
        println!("  ✓ C/C++ caching: active");
        Ok(false)
    } else {
        println!("  ✓ C/C++ caching: configured for new terminals");
        println!("    For this terminal, run:");
        println!("    {}", shell.command(&shim_dir)?);
        Ok(true)
    }
}

/// Write the startup file `edits` that change something, backing up each
/// one that exists and is not in `created`; each file they create joins it.
#[cfg(unix)]
fn apply_shell_edits(
    edits: &[crate::init_shell::Edit],
    created: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    for edit in edits.iter().filter(|edit| edit.changed()) {
        let back_up = backup_step(created, &edit.path, !edit.creates()).is_some();
        if let Some(backup) = edit.apply_keeping(back_up)? {
            println!(
                "    Backup: {}",
                crate::wrapper_config::display_path(&backup)
            );
        }
    }
    Ok(())
}

/// The login shell and the startup files `kache init` edits for it. `None`
/// for a shell it cannot set up.
#[cfg(unix)]
fn shell_startup_files() -> Result<Option<(crate::init_shell::Shell, Vec<std::path::PathBuf>)>> {
    let home = kache_fs::home_dir().context("could not find your home directory")?;
    let shell = std::env::var_os("SHELL")
        .as_deref()
        .and_then(|shell| crate::init_shell::Shell::detect(std::path::Path::new(shell)));
    let Some(shell) = shell else {
        return Ok(None);
    };
    let zdotdir = std::env::var_os("ZDOTDIR")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from);
    let paths = shell.paths(&home, zdotdir.as_deref(), xdg.as_deref())?;
    Ok(Some((shell, paths)))
}

#[cfg(unix)]
/// What `kache init` sets as Cargo's runner for the host target.
#[cfg(unix)]
const TEST_RUNNER: &str = "kache test-runner";

/// The host triple of the `rustc` Cargo would use here.
#[cfg(unix)]
fn rustc_host() -> Option<String> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = std::process::Command::new(rustc)
        .arg("-vV")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let host = crate::cache_key::rustc_host_triple(&stdout)?;
    output.status.success().then(|| host.to_owned())
}

/// What `init` says about test pacing once the startup files are set up.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum TestPacing {
    /// This terminal already runs tests through kache.
    Active,
    /// This terminal sets another runner, which the saved block leaves alone.
    Overridden(String),
    /// New terminals will run tests through kache.
    Pending,
}

#[cfg(unix)]
fn test_pacing(current: Option<&str>) -> TestPacing {
    match current {
        Some(TEST_RUNNER) => TestPacing::Active,
        Some(value) if !value.is_empty() => TestPacing::Overridden(value.to_owned()),
        _ => TestPacing::Pending,
    }
}

/// What `kache init` plans for test pacing: an export of
/// `CARGO_TARGET_<HOST>_RUNNER="kache test-runner"` in the shell startup
/// files.
#[cfg(unix)]
enum TestPlan {
    /// `--no-shell`.
    Skipped,
    /// rustc did not report its host target.
    NoHost,
    /// A login shell init cannot set up; the variable to export instead.
    Unsupported(String),
    /// A startup file init must not edit, and why.
    Manual(String),
    Ready {
        var: String,
        edits: Vec<crate::init_shell::Edit>,
    },
}

#[cfg(unix)]
impl TestPlan {
    /// The `test_runner` step; none with `--no-shell`.
    fn step(&self) -> Option<SetupStep> {
        let (action, paths) = match self {
            Self::Skipped => return None,
            Self::NoHost => ("unavailable", Vec::new()),
            Self::Unsupported(_) => ("unsupported", Vec::new()),
            Self::Manual(_) => ("manual", Vec::new()),
            Self::Ready { edits, .. } => (
                edit_action(edits.iter().any(|edit| edit.changed()), false),
                edits.iter().map(|edit| edit.path.clone()).collect(),
            ),
        };
        Some(SetupStep::new("test_runner", action, paths))
    }

    /// The startup files a run changes.
    fn changed_paths(&self) -> Vec<std::path::PathBuf> {
        match self {
            Self::Ready { edits, .. } => edits
                .iter()
                .filter(|edit| edit.changed())
                .map(|edit| edit.path.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether this terminal needs replacing once the plan is applied.
    fn activation_required(&self) -> bool {
        match self {
            Self::Ready { var, .. } => {
                test_pacing(std::env::var(var).ok().as_deref()) == TestPacing::Pending
            }
            _ => false,
        }
    }
}

#[cfg(unix)]
fn plan_test_runner(no_shell: bool) -> Result<TestPlan> {
    use crate::init_shell::{Edit, TEST_RUNNER_BLOCK};
    if no_shell {
        return Ok(TestPlan::Skipped);
    }
    let Some(host) = rustc_host() else {
        return Ok(TestPlan::NoHost);
    };
    let var = crate::test_runner::runner_var(&host);
    let Some((shell, paths)) = shell_startup_files()? else {
        return Ok(TestPlan::Unsupported(var));
    };
    let export = shell.export_unless_set(&var, TEST_RUNNER)?;
    let edits = paths
        .into_iter()
        .map(|path| Edit::plan_block(path, TEST_RUNNER_BLOCK, &export))
        .collect::<Result<Vec<_>>>();
    Ok(match edits {
        Ok(edits) => TestPlan::Ready { var, edits },
        Err(error) => TestPlan::Manual(error.to_string()),
    })
}

/// Export `CARGO_TARGET_<HOST>_RUNNER="kache test-runner"` from the shell
/// startup files, so `cargo test` in any project paces its test binaries.
/// Cargo ranks the variable above a project's own runner; `kache
/// test-runner` looks that runner up and still runs the binary under it.
/// Files in `created` are not backed up. Returns true when a new terminal is
/// needed.
#[cfg(unix)]
fn init_test_runner(
    plan: TestPlan,
    check: bool,
    created: &mut Vec<std::path::PathBuf>,
) -> Result<bool> {
    let (var, edits) = match plan {
        TestPlan::Skipped => {
            println!("  • Test pacing: skipped (--no-shell)");
            return Ok(false);
        }
        TestPlan::NoHost => {
            println!("  • Test pacing: skipped (rustc did not report its host target)");
            return Ok(false);
        }
        TestPlan::Unsupported(var) => {
            println!("  • Test pacing: shell not supported for automatic setup");
            println!("    Export {var}=\"{TEST_RUNNER}\" from your shell's startup file.");
            return Ok(false);
        }
        TestPlan::Manual(error) => {
            println!("  • Test pacing: shell config needs manual attention");
            println!("    {error}");
            println!("    Test pacing changed no shell files.");
            return Ok(false);
        }
        TestPlan::Ready { var, edits } => (var, edits),
    };
    if edits.iter().any(|edit| edit.changed()) {
        println!("  Test pacing: cargo test waits for the CPU slots builds use");
        for edit in edits.iter().filter(|edit| edit.changed()) {
            println!(
                "    Shell config: {}",
                crate::wrapper_config::display_path(&edit.path)
            );
        }
        if check {
            println!("    Would set {var} in new terminals.");
            return Ok(false);
        }
        apply_shell_edits(&edits, created)?;
    }
    match test_pacing(std::env::var(&var).ok().as_deref()) {
        TestPacing::Active => {
            println!("  ✓ Test pacing: active");
            Ok(false)
        }
        TestPacing::Overridden(value) => {
            println!("  • Test pacing: this terminal sets {var}={value:?}, which stays in effect");
            Ok(false)
        }
        TestPacing::Pending => {
            println!("  ✓ Test pacing: configured for new terminals");
            Ok(true)
        }
    }
}

/// An installer alias or stable PATH entry when one reaches this binary, so
/// an upgrade does not leave the farm dangling.
#[cfg(unix)]
fn shim_target() -> Result<kache_shims::Selection> {
    crate::platform::frontend_selection().context("locating the kache binary")
}

/// True when `dir` already links every name an install with `extra_names`
/// would create to the selected target. Used by `kache init` so a second run
/// is a no-op, and a compiler version installed since the first is linked.
#[cfg(unix)]
fn shim_dir_is_ready(dir: &std::path::Path, extra_names: &[String]) -> bool {
    let Ok(names) = shim_names(extra_names) else {
        return false;
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    shim_target().is_ok_and(|target| {
        kache_shims::farm::is_ready(dir, &target.path, &names, &kache_shims::RealFs)
            && kache_shims::farm::stale_versioned(
                dir,
                &target.path,
                &|name| keeps_shim(&names, &path, name),
                &kache_shims::RealFs,
            )
            .is_empty()
    })
}

/// Versioned shims in `dir` that a rerun of `kache init --shims` would
/// remove. `None` when there are none.
#[cfg(unix)]
fn stale_shims_check(dir: &std::path::Path, path: &std::ffi::OsStr) -> Option<Check> {
    let target = shim_target().ok()?;
    let stale = kache_shims::farm::stale_versioned(
        dir,
        &target.path,
        &|name| keeps_shim(&[], path, name),
        &kache_shims::RealFs,
    );
    (!stale.is_empty()).then(|| Check {
        label: "Stale shims",
        pass: false,
        detail: format!(
            "no real compiler on PATH behind {} in {}",
            stale.join(", "),
            term::home_path(dir)
        ),
        fix: Some("kache init --shims".into()),
    })
}

/// A shim stays while kache links its name or a real compiler of that name
/// is on `path`.
#[cfg(unix)]
fn keeps_shim(names: &[String], path: &std::ffi::OsStr, name: &str) -> bool {
    names.iter().any(|kept| kept == name) || crate::compiler::shim::real_compiler_on(path, name)
}

#[cfg(unix)]
fn shim_names(extra_names: &[String]) -> Result<Vec<String>> {
    let mut names: Vec<String> = kache_shims::farm::SHIM_NAMES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for extra in extra_names {
        if !crate::compiler::shim::invoked_as_compiler(extra) {
            anyhow::bail!("`{extra}` is not a compiler name kache can wrap");
        }
        if !names.iter().any(|n| n == extra) {
            names.push(extra.clone());
        }
    }
    Ok(names)
}

/// Populate `dir` with compiler-name symlinks pointing at this kache binary
/// (kunobi-ninja/kache#310).
///
/// Prepending the result to `PATH` routes every build's compiler calls through
/// kache with no `CC`/`CXX` edits and no per-project build-system changes.
///
/// Unix-only: it creates symlinks, and the Windows `.exe` shim story differs
/// (kunobi-ninja/kache#310). The unsupported message lives in the command
/// dispatch so this stays a single, fully testable definition rather than two
/// same-named ones the mutation lane cannot tell apart.
#[cfg(unix)]
pub(crate) fn install_shims_named(
    dir: &std::path::Path,
    force: bool,
    extra_names: &[String],
) -> anyhow::Result<()> {
    install_shims_named_with_output(dir, force, extra_names, true)
}

#[cfg(unix)]
fn install_shims_named_with_output(
    dir: &std::path::Path,
    force: bool,
    extra_names: &[String],
    verbose: bool,
) -> anyhow::Result<()> {
    let target = shim_target()?;
    let names = shim_names(extra_names)?;
    let layout = kache_shims::Layout::from_process();
    let mut report = kache_shims::install(dir, &target.path, &names, force, &layout)?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    report.pruned =
        kache_shims::farm::prune(dir, &target.path, &|name| keeps_shim(&names, &path, name))?;
    if verbose {
        for line in install_report_lines(dir, &target, &report, cfg!(target_os = "linux")) {
            println!("{line}");
        }
    }
    Ok(())
}

/// Whether an install left every entry as it was.
#[cfg(unix)]
fn install_changed_nothing(report: &kache_shims::Installed) -> bool {
    [
        &report.created,
        &report.repaired,
        &report.refreshed,
        &report.replaced,
        &report.pruned,
        &report.skipped,
    ]
    .iter()
    .all(|names| names.is_empty())
}

/// What `kache init --shims` prints. Pure so each outcome is testable.
/// `linux` adds the makepkg hint.
#[cfg(unix)]
fn install_report_lines(
    dir: &std::path::Path,
    target: &kache_shims::Selection,
    report: &kache_shims::Installed,
    linux: bool,
) -> Vec<String> {
    let note = (!target.stability.survives_upgrade()).then(|| {
        format!(
            "note: {}. Rerun `kache init --shims` after upgrading kache.",
            target.reason
        )
    });
    if install_changed_nothing(report) && report.marked {
        let mut lines = vec![format!(
            "Shims up to date in {} -> {}",
            dir.display(),
            target.path.display()
        )];
        lines.extend(note);
        return lines;
    }
    let mut lines = vec![format!(
        "Created {} in {} -> {}",
        term::counted(report.created.len() as u64, "shim", "shims"),
        dir.display(),
        target.path.display()
    )];
    if !report.created.is_empty() {
        lines.push(format!("  {}", report.created.join(", ")));
    }
    let groups = [
        (
            &report.repaired,
            "Repaired",
            ("broken shim", "broken shims"),
            "",
        ),
        (
            &report.refreshed,
            "Moved",
            ("shim", "shims"),
            " off a versioned path",
        ),
        (
            &report.replaced,
            "Replaced",
            ("existing entry", "existing entries"),
            "",
        ),
        (
            &report.pruned,
            "Removed",
            ("shim", "shims"),
            " with no compiler",
        ),
    ];
    for (names, verb, (one, many), tail) in groups {
        if !names.is_empty() {
            lines.push(format!(
                "{verb} {}{tail}: {}",
                term::counted(names.len() as u64, one, many),
                names.join(", ")
            ));
        }
    }
    if !report.skipped.is_empty() {
        lines.push(format!(
            "Skipped {}: {} (use --force to replace)",
            term::counted(
                report.skipped.len() as u64,
                "existing entry",
                "existing entries"
            ),
            report.skipped.join(", ")
        ));
    }
    lines.extend(note);
    if !report.marked {
        lines.push(format!(
            "Left {} without a {} marker: it holds files that are not kache links.",
            dir.display(),
            kache_shims::farm::MARKER
        ));
    }
    lines.push(String::new());
    lines.push("Add it to PATH ahead of your toolchain:".into());
    lines.push(format!("  export PATH=\"{}:$PATH\"", dir.display()));
    lines.push(String::new());
    // The ordering caveat is the one way this silently does nothing: a shim
    // dir appended rather than prepended is never consulted.
    lines.push(
        "The directory must come BEFORE the real toolchain on PATH, and the real \
         compilers must remain on PATH behind it — kache runs them."
            .into(),
    );
    lines.push(
        "Make, CMake, autotools, and Arch PKGBUILDs that invoke gcc/cc/clang \
         from PATH then go through kache. No CC/CXX edit and no shell wrapper."
            .into(),
    );
    lines.push(
        "Cargo commands on PATH also run through kache. The command remains cargo build/test/run."
            .into(),
    );
    if linux {
        lines.push(format!(
            "For makepkg, put PATH=\"{}:$PATH\" in ~/.makepkg.conf.",
            dir.display()
        ));
    }
    lines
}

#[cfg(all(test, unix))]
mod shim_install_tests {
    use kache_shims::farm::{SHIM_NAMES, has_marker};
    use std::os::unix::fs::PermissionsExt;

    fn install_shims(dir: &std::path::Path, force: bool) -> anyhow::Result<()> {
        super::install_shims_named(dir, force, &[])
    }

    fn target() -> std::path::PathBuf {
        super::shim_target().unwrap().path
    }

    fn is_symlink(path: &std::path::Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
    }

    #[test]
    fn the_shim_target_reaches_this_binary() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            kache_fs::file_identity(&target()).unwrap(),
            kache_fs::file_identity(&exe).unwrap()
        );
    }

    #[test]
    fn installs_a_symlink_for_every_shim_name() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        install_shims(&shims, false).unwrap();
        for name in SHIM_NAMES {
            let link = shims.join(name);
            assert!(is_symlink(&link), "{name} must be a symlink");
            assert_eq!(std::fs::read_link(&link).unwrap(), target(), "{name}");
        }
        assert!(has_marker(&shims));
    }

    #[test]
    fn extra_compiler_name_is_installed_next_to_the_canonical_farm() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        super::install_shims_named(&shims, false, &["gcc-13".into(), "gcc".into()]).unwrap();
        assert_eq!(std::fs::read_link(shims.join("gcc-13")).unwrap(), target());
        for name in SHIM_NAMES {
            assert!(
                is_symlink(&shims.join(name)),
                "{name} must still be installed"
            );
        }
        assert_eq!(
            std::fs::read_dir(&shims).unwrap().count(),
            SHIM_NAMES.len() + 2,
            "a canonical name given again is linked once"
        );
    }

    #[test]
    fn extra_name_that_is_not_a_compiler_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        let err = super::install_shims_named(&shims, false, &["gcc-ar".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("gcc-ar"), "{msg}");
        assert!(msg.contains("not a compiler name"), "{msg}");
        assert!(!shims.exists(), "a rejected name must not create the farm");
    }

    #[test]
    fn empty_dir_is_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !super::shim_dir_is_ready(dir.path(), &[]),
            "an empty directory must not count as an installed farm"
        );
    }

    #[test]
    fn install_makes_the_dir_ready_and_a_wrong_target_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        install_shims(&shims, false).unwrap();
        assert!(
            super::shim_dir_is_ready(&shims, &[]),
            "the installer must produce a farm that init treats as already done"
        );

        let not_kache = dir.path().join("other/not-kache");
        std::fs::create_dir_all(not_kache.parent().unwrap()).unwrap();
        std::fs::write(&not_kache, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&not_kache, std::fs::Permissions::from_mode(0o755)).unwrap();
        for name in SHIM_NAMES {
            std::fs::remove_file(shims.join(name)).unwrap();
            std::os::unix::fs::symlink(&not_kache, shims.join(name)).unwrap();
        }
        assert!(
            !super::shim_dir_is_ready(&shims, &[]),
            "links that do not point at this kache must not look ready"
        );
    }

    /// A compiler version installed after the farm was made leaves it not
    /// ready, so a rerun of `kache init` links it.
    #[test]
    fn a_versioned_compiler_the_farm_lacks_makes_it_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        let extra = ["clang-19".to_string()];
        install_shims(&shims, false).unwrap();
        assert!(!super::shim_dir_is_ready(&shims, &extra));

        super::install_shims_named(&shims, false, &extra).unwrap();
        assert_eq!(
            std::fs::read_link(shims.join("clang-19")).unwrap(),
            target()
        );
        assert!(super::shim_dir_is_ready(&shims, &extra));
    }

    /// A run that skips a real compiler must not mark the directory even when
    /// it created other links there. The marker would hide that compiler from
    /// every kache, so a shim elsewhere on PATH would run a different one.
    #[test]
    fn install_next_to_a_real_compiler_does_not_mark_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        let clang = shims.join("clang");
        std::fs::write(&clang, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&clang, std::fs::Permissions::from_mode(0o755)).unwrap();

        install_shims(&shims, false).unwrap();

        assert!(is_symlink(&shims.join("cc")), "free names are still linked");
        assert!(!has_marker(&shims));
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            crate::compiler::shim::resolve_real_compiler_on(
                "clang",
                std::slice::from_ref(&shims),
                Some(&exe)
            ),
            Some(clang),
            "the real clang must stay visible to shim resolution"
        );
    }

    /// An inspection failure that is NOT "missing" must surface rather than be
    /// treated as an empty slot.
    #[test]
    fn unreadable_target_directory_is_an_error() {
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("skipping: running as root, mode 000 does not deny access");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        std::fs::set_permissions(&shims, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = install_shims(&shims, false);
        std::fs::set_permissions(&shims, std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = format!("{:#}", result.unwrap_err());
        assert!(
            err.contains("inspecting"),
            "an inspection failure must be reported as such, got: {err}"
        );
    }

    fn path_with_real(dir: &std::path::Path, name: &str) -> std::ffi::OsString {
        let real = dir.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join(name), b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(real.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::join_paths([real]).unwrap()
    }

    #[test]
    fn a_shim_is_kept_for_a_linked_name_or_a_real_compiler() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_with_real(dir.path(), "gcc-999");
        assert!(super::keeps_shim(&[], &path, "gcc-999"));
        assert!(!super::keeps_shim(&[], &path, "gcc-998"));
        assert!(super::keeps_shim(&["gcc-998".into()], &path, "gcc-998"));
    }

    /// `gcc-999` stands in for a compiler that was upgraded away: no runner
    /// has one on PATH.
    #[test]
    fn a_versioned_shim_with_no_compiler_is_pruned_by_a_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        let gone = ["gcc-999".to_string()];
        super::install_shims_named(&shims, false, &gone).unwrap();
        assert!(
            super::shim_dir_is_ready(&shims, &gone),
            "a name being linked is kept"
        );
        assert!(
            !super::shim_dir_is_ready(&shims, &[]),
            "init must rerun to remove it"
        );

        super::install_shims_named(&shims, false, &[]).unwrap();
        assert!(std::fs::symlink_metadata(shims.join("gcc-999")).is_err());
        assert!(super::shim_dir_is_ready(&shims, &[]));
    }

    #[test]
    fn doctor_names_versioned_shims_with_no_compiler() {
        let dir = tempfile::tempdir().unwrap();
        let shims = dir.path().join("shims");
        super::install_shims_named(&shims, false, &["gcc-999".into()]).unwrap();

        let check = super::stale_shims_check(&shims, std::ffi::OsStr::new("")).unwrap();
        assert!(!check.pass);
        assert!(check.detail.contains("gcc-999"), "{}", check.detail);
        assert_eq!(check.fix.as_deref(), Some("kache init --shims"));

        let path = path_with_real(dir.path(), "gcc-999");
        assert!(super::stale_shims_check(&shims, &path).is_none());
    }

    fn selection(stability: kache_shims::Stability) -> kache_shims::Selection {
        kache_shims::Selection {
            path: "/opt/homebrew/opt/kache/bin/kache".into(),
            kind: kache_shims::Kind::Homebrew,
            stability,
            reason: "Homebrew's opt link does not reach this binary".into(),
        }
    }

    #[test]
    fn the_report_names_each_outcome() {
        let report = kache_shims::Installed {
            created: vec!["cc".into()],
            repaired: vec!["gcc".into(), "g++".into()],
            refreshed: vec!["clang".into()],
            replaced: vec!["c++".into()],
            current: vec!["clang++".into()],
            skipped: vec!["gcc-13".into()],
            pruned: vec!["gcc-14".into()],
            marked: true,
        };
        let dir = std::path::Path::new("/home/me/shims");
        let lines = super::install_report_lines(
            dir,
            &selection(kache_shims::Stability::InstallerManaged),
            &report,
            true,
        );
        let expected = [
            "Created 1 shim in /home/me/shims -> /opt/homebrew/opt/kache/bin/kache",
            "  cc",
            "Repaired 2 broken shims: gcc, g++",
            "Moved 1 shim off a versioned path: clang",
            "Replaced 1 existing entry: c++",
            "Removed 1 shim with no compiler: gcc-14",
            "Skipped 1 existing entry: gcc-13 (use --force to replace)",
            "",
            "Add it to PATH ahead of your toolchain:",
            "  export PATH=\"/home/me/shims:$PATH\"",
        ];
        assert_eq!(&lines[..expected.len()], expected);
        assert!(lines.last().unwrap().contains("~/.makepkg.conf"));
        let elsewhere = super::install_report_lines(
            dir,
            &selection(kache_shims::Stability::InstallerManaged),
            &report,
            false,
        );
        assert_eq!(&elsewhere[..], &lines[..lines.len() - 1]);
    }

    #[test]
    fn a_rerun_that_changes_nothing_says_the_shims_are_up_to_date() {
        let dir = std::path::Path::new("/home/me/shims");
        let current = kache_shims::Installed {
            current: vec!["cc".into(), "c++".into()],
            marked: true,
            ..Default::default()
        };
        let stable = selection(kache_shims::Stability::InstallerManaged);
        assert_eq!(
            super::install_report_lines(dir, &stable, &current, true),
            ["Shims up to date in /home/me/shims -> /opt/homebrew/opt/kache/bin/kache"]
        );
        let versioned = selection(kache_shims::Stability::Versioned);
        assert_eq!(
            super::install_report_lines(dir, &versioned, &current, true)[1],
            "note: Homebrew's opt link does not reach this binary. Rerun `kache init --shims` after upgrading kache."
        );
        // Any change, or a directory left unmarked, keeps the full report.
        let lists: [fn(&mut kache_shims::Installed) -> &mut Vec<String>; 6] = [
            |r| &mut r.created,
            |r| &mut r.repaired,
            |r| &mut r.refreshed,
            |r| &mut r.replaced,
            |r| &mut r.pruned,
            |r| &mut r.skipped,
        ];
        for list in lists {
            let mut changed = kache_shims::Installed {
                marked: true,
                ..Default::default()
            };
            list(&mut changed).push("gcc".into());
            let lines = super::install_report_lines(dir, &stable, &changed, true);
            assert!(!lines[0].starts_with("Shims up to date"), "{lines:?}");
        }
        let unmarked = kache_shims::Installed::default();
        let lines = super::install_report_lines(dir, &stable, &unmarked, true);
        assert!(lines[0].starts_with("Created 0 shims"), "{lines:?}");
    }

    #[test]
    fn the_report_warns_about_a_versioned_target_and_a_missing_marker() {
        let report = kache_shims::Installed::default();
        let dir = std::path::Path::new("/home/me/shims");
        let lines = super::install_report_lines(
            dir,
            &selection(kache_shims::Stability::Versioned),
            &report,
            true,
        );
        assert_eq!(
            &lines[..3],
            [
                "Created 0 shims in /home/me/shims -> /opt/homebrew/opt/kache/bin/kache",
                "note: Homebrew's opt link does not reach this binary. Rerun `kache init --shims` after upgrading kache.",
                "Left /home/me/shims without a .kache-shims marker: it holds files that are not kache links.",
            ]
        );
    }
}
