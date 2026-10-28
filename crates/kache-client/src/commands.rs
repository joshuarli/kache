// Engine command schema; build.rs exports the enum and derives client routing names.
#[derive(Subcommand)]
enum Commands {
    /// Run Cargo with canonical duplicate Cargo-home rustflags collapsed once
    #[command(hide = true)]
    Cargo {
        /// Built-in build/check arguments passed verbatim to Cargo (use `--` first)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<std::ffi::OsString>,
    },

    /// List cache entries, or one crate's entries in detail
    #[command(display_order = 22)]
    List {
        /// Crate name to show details for (omit to list all)
        crate_name: Option<String>,

        /// Sort by name, size, hits or age
        #[arg(long, value_enum, default_value_t = ListSort::Name)]
        sort: ListSort,

        /// Print directly instead of using a pager
        #[arg(long)]
        no_pager: bool,
    },

    /// Evict cache entries by the size and age limits, as the daemon does.
    /// Kept for scripts
    #[command(hide = true)]
    Gc {
        /// Evict entries older than this duration (e.g. 7d, 24h)
        #[arg(long, conflicts_with = "stale_schema")]
        max_age: Option<String>,

        /// Remove entries from old or unrecorded cache-key schemas
        #[arg(long)]
        stale_schema: bool,
    },

    /// Remove the whole cache or one crate's entries. Kept for scripts;
    /// `kache clean --cache` empties the cache too
    #[command(hide = true)]
    Purge {
        /// Only purge entries for this crate
        #[arg(long)]
        crate_name: Option<String>,
    },

    /// Free disk space: remove old target directories, or empty the cache
    #[command(display_order = 11)]
    Clean {
        /// Look for target/ directories under this directory instead of using
        /// the ones kache tracks
        #[arg(value_name = "DIR", conflicts_with_all = ["cache", "crate_name", "stale", "orphans"])]
        path: Option<PathBuf>,

        /// Remove obsolete units in live targets using a recorded Cargo command
        #[arg(long, visible_alias = "artifacts", conflicts_with_all = ["cache", "crate_name", "stale", "orphans", "stale_schema"])]
        units: bool,

        /// Show what would be removed and what kache keeps, and remove nothing
        #[arg(long, short = 'n')]
        dry_run: bool,

        /// Remove without asking. For scripts and cron; preview first with
        /// --dry-run
        #[arg(long, short = 'y')]
        yes: bool,

        /// Targets no build used for this long, and targets whose worktree
        /// is gone (default 14d)
        #[arg(long, value_name = "DURATION")]
        stale: Option<String>,

        /// Empty the cache instead. Asks first. The daemon already keeps it
        /// under its size limit
        #[arg(long)]
        cache: bool,

        /// Only targets whose worktree or workspace was deleted, however
        /// recently they were built
        #[arg(long, conflicts_with_all = ["stale", "cache", "crate_name"])]
        orphans: bool,

        /// With --cache: the same, kept for older scripts
        #[arg(long, hide = true, requires = "cache", conflicts_with_all = ["stale", "stale_schema"])]
        all: bool,

        /// Remove the cache entries of this crate
        #[arg(long = "crate", hide = true, value_name = "NAME", conflicts_with_all = ["cache", "stale"])]
        crate_name: Option<String>,

        /// With --cache, remove entries from old or unrecorded cache-key schemas
        #[arg(long, hide = true, requires = "cache", conflicts_with = "stale")]
        stale_schema: bool,

        /// Accepted for older scripts: tracked targets are the default now
        #[arg(long, hide = true)]
        tracked: bool,
    },

    /// Show tracked target directories. Kept for scripts; `kache clean
    /// --dry-run` shows the same targets
    #[command(hide = true, subcommand_required = false)]
    Targets {
        #[command(subcommand)]
        command: Option<TargetCommands>,
    },

