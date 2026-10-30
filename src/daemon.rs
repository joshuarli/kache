use anyhow::{Context, Result};
pub use kache_protocol::{
    CompileFinishedRequest, CompileStartedRequest,
    DaemonHealth, EffectiveConfig, EventStatsResponse, GcPolicyOutcome, HashFileRequest,
    HashFileResult, HashFilesRequest, InFlightEntry,
    PrestageRequest, StatsEntry,
    StatsRequest, TransferDirection, TransferEvent,
};
pub use kache_protocol::{
    GcBreakdown, GcRequest, GcRequestMode, Request, Response,
    StatsResponse,
};
use crate::lifecycle::Lifecycle;
use kache_core::time::UtcTime;
#[path = "daemon_lifecycle.rs"]
mod lifecycle_client;
pub(crate) use lifecycle_client::Ensured;
#[path = "daemon_execution.rs"]
mod execution;
#[path = "daemon_control.rs"]
mod lifecycle_control;
use deser::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::events;
use crate::store::Store;

#[path = "daemon_capacity.rs"]
mod capacity;
use capacity::DaemonCapacity;

const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(8);

/// Read timeout for a stats round trip. Generous: a busy daemon may be holding
/// the index lock when the request lands.
const STATS_READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Read timeout for the stats refetch after a stale daemon was replaced. The
/// replacement has just bound its socket and has nothing queued, so a slow
/// answer here means something is wrong rather than busy.
const STATS_REFETCH_TIMEOUT: Duration = Duration::from_secs(3);
const DAEMON_COORD_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

/// How often the daemon re-checks its config file for changes. On a change it
/// schedules a graceful restart so the new config (e.g. `local_max_size`) takes
/// effect — no manual `kache daemon stop`. Cheap (one small-file read); rare to
/// fire, so a coarse interval is fine.
const DAEMON_CONFIG_WATCH_INTERVAL: Duration = Duration::from_secs(15);
const DAEMON_COORD_STALE_AFTER: Duration = Duration::from_secs(15);
const VERSION: &str = crate::VERSION;
const FILE_HASH_MEMORY_CACHE_CAP: usize = 4096;
/// Age a blob file with no `blobs` row must reach before a daemon sweep
/// unlinks it. A put renames its blobs into place before it inserts their
/// rows, and an hour outlasts any put still in flight.
pub(crate) const ORPHAN_BLOB_GRACE: Duration = Duration::from_secs(3600);

/// Executable mtime. Coordinator records use it to tell daemons apart. The
/// legacy epoch fields carry it because 1.0.x and earlier decide replacement
/// by it; this release decides on the release version alone.
pub fn build_epoch() -> u64 {
    static BUILD_EPOCH: OnceLock<u64> = OnceLock::new();

    *BUILD_EPOCH.get_or_init(|| {
        std::env::current_exe()
            .and_then(std::fs::metadata)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[deser(rename_all = "snake_case")]
enum DaemonPhase {
    Starting,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DaemonCoordState {
    pid: u32,
    build_epoch: u64,
    phase: DaemonPhase,
    updated_at_ms: u64,
    #[deser(default, skip_serializing_if = Option::is_none)]
    control_version: Option<u32>,
}

#[derive(Debug, Clone)]
struct DaemonCoordFile {
    control_version: Option<u32>,
    path: PathBuf,
    pid: u32,
    build_epoch: u64,
}

impl DaemonCoordFile {
    fn for_socket(socket_path: &Path) -> Self {
        Self {
            path: daemon_state_path(socket_path),
            control_version: None,
            pid: std::process::id(),
            build_epoch: build_epoch(),
        }
    }

    fn write_phase(&self, phase: DaemonPhase) -> Result<()> {
        let state = DaemonCoordState {
            pid: self.pid,
            build_epoch: self.build_epoch,
            phase,
            updated_at_ms: unix_time_ms(),
            control_version: self.control_version,
        };
        write_json_atomically(&self.path, &state)
    }
}

struct DaemonCoordGuard {
    path: PathBuf,
}

/// Platform adapter for shared, inode-checked socket cleanup.
struct SocketCleanupGuard {
    _guard: kunobi_daemon::local::unix_socket::SocketGuard,
}
impl SocketCleanupGuard {
    fn new(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            _guard: kunobi_daemon::local::unix_socket::SocketGuard::capture(path)?,
        })
    }
}

impl DaemonCoordGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for DaemonCoordGuard {
    fn drop(&mut self) {
        if let Ok(bytes) = std::fs::read(&self.path)
            && deser_json::from_slice::<DaemonCoordState>(&bytes).is_ok_and(|state| {
                state.pid == std::process::id() && state.build_epoch == build_epoch()
            })
        {
            let _ = kunobi_daemon::RecordSlot::new(&self.path).remove_if_matches(&bytes);
        }
    }
}

fn daemon_state_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("state.json")
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    kunobi_daemon::RecordSlot::new(path).replace(&deser_json::to_string(value).map(String::into_bytes)?)?;
    Ok(())
}

fn read_daemon_state(socket_path: &Path) -> Option<DaemonCoordState> {
    let path = daemon_state_path(socket_path);
    let bytes = std::fs::read(path).ok()?;
    deser_json::from_slice(&bytes).ok()
}

/// Build epoch of a daemon that holds the run lock but has not bound its socket
/// yet, if one is coming up right now.
///
/// Read-only and instant: it inspects the coordinator state file rather than the
/// socket, which is what makes it usable from `doctor` during the window where a
/// stats request would only report "not reachable" (kunobi-ninja/kache#720).
///
/// A coordinator file outlives an unclean exit, so its mere existence proves
/// nothing, and neither does its PID: PIDs are recycled, and a fresh record whose
/// PID has been reused by an unrelated process would otherwise read as a live
/// starter. The run lock is what actually distinguishes them — a daemon takes it
/// before writing `Starting` and holds it for its whole life, so only a real
/// starter can be holding it. The lock is probed but never created here: `doctor`
/// reports on lock files, and a diagnostic that manufactures one would then flag
/// its own leftovers.
pub fn starting_daemon_epoch(config: &Config) -> Option<u64> {
    let socket_path = config.socket_path();
    let state = read_daemon_state(&socket_path)?;
    if state.phase != DaemonPhase::Starting
        || !daemon_state_is_recent(&state)
        || !process_is_alive(state.pid)
    {
        return None;
    }
    existing_daemon_run_lock_is_held(&socket_path)
        .ok()?
        .then_some(state.build_epoch)
}

fn daemon_state_is_recent(state: &DaemonCoordState) -> bool {
    // A timestamp in the future is not a fresh heartbeat, it is a clock that
    // moved: saturating to an age of zero would read a long-dead record as live
    // until the wall clock caught back up.
    unix_time_ms()
        .checked_sub(state.updated_at_ms)
        .is_some_and(|age_ms| age_ms <= DAEMON_COORD_STALE_AFTER.as_millis() as u64)
}

/// Compare release precedence, ignoring build metadata. Missing or malformed
/// versions are unknown and cannot authorize an automatic replacement.
pub(crate) fn compare_daemon_version(client: &str, daemon: &str) -> Option<std::cmp::Ordering> {
    let client = Release::parse(client)?;
    let daemon = Release::parse(daemon)?;
    Some(client.cmp_precedence(&daemon))
}

/// A `major.minor.patch` release with an optional prerelease, parsed
/// strictly enough that malformed versions stay unknown: three bare numeric
/// parts, then optional `-prerelease` and `+build` identifier runs.
#[derive(Debug, PartialEq, Eq)]
struct Release {
    core: [u64; 3],
    prerelease: Vec<String>,
}

impl Release {
    fn parse(version: &str) -> Option<Self> {
        let without_build = match version.split_once('+') {
            Some((core, build)) if valid_identifiers(build) => core,
            Some(_) => return None,
            None => version,
        };
        let (core, prerelease) = match without_build.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, pre),
            Some(_) => return None,
            None => (without_build, ""),
        };
        let mut parts = core.split('.');
        let mut numbered = [0u64; 3];
        for slot in &mut numbered {
            let part = parts.next()?;
            if part.is_empty() || (part.len() > 1 && part.starts_with('0')) {
                return None;
            }
            if !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *slot = part.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        let prerelease = if prerelease.is_empty() {
            Vec::new()
        } else {
            if !valid_identifiers(prerelease) {
                return None;
            }
            prerelease.split('.').map(str::to_string).collect()
        };
        Some(Release { core: numbered, prerelease })
    }

    fn cmp_precedence(&self, other: &Release) -> std::cmp::Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| cmp_prerelease(&self.prerelease, &other.prerelease))
    }
}

fn valid_identifiers(run: &str) -> bool {
    !run.is_empty()
        && run.split('.').all(|id| {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!id.bytes().all(|b| b.is_ascii_digit()) || id.len() == 1 || !id.starts_with('0'))
        })
}

fn cmp_prerelease(left: &[String], right: &[String]) -> std::cmp::Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => {
            for pair in left.iter().zip(right.iter()) {
                let ordering = cmp_prerelease_id(pair.0, pair.1);
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
    }
}

fn cmp_prerelease_id(left: &str, right: &str) -> std::cmp::Ordering {
    match (
        left.bytes().all(|b| b.is_ascii_digit()),
        right.bytes().all(|b| b.is_ascii_digit()),
    ) {
        (true, true) => left.len().cmp(&right.len()).then_with(|| left.cmp(right)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => left.cmp(right),
    }
}

pub(crate) fn client_version_is_newer(client: &str, daemon: &str) -> bool {
    compare_daemon_version(client, daemon) == Some(std::cmp::Ordering::Greater)
}

/// Whether `pid` may still be running (see [`alive_from_state`]).
pub(crate) fn process_is_alive(pid: u32) -> bool {
    alive_from_state(pid, || {
        kunobi_daemon::local::process_state(
            kunobi_daemon::ProcessId::new(pid).expect("alive_from_state rejects PID zero"),
        )
    })
}

/// Whether a process the OS reports as `state` may still be running. A state
/// the OS could not establish counts as
/// running: both callers wait or report rather than act on it, and waiting
/// is the safe error. PIDs 0 and 1 and values past `i32::MAX` never name a
/// daemon (the first is the kernel's, the second init's, the last read as
/// broadcasts), so a corrupt record naming one is not waited on.
fn alive_from_state(pid: u32, state: impl FnOnce() -> kunobi_daemon::local::ProcessState) -> bool {
    use kunobi_daemon::local::ProcessState;
    if pid <= 1 || i32::try_from(pid).is_err() {
        return false;
    }
    !matches!(state(), ProcessState::Exited)
}

fn wait_for_run_lock_release(socket_path: &Path, timeout: Duration) -> Result<bool> {
    Ok(
        kunobi_daemon::readiness::wait_until(Instant::now() + timeout, |_| {
            daemon_run_lock_is_held(socket_path).map(|held| (!held).then_some(()))
        })?
        .is_some(),
    )
}

/// Who started a daemon sweep. A requested `kache gc` waits for the lock
/// and runs its size pass; the timer skips a busy lock and follows the
/// shared trigger and backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GcDriver {
    Requested,
    Periodic,
}

