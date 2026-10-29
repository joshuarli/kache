use crate::*;
use deser::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoreSummary {
    pub path: PathBuf,
    pub bytes: u64,
    pub max_size: u64,
    pub entries: usize,
    pub blob_stats: Option<BlobStats>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[deser(rename_all = "snake_case")]
pub enum Request {
    /// Compilation uses its own prepare/commit/completion exchange. Older
    /// daemons reject this command before a client sends Commit.
    ExecuteV1(Box<execution::ExecutionRequest>),
    /// Read-only eligibility probe. It neither reserves a worker nor runs a compiler.
    ExecutionStatus {
        client_pid: u32,
        client_version: String,
        trusted_pipeline: bool,
    },
    /// Legacy GC wire command accepted from older clients.
    Gc(GcRequest),
    /// Policy-v2 GC command. Older daemons reject this unknown variant before
    /// mutation, closing the capability-probe/replacement race.
    GcV2(GcRequest),
    /// A wrapper saw the store over the automatic trigger. The daemon
    /// acknowledges at once and sweeps in the background; hints that arrive
    /// meanwhile coalesce. Older daemons reject the unknown variant, and the
    /// wrapper then spawns its own worker.
    GcHint,
    Stats(StatsRequest),
    Health,
    HashFiles(HashFilesRequest),
    CompileStarted(CompileStartedRequest),
    CompileFinished(CompileFinishedRequest),
    /// A wrapper hands the daemon a finished cc compile to store.
    /// Older handlers lack the atomic receipt; reject their protocol instead
    /// of allowing an ambiguous timeout to publish and log twice.
    #[deser(rename = "publish_cc_v2")]
    PublishCc(Box<PublishCcRequest>),
    /// A build of a target directory started: copy its recorded large
    /// executables beside their destinations (see `prestage`).
    /// Fire-and-forget; older daemons reject the unknown variant and the
    /// restore copies as before.
    Prestage(PrestageRequest),
    /// Cargo is probing a target directory it has not built: copy another
    /// checkout's registry units into it before Cargo checks freshness (see
    /// `target_seed`). The wrapper waits for the answer; older
    /// daemons reject the unknown variant and nothing is copied.
    SeedTarget(SeedTargetRequest),
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GcRequest {
    /// Legacy wire field retained so old clients and daemons can still
    /// exchange an explicit `--max-age` request during rolling upgrades.
    pub max_age_hours: Option<u64>,
    #[deser(default)]
    pub mode: GcRequestMode,
    /// Effective automatic age policy loaded by the requesting CLI. Old
    /// daemons ignore this unknown field; new daemons no longer substitute
    /// their startup config for a manual request.
    #[deser(default, skip_serializing_if = Option::is_none)]
    pub effective_max_age_hours: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[deser(rename_all = "snake_case")]
pub enum GcRequestMode {
    /// A pre-mode client. Resolve `Some` as explicit age and `None` using the
    /// daemon's configured automatic policy.
    #[default]
    Legacy,
    Automatic,
    ExplicitAge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatsResponse {
    #[deser(default)]
    pub stores: Vec<StoreSummary>,
    pub total_size: u64,
    pub max_size: u64,
    pub entry_count: usize,
    pub entries: Option<Vec<StatsEntry>>,
    pub events: EventStatsResponse,
    /// Content-dedup figures for the daemon's store. Defaulted for an old
    /// daemon so a new client can suppress the section instead of mixing in
    /// figures from its own differently configured store.
    #[deser(default)]
    pub blob_stats: Option<BlobStats>,
    /// Bounded recent session summaries from the daemon's event directory.
    #[deser(default)]
    pub recent_summaries: Vec<BuildSummaryEvent>,
    #[deser(default)]
    pub version: String,
    /// Daemon executable mtime. Clients from 1.0.x and earlier replace a daemon
    /// whose executable is older than theirs; later clients only display it.
    #[deser(default)]
    pub build_epoch: u64,
    /// GC request semantics supported by this daemon. Version 2 means the
    /// daemon applies age before duplicate/size pressure and reports a policy
    /// breakdown. Missing means an older daemon, so clients must not send a
    /// mutating GC request.
    #[deser(default)]
    pub gc_policy_version: u32,
    /// In-flight miss compiles registered by wrapper heartbeat monitors
    /// (kunobi-ninja/kache#131). Defaulted for old-daemon/new-client mixes.
    #[deser(default)]
    pub in_flight: Vec<InFlightEntry>,
    /// The configuration this daemon actually loaded (kunobi-ninja/kache#689).
    /// Defaulted to `None` so a daemon that predates the field is
    /// distinguishable from one that reported it — the CLI then falls back to
    /// its own config and labels the affected lines as client-derived.
    #[deser(default)]
    pub effective_config: Option<EffectiveConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GcBreakdown {
    pub mode: GcRequestMode,
    pub duplicate: GcPolicyOutcome,
    pub age: GcPolicyOutcome,
    pub size: GcPolicyOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub ok: bool,
    #[deser(skip_serializing_if = Option::is_none)]
    pub evicted: Option<usize>,
    #[deser(default, skip_serializing_if = is_false)]
    pub skipped: bool,
    #[deser(default, skip_serializing_if = Option::is_none)]
    pub gc: Option<GcBreakdown>,
    #[deser(skip_serializing_if = Option::is_none)]
    pub stats: Option<StatsResponse>,
    #[deser(default, skip_serializing_if = Option::is_none)]
    pub health: Option<DaemonHealth>,
    #[deser(default, skip_serializing_if = Option::is_none)]
    pub execution_status: Option<execution::ExecutionStatus>,
    #[deser(skip_serializing_if = Option::is_none)]
    pub hash_results: Option<Vec<HashFileResult>>,
    #[deser(skip_serializing_if = Option::is_none)]
    pub error: Option<String>,
}

impl Request {
    /// Whether the request comes from a build. Stats pollers, a TUI and GC
    /// commands do not keep the machine from counting as quiet. Execution
    /// proposals count only when the server accepts them.
    pub fn is_build_activity(&self) -> bool {
        !matches!(
            self,
            Request::ExecuteV1(_)
                | Request::ExecutionStatus { .. }
                | Request::Health
                | Request::Stats(_)
                | Request::Gc(_)
                | Request::GcV2(_)
                | Request::Shutdown
        )
    }

    /// Only a declared newer release may request replacement after admission.
    pub fn client_version(&self) -> Option<&str> {
        match self {
            Request::ExecuteV1(req) => Some(&req.client_version),
            Request::Stats(req) => req.client_version.as_deref(),
            Request::CompileStarted(req) => req.client_version.as_deref(),
            Request::PublishCc(req) => req.client_version.as_deref(),
            _ => None,
        }
    }
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            evicted: None,
            skipped: false,
            gc: None,
            stats: None,
            health: None,
            execution_status: None,
            hash_results: None,
            error: None,
        }
    }

    #[cfg(feature = "test-support")]
    pub fn ok_evicted(n: usize) -> Self {
        Self {
            ok: true,
            evicted: Some(n),
            skipped: false,
            gc: None,
            stats: None,
            health: None,
            execution_status: None,
            hash_results: None,
            error: None,
        }
    }

    pub fn ok_gc(total: usize, breakdown: GcBreakdown) -> Self {
        Self {
            evicted: Some(total),
            gc: Some(breakdown),
            ..Self::ok()
        }
    }

    pub fn ok_gc_skipped(breakdown: GcBreakdown) -> Self {
        Self {
            ok: true,
            evicted: Some(0),
            skipped: true,
            gc: Some(breakdown),
            stats: None,
            health: None,
            execution_status: None,
            hash_results: None,
            error: None,
        }
    }

    pub fn ok_stats(stats: StatsResponse) -> Self {
        Self {
            ok: true,
            evicted: None,
            skipped: false,
            gc: None,
            stats: Some(stats),
            health: None,
            execution_status: None,
            hash_results: None,
            error: None,
        }
    }

    pub fn ok_hash_results(results: Vec<HashFileResult>) -> Self {
        Self {
            ok: true,
            evicted: None,
            skipped: false,
            gc: None,
            stats: None,
            health: None,
            execution_status: None,
            hash_results: Some(results),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            evicted: None,
            skipped: false,
            gc: None,
            stats: None,
            health: None,
            execution_status: None,
            hash_results: None,
            error: Some(msg.into()),
        }
    }
}

impl GcRequest {
    pub fn automatic(effective_max_age_hours: u64) -> Self {
        Self {
            max_age_hours: None,
            mode: GcRequestMode::Automatic,
            effective_max_age_hours: Some(effective_max_age_hours),
        }
    }

    pub fn explicit_age(hours: u64) -> Self {
        Self {
            max_age_hours: Some(hours),
            mode: GcRequestMode::ExplicitAge,
            effective_max_age_hours: None,
        }
    }

    #[cfg(feature = "test-support")]
    pub fn legacy(max_age_hours: Option<u64>) -> Self {
        Self {
            max_age_hours,
            mode: GcRequestMode::Legacy,
            effective_max_age_hours: None,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}