    /// Set up caching for Cargo and C/C++ builds
    #[command(display_order = 1)]
    Init {
        /// Accept all default answers (non-interactive)
        #[arg(long, short = 'y')]
        yes: bool,

        /// Do not install the daemon as a login service
        #[arg(long)]
        no_service: bool,

        /// Leave shell setup alone: no C/C++ shims on PATH, test runner, or
        /// completions (Cargo native dependencies are still configured)
        #[arg(long)]
        no_shell: bool,

        /// Print what would change without modifying anything
        #[arg(long, visible_alias = "dry-run", short = 'n')]
        check: bool,

        /// Only create the compiler-name links to kache (Unix), in DIR or
        /// ~/.local/lib/kache/shims, and change nothing else
        #[arg(
            long,
            value_name = "DIR",
            num_args = 0..=1,
            conflicts_with_all = ["check", "no_service", "no_shell"]
        )]
        shims: Option<Option<PathBuf>>,

        /// With --shims, also link every other compiler name on PATH (target
        /// triplets)
        #[arg(long, requires = "shims")]
        from_path: bool,

        /// With --shims, link only the unversioned names, not the versioned
        /// compilers on PATH (clang-19, g++-13)
        #[arg(long, requires = "shims", conflicts_with = "from_path")]
        canonical_only: bool,

        /// With --shims, replace existing entries instead of refusing
        #[arg(long, requires = "shims")]
        force: bool,
    },

    /// Diagnose setup issues and verify cache integrity
    #[command(display_order = 2)]
    Doctor {
        /// Migrate from sccache (Cargo wrapper, running server) and restart a daemon older than this binary
        #[arg(long)]
        fix: bool,

        /// Also remove sccache cache and binary (requires --fix)
        #[arg(long, requires = "fix")]
        purge_sccache: bool,

        /// Verify cache integrity (entries, blobs, metadata)
        #[arg(long)]
        verify: bool,

        /// Also verify blob checksums (slower, implies --verify)
        #[arg(long)]
        checksums: bool,

        /// Remove corrupted entries (implies --verify)
        #[arg(long)]
        repair: bool,

        /// Compile a temporary Rust library and verify a local cache hit
        #[arg(long)]
        probe: bool,
    },

    /// Show the background daemon's status, or start, stop or install it
    #[command(subcommand_required = false, display_order = 40)]
    Daemon {
        #[command(subcommand)]
        command: Option<DaemonCommands>,
    },

    /// Watch builds and the cache live
    #[command(display_order = 12)]
    Monitor {
        /// Preload events from this window (e.g. 15m, 2h, 7d; a bare number
        /// is hours)
        #[arg(long)]
        since: Option<String>,
    },

    /// Show hit rate, time saved and cache size; --full for the whole report
    #[command(display_order = 10, visible_alias = "status")]
    Stats {
        /// Event window (e.g. 15m, 2h, 7d; a bare number is hours)
        #[arg(long, default_value = "24h")]
        since: String,

        /// Report the latest recorded activity session. Sessions may span Cargo
        /// commands; events without IDs use a five-minute idle gap per root.
        #[arg(long, conflicts_with = "since")]
        last_build: bool,

        /// Only this build tree/root: the latest session in it with
        /// `--last-build`, its compiler events with `--full`
        #[arg(long)]
        root: Option<PathBuf>,

        /// The full build report instead of the summary
        #[arg(long)]
        full: bool,

        /// Write the full report as json, markdown, github, or trace
        /// (Perfetto / Chrome trace). Implies --full
        #[arg(long, value_name = "FORMAT", value_enum)]
        format: Option<cli::ReportFormat>,

        /// Write the full report to a file instead of stdout. Implies --full
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// Entries in each top list of the full report. Implies --full
        #[arg(long)]
        top: Option<usize>,

        /// Also append the full report's summary and timing, with the host's
        /// load, to `<cache dir>/telemetry/sessions.jsonl`, which outlives the
        /// runtime dir a CI job deletes. Implies --full
        #[arg(long)]
        record: bool,

        /// Replace cache keys, roots, and object paths in the full report
        /// with `redacted`. Implies --full
        #[arg(long)]
        redact: bool,
    },

    /// Write cache counters as OTLP JSON
    #[command(hide = true)]
    Telemetry {
        #[command(subcommand)]
        command: TelemetryCommands,
    },

    /// Explain why a crate, or the latest build, missed the cache
    #[command(display_order = 20, alias = "why-miss")]
    Explain {
        /// The crate to explain. Without one, compare the latest build's
        /// misses with the build before it
        crate_name: Option<String>,

        /// List every skip and passthrough across the retained build history,
        /// with counts by reason and the affected crates
        #[arg(long, conflicts_with = "crate_name")]
        all: bool,

        /// Without a crate: only builds recorded for this build tree
        #[arg(long, conflicts_with = "crate_name")]
        root: Option<PathBuf>,
    },

    /// Compare misses in the two newest sessions of one build root. Kept
    /// for scripts; `kache explain` with no crate explains the same build
    #[command(hide = true)]
    Diff {
        /// Only sessions recorded for this build tree
        #[arg(long)]
        root: Option<PathBuf>,
    },

    /// The full build report. Kept for scripts; `kache stats --full` is the
    /// same report
    #[command(hide = true)]
    Report {
        /// Output format: json, trace (also perfetto, chrome-trace), markdown,
        /// github, text
        #[arg(long, value_enum, default_value_t = cli::ReportFormat::Text)]
        format: cli::ReportFormat,

        /// Event window (e.g. 15m, 2h, 7d; a bare number is hours)
        #[arg(long, default_value = "24h")]
        since: String,

        /// Report the latest recorded activity session. Sessions may span Cargo
        /// commands; events without IDs use a five-minute idle gap per root.
        #[arg(long, conflicts_with = "since")]
        last_build: bool,

        /// Only include compiler events from this build tree/root
        #[arg(long)]
        root: Option<PathBuf>,

        /// Write output to a file instead of stdout
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// Number of top entries to show
        #[arg(long, default_value = "10")]
        top: usize,

        /// Also append this report's summary and timing, with the host's load,
        /// to `<cache dir>/telemetry/sessions.jsonl`, which outlives the
        /// runtime dir a CI job deletes
        #[arg(long)]
        record: bool,

        /// Replace cache keys, roots, and object paths with `redacted`
        #[arg(long)]
        redact: bool,
    },

    /// Edit or inspect configuration (opens the editor without a subcommand)
    #[command(display_order = 3)]
    Config {
        #[command(subcommand)]
        command: Option<config_cli::Command>,
    },

    /// Create compiler-name links to kache in a directory you put first on
    /// PATH. Kept for scripts and packages; `kache init --shims` does the
    /// same in the default directory
    #[command(hide = true)]
    InstallShims {
        /// Directory to populate. Defaults to ~/.local/lib/kache/shims
        #[arg(value_name = "DIR")]
        dir: Option<PathBuf>,

        /// Replace existing entries instead of refusing
        #[arg(long)]
        force: bool,

        /// Also wrap every other compiler name on PATH (target triplets)
        #[arg(long)]
        from_path: bool,

        /// Link only the unversioned names, not the versioned compilers on PATH
        #[arg(long, conflicts_with = "from_path")]
        canonical_only: bool,
    },

    /// Generate shell completion scripts
    #[command(display_order = 41)]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}