impl GcDriver {
    /// A sweep the user asked for may evict fresh imports; the timer keeps
    /// them (#1008).
    fn sweep_origin(self) -> crate::store::SweepOrigin {
        match self {
            GcDriver::Requested => crate::store::SweepOrigin::Requested,
            GcDriver::Periodic => crate::store::SweepOrigin::Automatic,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum GcPolicy {
    Automatic { max_age_hours: u64 },
    ExplicitAge { hours: u64 },
}

impl GcPolicy {
    fn mode(self) -> GcRequestMode {
        match self {
            Self::Automatic { .. } => GcRequestMode::Automatic,
            Self::ExplicitAge { .. } => GcRequestMode::ExplicitAge,
        }
    }
}

/// A GC policy can select the same protected entry in several sweeps. The
/// wire format carries counts rather than keys, so an exact set union is not
/// available here. Report the largest single-sweep count as a non-duplicating
/// lower bound; explicit-age requests have no size sweep and use their sole
/// policy count directly.
fn gc_entries_pinned_lower_bound(
    policy: GcPolicy,
    duplicate: usize,
    age: usize,
    size: usize,
) -> usize {
    match policy {
        GcPolicy::ExplicitAge { .. } => age,
        GcPolicy::Automatic { .. } => duplicate.max(age).max(size),
    }
}

fn resolve_gc_request(request: &GcRequest, daemon_max_age_hours: u64) -> Result<GcPolicy> {
    Ok(match request.mode {
        GcRequestMode::Automatic => GcPolicy::Automatic {
            max_age_hours: request.effective_max_age_hours.ok_or_else(|| {
                anyhow::anyhow!("automatic GC request is missing effective_max_age_hours")
            })?,
        },
        GcRequestMode::ExplicitAge => GcPolicy::ExplicitAge {
            hours: request.max_age_hours.ok_or_else(|| {
                anyhow::anyhow!("explicit_age GC request is missing max_age_hours")
            })?,
        },
        GcRequestMode::Legacy => match request.max_age_hours {
            Some(hours) => GcPolicy::ExplicitAge { hours },
            None => GcPolicy::Automatic {
                max_age_hours: daemon_max_age_hours,
            },
        },
    })
}

/// Cache dir a RemoteCheck should import into.
///
/// `None` / blank keeps the main store. A path is admitted only when it is
/// the main cache dir or a configured volume shard.
fn stats_window(request: &StatsRequest) -> crate::since::SinceWindow {
    use crate::since::SinceWindow;
    match (request.event_secs, request.event_hours) {
        (Some(secs), _) => SinceWindow::from_secs(secs),
        (None, Some(hours)) => SinceWindow::from_hours(hours).unwrap_or(SinceWindow::DEFAULT),
        (None, None) => SinceWindow::DEFAULT,
    }
}

fn effective_config(
    config: &Config,
    provenance: &crate::config::ConfigFileProvenance,
) -> EffectiveConfig {
    EffectiveConfig {
        process: Some(crate::platform::process_context()),
        max_size: config.max_size,
        cache_dir: config.cache_dir.display().to_string(),
        runtime_dir: config.runtime_dir.display().to_string(),
        config_path: provenance.path.display().to_string(),
        config_fingerprint: Some(provenance.fingerprint.clone()),
        socket_path: config.socket_path().display().to_string(),
        started_at_ms: unix_time_ms(),
        auto_recover_min_free_bytes: Some(config.auto_recover_min_free_bytes),

    }
}

fn gc_policy_outcome(stats: &crate::store::GcStats) -> GcPolicyOutcome {
    GcPolicyOutcome {
        entries_evicted: stats.entries_evicted,
        bytes_freed: stats.bytes_freed,
        disk_bytes_reclaimed: stats.disk_bytes_reclaimed,
        entries_pinned: stats.entries_pinned,
        entries_unreclaimable: stats.entries_unreclaimable,
        entries_failed: stats.entries_failed,
        entries_locked: stats.entries_locked,
        evict_write_ms: stats.evict_write_ms,
        bytes_held: stats.bytes_held,
    }
}

// ── Transfer tracking ────────────────────────────────────────────

/// Ms since the Unix epoch (0 on a pre-epoch clock).
fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileHashCacheKey {
    path: String,
    size: i64,
    mtime_ns: i64,
    ctime_ns: i64,
    inode: i64,
}

#[derive(Debug, Clone)]
struct GcRunReport {
    mode: GcRequestMode,
    duplicate: crate::store::GcStats,
    age: crate::store::GcStats,
    size: crate::store::GcStats,
    total: crate::store::GcStats,
}

impl GcRunReport {
    fn skipped(mode: GcRequestMode) -> Self {
        Self {
            mode,
            duplicate: crate::store::GcStats::default(),
            age: crate::store::GcStats::default(),
            size: crate::store::GcStats::default(),
            total: crate::store::GcStats {
                skipped: true,
                ..Default::default()
            },
        }
    }

    fn breakdown(&self) -> GcBreakdown {
        GcBreakdown {
            mode: self.mode,
            duplicate: gc_policy_outcome(&self.duplicate),
            age: gc_policy_outcome(&self.age),
            size: gc_policy_outcome(&self.size),
        }
    }
}

// ── Daemon (the "lib" — all business logic, no I/O) ─────────────

pub(crate) struct Daemon {
    config: Config,
    store: OnceLock<Mutex<Store>>,
    /// Finished cc compiles handed off by wrappers, waiting to be stored.
    publish_queue: crate::daemon_publish::PublishQueue,
    /// In-flight miss compiles keyed by child PID (kunobi-ninja/kache#131).
    /// Upserted by CompileStarted, removed by CompileFinished, and pruned by
    /// liveness/age on both read (stats) and write (register) paths — a
    /// crashed wrapper must not leave a ghost entry forever.
    in_flight_compiles: std::sync::Mutex<HashMap<u32, CompileStartedRequest>>,
    version: String,
    /// What this daemon actually loaded, reported in every stats response so
    /// daemon-backed CLI reads can render the daemon's view and name a
    /// CLI/daemon config divergence (kunobi-ninja/kache#689).
    effective_config: EffectiveConfig,
    file_hash_cache: Arc<Mutex<HashMap<FileHashCacheKey, String>>>,
    /// When the last build request arrived. Index compaction waits for a
    /// gap here, because a build made of cache hits holds no compile permit.
    request_clock: Arc<crate::maintenance::RequestClock>,
    /// Set while a hinted sweep is queued or running; further hints coalesce.
    gc_hint_pending: AtomicBool,
    /// Bound pipeline requests. Compiler scheduling has its own permits.
    execution_workers: Arc<crate::lifecycle::Semaphore>,
    capacity: DaemonCapacity,
}

impl Daemon {
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    pub(crate) fn publish_queue(&self) -> &crate::daemon_publish::PublishQueue {
        &self.publish_queue
    }

    #[cfg(test)]
    pub fn new(config: Config) -> Self {
        let provenance = crate::config::ConfigFileProvenance::current();
        let capacity = DaemonCapacity::discover(config.daemon_execution_concurrency);
        Self::new_with_provenance(config, &provenance, capacity)
    }

    fn new_with_provenance(
        config: Config,
        provenance: &crate::config::ConfigFileProvenance,
        capacity: DaemonCapacity,
    ) -> Self {
        Self {
            store: OnceLock::new(),
            publish_queue: crate::daemon_publish::PublishQueue::new(),
            in_flight_compiles: std::sync::Mutex::new(HashMap::new()),
            version: VERSION.to_string(),
            effective_config: effective_config(&config, provenance),
            file_hash_cache: Arc::new(Mutex::new(HashMap::new())),
            request_clock: Arc::new(crate::maintenance::RequestClock::new()),
            gc_hint_pending: AtomicBool::new(false),
            execution_workers: Arc::new(crate::lifecycle::Semaphore::new(capacity.executions)),
            capacity,
            config,
        }
    }

    fn store_lock(&self) -> Result<&Mutex<Store>> {
        if let Some(store) = self.store.get() {
            return Ok(store);
        }

        let store = Store::open(&self.config)?;
        let _ = self.store.set(Mutex::new(store));

        self.store
            .get()
            .ok_or_else(|| anyhow::anyhow!("daemon store failed to initialize"))
    }

    pub(crate) fn with_store<T>(&self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        let guard = self
            .store_lock()?
            .lock()
            .map_err(|_| anyhow::anyhow!("daemon store mutex poisoned"))?;
        f(&guard)
    }

    #[cfg(test)]
    pub fn handle_request_sync(&self, req: &Request) -> Response {
        match req {
            Request::Gc(gc) | Request::GcV2(gc) => self.handle_gc(gc),
            Request::GcHint => {
                if self.claim_gc_hint() {
                    self.run_hinted_sweep();
                }
                Response::ok()
            }
            Request::Stats(sr) => self.handle_stats(sr),
            Request::Health => self.handle_health(),
            Request::HashFiles(req) => self.handle_hash_files(req),
            Request::CompileStarted(req) => self.handle_compile_started(req.clone()),
            Request::CompileFinished(req) => self.handle_compile_finished(req),
            Request::ExecuteV1(_)
            | Request::ExecutionStatus { .. }
            | Request::PublishCc(_)
            | Request::Prestage(_) => {
                Response::err("execute/publish_cc/prestage need a connection")
            }
            Request::Shutdown => Response::ok(),
        }
    }

    fn handle_health(&self) -> Response {
        Response {
            health: Some(DaemonHealth {
                version: self.version.clone(),
                build_epoch: build_epoch(),
            }),
            ..Response::ok()
        }
    }

    /// Handle a stats request — reads store and event log.
    pub fn handle_stats(&self, req: &StatsRequest) -> Response {
        let inventory = match self.with_store(|store| {
            crate::store_view::read_with_main(
                &self.config,
                store,
                req.include_entries,
                req.sort_by.as_deref().unwrap_or("size"),
            )
        }) {
            Ok(inventory) => inventory,
            Err(error) => return Response::err(format!("store open failed: {error}")),
        };

        let since = stats_window(req).cutoff(UtcTime::now());
        let es =
            events::event_stats_since(&self.config.event_log_path(), since).unwrap_or_default();
        let _ = req.include_summaries;

        let in_flight = self.in_flight_snapshot();

        Response::ok_stats(StatsResponse {
            total_size: inventory.total_size,
            max_size: inventory.max_size,
            entry_count: inventory.entry_count,
            entries: req.include_entries.then_some(inventory.entries),
            stores: inventory.stores,
            events: EventStatsResponse {
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
            blob_stats: Some(inventory.blob_stats),
            version: self.version.clone(),
            build_epoch: build_epoch(),
            gc_policy_version: GC_POLICY_PROTOCOL_VERSION,
            in_flight,
            effective_config: Some(self.effective_config.clone()),
        })
    }

    /// Upsert an in-flight compile (kunobi-ninja/kache#131). Sync and tiny —
    /// no offload needed. Prunes on the way in so the map can't accumulate
    /// ghosts even if nobody ever asks for stats.
    pub fn handle_compile_started(&self, req: CompileStartedRequest) -> Response {        if let Ok(mut map) = self.in_flight_compiles.lock() {
            prune_in_flight(&mut map);
            map.insert(req.pid, req);
        }
        Response::ok()
    }

    /// Flush a batch of entries stored without an fsync, in the main store and
    /// in each `[cache.volumes]` shard. Uses its own store connections, as GC
    /// does, so a sweep of slow writes never holds the mutex a lookup needs.
    fn flush_pending_durability(&self) {
        Self::flush_store_durability(&self.config);
        for shard in &self.config.volume_stores {
            // Opening a shard without an index would create it on a volume
            // that may not be mounted.
            if crate::volume_gc::shard_has_index(&shard.store) {
                Self::flush_store_durability(&self.config.for_volume_store(shard, |_| None));
            }
        }
    }

    fn flush_store_durability(config: &Config) {
        let flushed = (|| -> Result<usize> {
            let store = Store::open(config)?;
            if store.pending_durability()? == 0 {
                return Ok(0);
            }
            let Some(_lock) = store.try_durability_flush_lock()? else {
                return Ok(0);
            };
            store.flush_durability(crate::cli::DURABILITY_FLUSH_BATCH)
        })();
        match flushed {
            Ok(0) => {}
            Ok(n) => log::debug!("flushed {n} entries to disk"),
            Err(error) => log::debug!("durability flush failed: {error:#}"),
        }
    }

    pub fn handle_compile_finished(&self, req: &CompileFinishedRequest) -> Response {
        if let Ok(mut map) = self.in_flight_compiles.lock()
            && let Some(entry) = map.get(&req.pid)
            && (req.started_at_ms == 0 || entry.started_at_ms == req.started_at_ms)
        {
            map.remove(&req.pid);
        }
        Response::ok()
    }

    /// Snapshot the in-flight registry for stats consumers, computing
    /// elapsed/ETA from wall-clock and pruning dead entries first.
    fn in_flight_snapshot(&self) -> Vec<InFlightEntry> {
        let Ok(mut map) = self.in_flight_compiles.lock() else {
            return Vec::new();
        };
        prune_in_flight(&mut map);
        let now_ms = unix_time_ms();
        let mut entries: Vec<InFlightEntry> = map
            .values()
            .map(|c| {
                let elapsed_s = now_ms.saturating_sub(c.started_at_ms) / 1000;
                let typical_s = c.typical_ms.map(|ms| ms.div_ceil(1000));
                InFlightEntry {
                    crate_name: c.crate_name.clone(),
                    root: c.root.clone(),
                    pid: c.pid,
                    elapsed_s,
                    typical_s,
                    eta_s: typical_s.map(|t| t.saturating_sub(elapsed_s)),
                }
            })
            .collect();
        // Oldest first — the entry a user is most likely waiting on.
        entries.sort_by_key(|e| std::cmp::Reverse(e.elapsed_s));
        entries
    }

    pub fn handle_hash_files(&self, req: &HashFilesRequest) -> Response {
        self.handle_hash_files_at(req, crate::cache_key::wall_clock_ns)
    }

    /// [`Self::handle_hash_files`] with the wall clock supplied.
    fn handle_hash_files_at(&self, req: &HashFilesRequest, clock: impl Fn() -> i64) -> Response {
        let mut results = Vec::with_capacity(req.files.len());

        for file in &req.files {
            let key = FileHashCacheKey {
                path: file.path.clone(),
                size: file.size,
                mtime_ns: file.mtime_ns,
                ctime_ns: file.ctime_ns,
                inode: file.inode,
            };

            if let Ok(cache) = self.file_hash_cache.lock()
                && let Some(hash) = cache.get(&key).cloned()
            {
                results.push(HashFileResult {
                    path: file.path.clone(),
                    size: file.size,
                    mtime_ns: file.mtime_ns,
                    ctime_ns: file.ctime_ns,
                    inode: file.inode,
                    hash: Some(hash),
                    cache_hit: true,
                    bytes_hashed: 0,
                    error: None,
                });
                continue;
            }

            // Read before the stats below, so it is no later than the moment
            // either of them saw the stamp.
            let observed_ns = clock();
            match std::fs::metadata(&file.path) {
                Ok(metadata)
                    if i64::try_from(metadata.len()).unwrap_or(i64::MAX) == file.size
                        && crate::cache_key::metadata_mtime_ns(&metadata) == file.mtime_ns
                        && crate::cache_key::metadata_ctime_ns(&metadata) == file.ctime_ns
                        && crate::cache_key::metadata_inode(&metadata) == file.inode => {}
                Ok(_) => {
                    results.push(HashFileResult {
                        path: file.path.clone(),
                        size: file.size,
                        mtime_ns: file.mtime_ns,
                        ctime_ns: file.ctime_ns,
                        inode: file.inode,
                        hash: None,
                        cache_hit: false,
                        bytes_hashed: 0,
                        error: Some("file metadata changed before hashing".into()),
                    });
                    continue;
                }
                Err(e) => {
                    results.push(HashFileResult {
                        path: file.path.clone(),
                        size: file.size,
                        mtime_ns: file.mtime_ns,
                        ctime_ns: file.ctime_ns,
                        inode: file.inode,
                        hash: None,
                        cache_hit: false,
                        bytes_hashed: 0,
                        error: Some(e.to_string()),
                    });
                    continue;
                }
            }

            // A stamp that had changed within the settle window when it was
            // observed can be kept by a second write in the same tick, so
            // its hash goes back to this caller and is remembered nowhere.
            let settled = crate::cache_key::stamp_is_settled(
                &crate::cache_key::FileFingerprint {
                    path: file.path.clone(),
                    size: file.size,
                    mtime_ns: file.mtime_ns,
                    ctime_ns: file.ctime_ns,
                    inode: file.inode,
                },
                observed_ns,
            );

            // #281: hold the store mutex only for the cheap cache lookup and
            // record; run the blake3 read of the whole file OUTSIDE the lock so
            // it can't stall a concurrent RemoteCheck's `import_restored_entry`.
            let path = Path::new(&file.path);
            let computed: anyhow::Result<(String, bool, u64)> =
                match self.with_store(|store| Ok(store.file_hash_lookup(path))) {
                    Ok(crate::cache_key::FileHashLookup::Hit(hash)) => Ok((hash, true, 0)),
                    Ok(crate::cache_key::FileHashLookup::NeedsHash(fp)) => {
                        crate::cache_key::hash_file(path).map(|hash| {
                            // The lookup took its own stamp, after `observed_ns`.
                            if crate::cache_key::stamp_is_settled(&fp, observed_ns) {
                                // Brief re-lock just to persist the result.
                                let _ = self.with_store(|store| {
                                    store.file_hash_record(&fp, &hash);
                                    Ok(())
                                });
                            }
                            (hash, false, file.size.max(0) as u64)
                        })
                    }
                    Ok(crate::cache_key::FileHashLookup::Uncacheable) => {
                        crate::cache_key::hash_file(path)
                            .map(|hash| (hash, false, file.size.max(0) as u64))
                    }
                    Err(e) => Err(e),
                };

            match computed {
                Ok((hash, cache_hit, bytes_hashed)) => {
                    if settled && let Ok(mut cache) = self.file_hash_cache.lock() {
                        if cache.len() >= FILE_HASH_MEMORY_CACHE_CAP {
                            cache.clear();
                        }
                        cache.insert(key, hash.clone());
                    }

                    results.push(HashFileResult {
                        path: file.path.clone(),
                        size: file.size,
                        mtime_ns: file.mtime_ns,
                        ctime_ns: file.ctime_ns,
                        inode: file.inode,
                        hash: Some(hash),
                        cache_hit,
                        bytes_hashed,
                        error: None,
                    });
                }
                Err(e) => results.push(HashFileResult {
                    path: file.path.clone(),
                    size: file.size,
                    mtime_ns: file.mtime_ns,
                    ctime_ns: file.ctime_ns,
                    inode: file.inode,
                    hash: None,
                    cache_hit: false,
                    bytes_hashed: 0,
                    error: Some(e.to_string()),
                }),
            }
        }

        Response::ok_hash_results(results)
    }

    /// Handle a GC request — pure logic against the store.
    pub fn handle_gc(&self, req: &GcRequest) -> Response {
        let policy = match resolve_gc_request(req, self.config.gc_max_age_hours) {
            Ok(policy) => policy,
            Err(e) => return Response::err(format!("invalid GC request: {e}")),
        };
        match self.run_gc(policy, GcDriver::Requested) {
            Ok(report) if report.total.skipped => Response::ok_gc_skipped(report.breakdown()),
            Ok(report) => Response::ok_gc(report.total.entries_evicted, report.breakdown()),
            Err(e) => Response::err(format!("gc failed: {e}")),
        }
    }

    /// Finalize the active plan if it has been inactive for `inactivity_ms`.
    /// Called from the periodic sweep; cargo gives no positive end-of-build
    /// signal, so inactivity IS the end signal (#583 P0.5).
    fn claim_gc_hint(&self) -> bool {
        !self.gc_hint_pending.swap(true, Ordering::SeqCst)
    }

    /// Stage the target directory's recorded executables in the background;
    /// the wrapper that hinted does not wait.
    fn handle_prestage(self: &Arc<Self>, req: &PrestageRequest) -> Response {
        let cache_dir = self.config.cache_dir.clone();
        let store_dir = self.config.store_dir();
        let target_dir = PathBuf::from(&req.target_dir);
        let _ = std::thread::Builder::new()
            .name("kache-prestage".to_string())
            .spawn(move || {
                crate::prestage::stage(&cache_dir, &target_dir, |hash| {
                    crate::store::blob_path_in_store_dir(&store_dir, hash)
                });
            });
        Response::ok()
    }

    /// A wrapper's size-pressure hint: acknowledge now, sweep on the blocking
    /// pool (#281). One sweep, where the wrapper's own worker sweeps twice:
    /// that worker exits and has no later chance at entries a live build
    /// pins, while the daemon sweeps again on the next hint or upload after
    /// the backoff.
    fn handle_gc_hint(self: &Arc<Self>) -> Response {
        if self.claim_gc_hint() {
            let daemon = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name("kache-gc-hint".to_string())
                .spawn(move || daemon.run_hinted_sweep());
        }
        Response::ok()
    }

    /// A delegated compiler already resolved its cache policy. Keep that
    /// snapshot when it requests eviction after a put. The wrapper's shared
    /// throttle limits requests; each sweep also takes the store's gc.lock.
    pub(crate) fn request_gc_for_invocation(self: &Arc<Self>, config: Config) {
        let daemon = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("kache-invocation-gc".to_string())
            .spawn(move || {
                if let Err(error) = daemon.sweep_invocation_under_size_pressure(&config) {
                    log::warn!("invocation GC sweep failed: {error:#}");
                }
            });
    }

    fn sweep_invocation_under_size_pressure(&self, config: &Config) -> Result<()> {
        let main = self.sweep_shard_under_size_pressure(config);
        crate::volume_gc::run_on_volume_shards(config, |shard| {
            self.sweep_shard_under_size_pressure(shard)
        });
        main
    }

    fn run_hinted_sweep(&self) {
        // Released on unwind too, or one panic would swallow every later hint.
        struct Release<'a>(&'a AtomicBool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _release = Release(&self.gc_hint_pending);
        if let Err(e) = self.sweep_under_size_pressure() {
            log::warn!("hinted GC sweep failed: {e:#}");
        }
    }

    /// The sweep behind the post-upload check and wrapper hints, of the main
    /// store and then each `[cache.volumes]` shard. A shard's failure is
    /// logged and never fails the main store's sweep.
    fn sweep_under_size_pressure(&self) -> Result<()> {
        let main = self.sweep_main_under_size_pressure();
        crate::volume_gc::run_on_volume_shards(&self.config, |shard| {
            self.sweep_shard_under_size_pressure(shard)
        });
        main
    }

    /// One shard's size-pressure sweep, under that shard's `gc.lock`.
    fn sweep_shard_under_size_pressure(&self, shard: &Config) -> Result<()> {
        crate::disk_recovery::run(shard);
        let store = Store::open(shard)?;
        let Some(_gc_lock) = store.try_gc_lock()? else {
            log::debug!(
                "gc.lock of shard {} held by another GC; skipping size-pressure eviction",
                shard.cache_dir.display()
            );
            return Ok(());
        };
        let size = store.physical_size()?;
        self.size_pressure_sweep_if_due(shard, size)
    }

    /// The main store's size-pressure sweep. Skips when another driver holds
    /// `gc.lock`, the store is under the trigger, or the backoff holds; each
    /// costs a lock attempt and one size query.
    fn sweep_main_under_size_pressure(&self) -> Result<()> {
        crate::disk_recovery::run(&self.config);
        let Some((_gc_lock, size)) = self.with_store(|store| {
            let Some(lock) = store.try_gc_lock()? else {
                return Ok(None);
            };
            // The size check is cheap; release the daemon's Store mutex
            // before the long eviction scan and per-entry removals.
            Ok(Some((lock, store.physical_size()?)))
        })?
        else {
            log::debug!("gc.lock held by another GC; skipping size-pressure eviction");
            return Ok(());
        };
        self.size_pressure_sweep_if_due(&self.config, size)
    }

    /// Sweep the store of `config`, now at `size`, if the shared trigger
    /// says a sweep is due. Caller holds that store's `gc.lock`.
    fn size_pressure_sweep_if_due(&self, config: &Config, size: u64) -> Result<()> {
        if !crate::wrapper::auto_gc_sweep_due(config, size) {
            return Ok(());
        }
        // Under gc.lock like every driver, so the totals cannot race.
        let store = Store::open(config)?;
        let stats = self.automatic_size_pass(config, &store, size)?;
        if let Err(e) = crate::report::record_gc_run(
            config,
            "daemon",
            crate::store::SweepOrigin::Automatic,
            &stats,
        ) {
            log::warn!("recording size-pressure GC run: {e:#}");
        }
        Ok(())
    }

    /// The size pass of an automatic sweep the shared trigger found due.
    /// Records where the store ended, so a sweep that could not clear the
    /// pressure backs off every automatic driver. Caller holds `gc.lock`.
    fn automatic_size_pass(
        &self,
        config: &Config,
        store: &Store,
        size: u64,
    ) -> Result<crate::store::GcStats> {
        log::info!(
            "store {} size {} over the automatic trigger (max {}), running LRU eviction",
            config.cache_dir.display(),
            size,
            config.max_size
        );
        let started = Instant::now();
        let mut stats = store.evict_for(crate::store::SweepOrigin::Automatic)?;
        stats.duration_ms = started.elapsed().as_millis() as u64;
        if let Ok(after) = store.physical_size() {
            crate::wrapper::record_auto_gc_outcome(config, after, stats.bytes_held);
        }
        Ok(stats)
    }

    /// The size pass of a full sweep. A requested `kache gc` always runs it
    /// and leaves the backoff alone. The timer runs it only when the shared
    /// trigger says a sweep is due; its age and duplicate passes are not
    /// size pressure and stay on schedule.
    fn size_pass(
        &self,
        config: &Config,
        driver: GcDriver,
        store: &Store,
    ) -> Result<crate::store::GcStats> {
        if driver == GcDriver::Requested {
            return store.evict();
        }
        let size = store.physical_size()?;
        if !crate::wrapper::auto_gc_sweep_due(config, size) {
            log::info!("periodic GC: size pass not due (under the trigger or backing off)");
            return Ok(crate::store::GcStats::default());
        }
        self.automatic_size_pass(config, store, size)
    }

    /// Sweep the main store, then each `[cache.volumes]` shard with its own
    /// budget (kunobi-ninja/kache#974). The report describes the main store.
    /// Each store's `gc.lock` is its own: a shard another GC holds is skipped,
    /// and a busy main store does not stop the shards from being swept.
    fn run_gc(&self, policy: GcPolicy, driver: GcDriver) -> Result<GcRunReport> {
        let report = self.run_gc_store(&self.config, policy, driver)?;
        crate::volume_gc::run_on_volume_shards(&self.config, |shard| {
            self.run_gc_store(shard, policy, driver)
        });
        Ok(report)
    }

    /// Core GC logic for the store of `config` with an explicit policy and
    /// per-policy result accounting. Holds that store's `gc.lock`.
    fn run_gc_store(
        &self,
        config: &Config,
        policy: GcPolicy,
        driver: GcDriver,
    ) -> Result<GcRunReport> {
        let start = Instant::now();
        let mode = policy.mode();
        // Cross-process GC mutual exclusion (kunobi-ninja/kache#326): if another
        // GC driver holds gc.lock, automatic sweeps skip. Requested sweeps
        // wait so a startup sweep cannot discard the requested age policy.
        // GC may scan and remove thousands of entries. Use its own connection
        // so daemon lookups and uploads can still reach the main Store mutex.
        // SQLite and gc.lock continue to serialize the actual writes.
        let gc_store = Store::open(config)?;
        let _gc_lock = match driver {
            GcDriver::Requested => gc_store.acquire_gc_lock()?,
            GcDriver::Periodic => match gc_store.try_gc_lock()? {
                Some(lock) => lock,
                None => {
                    log::info!("gc.lock held by another GC; skipping this run");
                    return Ok(GcRunReport::skipped(mode));
                }
            },
        };
        let (dedup_stats, evict_stats, age_evict_stats, orphan_stats) =
            (|| -> Result<_> {
                let store = &gc_store;
                // Backfill content_hash for legacy entries
                let backfilled = store.backfill_content_hashes().unwrap_or(0);
                if backfilled > 0 {
                    log::info!("backfilled {backfilled} content hashes");
                }

                // Backfill rebuild cost for entries written before it was
                // indexed (#594), so a value-aware policy has data to work with.
                let costs = store.backfill_compile_times().unwrap_or(0);
                if costs > 0 {
                    log::info!("backfilled {costs} compile times");
                }

                // Backfill entry→blob rows for entries written before the
                // table existed (#608), so eviction can rank on the bytes an
                // entry would actually free.
                let mapped = store.backfill_entry_blobs().unwrap_or(0);
                if mapped > 0 {
                    log::info!("backfilled {mapped} entry blob maps");
                }

                match store.file_hash_cache().prune_cc_preprocess_memos() {
                    Ok((memos, inputs)) => log::debug!("gc: pruned C/C++ memos; memos={memos} inputs={inputs}"),
                    Err(error) => log::warn!("gc: C/C++ memo pruning failed: {error}"),
                }
                let markers = crate::wrapper::prune_session_markers(
                    &self.config,
                    crate::wrapper::SESSION_MARKER_RETENTION,
                    std::time::SystemTime::now(),
                );
                log::debug!("gc: pruned build-session markers; markers={markers}");

                // Bound the post-eviction demand log, and report what it says
                // so far: a high demand rate means eviction is discarding
                // entries the build still wants (#594).
                let pruned = store
                    .prune_tombstones(crate::store::TOMBSTONE_RETENTION_DAYS)
                    .unwrap_or(0);
                if let Ok((tracked, demanded)) = store.tombstone_stats()
                    && tracked > 0
                {
                    log::info!(
                        "gc: post-eviction demand; tracked={tracked} demanded={demanded} pruned={pruned} demand_rate_pct={}",
                        demanded * 100 / tracked.max(1)
                    );
                }
                // The #594 policy comparison: demand rate on entries the
                // value-density shadow would have KEPT vs entries it agreed
                // to evict. A markedly higher rate on the kept cohort is the
                // evidence for flipping the live policy; comparable rates
                // are the evidence against.
                if let Ok(split) = store.shadow_demand_split()
                    && split.agreed + split.shadow_kept > 0
                {
                    log::info!(
                        "gc: post-eviction demand by shadow verdict (value-density, #594); shadow_agreed={} shadow_agreed_demanded={} shadow_kept={} shadow_kept_demanded={}",
                        split.agreed,
                        split.agreed_demanded,
                        split.shadow_kept,
                        split.shadow_kept_demanded
                    );
                }
                // Both sides over the same sweeps: what each policy's
                // evictions cost in rebuilds the build later asked for.
                if let Ok(cost) = store.shadow_counterfactual()
                    && cost.shadow.evicted > 0
                {
                    log::info!(
                        "gc: rebuild cost of each policy's evictions (value-density, #594); live_evicted={} live_bytes={} live_demanded_compile_time_ms={} shadow_evicted={} shadow_bytes={} shadow_demanded_compile_time_ms={}",
                        cost.live.evicted,
                        cost.live.bytes,
                        cost.live.demanded_compile_time_ms,
                        cost.shadow.evicted,
                        cost.shadow.bytes,
                        cost.shadow.demanded_compile_time_ms
                    );
                }

                let (dedup_stats, age_evict_stats, evict_stats) = match policy {
                    GcPolicy::ExplicitAge { hours } => (
                        crate::store::GcStats::default(),
                        store.evict_older_than(hours)?,
                        crate::store::GcStats::default(),
                    ),
                    GcPolicy::Automatic { max_age_hours } => {
                        // Expire opt-in stale entries first. Duplicate and size
                        // pressure then observe the reduced physical store and
                        // cannot evict fresh entries for pressure age already
                        // relieved.
                        let age_stats = if max_age_hours > 0 {
                            store.evict_older_than(max_age_hours)?
                        } else {
                            crate::store::GcStats::default()
                        };
                        let duplicate_stats = store
                            .evict_duplicate_entries_for(driver.sweep_origin())
                            .unwrap_or_default();
                        let size_stats = self.size_pass(config, driver, store)?;
                        (duplicate_stats, age_stats, size_stats)
                    }
                };
                if dedup_stats.entries_evicted > 0 {
                    log::info!("evicted {} duplicate entries", dedup_stats.entries_evicted);
                }
                if age_evict_stats.entries_evicted > 0 {
                    log::info!(
                        "evicted {} entries by age policy",
                        age_evict_stats.entries_evicted
                    );
                }

                crate::build_script::sweep_hermetic_out_dirs_for_gc(&self.config.cache_dir);

                // Reclaim orphaned blob files (crash mid-put, or a meta-less
                // remove_entry that couldn't decrement refcounts). The grace
                // leaves blobs a concurrent build is materializing untouched; they
                // get reclaimed on a later pass once settled.
                let orphan_stats = store
                    .sweep_orphan_blobs(ORPHAN_BLOB_GRACE)
                    .unwrap_or_default();
                // Same grace for put-phase staging snapshots abandoned by a
                // crash between staging and publish (review finding #3).
                let staging_stats = store.sweep_stale_staging(crate::store::STAGING_SWEEP_GRACE);
                // Handoff request directories the staging sweep does not
                // enter: a daemon that died with publications queued.
                let handoffs = crate::daemon_publish::sweep_orphaned_handoffs(
                    &self.config,
                    crate::store::STAGING_SWEEP_GRACE,
                );
                if handoffs.removed > 0 {
                    log::info!(
                        "swept {} abandoned cc handoffs ({})",
                        handoffs.removed,
                        crate::report::format_bytes(handoffs.bytes_reclaimed)
                    );
                }
                if staging_stats.removed > 0 {
                    log::info!(
                        "swept {} stale staging files ({})",
                        staging_stats.removed,
                        crate::report::format_bytes(staging_stats.bytes_reclaimed)
                    );
                }
                if orphan_stats.removed > 0 {
                    log::info!(
                        "swept {} of {} blobs as orphans ({} reclaimed)",
                        orphan_stats.removed,
                        orphan_stats.scanned,
                        crate::report::format_bytes(orphan_stats.bytes_reclaimed)
                    );
                }

                Ok((
                    dedup_stats,
                    evict_stats,
                    age_evict_stats,
                    orphan_stats,
                ))
            })()?;

        // Clean up stale tool-version cache files (rustc-ver-*.txt, linker-ver-*.txt).
        // Each toolchain update leaves behind orphaned files keyed by the old binary mtime.
        Self::clean_tool_version_caches(&self.config.cache_dir);

        // Key lock files and input predictions grow with every distinct key
        // and eviction removes neither (#1126). Still under gc.lock.
        let housekeeping = gc_store.sweep_housekeeping();
        let prestage_pruned =
            crate::prestage::prune(&self.config.cache_dir, std::time::SystemTime::now());
        log::info!(
            "gc: housekeeping; key_locks_removed={} key_locks_remaining={} predictions_pruned={} file_hashes_pruned={} prestage_pruned={prestage_pruned}",
            housekeeping.key_locks_removed,
            housekeeping.key_locks_remaining,
            housekeeping.predictions_pruned,
            housekeeping.file_hashes_pruned
        );

        // Aggregate stats
        let stats = crate::store::GcStats {
            entries_evicted: dedup_stats.entries_evicted
                + evict_stats.entries_evicted
                + age_evict_stats.entries_evicted,
            bytes_freed: dedup_stats.bytes_freed
                + evict_stats.bytes_freed
                + age_evict_stats.bytes_freed
                + orphan_stats.bytes_reclaimed,
            blobs_removed: dedup_stats.blobs_removed
                + evict_stats.blobs_removed
                + age_evict_stats.blobs_removed
                + orphan_stats.removed,
            duration_ms: start.elapsed().as_millis() as u64,
            skipped: false,
            entries_pinned: gc_entries_pinned_lower_bound(
                policy,
                dedup_stats.entries_pinned,
                age_evict_stats.entries_pinned,
                evict_stats.entries_pinned,
            ),
            entries_unreclaimable: dedup_stats.entries_unreclaimable
                + evict_stats.entries_unreclaimable
                + age_evict_stats.entries_unreclaimable,
            // Only the size pass measures it.
            bytes_held: evict_stats.bytes_held,
            disk_bytes_reclaimed: dedup_stats.disk_bytes_reclaimed
                + evict_stats.disk_bytes_reclaimed
                + age_evict_stats.disk_bytes_reclaimed,
            entries_failed: dedup_stats.entries_failed
                + evict_stats.entries_failed
                + age_evict_stats.entries_failed,
            entries_locked: dedup_stats.entries_locked
                + evict_stats.entries_locked
                + age_evict_stats.entries_locked,
            entries_busy_snapshot: dedup_stats.entries_busy_snapshot
                + evict_stats.entries_busy_snapshot
                + age_evict_stats.entries_busy_snapshot,
            entries_recent_prefiltered: dedup_stats.entries_recent_prefiltered
                + evict_stats.entries_recent_prefiltered
                + age_evict_stats.entries_recent_prefiltered,
            entries_import_pinned: dedup_stats.entries_import_pinned
                + evict_stats.entries_import_pinned
                + age_evict_stats.entries_import_pinned,
            evict_write_ms: dedup_stats.evict_write_ms
                + evict_stats.evict_write_ms
                + age_evict_stats.evict_write_ms,
            housekeeping: Some(housekeeping),
        };

        log::info!(
            "gc complete: {} entries evicted, {} freed, {} blobs removed in {}ms",
            stats.entries_evicted,
            crate::report::format_bytes(stats.bytes_freed),
            stats.blobs_removed,
            stats.duration_ms,
        );

        // Persist GC stats for reports and machine telemetry. Still under
        // gc.lock, so the record cannot race another driver.
        if let Err(e) =
            crate::report::record_gc_run(config, "daemon", driver.sweep_origin(), &stats)
        {
            log::debug!(
                "gc: could not record {}: {e:#}",
                crate::report::GC_STATS_FILE
            );
        }

        Ok(GcRunReport {
            mode,
            duplicate: dedup_stats,
            age: age_evict_stats,
            size: evict_stats,
            total: stats,
        })
    }

    /// Remove tool-version cache files older than 7 days.
    fn clean_tool_version_caches(cache_dir: &Path) {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 24 * 3600);

        let Ok(entries) = std::fs::read_dir(cache_dir) else {
            return;
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name.starts_with("rustc-ver-") || name.starts_with("linker-ver-"))
                && name.ends_with(".txt")
                && let Ok(meta) = entry.metadata()
                && let Ok(modified) = meta.modified()
                && modified < cutoff
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

// ── Server (thin I/O shell) ──────────────────────────────────────

/// Run the daemon server (foreground, blocking).
pub fn run_server(
    config: &Config,
    provenance: &crate::config::ConfigFileProvenance,
    readiness: Option<kunobi_daemon::readiness::channel::Notifier>,
) -> Result<()> {
    // Acquire an exclusive file lock to guarantee only one daemon process runs
    // at a time.  We use a dedicated "daemon.run.lock" (separate from the
    // "daemon.lock" that start_daemon_background uses to serialize *spawning*)
    // so the two never deadlock.
    //
    // The lock is held for the daemon's entire lifetime and is automatically
    // released when this function returns or the process exits/crashes.
    let socket_path = config.socket_path();
    let lock_path = socket_path.with_extension("run.lock");
    std::fs::create_dir_all(socket_path.parent().unwrap())?;

    let Some(_lock) =
        kunobi_daemon::ProcessLock::try_acquire(&lock_path).context("acquiring daemon run lock")?
    else {
        log::info!("another daemon holds the run lock, exiting");
        return Ok(());
    };
    match crate::platform::raise_open_file_limit() {
        Ok(Some((before, after))) => {
            log::info!("raised the open-file limit from {before} to {after}");
        }
        Ok(None) => {}
        Err(error) => log::warn!("could not raise the open-file limit: {error}"),
    }
    let coord = DaemonCoordFile::for_socket(&socket_path);
    coord
        .write_phase(DaemonPhase::Starting)
        .context("writing daemon coordinator state")?;
    let _coord_guard = DaemonCoordGuard::new(coord.path.clone());

    let capacity = DaemonCapacity::discover(config.daemon_execution_concurrency);
    log::info!(
        "daemon request capacity; pipeline_requests={} rpc_handlers={} blocking_threads={}",
        capacity.executions,
        capacity.connections,
        capacity.blocking_threads
    );
    let _ = capacity.blocking_threads;
    server_main(config, provenance, coord, readiness, capacity)
}

/// One upload worker job, start to finish: retried while its outcome is
/// retryable, then its durable intent retired.
///
fn daemon_idle_timeout(seconds: u64) -> Option<Duration> {
    std::num::NonZeroU64::new(seconds).map(|seconds| Duration::from_secs(seconds.get()))
}

/// The shutdown warning for `unpublished` hand-offs the publish worker never
/// stored, or `None` when it stored them all.
fn publish_drain_warning(unpublished: usize) -> Option<String> {
    (unpublished > 0).then(|| {
        format!(
            "publish worker stalled at shutdown; {unpublished} queued hand-offs were not stored"
        )
    })
}

/// Drain accepted publications until the worker finishes or stops making
/// progress.
///
/// Accepted hand-offs hold their key locks. The worker keeps going for as long
/// as it makes progress: a job it never reaches is an entry the next build
/// compiles again.
pub(crate) fn drain_publish_queue(
    daemon: &Daemon,
    publish_done: std::sync::mpsc::Receiver<()>,
) {
    daemon.publish_queue.close();
    let publish_progress = daemon.publish_queue.progress();
    let unpublished = crate::daemon_publish::drain_publications(
        publish_done,
        &publish_progress,
        crate::daemon_publish::PUBLISH_DRAIN_STALL,
    );
    if let Some(warning) = publish_drain_warning(unpublished) {
        log::warn!("{warning}");
    }
}

fn server_main(
    config: &Config,
    provenance: &crate::config::ConfigFileProvenance,
    mut coord: DaemonCoordFile,
    readiness: Option<kunobi_daemon::readiness::channel::Notifier>,
    capacity: DaemonCapacity,
) -> Result<()> {
    let socket_path = config.socket_path();
    std::fs::create_dir_all(socket_path.parent().unwrap())?;

    let Some(listener) = crate::transport::bind_daemon_listener(&socket_path)? else {
        log::info!("another daemon owns the endpoint");
        return Ok(());
    };
    listener
        .set_nonblocking(true)
        .context("daemon listener nonblocking")?;
    let _socket_guard = SocketCleanupGuard::new(&socket_path)?;

    let lifecycle = Arc::new(Lifecycle::default());
    let mut control = lifecycle_control::serve(config, Arc::clone(&lifecycle))?;
    coord.control_version = Some(kunobi_daemon::wire::VERSION);
    coord.write_phase(DaemonPhase::Starting)?;

    let daemon = Arc::new(Daemon::new_with_provenance(
        config.clone(),
        provenance,
        capacity,
    ));

    log::info!("daemon listening on {}", socket_path.display());

    // Exclude cache dir from Time Machine / Spotlight (once, not per-crate).
    #[cfg(target_os = "macos")]
    // Fire-and-forget (#588): the tmutil half runs on a detached thread with
    // its own timeout, so daemon readiness never gates on backupd.
    let _ = crate::store::exclude_from_indexing(&config.cache_dir);

    // The daemon is the longest-lived writer of the WAL index, so a cache dir on
    // a network or guest-visible mount is worth flagging here too — its log is
    // where a user looks after the fact, and a daemonised setup may never show a
    // wrapper's stderr (kunobi-ninja/kache#415). Log-only: the wrapper owns the
    // stderr advisory and its once-per-session dedup.
    match crate::cache_fs::classify(&crate::cache_fs::probe(&config.cache_dir)) {
        crate::cache_fs::CacheFsVerdict::NotLocal { name } => log::warn!(
            "cache directory is not on host-local storage: the WAL index needs working \
             file locking and a single writing machine, and can be corrupted on a shared \
             or network mount. Set KACHE_CACHE_DIR to a local path; to share artifacts \
             between machines use a remote cache instead; cache_dir={} filesystem={name}",
            config.cache_dir.display()
        ),
        verdict => log::debug!(
            "cache filesystem locality check; cache_dir={} verdict={verdict:?}",
            config.cache_dir.display()
        ),
    }

    // Publication of compiles handed off by cc wrappers: one worker thread
    // with its own store connection, fed by a bounded queue the handler
    // fills only after it holds the key lock (see `daemon_publish`).
    let (publish_tx, publish_rx) =
        std::sync::mpsc::sync_channel::<crate::daemon_publish::PublishJob>(
            crate::daemon_publish::PUBLISH_QUEUE_CAPACITY,
        );
    daemon.publish_queue.set_sender(
        publish_tx,
        crate::daemon_publish::PUBLISH_QUEUE_CAPACITY,
    );
    let publish_done = crate::daemon_publish::spawn_publish_worker(daemon.clone(), publish_rx)
        .context("starting the publish worker")?;

    struct StopThread {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl StopThread {
        fn stop(mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    fn stop_thread(name: &str, f: impl FnOnce(&AtomicBool) + Send + 'static) -> StopThread {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || f(&stopping))
            .expect("spawning daemon background thread");
        StopThread {
            stop,
            thread: Some(thread),
        }
    }
    fn sleep_while_running(stop: &AtomicBool, duration: Duration) -> bool {
        let deadline = Instant::now() + duration;
        loop {
            if stop.load(Ordering::Acquire) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(
                Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    // Entries a miss or a download stored without an fsync
    // (`cache.deferred_durability`).
    // Short interval: until an entry is flushed every hit on it re-reads its
    // blobs to verify them, and the wrapper hands the work here precisely so
    // the build does not wait for the disk.
    let durability_daemon = daemon.clone();
    let durability = stop_thread("kache-durability", move |stop| loop {
        // A panic in one flush must not stop the later ones.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            durability_daemon.flush_pending_durability()
        }));
        if sleep_while_running(stop, std::time::Duration::from_secs(2)) {
            return;
        }
    });

    // Periodic GC task: run immediately on startup, then every 6 hours.
    let gc_daemon = daemon.clone();
    let gc = stop_thread("kache-gc", move |stop| loop {
        log::info!("periodic GC sweep starting");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gc_daemon.run_gc(
                GcPolicy::Automatic {
                    max_age_hours: gc_daemon.config.gc_max_age_hours,
                },
                GcDriver::Periodic,
            )
        }));
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => log::warn!("periodic GC failed: {e}"),
            Err(_) => log::warn!("periodic GC task panicked"),
        }
        if sleep_while_running(stop, std::time::Duration::from_secs(6 * 3600)) {
            return;
        }
    });

    let maintenance_handle = config
        .index_auto_compact
        .then(|| crate::maintenance::spawn_periodic(config.clone(), daemon.request_clock.clone()));

    // Background blob migration: lazily migrate legacy entries on startup
    let migration_config = config.clone();
    let _ = std::thread::Builder::new()
        .name("kache-migration".to_string())
        .spawn(move || {
            let result = if let Ok(store) = Store::open(&migration_config) {
                store.migrate_to_blobs(|_, _| {})
            } else {
                Err(anyhow::anyhow!("failed to open store for migration"))
            };

            if let Ok(stats) = result
                && stats.entries_migrated > 0
            {
                log::info!(
                    "background migration: migrated {} entries",
                    stats.entries_migrated,
                );
            }
        });

    // Readiness is published only after application setup can enter the accept loop.
    control.service.mark_ready();
    coord.write_phase(DaemonPhase::Ready)?;
    if let Some(notifier) = readiness {
        let _ = notifier.ready();
    }
    let heartbeat_coord = coord.clone();
    let heartbeat = stop_thread("kache-heartbeat", move |stop| {
        if let Err(e) = heartbeat_coord.write_phase(DaemonPhase::Ready) {
            log::debug!("daemon coordinator heartbeat failed: {e}");
        }
        loop {
            if sleep_while_running(stop, DAEMON_COORD_HEARTBEAT_INTERVAL) {
                return;
            }
            if let Err(e) = heartbeat_coord.write_phase(DaemonPhase::Ready) {
                log::debug!("daemon coordinator heartbeat failed: {e}");
            }
        }
    });

    // Config watchdog: the daemon loads its config once at startup, so an edit
    // to the config file (e.g. `local_max_size`) would otherwise require a
    // manual `kache daemon stop`. Periodically re-fingerprint the active config
    // file; on a change, schedule a graceful restart so the service manager (or
    // the next build's auto-spawn) brings the daemon back up with the new
    // config. This watches only the file the daemon itself resolved — it sends
    // no per-client signal, so it can't thrash across projects.
    let config_provenance = provenance.clone();
    let config_watch_lifecycle = Arc::clone(&lifecycle);

    let config_watch = stop_thread("kache-config-watch", move |stop| loop {
        if sleep_while_running(stop, DAEMON_CONFIG_WATCH_INTERVAL) {
            return;
        }
        if !config_watch_lifecycle.accepting_calls() {
            return;
        }
        if crate::config::config_file_has_changed(&config_provenance) {
            log::info!("config file changed on disk, scheduling restart to reload it");
            config_watch_lifecycle.start_drain();
            return;
        }
    });

    // Idle watchdog: exit if no connections received for this duration.
    // Prevents zombie daemons from accumulating when the user isn't building.
    // The daemon will be auto-started again on the next build.
    // Configurable via KACHE_DAEMON_IDLE_TIMEOUT or config.toml; 0 = disabled.
    let shutdown_lifecycle = Arc::clone(&lifecycle);
    let _ = std::thread::Builder::new()
        .name("kache-shutdown".to_string())
        .spawn(move || {
            shutdown_signal();
            log::info!("shutdown signal received, draining...");
            shutdown_lifecycle.start_drain();
        });

    let idle_timeout = daemon_idle_timeout(config.daemon_idle_timeout_secs);

    accept_loop(
        &listener,
        &daemon,
        &lifecycle,
        idle_timeout,
        CONNECTION_HANDLER_DRAIN_TIMEOUT,
    );

    gc.stop();
    durability.stop();
    if let Some(h) = maintenance_handle {
        h.stop();
    }
    // Stop producers before draining the tasks and taking the final counters.
    heartbeat.stop();
    config_watch.stop();

    drain_publish_queue(&daemon, publish_done);
    drop(daemon);
    // Handlers and uploads are done, so the index is as idle as this daemon
    // will see it. Quiet rules only: a contended index yields at once, and a
    // large store or live index is skipped, so shutdown stays short.
    if config.index_auto_compact {
        crate::maintenance::run_at_shutdown(config.clone());
    }

    control.finish();
    // Socket file is cleaned up by `_socket_guard` (Drop).
    log::info!("daemon stopped");
    Ok(())
}

/// How long the accept loop pauses after the process runs out of file
/// descriptors. Retrying at once spins on the same error until a handler
/// closes one; the queued clients wait in the listen backlog either way.
const ACCEPT_EXHAUSTED_BACKOFF: Duration = Duration::from_millis(50);

/// The pause before accepting again after `error`, if any.
fn accept_backoff(error: &std::io::Error) -> Option<Duration> {
    if matches!(error.raw_os_error(), Some(libc::EMFILE | libc::ENFILE)) {
        return Some(ACCEPT_EXHAUSTED_BACKOFF);
    }
    None
}

/// Maximum time to let accepted IPC handlers finish their current response
/// during shutdown. A bounded drain preserves in-flight replies (including the
/// shutdown acknowledgement) without letting a silent client hold the daemon
/// open forever.
const CONNECTION_HANDLER_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

fn accept_loop(
    listener: &std::os::unix::net::UnixListener,
    daemon: &Arc<Daemon>,
    lifecycle: &Arc<Lifecycle>,
    idle_timeout: Option<Duration>,
    handler_drain_timeout: Duration,
) {
    let mut last_activity = Instant::now();
    let mut handlers: Vec<std::thread::JoinHandle<()>> = Vec::new();

    // Bound the number of connection handlers doing work at once. Excess
    // connections wait for a permit instead of all running concurrently, so
    // a burst of local clients can't pile up active handlers.
    let conn_limiter = Arc::new(crate::lifecycle::Semaphore::new(daemon.capacity.connections));

    loop {
        if !lifecycle.accepting_calls() {
            log::info!("shutdown requested via protocol, draining...");
            break;
        }

        // A compiler can be silent for minutes. Accepted work owns activity
        // independently of optional progress or heartbeat traffic.
        if lifecycle.snapshot().active != 0 {
            last_activity = Instant::now();
        }
        if let Some(timeout) = idle_timeout
            && last_activity.elapsed() > timeout
        {
            log::info!("daemon idle for {:?}, shutting down", timeout);
            break;
        }

        match listener.accept() {
            Ok((stream, _)) => {
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let request_started_at = Instant::now();
                last_activity = request_started_at;
                let d = daemon.clone();
                let flag = lifecycle.clone();
                let limiter = conn_limiter.clone();
                match std::thread::Builder::new()
                    .name("kache-connection".to_string())
                    .spawn(move || {
                        let result = std::panic::catch_unwind(
                            std::panic::AssertUnwindSafe(|| {
                                handle_connection_after_queue(
                                    stream,
                                    &d,
                                    &flag,
                                    limiter,
                                    request_started_at,
                                )
                            }),
                        );
                        match result {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                if e.downcast_ref::<std::io::Error>()
                                    .is_some_and(is_client_disconnect)
                                {
                                    log::debug!(
                                        "connection handler: client disconnected: {e}"
                                    );
                                } else {
                                    log::warn!("connection handler error: {e}");
                                }
                            }
                            Err(_) => {
                                log::warn!("connection handler task failed");
                            }
                        }
                    }) {
                    Ok(thread) => handlers.push(thread),
                    Err(error) => log::warn!("connection handler spawn failed: {error}"),
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => {
                log::warn!("accept error: {e}", e = error);
                if let Some(pause) = accept_backoff(&error) {
                    std::thread::sleep(pause);
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        // Reap finished handlers so the vector does not grow without bound.
        handlers.retain(|thread| !thread.is_finished());
    }

    // Every accepted connection is owned by this loop. Tell persistent
    // handlers to stop after their current response, then wait for those
    // responses under a deadline. This must happen before `server_main`
    // drains publications.
    lifecycle.start_drain();
    let drain_deadline = Instant::now() + handler_drain_timeout;
    lifecycle.drain_until(drain_deadline);
    if drain_connection_handlers(
        &mut handlers,
        drain_deadline.saturating_duration_since(Instant::now()),
    ) {
        log::warn!(
            "connection handler drain timed out; leaving remaining handlers to their clients; timeout_ms={}",
            handler_drain_timeout.as_millis() as u64
        );
    }
}

/// Drain all accepted connection handlers under one deadline. Returns true if
/// unfinished handlers were left running: threads cannot be cancelled, so a
/// silent client outlives the drain and dies with the process.
fn drain_connection_handlers(
    handlers: &mut Vec<std::thread::JoinHandle<()>>,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    for thread in handlers.iter() {
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    for thread in handlers.drain(..) {
        let _ = thread.join();
    }
    false
}

/// Populate the key cache by listing every key in the remote.
/// Max bytes for a single request frame (one '\n'-terminated line). Requests
/// are small JSON objects; cap the buffer so a local client that streams bytes
/// without a newline can't drive the per-connection allocation arbitrarily high.
const MAX_REQUEST_FRAME_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Read one '\n'-terminated request frame, bounded to [`MAX_REQUEST_FRAME_BYTES`].
/// Strips a trailing '\n'/'\r\n', returns `Ok(None)` on clean EOF, and yields
/// a final unterminated line — but rejects (with `InvalidData`) a frame that
/// grows past the cap instead of buffering it without limit. A read timeout
/// surfaces as `TimedOut` so the caller can recheck shutdown before retrying.
fn read_bounded_line(
    reader: &mut std::io::BufReader<std::os::unix::net::UnixStream>,
    buf: &mut Vec<u8>,
) -> std::io::Result<Option<String>> {
    use std::io::BufRead;
    buf.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok((!buf.is_empty()).then(|| decode_request_frame(buf)));
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..pos]);
            reader.consume(pos + 1);
            return Ok(Some(decode_request_frame(buf)));
        }
        buf.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
        if buf.len() > MAX_REQUEST_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request frame exceeds maximum size",
            ));
        }
    }
}

/// Read the next request only while the daemon is still accepting work.
/// Checking on both sides of the read covers handlers already parked between
/// persistent requests when another connection initiates shutdown. A read
/// timeout retries so a quiet persistent connection still observes the drain.
fn read_request_before_shutdown(
    lifecycle: &Arc<Lifecycle>,
    reader: &mut std::io::BufReader<std::os::unix::net::UnixStream>,
    buf: &mut Vec<u8>,
) -> std::io::Result<Option<String>> {
    if !lifecycle.accepting_calls() {
        return Ok(None);
    }
    loop {
        match read_bounded_line(reader, buf) {
            Ok(line) => {
                if !lifecycle.accepting_calls() {
                    return Ok(None);
                }
                return Ok(line);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if !lifecycle.accepting_calls() {
                    return Ok(None);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn decode_request_frame(buf: &[u8]) -> String {
    let mut s = String::from_utf8_lossy(buf).into_owned();
    if s.ends_with('\r') {
        s.pop();
    }
    s
}

fn handle_connection_after_queue(
    stream: std::os::unix::net::UnixStream,
    daemon: &Arc<Daemon>,
    lifecycle: &Arc<Lifecycle>,
    limiter: Arc<crate::lifecycle::Semaphore>,
    request_started_at: Instant,
) -> Result<()> {
    // Peer-credential gate before anything else: the check is a cheap
    // syscall, so unauthenticated peers are dropped before they can wait on
    // the connection limiter or read a single request frame.
    if let Err(error) =
        crate::transport::require_self_peer(crate::transport::peer_euid_std(&stream))
    {
        log::warn!("rejected IPC connection from another local user: {error}");
        return Ok(());
    }
    let _permit = loop {
        if !lifecycle.accepting_calls() {
            return Ok(());
        }
        if let Some(permit) = limiter.try_acquire() {
            break permit;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    handle_connection_started_at(stream, daemon, lifecycle, request_started_at)
}

#[cfg(test)]
pub(crate) fn handle_connection(
    stream: std::os::unix::net::UnixStream,
    daemon: &Arc<Daemon>,
    lifecycle: &Arc<Lifecycle>,
) -> Result<()> {
    handle_connection_started_at(stream, daemon, lifecycle, Instant::now())
}

fn handle_connection_started_at(
    stream: std::os::unix::net::UnixStream,
    daemon: &Arc<Daemon>,
    lifecycle: &Arc<Lifecycle>,
    _request_started_at: Instant,
) -> Result<()> {
    // Clone for the reader; the owned stream stays for writes. Both share the
    // same socket. A read timeout lets a quiet persistent connection observe
    // the drain instead of parking in the kernel forever.
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = std::io::BufReader::new(stream.try_clone()?);
    let mut stream = stream;
    let mut frame = Vec::new();

    loop {
        let line = match read_request_before_shutdown(lifecycle, &mut reader, &mut frame) {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(e) if is_client_disconnect(&e) => {
                // Fire-and-forget client closed abruptly — not an error.
                log::debug!("client disconnected mid-read: {e}");
                break;
            }
            Err(e) => return Err(e.into()),
        };
        let Some(_request) = lifecycle.begin() else {
            break;
        };
        let start = Instant::now();
        let parsed = deser_json::from_str::<Request>(&line);

        let client_version = parsed
            .as_ref()
            .ok()
            .and_then(Request::client_version)
            .map(str::to_owned);

        if parsed.as_ref().is_ok_and(Request::is_build_activity) {
            daemon.request_clock.touch(Instant::now());
        }

        let resp = match parsed {
            Ok(Request::ExecutionStatus {
                client_pid,
                client_version,
                trusted_pipeline,
            }) => {
                let mut response = Response::ok();
                response.execution_status = Some(execution::probe(
                    &stream,
                    client_pid,
                    &client_version,
                    trusted_pipeline,
                    daemon,
                ));
                response
            }
            Ok(Request::ExecuteV1(req)) => {
                let result = execution::handle(&stream, &mut reader, *req, daemon, lifecycle);
                retire_for_newer_client(daemon, lifecycle, client_version.as_deref());
                return result;
            }
            Ok(Request::Gc(req) | Request::GcV2(req)) => daemon.handle_gc(&req),
            Ok(Request::GcHint) => daemon.handle_gc_hint(),
            Ok(Request::Prestage(req)) => daemon.handle_prestage(&req),
            Ok(Request::Health) => daemon.handle_health(),
            Ok(Request::Stats(req)) => daemon.handle_stats(&req),
            Ok(Request::HashFiles(req)) => daemon.handle_hash_files(&req),
            Ok(Request::CompileStarted(req)) => daemon.handle_compile_started(req),
            Ok(Request::CompileFinished(req)) => daemon.handle_compile_finished(&req),
            Ok(Request::PublishCc(req)) => daemon.handle_publish_cc(*req),
            Ok(Request::Shutdown) => {
                lifecycle.start_drain();
                // Wake the accept loop so it breaks now rather than on the next
                // periodic tick (issue #288).

                Response::ok()
            }
            Err(e) => {
                log::warn!("invalid request from client: {e}");
                Response::err(format!("invalid request: {e}"))
            }
        };
        let elapsed = start.elapsed();

        retire_for_newer_client(daemon, lifecycle, client_version.as_deref());

        if let Some((error, routine)) = request_failure(&resp) {
            let elapsed_ms = elapsed.as_millis() as u64;
            if routine {
                log::debug!("request failed; elapsed_ms={elapsed_ms} error={error}");
            } else {
                log::warn!("request failed; elapsed_ms={elapsed_ms} error={error}");
            }
        }

        let mut resp_line = deser_json::to_string(&resp)?;
        resp_line.push('\n');
        if let Err(e) = std::io::Write::write_all(&mut stream, resp_line.as_bytes()) {
            // Client closed without reading (fire-and-forget mode) — not an error.
            log::debug!("response write failed (client likely closed): {e}");
            break;
        }
    }

    Ok(())
}

fn retire_for_newer_client(daemon: &Daemon, lifecycle: &Lifecycle, client_version: Option<&str>) {
    // Accepted work keeps its response and existing drain guarantees.
    // Legacy epochs are ignored: rebuilding a client is not an upgrade.
    if client_version.is_some_and(|version| client_version_is_newer(version, &daemon.version))
        && lifecycle.accepting_calls()
    {
        log::info!(
            "newer client release requested daemon replacement; daemon_version={} client_version={}",
            daemon.version,
            client_version.unwrap_or_default()
        );
        lifecycle.start_drain();
    }
}

/// Returns true for I/O errors that mean the client disconnected, so the
/// daemon can downgrade the log level instead of warning on every occurrence.
/// A failed response's error, and whether the client recovers from it on its
/// own (logged at debug instead of warn).
fn request_failure(resp: &Response) -> Option<(&str, bool)> {
    (!resp.ok).then(|| {
        let error = resp.error.as_deref().unwrap_or("unknown");
        (error, crate::daemon_publish::refusal_is_routine(error))
    })
}

fn is_client_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
    ) || e.raw_os_error() == Some(32) // EPIPE on macOS may report as ErrorKind::Other
}

fn rotate_daemon_log_if_large(log_path: &Path) {
    if std::fs::metadata(log_path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        let _ = std::fs::write(log_path, b"--- log rotated ---\n");
    }
}

use crate::platform::wait_for_shutdown as shutdown_signal;

// ── Client ───────────────────────────────────────────────────────

pub struct GcRequestOutcome {
    pub evicted: Option<usize>,
    pub skipped: bool,
    pub breakdown: Option<GcBreakdown>,
}

const GC_POLICY_PROTOCOL_VERSION: u32 = 2;

fn require_gc_policy_support(stats: &StatsResponse) -> Result<()> {
    if stats.gc_policy_version < GC_POLICY_PROTOCOL_VERSION {
        anyhow::bail!(
            "connected daemon predates GC policy version {GC_POLICY_PROTOCOL_VERSION}; refusing \
             to send a mutating GC request"
        );
    }
    Ok(())
}

fn require_daemon_started(started: bool) -> Result<()> {
    anyhow::ensure!(started, "could not reach or start daemon");
    Ok(())
}

fn gc_outcome_from_response(resp: Response) -> Result<GcRequestOutcome> {
    if !resp.ok {
        anyhow::bail!("daemon GC error: {}", resp.error.unwrap_or_default());
    }
    if resp.gc.is_none() {
        anyhow::bail!(
            "connected daemon omitted GC policy reporting; refusing to accept ambiguous semantics"
        );
    }
    Ok(GcRequestOutcome {
        evicted: resp.evicted,
        skipped: resp.skipped,
        breakdown: resp.gc,
    })
}

/// How long a wrapper waits for the daemon to acknowledge a GC hint. The
/// daemon answers before it sweeps, so this bounds a saturated daemon, never
/// a sweep.
const GC_HINT_ACK_TIMEOUT: Duration = Duration::from_millis(500);

/// Tell a running daemon the store is under size pressure. True when the
/// daemon took the hint and owns the sweep. False when no daemon listens, it
/// predates the hint, or it did not answer in time; the caller then sweeps
/// itself. Never starts a daemon: [`send_gc_request`] probes stats, starts
/// one and waits out the whole sweep, none of which a compile may pay for.
pub fn send_gc_hint(config: &Config) -> bool {
    gc_hint_accepted(send_request_with_timeout(
        &config.socket_path(),
        &Request::GcHint,
        GC_HINT_ACK_TIMEOUT,
    ))
}

fn gc_hint_accepted(reply: Result<String>) -> bool {
    reply
        .ok()
        .and_then(|line| deser_json::from_str::<Response>(&line).ok())
        .is_some_and(|resp| resp.ok)
}

const GC_REQUEST_TIMEOUT: Duration = Duration::from_secs(3600);

/// Send a GC request to the daemon. Auto-starts daemon if needed.
pub fn send_gc_request(config: &Config, max_age_hours: Option<u64>) -> Result<GcRequestOutcome> {
    send_gc_request_with_timeout(
        config,
        max_age_hours,
        GC_REQUEST_TIMEOUT,
        start_daemon_background,
    )
}

fn send_gc_request_with_timeout(
    config: &Config,
    max_age_hours: Option<u64>,
    read_timeout: Duration,
    mut start: impl FnMut() -> Result<bool>,
) -> Result<GcRequestOutcome> {
    let socket_path = config.socket_path();

    // Capability-check before mutation. New clients send v2 fields that an
    // old daemon silently ignores, so discovering incompatibility from the GC
    // response would be too late: the old daemon may already have evicted in
    // duplicate/size-before-age order (or run duplicate GC for --max-age).
    match send_stats_request(config, false, None, None) {
        Ok(stats) => require_gc_policy_support(&stats)?,
        Err(_) => {
            require_daemon_started(start()?)?;
            let stats = send_stats_request(config, false, None, None)
                .context("probing GC policy support after daemon start")?;
            require_gc_policy_support(&stats)?;
        }
    }

    // gc_v2 is itself the atomic compatibility gate: an old daemon cannot
    // deserialize it, even if it replaced the probed daemon between sockets.
    let req = Request::GcV2(match max_age_hours {
        Some(hours) => GcRequest::explicit_age(hours),
        None => GcRequest::automatic(config.gc_max_age_hours),
    });

    let try_send = |path: &Path| -> Result<Response> {
        let resp_str = send_request_with_timeout(path, &req, read_timeout)?;
        let resp: Response = deser_json::from_str(&resp_str)?;
        Ok(resp)
    };

    match try_send(&socket_path) {
        Ok(resp) => gc_outcome_from_response(resp),
        Err(error) if error.is::<DaemonConnectionFailure>() => {
            // The daemon may have exited after the capability probe. Any
            // replacement must pass the same pre-mutation check before retry.
            require_daemon_started(start()?)?;
            let stats = send_stats_request(config, false, None, None)
                .context("probing GC policy support before retry")?;
            require_gc_policy_support(&stats)?;
            let resp = try_send(&socket_path)?;
            gc_outcome_from_response(resp)
        }
        // A write or read failure may follow an accepted request. Its sweep
        // keeps running; replaying it can repeat eviction or discard its policy.
        Err(error) => Err(error.context("GC outcome unknown; request was not resent")),
    }
}

pub fn send_hash_files_request(
    socket_path: &Path,
    files: Vec<HashFileRequest>,
) -> Result<Vec<HashFileResult>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    if !socket_path.exists() {
        anyhow::bail!("daemon socket does not exist: {}", socket_path.display());
    }

    let req = Request::HashFiles(HashFilesRequest { files });
    let resp_str = send_request_with_timeout(socket_path, &req, std::time::Duration::from_secs(3))?;
    hash_files_results_from_response_line(&resp_str)
}

fn hash_files_results_from_response_line(resp_str: &str) -> Result<Vec<HashFileResult>> {
    let resp: Response = deser_json::from_str(resp_str)?;
    if !resp.ok {
        anyhow::bail!(
            "daemon hash_files error: {}",
            resp.error.unwrap_or_default()
        );
    }
    Ok(resp.hash_results.unwrap_or_default())
}

/// Hint the daemon to stage `target_dir`'s recorded executables.
/// Non-blocking, fire-and-forget.
pub fn send_prestage(config: &Config, target_dir: &Path) {
    let req = Request::Prestage(PrestageRequest {
        target_dir: target_dir.to_string_lossy().into_owned(),
    });
    if let Err(e) = send_request_fire_and_forget(&config.socket_path(), &req) {
        log::debug!("prestage hint: daemon unreachable ({e}), skipping");
    }
}

/// Max age before an in-flight compile entry is dropped even if a process
/// with that PID is still alive — PID reuse must not resurrect a ghost.
const IN_FLIGHT_MAX_AGE_MS: u64 = 6 * 60 * 60 * 1000;

/// Drop registry entries whose process is gone or whose age is absurd. Called
/// on both the register and snapshot paths (kunobi-ninja/kache#131) — a
/// wrapper killed by OOM or ^C never sends CompileFinished.
fn prune_in_flight(map: &mut HashMap<u32, CompileStartedRequest>) {
    let now = unix_time_ms();
    map.retain(|&pid, c| {
        now.saturating_sub(c.started_at_ms) <= IN_FLIGHT_MAX_AGE_MS && pid_alive(pid)
    });
}

/// Is a process with this PID alive? `kill(pid, 0)` probes without signaling:
/// success or EPERM (alive, not ours) both mean alive; ESRCH means gone.
///
/// This runs inside a `retain` over every in-flight compile, so it stays a
/// bare probe rather than [`process_is_alive`]. It refuses PIDs 0 and 1, because
/// `kill(-1, 0)` succeeds whenever anything is signalable and would keep
/// bogus entries alive in the map forever.
fn pid_alive(pid: u32) -> bool {
    if pid <= 1 || i32::try_from(pid).is_err() {
        return false;
    }
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Register an in-flight compile (kunobi-ninja/kache#131). Fire-and-forget
/// from the wrapper's heartbeat monitor thread. Never auto-starts the daemon —
/// observability is not worth a daemon spawn — and never fails the build.
/// Takes the socket path rather than `&Config` so the monitor thread's context
/// stays a couple of PathBufs.
pub fn send_compile_started(socket_path: &std::path::Path, req: CompileStartedRequest) {
    // Probe before connecting (same pattern as send_remote_check): with no
    // daemon this returns immediately, and a wedged socket can't stall the
    // monitor thread — a lost registration only costs panel visibility, and
    // a lost Finished self-heals via liveness pruning.
    if !crate::transport::is_reachable(socket_path) {
        return;
    }
    let req = Request::CompileStarted(req);
    if let Err(e) = send_request_fire_and_forget(socket_path, &req) {
        log::debug!("compile-started: daemon unreachable ({e}), skipping");
    }
}

/// Deregister a finished compile — fire-and-forget counterpart of
/// [`send_compile_started`].
pub fn send_compile_finished(socket_path: &std::path::Path, pid: u32, started_at_ms: u64) {
    if !crate::transport::is_reachable(socket_path) {
        return;
    }
    let req = Request::CompileFinished(CompileFinishedRequest { pid, started_at_ms });
    if let Err(e) = send_request_fire_and_forget(socket_path, &req) {
        log::debug!("compile-finished: daemon unreachable ({e}), skipping");
    }
}

/// Verify readiness without waiting for store locks, scans or maintenance.
/// Legacy daemons use their existing health request; an unsupported reply requires restart.
pub fn send_health_request(config: &Config) -> Result<DaemonHealth> {
    let health = fetch_daemon_health(config)?;
    anyhow::ensure!(
        !client_version_is_newer(VERSION, &health.version),
        "daemon needs an upgrade"
    );
    Ok(health)
}

fn fetch_daemon_health(config: &Config) -> Result<DaemonHealth> {
    if let Some(health) =
        lifecycle_control::health(config, Instant::now() + Duration::from_secs(2))?
    {
        anyhow::ensure!(health.ready && !health.draining, "daemon is not ready");
        return Ok(DaemonHealth {
            version: health.build,
            build_epoch: health.revision,
        });
    }
    let response = send_request_with_timeout(
        &config.socket_path(),
        &Request::Health,
        Duration::from_secs(2),
    )?;
    parse_daemon_health(&response)
}

fn parse_daemon_health(response: &str) -> Result<DaemonHealth> {
    let response: Response = deser_json::from_str(response)?;
    anyhow::ensure!(response.ok, "daemon rejected readiness check");
    response.health.context("daemon omitted readiness response")
}

/// Send a stats request to the daemon. No auto-start — stats are best-effort.
/// Returns Err if daemon is unreachable.
pub fn send_stats_request(
    config: &Config,
    include_entries: bool,
    sort_by: Option<&str>,
    window: Option<crate::since::SinceWindow>,
) -> Result<StatsResponse> {
    send_stats_request_options(config, include_entries, false, sort_by, window)
}

/// Read stats without requesting a drain, starting a daemon, or waiting for a
/// replacement. Legacy daemons also receive epoch zero, meaning unknown.
pub fn send_stats_request_without_restart(
    config: &Config,
    include_entries: bool,
) -> Result<StatsResponse> {
    fetch_stats(
        config,
        include_entries,
        false,
        None,
        None,
        STATS_READ_TIMEOUT,
        false,
    )
}

pub(crate) fn send_stats_request_options(
    config: &Config,
    include_entries: bool,
    include_summaries: bool,
    sort_by: Option<&str>,
    window: Option<crate::since::SinceWindow>,
) -> Result<StatsResponse> {
    let stats = fetch_stats(
        config,
        include_entries,
        include_summaries,
        sort_by,
        window,
        STATS_READ_TIMEOUT,
        true,
    )?;

    refresh_stale_response(
        stats,
        VERSION,
        |stats| &stats.version,
        || restart_daemon_for_stale_client(config),
        || {
            fetch_stats(
                config,
                include_entries,
                include_summaries,
                sort_by,
                window,
                STATS_REFETCH_TIMEOUT,
                true,
            )
        },
    )
}

fn refresh_stale_response<T>(
    stats: T,
    client_version: &str,
    version: impl Fn(&T) -> &str,
    restart: impl FnOnce() -> Result<bool>,
    refetch: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if !client_version_is_newer(client_version, version(&stats)) {
        return Ok(stats);
    }
    log::info!(
        "older daemon release detected, replacing; daemon_version={} client_version={client_version}",
        version(&stats)
    );
    anyhow::ensure!(restart()?, "replacement daemon did not become ready");
    let fresh = refetch().context("reading replacement daemon response")?;
    anyhow::ensure!(
        matches!(
            compare_daemon_version(client_version, version(&fresh)),
            Some(std::cmp::Ordering::Equal | std::cmp::Ordering::Less)
        ),
        "replacement daemon did not report the required release version"
    );
    Ok(fresh)
}

/// One stats round trip: no auto-start, no restart, no retry.
fn fetch_stats(
    config: &Config,
    include_entries: bool,
    include_summaries: bool,
    sort_by: Option<&str>,
    window: Option<crate::since::SinceWindow>,
    read_timeout: Duration,
    request_upgrade: bool,
) -> Result<StatsResponse> {
    let req = Request::Stats(StatsRequest {
        include_entries,
        include_summaries,
        sort_by: sort_by.map(String::from),
        // Rounded up, not down: a daemon that predates `event_secs` should
        // answer with a superset of a sub-hour window rather than nothing.
        event_hours: window.map(|w| w.secs().div_ceil(3600)),
        event_secs: window.map(crate::since::SinceWindow::secs),
        client_epoch: if request_upgrade { build_epoch() } else { 0 },
        client_version: request_upgrade.then(|| VERSION.to_owned()),
    });

    let resp_str = send_request_with_timeout(&config.socket_path(), &req, read_timeout)?;
    let resp: Response = deser_json::from_str(&resp_str)?;

    if resp.ok {
        resp.stats
            .ok_or_else(|| anyhow::anyhow!("stats response missing payload"))
    } else {
        anyhow::bail!("daemon stats error: {}", resp.error.unwrap_or_default())
    }
}

/// Send a shutdown request to the running daemon.
///
/// Wait for ownership release after a verified drain acknowledgement. A timeout
/// leaves unfinished work running and reports failure.
pub fn send_shutdown_request(config: &Config) -> Result<()> {
    if let Err(error) = request_shutdown(config) {
        if daemon_is_absent(&error, daemon_run_lock_is_held(&config.socket_path())?) {
            eprintln!("daemon is not running");
            return Ok(());
        }
        return Err(error);
    }
    anyhow::ensure!(
        wait_for_run_lock_release(&config.socket_path(), Duration::from_secs(35))?,
        "daemon is still draining; ownership has not been released"
    );
    eprintln!("daemon stopped");
    Ok(())
}

fn request_shutdown(config: &Config) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    if lifecycle_control::request(config, kunobi_daemon::wire::operation::DRAIN, deadline)?
        .is_none()
    {
        let response =
            lifecycle_control::legacy_request(&config.socket_path(), &Request::Shutdown, deadline)?;
        let response: Response = deser_json::from_str(&response)?;
        anyhow::ensure!(response.ok, "daemon rejected shutdown");
    }
    Ok(())
}

/// Whether a failed shutdown request found no daemon at all: nothing
/// accepted the connection and no process holds the daemon's run lock.
fn daemon_is_absent(error: &anyhow::Error, run_lock_held: bool) -> bool {
    !run_lock_held && lifecycle_client::transient(error)
}

/// `kache daemon start`: start the daemon unless a ready one is running.
pub fn start(config: &Config) -> Result<Ensured> {
    lifecycle_client::ensure_outcome(config, false)
}

/// Drain and replace this cache's daemon through the shared exclusive coordinator.
/// Returns true only after the replacement answers a compatible readiness probe.
pub fn restart(config: &Config) -> Result<bool> {
    lifecycle_client::ensure(config, true)
}

/// Whether a daemon process owns this cache now, ready or not.
pub fn is_running(config: &Config) -> Result<bool> {
    daemon_run_lock_is_held(&config.socket_path())
}

/// Best-effort restart for stale-daemon detection from stats polling.
/// This path is intentionally outside build hot paths, so a short bounded wait
/// is acceptable to keep status output current.
pub(crate) fn restart_daemon_for_stale_client(config: &Config) -> Result<bool> {
    // Keep service-managed daemons under their manager after an upgrade.
    // A protocol shutdown followed by a direct spawn leaves launchd/systemd
    // stopped after a successful exit and gives the replacement no supervisor.
    restart(config)
}

#[derive(Debug)]
struct DaemonConnectionFailure;

impl std::fmt::Display for DaemonConnectionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("daemon connection failed before sending request")
    }
}

impl std::error::Error for DaemonConnectionFailure {}

/// Send a request to the daemon with a configurable read timeout.
pub(crate) fn send_request_with_timeout(
    socket_path: &Path,
    req: &Request,
    read_timeout: std::time::Duration,
) -> Result<String> {
    send_request_with_socket_timeout(socket_path, req, read_timeout)
}

pub(crate) fn compiler_execution_status(
    config: &Config,
) -> Result<kache_protocol::execution::ExecutionStatus> {
    let request = Request::ExecutionStatus {
        client_pid: std::process::id(),
        client_version: crate::VERSION.into(),
        trusted_pipeline: config.trust_daemon_execution,
    };
    let response =
        send_request_with_timeout(&config.socket_path(), &request, Duration::from_secs(3))?;
    let response: Response = deser_json::from_str(&response)?;
    response
        .execution_status
        .filter(kache_protocol::execution::ExecutionStatus::compatible)
        .context("daemon execution protocol differs; restart it with this release")
}

fn send_request_with_socket_timeout(
    socket_path: &Path,
    req: &Request,
    read_timeout: std::time::Duration,
) -> Result<String> {
    use std::io::{BufRead, Write};

    let mut stream = crate::transport::connect_sync(socket_path, std::time::Duration::from_secs(2))
        .with_context(|| format!("connecting to daemon socket {}", socket_path.display()))
        .context(DaemonConnectionFailure)?;

    let _ = stream.set_read_timeout(Some(read_timeout));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));

    let mut line = deser_json::to_string(req)?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .context("writing request to daemon")?;
    stream.flush().context("flushing request to daemon")?;

    let mut reader = std::io::BufReader::new(&stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).with_context(|| {
        format!(
            "reading response from daemon (timeout {:?}, socket {})",
            read_timeout,
            socket_path.display()
        )
    })?;

    Ok(resp)
}

/// Send a request to the daemon without waiting for a response.
///
/// Used for fire-and-forget operations (upload, prefetch) where the client
/// doesn't need confirmation.  The request is written into the kernel's
/// socket buffer and the connection is closed immediately — the daemon reads
/// and processes it whenever the Tokio runtime gets around to it.
///
/// This avoids the read-timeout failures that occur when the daemon's runtime
/// is saturated (e.g. during S3 key-cache population at startup).
fn send_request_fire_and_forget(socket_path: &Path, req: &Request) -> Result<()> {
    let mut line = deser_json::to_string(req)?;
    line.push('\n');
    send_line_fire_and_forget(socket_path, line, FIRE_AND_FORGET_SEND_WAIT)
}

/// How long a fire-and-forget request may take to hand its line to the
/// daemon: the socket's send timeout.
const FIRE_AND_FORGET_SEND_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

fn send_line_fire_and_forget(
    socket_path: &Path,
    line: String,
    send_wait: std::time::Duration,
) -> Result<()> {
    use std::io::Write;

    let mut stream = crate::transport::connect_sync(socket_path, std::time::Duration::from_secs(2))
        .with_context(|| format!("connecting to daemon socket {}", socket_path.display()))
        .context(DaemonConnectionFailure)?;

    let _ = stream.set_write_timeout(Some(send_wait));

    stream
        .write_all(line.as_bytes())
        .context("writing request to daemon")?;
    stream.flush().context("flushing request to daemon")?;

    // Don't read a response — just close. The daemon will see EOF on the
    // read half after processing the line and silently skip the response write.
    Ok(())
}

pub fn start_daemon_background() -> Result<bool> {
    let ready = start_daemon_background_inner()?;
    if !ready {
        log::warn!("daemon did not start after recovery");
    }
    Ok(ready)
}

fn start_daemon_background_inner() -> Result<bool> {
    lifecycle_client::ensure(&Config::load()?, false)
}

fn daemon_run_lock_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("run.lock")
}

fn daemon_run_lock_is_held(socket_path: &Path) -> Result<bool> {
    kunobi_daemon::ProcessLock::is_held(daemon_run_lock_path(socket_path))
        .context("observing daemon run lock")
}

/// Observe the daemon run lock without creating it: a missing file reads as
/// "not held", so a probe on a host that never ran a daemon leaves nothing
/// behind for `doctor` to report.
pub(crate) fn existing_daemon_run_lock_is_held(socket_path: &Path) -> Result<bool> {
    daemon_run_lock_is_held(socket_path)
}

/// `kache daemon run` for the binary at `exe`. A wrapper running behind a
/// compiler shim can start the daemon, and there `exe` can be the shim, so
/// this goes through [`crate::platform::self_command`].
fn daemon_run_command(exe: &Path) -> std::process::Command {
    let mut command = crate::platform::self_command(exe, "daemon");
    command.arg("run");
    command
}

/// Start `kache daemon run` detached from this process, with stderr going to
/// `stderr_target`.
///
/// kunobi-daemon does the detaching. The daemon leads a new session,
/// so Ctrl-C and the hangup of the terminal a build runs in do not reach it
/// (kunobi-ninja/kache#1205), and it holds no descriptor of ours beyond its
/// standard streams.
fn spawn_detached_daemon(
    exe: &Path,
    stderr_target: kunobi_daemon::launch::DaemonOutput,
) -> Result<kunobi_daemon::launch::DaemonChild> {
    let command = daemon_run_command(exe);
    let mut daemon = detached_command(&command);
    daemon.stderr(stderr_target).readiness_channel();
    let child = {
        let _admission = kache_ipc::descriptors::spawn_guard();
        daemon.spawn().context("spawning daemon process")?
    };
    if child.in_callers_job() {
        log::debug!(
            "the daemon (PID {}) shares the build's job object and stops if the build is \
             interrupted; `kache daemon install` starts it outside any build",
            child.id()
        );
    }
    Ok(child)
}

/// The daemon command `command` describes, as kunobi-daemon's detached
/// spawn: the same program, arguments and environment changes. Its standard
/// streams start as the null device.
///
/// argv[0] is not carried over: kunobi-daemon cannot set it. That is safe
/// because [`crate::platform::SELF_SPAWN_ENV`] and the `daemon` argument
/// already route the child as a self-spawn before any compiler-shim name is
/// considered.
fn detached_command(command: &std::process::Command) -> kunobi_daemon::launch::DaemonCommand {
    let mut daemon = kunobi_daemon::launch::DaemonCommand::new(command.get_program());
    daemon.args(command.get_args());
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => daemon.env(name, value),
            None => daemon.env_remove(name),
        };
    }
    daemon
}

// ── Tests ────────────────────────────────────────────────────────

// Daemon tests exercise Unix semantics directly — socket *files* on disk,
// POSIX process termination via `sh`.
#[cfg(test)]
mod tests;
