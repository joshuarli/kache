// Engine command schema; build.rs exports the enum and derives client routing names.
pub(crate) enum Commands {
    /// Run Cargo with canonical duplicate Cargo-home rustflags collapsed once
    Cargo {
        /// Built-in build/check arguments passed verbatim to Cargo (use `--` first)
        args: Vec<std::ffi::OsString>,
    },

    /// List cache entries, or one crate's entries in detail
    List {
        /// Crate name to show details for (omit to list all)
        crate_name: Option<String>,

        /// Sort by name, size, hits or age
        sort: ListSort,

        /// Print directly instead of using a pager
        no_pager: bool,
    },

    /// Evict cache entries by the size and age limits, as the daemon does.
    /// Kept for scripts
    Gc {
        /// Evict entries older than this duration (e.g. 7d, 24h)
        max_age: Option<String>,

        /// Remove entries from old or unrecorded cache-key schemas
        stale_schema: bool,
    },

    /// Remove the whole cache or one crate's entries. Kept for scripts;
    /// `kache clean --cache` empties the cache too
    Purge {
        /// Only purge entries for this crate
        crate_name: Option<String>,
    },

    /// Free disk space: remove old target directories, or empty the cache
    Clean {
        /// Look for target/ directories under this directory instead of using
        /// the ones kache tracks
        path: Option<PathBuf>,

        /// Remove obsolete units in live targets using a recorded Cargo command
        units: bool,

        /// Show what would be removed and what kache keeps, and remove nothing
        dry_run: bool,

        /// Remove without asking. For scripts and cron; preview first with
        /// --dry-run
        yes: bool,

        /// Targets no build used for this long, and targets whose worktree
        /// is gone (default 14d)
        stale: Option<String>,

        /// Empty the cache instead. Asks first. The daemon already keeps it
        /// under its size limit
        cache: bool,

        /// Only targets whose worktree or workspace was deleted, however
        /// recently they were built
        orphans: bool,

        /// With --cache: the same, kept for older scripts
        all: bool,

        /// Remove the cache entries of this crate
        crate_name: Option<String>,

        /// With --cache, remove entries from old or unrecorded cache-key schemas
        stale_schema: bool,

        /// Accepted for older scripts: tracked targets are the default now
        tracked: bool,
    },

    /// Show tracked target directories. Kept for scripts; `kache clean
    /// --dry-run` shows the same targets
    Targets {
        command: Option<TargetCommands>,
    },

    /// Set up caching for Cargo and C/C++ builds
    Init {
        /// Accept all default answers (non-interactive)
        yes: bool,

        /// Do not install the daemon as a login service
        no_service: bool,

        /// Leave shell setup alone: no C/C++ shims on PATH or test runner
        /// (Cargo native dependencies are still configured)
        no_shell: bool,

        /// Print what would change without modifying anything
        check: bool,

        /// Only create the compiler-name links to kache (Unix), in DIR or
        /// ~/.local/lib/kache/shims, and change nothing else
        shims: Option<Option<PathBuf>>,

        /// With --shims, also link every other compiler name on PATH (target
        /// triplets)
        from_path: bool,

        /// With --shims, link only the unversioned names, not the versioned
        /// compilers on PATH (clang-19, g++-13)
        canonical_only: bool,

        /// With --shims, replace existing entries instead of refusing
        force: bool,
    },

    /// Diagnose setup issues and verify cache integrity
    Doctor {
        /// Migrate from sccache (Cargo wrapper, running server) and restart a daemon older than this binary
        fix: bool,

        /// Also remove sccache cache and binary (requires --fix)
        purge_sccache: bool,

        /// Verify cache integrity (entries, blobs, metadata)
        verify: bool,

        /// Also verify blob checksums (slower, implies --verify)
        checksums: bool,

        /// Remove corrupted entries (implies --verify)
        repair: bool,

        /// Compile a temporary Rust library and verify a local cache hit
        probe: bool,
    },

    /// Show the background daemon's status, or start, stop or install it
    Daemon {
        command: Option<DaemonCommands>,
    },

    /// Show hit rate, time saved and cache size; --full for the whole report
    Stats {
        /// Event window (e.g. 15m, 2h, 7d; a bare number is hours)
        since: String,

        /// Report the latest recorded activity session. Sessions may span Cargo
        /// commands; events without IDs use a five-minute idle gap per root.
        last_build: bool,

        /// Only this build tree/root: the latest session in it with
        /// `--last-build`, its compiler events with `--full`
        root: Option<PathBuf>,

        /// The full build report instead of the summary
        full: bool,

        /// Write the full report as json, markdown, github, or trace
        /// (Perfetto / Chrome trace). Implies --full
        format: Option<cli::ReportFormat>,

        /// Write the full report to a file instead of stdout. Implies --full
        output: Option<PathBuf>,

        /// Entries in each top list of the full report. Implies --full
        top: Option<usize>,

        /// Also append the full report's summary and timing, with the host's
        /// load, to `<cache dir>/telemetry/sessions.jsonl`, which outlives the
        /// runtime dir a CI job deletes. Implies --full
        record: bool,

        /// Replace cache keys, roots, and object paths in the full report
        /// with `redacted`. Implies --full
        redact: bool,
    },

    /// Write cache counters as OTLP JSON
    Telemetry {
        command: TelemetryCommands,
    },

    /// Explain why a crate, or the latest build, missed the cache
    Explain {
        /// The crate to explain. Without one, compare the latest build's
        /// misses with the build before it
        crate_name: Option<String>,

        /// List every skip and passthrough across the retained build history,
        /// with counts by reason and the affected crates
        all: bool,

        /// Without a crate: only builds recorded for this build tree
        root: Option<PathBuf>,
    },

    /// Compare misses in the two newest sessions of one build root. Kept
    /// for scripts; `kache explain` with no crate explains the same build
    Diff {
        /// Only sessions recorded for this build tree
        root: Option<PathBuf>,
    },

    /// The full build report. Kept for scripts; `kache stats --full` is the
    /// same report
    Report {
        /// Output format: json, trace (also perfetto, chrome-trace), markdown,
        /// github, text
        format: cli::ReportFormat,

        /// Event window (e.g. 15m, 2h, 7d; a bare number is hours)
        since: String,

        /// Report the latest recorded activity session. Sessions may span Cargo
        /// commands; events without IDs use a five-minute idle gap per root.
        last_build: bool,

        /// Only include compiler events from this build tree/root
        root: Option<PathBuf>,

        /// Write output to a file instead of stdout
        output: Option<PathBuf>,

        /// Number of top entries to show
        top: usize,

        /// Also append this report's summary and timing, with the host's load,
        /// to `<cache dir>/telemetry/sessions.jsonl`, which outlives the
        /// runtime dir a CI job deletes
        record: bool,

        /// Replace cache keys, roots, and object paths with `redacted`
        redact: bool,
    },

    /// Inspect or change configuration values (lists values without a subcommand)
    Config {
        command: Option<config_cli::Command>,
    },

    /// Create compiler-name links to kache in a directory you put first on
    /// PATH. Kept for scripts and packages; `kache init --shims` does the
    /// same in the default directory
    InstallShims {
        /// Directory to populate. Defaults to ~/.local/lib/kache/shims
        dir: Option<PathBuf>,

        /// Replace existing entries instead of refusing
        force: bool,

        /// Also wrap every other compiler name on PATH (target triplets)
        from_path: bool,

        /// Link only the unversioned names, not the versioned compilers on PATH
        canonical_only: bool,
    },

}
