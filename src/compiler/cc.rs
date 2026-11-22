//! C-family compiler (cc / gcc / g++ / clang / clang++ / c++).
//!
//! **C/C++ caching is live for the single-source `-c` compile.**
//! A `cc -c foo.c -o foo.o` invocation gets a content-addressed
//! cache entry; an identical re-invocation restores the `.o` and, when
//! requested, its `.d` dep-info sidecar without running the compiler.
//!
//! What's cached:
//! - **`-c` object compiles**, exactly one source per invocation.
//!   The cache key is the preprocessor expansion (`cc -E` with
//!   `SOURCE_DATE_EPOCH` pinned) plus compiler identity, target
//!   arch, and codegen flags. The preprocessor hash captures the
//!   source and every transitively-included header. The cold probe also writes
//!   a private complete dependency list; a later local invocation can reuse
//!   the expansion hash only while every source/header fingerprint matches,
//!   otherwise it preprocesses again. The key also digests **names** in
//!   user include dirs (`-I`, `-iquote`, and the source file's directory)
//!   so a header that appears in an earlier search dir cannot shadow a
//!   previously-read one without changing the key. Line markers are
//!   keyed apart from the text, prefix-mapped, so line positions count
//!   while header *paths* under a mapped root don't leak — the object key
//!   remains portable across worktrees, while the optimization itself is
//!   deliberately local.
//!
//! What passes through (refused, see [`CcArgs::refuse_reasons`]):
//! - Link mode (always passthrough)
//!   (epic #762 / #259)
//! - Assemble (`-S`) mode
//! - Multi-source compiles, multi-arch fat binaries
//! - Response files, precompiled headers, modules, `-o -`
//! - Coverage, split DWARF and serialized diagnostics outside `-c`
//! - Any flag not classified by [`CC_FLAGS`] (see [`classify_cc_flag`])
//!   — an unmodeled codegen flag, a cross-target, profiling, or simply
//!   a flag kache has not classified. Refused so an unknown flag is
//!   never silently cached. The table is declarative; see
//!   [`crate::compiler::flags`] for the matcher / classification
//!   vocabulary it uses.
//!
//! Future work (separate PRs):
//! - Link-mode / whole-executable caching
//! - `ar` archive caching
//! - Mach-O OSO record stripping for cross-machine sharing of *linked*
//!   artifacts (issue #78) — deferred until link-mode caching exists, since
//!   `-c` object compiles carry no linker-emitted `N_OSO` records. The
//!   SDKROOT half of #78 is handled: the Apple SDK path is mapped to the
//!   `/kache/sdkroot` prefix-map target ([`CC_SDKROOT_SENTINEL`]).

use crate::invocation::process::CommandExt as _;
use anyhow::{Context, Result};
use regex_lite::Regex;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::invocation::InvocationEnv;

use super::flags::{Dialect, FlagClass, FlagSpec, Matcher};
use super::{
    Artifact, ArtifactKind, ArtifactSet, CompileResult, Compiler, CompilerAdapter, CompilerId,
    KeyCtx, RefuseReason, classify_by_filename,
};

/// Compiler driver family of a cc-wrapper invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolFamily {
    /// gcc / cc and compatible drivers.
    Gnu,
    /// clang / clang++ in the default (gcc-compatible) driver mode.
    Clang,
    /// clang in MSVC driver mode (`clang-cl` or `--driver-mode=cl`).
    ClangCl,
}

/// Store name for a `-E` expansion that went to stdout (no `-o`).
pub(crate) const CC_STDOUT_STORE_NAME: &str = "stdout.i";

/// `-E` without `-o` (and without `-o -`): the expansion is process stdout.
pub(crate) fn cc_expansion_is_stdout(parsed: &CcArgs) -> bool {
    parsed.mode == CompileMode::Preprocess && parsed.output.is_none()
}

/// The `-E` / `/E` key probe sets this so a kache shim does not cache
/// the probe as a user-facing stdout preprocess.
#[cfg(test)]
pub(crate) fn cc_is_internal_key_probe() -> bool {
    cc_is_internal_key_probe_for(&InvocationEnv::capture())
}

pub(crate) fn cc_is_internal_key_probe_for(env: &InvocationEnv) -> bool {
    env.var_os("KACHE_CC_KEY_PROBE").is_some()
}

impl ToolFamily {
    /// The flag dialect this family speaks (Gnu and Clang share one).
    pub fn dialect(self) -> Dialect {
        match self {
            ToolFamily::Gnu | ToolFamily::Clang => Dialect::Gnu,
            ToolFamily::ClangCl => Dialect::Cl,
        }
    }

    /// Detect the family from argv0 and the argument list.
    ///
    /// `clang-cl` (including versioned/target-prefixed forms) or any argv
    /// carrying `--driver-mode=cl` is `ClangCl`. A
    /// `clang`/`clang++`/`clang-<n>` basename is `Clang`.
    /// `zigcc` wrappers (cargo-zigbuild) are also `Clang` — zig's cc is
    /// clang-based.
    /// Everything else (gcc, cc, g++, c++) is `Gnu`. Bare `cl` is NOT
    /// special-cased — real MSVC `cl.exe` stays out of scope.
    pub fn detect(program: &str, rest: &[String]) -> ToolFamily {
        let name = super::command_basename(program)
            .map(super::strip_windows_exe_suffix)
            .unwrap_or(program)
            .to_ascii_lowercase();
        if rest.iter().any(|a| a == "--driver-mode=cl") {
            return ToolFamily::ClangCl;
        }
        if name == "zigcc" || name.starts_with("zigcc-") {
            return ToolFamily::Clang;
        }
        named_tool_family(&name).unwrap_or(ToolFamily::Gnu)
    }
}

pub const CC_ID: CompilerId = CompilerId::new("cc");
pub const ADAPTER: CompilerAdapter =
    CompilerAdapter::new(CC_ID, "C-family compiler", CcCompiler::recognizes_for);

/// What stage the compiler is being asked to produce.
///
/// Cargo's `cc` crate (and most build systems) use `-c` for the
/// per-file compile step that produces a `.o`, then a separate
/// invocation that links them into the final executable / library.
/// Caching is most valuable for `Compile` mode (the per-file work
/// gets reused across invocations); `Link` mode caching is harder
/// (depends on every input `.o`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileMode {
    /// `-c`: produce object file(s) from source. The default cache
    /// target for kache's cc support.
    Compile,
    /// (no `-c` flag): compile + link, producing an executable or
    /// dynamic library. Realistic to cache eventually but more
    /// failure-prone (linker version, link order, native lib search
    /// paths).
    Link,
    /// `-E`: preprocess only — emits the source after macro expansion.
    /// Used by build systems for header probing; rarely cached.
    /// Note: also matches the `cc` crate's family probe shape, which
    /// is handled BEFORE this parser via [`CcCompiler::recognizes_family_probe`].
    Preprocess,
    /// `-S`: produce assembly output. Niche; same caching profile
    /// as `Compile` in principle but rarely worth the engineering.
    Assemble,
    /// `-x <lang>-header`: precompile a header into a PCH. The PCH records
    /// each input's modification time and size, which a consumer checks.
    Precompile,
}

/// `-O0` … `-O3`, plus the size and debug variants. Stored as the
/// raw character (`'0'`..`'3'`, `'s'`, `'z'`, `'g'`) so the cache
/// key can hash it directly without re-stringification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    O0,
    O1,
    O2,
    O3,
    /// `-Os` — optimize for size.
    Os,
    /// `-Oz` — optimize for size, more aggressive (clang-only).
    Oz,
    /// `-Og` — optimize while preserving debuggability.
    Og,
}

/// Dependency-info generation flags (`-MMD` / `-MD` / `-MF` / `-MT`).
///
/// Cargo uses these to figure out which headers a `.o` depends on
/// for incremental rebuild. kache caches the `.o` directly, so the
/// dep-info file is generated as a side effect — but its CONTENTS
/// (a Make-style dependency list) embed absolute paths that need
/// the same path-normalization treatment as rustc's dep-info.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DepInfoSpec {
    /// True when the invocation actually asks the compiler to emit dep-info
    /// in compile mode (`-MD` / `-MMD`). Path/target modifiers alone do not
    /// create a depfile.
    pub emit: bool,
    /// `-MD` (true) or `-MMD` (false). True = include system headers
    /// in the dep-info output; false = user headers only.
    pub include_system: bool,
    /// `-MP`: add phony targets for each dependency.
    pub phony_targets: bool,
    /// `-MG`: treat missing headers as generated files in dependency output.
    pub missing_generated: bool,
    /// `-MF foo.d`: where to write the dep-info file. `None` means
    /// the compiler picks a default (typically next to the `.o`).
    pub output: Option<PathBuf>,
    /// `-MT` / `-MQ` targets in argv order, `true` for `-MQ`. The compiler
    /// names every one; with none it names the output.
    pub targets: Vec<(String, bool)>,
}

/// Stands for the object path in a stored dep-info file whose only target
/// is the object, so the entry restores under another object path.
pub(crate) const CC_DEPINFO_OBJECT_PLACEHOLDER: &str = "@kache-object@";

/// The dep-info target to store as [`CC_DEPINFO_OBJECT_PLACEHOLDER`]: the
/// output's spelling, when it is the file's only target and Make writes it
/// unquoted.
pub(crate) fn cc_depinfo_object_target(parsed: &CcArgs) -> Option<String> {
    let depinfo = parsed.depinfo.as_ref().filter(|d| d.emit)?;
    let object = parsed.object_output_path()?.to_str()?.to_owned();
    let only_target = match depinfo.targets.as_slice() {
        [] => true,
        [(target, _)] => *target == object,
        _ => false,
    };
    let plain = !object.is_empty()
        && object
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/+,=@_~".contains(&b));
    (only_target && plain).then_some(object)
}

/// Parsed C-family invocation.
///
/// Field order roughly matches the cache-key construction order
/// (compiler family + version, then flags affecting code gen, then
/// flags affecting layout, then sources). Keeping that consistency
/// makes the cache_key implementation (PR5-B) easier to read.
#[derive(Debug, Clone)]
pub struct CcArgs {
    /// Environment and filesystem context owned by this compilation.
    pub(crate) invocation_env: InvocationEnv,
    /// argv[0] — the compiler binary path the wrapper was invoked as.
    pub program: String,
    /// argv[1..] with supported response files expanded for parsing/keying.
    pub rest: Vec<String>,
    pub(crate) response_file_digests: Vec<String>,
    pub(crate) original_args: Option<Vec<String>>,

    /// Source files (`.c`, `.cpp`, `.cc`, `.cxx`, `.m`, `.mm`).
    /// May be empty for link-only invocations or pure flag probes.
    pub sources: Vec<PathBuf>,
    /// Output path from `-o`. `None` = compiler default (varies by mode).
    pub output: Option<PathBuf>,
    /// What stage the compiler was asked to produce.
    pub mode: CompileMode,
    /// Include search paths from `-I dir` / `-Idir` (in declaration
    /// order — order matters for header search semantics).
    pub includes: Vec<PathBuf>,
    /// Defines from `-D NAME` / `-D NAME=VALUE` (declaration order).
    pub defines: Vec<(String, Option<String>)>,
    /// Optimization level.
    pub optimization: Option<OptLevel>,
    /// Debug-info level: `0` = none (`-g0`), through `3` = max
    /// (`-g3`). Bare `-g` is treated as `2` (compiler default).
    pub debug_level: Option<u8>,
    /// Language standard from `-std=c11` / `-std=c++17` etc.
    /// Stored without the `-std=` prefix.
    pub std: Option<String>,
    /// Position-independent code (`-fPIC` / `-fpic`).
    pub pic: bool,
    /// Dependency-info generation flags. `None` = no dep-info.
    pub depinfo: Option<DepInfoSpec>,
    /// Language override from `-x c` / `-x c++` / `-x objective-c`.
    /// Without this flag, the compiler infers from source extension.
    pub language_override: Option<String>,
    /// Detected compiler driver family (selects the flag dialect).
    pub family: ToolFamily,
    pub(crate) probed_family: Option<crate::probe::ProbedFamily>,
    /// Path ownership captured at parse time for portable dependency sidecars.
    pub(crate) depinfo_roots: Vec<(PathBuf, String, u8)>,
    /// clang's `--serialize-diagnostics <path>` output.
    pub serialize_diagnostics: Option<PathBuf>,
    /// clang's `-include-pch <path>` input.
    pub include_pch: Option<PathBuf>,
}

/// Source file extensions the parser recognizes as C-family input.
/// Anything else gets ignored (left in `rest` for passthrough).
const SOURCE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "cxx", "c++", "C", // C / C++
    "m", "mm", "M", // Objective-C / Objective-C++
    "i", "ii", // already-preprocessed
    "S", "s", "sx", // assembly
];

/// `-x` language overrides whose compilation is representable by one
/// preprocessor output and one code-generation pass. Anything outside
/// this list is refused: multi-pass languages (CUDA, HIP) compile the
/// same TU once per target with different predefined macros
/// (`__CUDA_ARCH__`), so a single `-E` output cannot soundly key them.
const LANGUAGE_OVERRIDE_ALLOWLIST: &[&str] = &[
    "c",
    "c++",
    "c-header",
    "c++-header",
    "objective-c-header",
    "objective-c++-header",
    "objective-c",
    "objective-c++",
    "assembler",
    "assembler-with-cpp",
    "cpp-output",
    "c++-cpp-output",
    "objective-c-cpp-output",
    "objective-c++-cpp-output",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CcArgValueForm {
    Flag,
    Separated,
    Concatenated { prefix: &'static str },
    CanBeSeparated { prefix: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CcArgAction {
    SetMode(CompileMode),
    SetOutput,
    SetPic,
    SetDebugLevel(u8),
    SetOptimization(OptLevel),
    SetStd,
    DepIncludeSystem(bool),
    DepPhonyTargets,
    DepMissingGenerated,
    DepOutput,
    DepTarget,
    LanguageOverride,
    SerializeDiagnostics,
    AuxInfo,
    IncludePch,
    Include,
    /// clang-cl `/imsvc`: a system include directory. Its headers reach the
    /// key through the preprocessor expansion like `-isystem`'s; the
    /// parser only has to consume the value.
    SystemInclude,
    Define,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CcArgBucket {
    Structural,
    ModeledInKey,
    ProbeKeyed,
    Preprocessor,
    RawKeyed,
    Artifact,
    NoObjectEffect,
    TooHard,
}

#[derive(Debug, Clone, Copy)]
struct CcArgSpec {
    matcher: Matcher,
    value_form: CcArgValueForm,
    action: CcArgAction,
    bucket: CcArgBucket,
    source: &'static str,
    /// Dialect this row applies to. `None` = any dialect.
    dialect: Option<Dialect>,
}

#[derive(Debug, Clone)]
struct ParsedCcArg {
    spec: &'static CcArgSpec,
    value: Option<String>,
    consumed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CcArgAnalysis<'a> {
    arg: &'a str,
    class: Option<FlagClass>,
    bucket: CcArgBucket,
    normalized: Vec<String>,
    refusal: Option<&'static str>,
    source: Option<&'static str>,
}

/// A file a `-c` compile writes beside its object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CcSidecar {
    CoverageNotes,
    SplitDwarf,
    SerializedDiagnostics,
    JsonDiagnostics,
    StackUsage,
    OptimizationRecord,
    Callgraph,
    AssemblerListing,
    AuxInfo,
}

impl CcSidecar {
    const ALL: [CcSidecar; 9] = [
        CcSidecar::CoverageNotes,
        CcSidecar::SplitDwarf,
        CcSidecar::SerializedDiagnostics,
        CcSidecar::JsonDiagnostics,
        CcSidecar::StackUsage,
        CcSidecar::OptimizationRecord,
        CcSidecar::Callgraph,
        CcSidecar::AssemblerListing,
        CcSidecar::AuxInfo,
    ];

    pub(crate) fn store_name(self) -> &'static str {
        match self {
            CcSidecar::CoverageNotes => "sidecar.gcno",
            CcSidecar::SplitDwarf => "sidecar.dwo",
            CcSidecar::SerializedDiagnostics => "sidecar.dia",
            CcSidecar::JsonDiagnostics => "sidecar.gcc.json",
            CcSidecar::StackUsage => "sidecar.su",
            CcSidecar::OptimizationRecord => "sidecar.opt.yaml",
            CcSidecar::Callgraph => "sidecar.ci",
            CcSidecar::AssemblerListing => "sidecar.listing",
            CcSidecar::AuxInfo => "sidecar.aux",
        }
    }

    pub(crate) fn from_store_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sidecar| sidecar.store_name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CcSidecarPresence {
    /// A successful compile always writes it.
    Required,
    /// Whether it is written depends on the compiler or target.
    IfWritten,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CcSidecarOutput {
    pub(crate) role: CcSidecar,
    pub(crate) path: PathBuf,
    pub(crate) presence: CcSidecarPresence,
}

static CC_ARG_SPECS: &[CcArgSpec] = &[
    CcArgSpec {
        matcher: Matcher::Exact("-aux-info"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::AuxInfo,
        bucket: CcArgBucket::RawKeyed,
        source: "GCC auxiliary declarations output",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-c"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetMode(CompileMode::Compile),
        bucket: CcArgBucket::Structural,
        source: "compile mode marker",
        dialect: None,
    },
    CcArgSpec {
        // MSVC `/c` slash spelling of the compile-only marker. clang-cl
        // accepts both `-c` and `/c`; without this kache misreads `/c`
        // builds as link mode and passes them through (box-confirmed).
        matcher: Matcher::Exact("/c"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetMode(CompileMode::Compile),
        bucket: CcArgBucket::Structural,
        source: "compile mode marker (cl)",
        dialect: Some(Dialect::Cl),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-E"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetMode(CompileMode::Preprocess),
        bucket: CcArgBucket::Structural,
        source: "preprocess mode marker",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-S"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetMode(CompileMode::Assemble),
        bucket: CcArgBucket::Structural,
        source: "assembly mode marker",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-o"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::SetOutput,
        bucket: CcArgBucket::Artifact,
        source: "primary output path",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-fPIC"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetPic,
        bucket: CcArgBucket::ModeledInKey,
        source: "position-independent code",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-fpic"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetPic,
        bucket: CcArgBucket::ModeledInKey,
        source: "position-independent code",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-g"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetDebugLevel(2),
        bucket: CcArgBucket::ModeledInKey,
        source: "debug-info level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-g0"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetDebugLevel(0),
        bucket: CcArgBucket::ModeledInKey,
        source: "debug-info level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-g1"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetDebugLevel(1),
        bucket: CcArgBucket::ModeledInKey,
        source: "debug-info level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-g2"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetDebugLevel(2),
        bucket: CcArgBucket::ModeledInKey,
        source: "debug-info level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-g3"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetDebugLevel(3),
        bucket: CcArgBucket::ModeledInKey,
        source: "debug-info level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-O"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::O1),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-O0"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::O0),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-O1"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::O1),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-O2"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::O2),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-O3"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::O3),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-Os"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::Os),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-Oz"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::Oz),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Exact("-Og"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::SetOptimization(OptLevel::Og),
        bucket: CcArgBucket::ModeledInKey,
        source: "optimization level",
        dialect: None,
    },
    // ── GNU dep-info rows — gnu dialect only ─────────────────────
    // In clang-cl mode, `-MD`/`-MMD` are CRT-selection flags (matching
    // MSVC `/MD`/`/MDd`), `-MT`/`-MF`/`-MQ` are also CRT/output
    // spellings, and `-MP`/`-MG` are unrelated. Tagging these rows
    // `Dialect::Gnu` makes the parser skip them entirely under clang-cl
    // so they fall through to the flag classifier / unknown-flag path.
    CcArgSpec {
        matcher: Matcher::Exact("-MD"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::DepIncludeSystem(true),
        bucket: CcArgBucket::NoObjectEffect,
        source: "dependency sidecar",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-MMD"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::DepIncludeSystem(false),
        bucket: CcArgBucket::NoObjectEffect,
        source: "dependency sidecar",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-MP"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::DepPhonyTargets,
        bucket: CcArgBucket::NoObjectEffect,
        source: "dependency sidecar phony targets",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-MG"),
        value_form: CcArgValueForm::Flag,
        action: CcArgAction::DepMissingGenerated,
        bucket: CcArgBucket::NoObjectEffect,
        source: "dependency sidecar generated headers",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-MF"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::DepOutput,
        bucket: CcArgBucket::Artifact,
        source: "dependency output path",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-MT"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::DepTarget,
        bucket: CcArgBucket::NoObjectEffect,
        source: "dependency target",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-include-pch"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::IncludePch,
        bucket: CcArgBucket::Preprocessor,
        source: "clang precompiled header input, keyed by content and input stamps",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("--serialize-diagnostics"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::SerializeDiagnostics,
        bucket: CcArgBucket::Artifact,
        source: "clang serialized diagnostics output path",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-serialize-diagnostics"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::SerializeDiagnostics,
        bucket: CcArgBucket::Artifact,
        source: "clang serialized diagnostics output path",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Exact("-MQ"),
        value_form: CcArgValueForm::Separated,
        action: CcArgAction::DepTarget,
        bucket: CcArgBucket::NoObjectEffect,
        source: "dependency target",
        dialect: Some(Dialect::Gnu),
    },
    CcArgSpec {
        matcher: Matcher::Prefix("-x"),
        value_form: CcArgValueForm::CanBeSeparated { prefix: "-x" },
        action: CcArgAction::LanguageOverride,
        bucket: CcArgBucket::ProbeKeyed,
        source: "language override",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Prefix("-I"),
        value_form: CcArgValueForm::CanBeSeparated { prefix: "-I" },
        action: CcArgAction::Include,
        bucket: CcArgBucket::Preprocessor,
        source: "include search path",
        dialect: None,
    },
    // clang-cl's system include directory, separate or joined. cargo-xwin
    // passes the CRT and Windows SDK headers this way.
    CcArgSpec {
        matcher: Matcher::Prefix("/imsvc"),
        value_form: CcArgValueForm::CanBeSeparated { prefix: "/imsvc" },
        action: CcArgAction::SystemInclude,
        bucket: CcArgBucket::Preprocessor,
        source: "system include search path",
        dialect: Some(Dialect::Cl),
    },
    CcArgSpec {
        matcher: Matcher::Prefix("-imsvc"),
        value_form: CcArgValueForm::CanBeSeparated { prefix: "-imsvc" },
        action: CcArgAction::SystemInclude,
        bucket: CcArgBucket::Preprocessor,
        source: "system include search path",
        dialect: Some(Dialect::Cl),
    },
    CcArgSpec {
        matcher: Matcher::Prefix("-D"),
        value_form: CcArgValueForm::CanBeSeparated { prefix: "-D" },
        action: CcArgAction::Define,
        bucket: CcArgBucket::Preprocessor,
        source: "preprocessor define",
        dialect: None,
    },
    CcArgSpec {
        matcher: Matcher::Prefix("-std="),
        value_form: CcArgValueForm::Concatenated { prefix: "-std=" },
        action: CcArgAction::SetStd,
        bucket: CcArgBucket::ModeledInKey,
        source: "language standard",
        dialect: None,
    },
    // ── clang-cl output and standard rows (#285) ─────────────────
    // clang-cl uses `-Fo<obj>` / `/Fo<obj>` (concatenated, no space)
    // for the object output path, analogous to gnu `-o <obj>`.
    CcArgSpec {
        matcher: Matcher::Prefix("-Fo"),
        value_form: CcArgValueForm::Concatenated { prefix: "-Fo" },
        action: CcArgAction::SetOutput,
        bucket: CcArgBucket::Artifact,
        source: "clang-cl object output (#285)",
        dialect: Some(Dialect::Cl),
    },
    CcArgSpec {
        matcher: Matcher::Prefix("/Fo"),
        value_form: CcArgValueForm::Concatenated { prefix: "/Fo" },
        action: CcArgAction::SetOutput,
        bucket: CcArgBucket::Artifact,
        source: "clang-cl object output (#285)",
        dialect: Some(Dialect::Cl),
    },
    // clang-cl language standard: `-std:c++20` / `/std:c++20`.
    // The value after the prefix (e.g. `c++20`) is stored in `parsed.std`.
    CcArgSpec {
        matcher: Matcher::Prefix("-std:"),
        value_form: CcArgValueForm::Concatenated { prefix: "-std:" },
        action: CcArgAction::SetStd,
        bucket: CcArgBucket::ModeledInKey,
        source: "clang-cl language standard (#285)",
        dialect: Some(Dialect::Cl),
    },
    CcArgSpec {
        matcher: Matcher::Prefix("/std:"),
        value_form: CcArgValueForm::Concatenated { prefix: "/std:" },
        action: CcArgAction::SetStd,
        bucket: CcArgBucket::ModeledInKey,
        source: "clang-cl language standard (#285)",
        dialect: Some(Dialect::Cl),
    },
];

impl CcArgs {
    pub(crate) fn execution_args(&self) -> &[String] {
        self.original_args.as_deref().unwrap_or(&self.rest)
    }

    #[cfg(test)]
    pub fn parse(args: &[String]) -> Result<Self> {
        Self::parse_for(args, &InvocationEnv::capture())
    }

    pub(crate) fn parse_for(args: &[String], env: &InvocationEnv) -> Result<Self> {
        let (program, rest) = args
            .split_first()
            .context("cc invocation missing argv[0]")?;

        let family = ToolFamily::detect(program, rest);
        let probed_family = if family.dialect() == Dialect::Gnu
            && rest
                .iter()
                .any(|arg| arg.starts_with("-Wp,") || arg == "-emit-llvm" || cc_report_flag(arg))
        {
            crate::probe::probe_compiler_family_for(program, env)
        } else {
            None
        };

        let mut parsed = CcArgs {
            invocation_env: env.clone(),
            program: program.clone(),
            rest: rest.to_vec(),
            response_file_digests: Vec::new(),
            original_args: None,
            sources: Vec::new(),
            output: None,
            mode: CompileMode::Link, // default: compile + link
            includes: Vec::new(),
            defines: Vec::new(),
            optimization: None,
            debug_level: None,
            std: None,
            pic: false,
            depinfo: None,
            language_override: None,
            family,
            probed_family,
            depinfo_roots: Vec::new(),
            serialize_diagnostics: None,
            include_pch: None,
        };

        // Walk argv through a table-driven parser so spelling variants
        // like `-x c` / `-xc` and `-I dir` / `-Idir` share one rule.
        let mut depinfo: Option<DepInfoSpec> = None;
        let mut headers = Vec::new();
        let mut idx = 0;
        while idx < rest.len() {
            if let Some(arg) = parse_cc_arg_at(rest, idx, family.dialect()) {
                apply_cc_arg(&mut parsed, &mut depinfo, &arg);
                idx += arg.consumed;
                continue;
            }

            let arg = &rest[idx];
            if !arg.starts_with('-') && looks_like_source(arg) {
                parsed.sources.push(PathBuf::from(arg));
            } else if !arg.starts_with('-') && looks_like_header(arg) {
                headers.push(PathBuf::from(arg));
            }
            idx += 1;
        }
        parsed.depinfo = depinfo;
        if parsed.depinfo.is_none() {
            parsed.depinfo = cc_wp_dependency_spec(&parsed);
        }
        let header_language = parsed
            .language_override
            .as_deref()
            .is_some_and(|language| language.ends_with("-header"));
        if header_language && matches!(parsed.mode, CompileMode::Link | CompileMode::Compile) {
            parsed.mode = CompileMode::Precompile;
            parsed.sources.extend(headers);
        }

        Ok(parsed)
    }

    /// Enumerate refuse-to-cache reasons the parsed invocation
    /// triggers. Returns an empty vector for "looks safe to cache".
    ///
    /// Each detection is conservative — we'd rather refuse a
    /// cacheable invocation than miscache an unsafe one. Specific
    /// patterns covered:
    ///
    /// - **Response files** (`@file.rsp`): the actual flags live in
    ///   another file we'd need to read + hash separately.
    /// - **Multi-arch fat binaries** (`-arch x86_64 -arch arm64`):
    ///   output is a single file containing multiple object slices,
    ///   doesn't fit the per-source-per-output model.
    /// - **Sidecars outside `-c`**: coverage notes, split DWARF and
    ///   serialized diagnostics are modeled for object compiles only
    ///   ([`CcArgs::sidecar_outputs`]).
    /// - **Precompiled headers** (`-include-pch`, `-emit-pch`):
    ///   PCHs are non-portable across compiler versions and depend
    ///   on the entire include graph at PCH-build time.
    /// - **Implicit modules** (`-fmodules`, `-fcxx-modules`): the module
    ///   cache is built and read behind the compile. Explicit consumers
    ///   (`-fmodule-file=`, `-fprebuilt-module-path=`) are keyed by
    ///   [`cc_module_inputs_key`]; producing a module file is unclassified.
    /// - **Any flag not classified by [`CC_FLAGS`]**: the cache key
    ///   captures the preprocessor expansion plus the codegen flags
    ///   kache explicitly models (`FlagClass::ModeledInKey`) plus the
    ///   resolved `cc -###` tokens (`FlagClass::CapturedByProbe`). A
    ///   flag whose object-file effect is in none of those — an
    ///   unmodeled codegen flag (`-Ofast`, `-fsanitize=address`),
    ///   profiling (`-pg`), or a flag kache has
    ///   never seen — would miscache. The table is the source of truth;
    ///   anything it does not classify is refused with the offending
    ///   flags named in the reason.
    /// - **Output to stdout** (`-o -`): not a cacheable artifact.
    /// - **Assemble mode**: `-S` without `-o`, or under clang-cl, is still
    ///   refused. `-S -o` and `-E` to a named file or to stdout are cached.
    pub fn refuse_reasons(&self, extra_allowlist_flags: &[String]) -> Vec<RefuseReason> {
        let mut reasons = Vec::new();

        if self.emits_llvm()
            && (self.probed_family != Some(crate::probe::ProbedFamily::Clang)
                || !matches!(self.mode, CompileMode::Compile | CompileMode::Assemble)
                || self.is_universal())
        {
            reasons.push(RefuseReason::Unsupported("cc LLVM emission requires a verified single-target Clang -c or -S invocation — not yet"));
        }

        if self.rest.iter().any(|arg| arg.starts_with("-Wp,"))
            && cc_wp_dependency_spec(self).is_none()
        {
            reasons.push(RefuseReason::Unsupported(
                "cc mixed or unsafe -Wp dependency options — not yet",
            ));
        }

        if self.family == ToolFamily::Gnu {
            for (variable, reason) in [
                ("DEPENDENCIES_OUTPUT", "cc DEPENDENCIES_OUTPUT dependency output — not yet"),
                ("SUNPRO_DEPENDENCIES", "cc SUNPRO_DEPENDENCIES dependency output — not yet"),
            ] {
                if self.invocation_env.var_os(variable).is_some() {
                    reasons.push(RefuseReason::Unsupported(reason));
                }
            }
        }

        // ── Non-`-c` mode refusals (short-circuit) ──
        //
        // First, check whether the invocation is the `-c` object
        // compile shape the flag classifier was designed for. If not,
        // the flag-classifier output below ("unsupported flag(s): -E")
        // is misleading — those flags aren't blocking caching of a
        // compile, they belong to a different invocation pattern.
        // Short-circuit to keep the reason list focused.
        //
        // All variants here are `Unsupported` — none of them are
        // conceptually uncacheable. `-E` / `-S` / link / output-to-
        // stdout are all deterministic input-to-output functions; the
        // reason kache doesn't cache them today is engineering
        // priority, not feasibility. Messages include "(not yet
        // supported)" so this is explicit in the bench output and
        // anyone reading `kache report`.
        match self.mode {
            CompileMode::Compile => {}
            CompileMode::Link => reasons.push(RefuseReason::Unsupported(
                "cc link mode (whole-program caching) — not yet",
            )),
            // `-E` to a named file or to stdout is a single-output compile:
            // the expansion is the artifact. `-o -` is still refused below.
            CompileMode::Preprocess => {}
            // `-S` to a named file is a single-output compile like `-c`.
            CompileMode::Assemble if (self.output.is_some() || self.emits_llvm()) && self.family.dialect() == Dialect::Gnu => {}
            CompileMode::Assemble => reasons.push(RefuseReason::Unsupported(
                "cc assembly mode -S without -o — not yet",
            )),
            CompileMode::Precompile
                if self.output.is_some()
                    && self.family.dialect() == Dialect::Gnu
                    && self.include_pch.is_none() => {}
            CompileMode::Precompile => reasons.push(RefuseReason::Unsupported(
                "cc precompiled header without -o, or built from another PCH — not yet",
            )),
        }

        // Output to stdout — `-o -` is unambiguous; an `-o` followed
        // by a literal `-` arg. Cacheable in principle (cache the
        // stdout bytes); not yet implemented.
        if let Some(output) = &self.output
            && output.as_os_str() == "-"
        {
            reasons.push(RefuseReason::Unsupported("cc output to stdout — not yet"));
        }

        // If a non-`-c` refusal accumulated, return early — running
        // the flag classifier or feature checks would add misleading
        // noise ("unsupported flag(s): -E" when the real cause is
        // "this is preprocessor mode"). The single-source check is NOT
        // short-circuited here: a feature like a response file
        // (`@foo.opts`) appears to the parser as zero sources, and the
        // feature explanation is more useful than the bare symptom.
        if !reasons.is_empty() {
            return reasons;
        }

        if self.requires_compiler_output_semantics() {
            reasons.push(RefuseReason::Unsupported(
                "existing output path requires compiler write semantics — caching not yet supported",
            ));
        }

        // ── Feature refusals ──
        //
        // The invocation IS a single-source object compile, but uses a
        // feature kache doesn't model yet. These are the actionable
        // refusals: adding support would convert future invocations
        // into hits.

        // Multi-pass languages. CUDA and HIP split compilation into
        // host and device passes over the same TU; `__CUDA_ARCH__`
        // differs between them, so one `-E` output cannot safely key
        // the invocation. Fail closed: only overrides known to be
        // single-pass stay cacheable.
        if let Some(language) = &self.language_override
            && !LANGUAGE_OVERRIDE_ALLOWLIST.contains(&language.as_str())
        {
            reasons.push(RefuseReason::Unsupported(
                "cc language override -x outside the single-pass C family — not yet",
            ));
        }

        // CUDA sources are not in SOURCE_EXTENSIONS, so they stay in
        // `rest`; recognizing them here avoids misreporting a
        // CUDA-shaped compile as having no source file.
        let cuda_input = self
            .rest
            .iter()
            .any(|arg| arg.ends_with(".cu") || arg.ends_with(".cuh"));
        if cuda_input {
            reasons.push(RefuseReason::Unsupported(
                "cc CUDA source input (.cu/.cuh) — not yet",
            ));
        }

        // Response files: any arg starting with `@` (typically a
        // path to a file containing additional flags). The flags
        // inside aren't visible to our parser without recursive
        // expansion + path normalization.
        if self.rest.iter().any(|a| a.starts_with('@')) {
            reasons.push(RefuseReason::Unsupported(
                "cc response file @file (expansion) — not yet",
            ));
        }

        if self.emits_reports() {
            let supported = self.probed_family.is_some_and(|family| {
                self.rest.iter().filter(|arg| cc_report_flag(arg)).all(|arg| {
                    if arg == "-fsave-optimization-record" {
                        family == crate::probe::ProbedFamily::Clang
                    } else {
                        family == crate::probe::ProbedFamily::Gnu
                    }
                })
            });
            let unsafe_option = self.rest.iter().any(|arg| {
                arg.starts_with("-flto") || arg.starts_with("-dump")
                    || arg.starts_with("-Xclang") || arg.starts_with("-Xassembler")
                    || (arg.starts_with("-fopt-info") && arg.contains('='))
                    || arg == "-Wa,-adhln=" || arg == "-Wa,-adhln=-"
            });
            let invalid_aux = self.rest.iter().enumerate().any(|(i, arg)| {
                arg == "-aux-info" && self.rest.get(i + 1).is_none_or(|path| path.is_empty() || path.starts_with('-'))
            });
            let repeated_output = self.rest.iter().filter(|arg| arg.starts_with("-Wa,-adhln")).count() > 1
                || self.rest.iter().filter(|arg| *arg == "-aux-info").count() > 1;
            if !supported || unsafe_option || invalid_aux || repeated_output {
                reasons.push(RefuseReason::Unsupported("cc reports with unknown compiler family, append outputs, LTO, dump overrides or invalid output paths — not yet"));
            }
            if self.lists_assembler_source() && self.rest.iter().any(|arg| {
                arg.starts_with("-ffile-prefix-map=") || arg.starts_with("-fdebug-prefix-map=")
            }) {
                reasons.push(RefuseReason::Unsupported("cc assembler listing with user prefix mapping may read an untracked source — not yet"));
            }
        }

        // Universal objects are keyed slice by slice. Sidecars would need
        // one file per slice.
        if self.is_universal()
            && (self.family.dialect() != Dialect::Gnu
                || !self.sidecar_outputs().is_empty()
                || self.coverage_instrumented()
                || self.emits_reports())
        {
            reasons.push(RefuseReason::Unsupported(
                "cc multi-arch -arch X -arch Y outside a plain clang compile — not yet",
            ));
        }

        // clang applies `-Xarch_<arch> <operand>` to that slice alone and
        // ignores it when no slice matches. Only universal compiles key each
        // slice from its own arguments; elsewhere the parse would apply the
        // operand clang ignores.
        let slices = self.arch_slices();
        let mut args = self.rest.iter();
        while let Some(arg) = args.next() {
            let Some(arch) = arg.strip_prefix("-Xarch_") else { continue };
            let operand = args.next();
            if slices.len() < 2
                || !slices.iter().any(|slice| slice == arch)
                || operand.is_none_or(|operand| operand == "-arch" || operand.starts_with("-Xarch_"))
            {
                reasons.push(RefuseReason::Unsupported(
                    "cc -Xarch_<arch> outside a universal compile that builds <arch> — not yet",
                ));
                break;
            }
        }

        // Sidecars are modeled for `-c` only. `-S` still writes the notes
        // file, and `-E` still writes serialized diagnostics.
        if self.mode != CompileMode::Compile
            && (self.coverage_instrumented()
                || self.splits_dwarf()
                || self.serialize_diagnostics.is_some()
                || self.emits_reports())
        {
            reasons.push(RefuseReason::Unsupported(
                "cc coverage, split DWARF or serialized diagnostics outside -c — not yet",
            ));
        }

        // GCC diagnostic outputs that write files a hit would not restore.
        // A SARIF log records the run's own start and end times and cc1's
        // temporary file names, so no stored log matches a fresh compile's.
        let unmodeled_json = self.rest.iter().any(|arg| arg == "-fdiagnostics-format=json-file")
            && self.gcc_json_diagnostics_file().is_none();
        if unmodeled_json
            || self.rest.iter().any(|arg| {
                arg == "-fdiagnostics-format=sarif-file"
                    || arg.starts_with("-fdiagnostics-add-output=")
                    || arg.starts_with("-fdiagnostics-set-output=")
            })
        {
            reasons.push(RefuseReason::Unsupported(
                "cc diagnostics written to a file — not yet",
            ));
        }

        // Precompiled headers. A `-c` consumer of `-include-pch` and a
        // `-x <lang>-header` build are modeled; the cc1 spelling is not.
        let unmodeled_pch_use = self.include_pch.is_some() && self.mode != CompileMode::Compile;
        if unmodeled_pch_use {
            reasons.push(RefuseReason::Unsupported("cc precompiled headers — not yet"));
        }
        for flag in &["-emit-pch"] {
            if self.rest.iter().any(|a| a == flag) {
                reasons.push(RefuseReason::Unsupported(
                    "cc precompiled headers — not yet",
                ));
                break;
            }
        }
        // `*.pch` / `*.gch` as a forced-include argument also indicates PCH.
        // All three spellings of the same option have to be checked, or the
        // long forms (modeled `PreprocessorCaptured` for #580) would carry a
        // PCH past a refusal the short form catches.
        let is_pch = |p: &str| p.ends_with(".pch") || p.ends_with(".gch");
        let mut iter = self.rest.iter().peekable();
        while let Some(arg) = iter.next() {
            let pch = match arg.strip_prefix("--include=") {
                Some(value) => is_pch(value),
                None => {
                    (arg == "-include" || arg == "--include")
                        && iter.peek().is_some_and(|next| is_pch(next))
                }
            };
            if pch {
                reasons.push(RefuseReason::Unsupported(
                    "cc precompiled headers — not yet",
                ));
                break;
            }
        }

        // Modules (clang/gcc).
        for flag in &["-fmodules", "-fcxx-modules"] {
            if self.rest.iter().any(|a| a == flag) {
                reasons.push(RefuseReason::Unsupported("cc modules — not yet"));
                break;
            }
        }

        // Classifier gate — the structural safety net.
        //
        // kache's cc cache key captures the preprocessor expansion
        // plus the codegen flags it *explicitly* models (optimization,
        // debug level, `-std`, PIC, target arch) plus the resolved
        // `cc -###` token stream. A flag whose effect is captured by
        // none of those would change the object file WITHOUT changing
        // the key — a silent miscache.
        //
        // [`CC_FLAGS`] declares which flags fall into which category;
        // [`classify_cc_flag`] returns `None` for anything outside the
        // table. Unclassified flags include the genuinely unsafe
        // (`-Ofast`, `-march=native`), the cross-targets (`-target`,
        // `--target=`), profiling (`-pg`), and any flag kache has not
        // yet seen — all force a passthrough. The rejected flags are
        // named in the reason so it is visible which flags blocked
        // caching (and therefore which rows to add to `CC_FLAGS`).
        let rejected = classify_and_trace_cc_flags(self, extra_allowlist_flags);
        if !rejected.is_empty() {
            // Leak a per-invocation summary so it can ride in
            // `RefuseReason::Unsupported(&'static str)`. The wrapper
            // process handles one compile then exits, so the leak is
            // bounded and short-lived.
            let detail: &'static str = Box::leak(
                format!("cc unsupported flag(s): {} — not yet", rejected.join(" "))
                    .into_boxed_str(),
            );
            log::debug!("{detail} — passthrough");
            reasons.push(RefuseReason::Unsupported(detail));
        }

        // Single-source contract — last, so feature refusals
        // (response file, PCH-as-input, ...) get a chance to explain
        // *why* there's no parseable single source. Without that
        // ordering, `cc @foo.opts` would land here instead of getting
        // the more specific "response file (@file)" reason.
        //
        // Reported as `Unsupported` with "(not yet supported)" wording:
        // multi-source `cc -c a.c b.c` is conceptually N independent
        // single-source compiles bundled into one invocation —
        // per-source caching is on the roadmap, just unimplemented.
        // Zero-source falls under the same "kache doesn't yet handle
        // this invocation pattern" bucket; a future expansion of
        // response files or improved probe-vs-compile detection would
        // convert most of these.
        if self.sources.len() > 1 {
            reasons.push(RefuseReason::Unsupported(
                "cc multi-source compile (per-source split) — not yet",
            ));
        } else if self.sources.is_empty() && !cuda_input {
            // Suppressed for CUDA inputs: the dedicated refusal above
            // already names the real cause.
            reasons.push(RefuseReason::Unsupported("cc no source file — not yet"));
        }

        reasons
    }

    /// The object file a `-c` compile produces.
    ///
    /// `-o <path>` if explicit; otherwise the compiler default — the
    /// source file's stem with a `.o` (gnu dialect) or `.obj` (cl
    /// dialect) extension, in the current working directory. Returns
    /// `None` only for degenerate invocations with no source (which
    /// `refuse_reasons` already rejects, so callers on the cache path
    /// won't hit `None`).
    pub fn object_output_path(&self) -> Option<PathBuf> {
        if let Some(o) = &self.output {
            return Some(o.clone());
        }
        let stem = self.sources.first()?.file_stem()?;
        let ext = match self.family.dialect() {
            Dialect::Cl => "obj",
            Dialect::Gnu if self.emits_llvm() && self.mode == CompileMode::Assemble => "ll",
            Dialect::Gnu if self.emits_llvm() => "bc",
            Dialect::Gnu => "o",
        };
        Some(PathBuf::from(format!("{}.{ext}", stem.to_string_lossy())))
    }

    /// The dep-info file a compile produces when `-MD` / `-MMD` is active.
    ///
    /// `-MF <path>` wins. Otherwise gcc/clang derive the depfile from the
    /// object output by replacing its extension with `.d`.
    pub fn depinfo_output_path(&self) -> Option<PathBuf> {
        let depinfo = self.depinfo.as_ref()?;
        if !depinfo.emit {
            return None;
        }
        if let Some(output) = &depinfo.output {
            return Some(output.clone());
        }
        let mut object = self.object_output_path()?;
        object.set_extension("d");
        Some(object)
    }

    /// The `-arch` values in order, each once.
    pub(crate) fn arch_slices(&self) -> Vec<String> {
        let mut slices: Vec<String> = Vec::new();
        for pair in self.rest.windows(2).filter(|pair| pair[0] == "-arch") {
            if !slices.contains(&pair[1]) {
                slices.push(pair[1].clone());
            }
        }
        slices
    }

    /// A universal object: one compile writes a slice per `-arch`.
    pub(crate) fn is_universal(&self) -> bool {
        self.arch_slices().len() > 1
    }

    /// This invocation as one slice would see it, for keying that slice: its
    /// `-arch` flags narrowed to `arch`, each `-Xarch_<arch>` operand kept
    /// and every other slice's operands dropped.
    fn single_slice(&self, arch: &str) -> Result<CcArgs> {
        let mut argv = vec![self.program.clone()];
        let mut args = self.rest.iter();
        let mut placed = false;
        while let Some(arg) = args.next() {
            if arg == "-arch" {
                args.next();
                if !placed {
                    argv.extend(["-arch".to_string(), arch.to_string()]);
                    placed = true;
                }
            } else if let Some(target) = arg.strip_prefix("-Xarch_") {
                if let Some(operand) = args.next()
                    && target == arch
                {
                    argv.push(operand.clone());
                }
            } else {
                argv.push(arg.clone());
            }
        }
        let mut slice = CcArgs::parse_for(&argv, &self.invocation_env)?;
        slice.depinfo_roots = self.depinfo_roots.clone();
        Ok(slice)
    }

    /// The files a `-c` compile writes beside its object.
    ///
    /// GCC and clang both name the notes and split-DWARF files after the
    /// object with its last extension replaced. Without `-g`, GCC still
    /// writes a `.dwo` for `-gsplit-dwarf` and clang does not, and Mach-O
    /// targets never write one, so the `.dwo` is only certain under debug
    /// info on other hosts.
    pub(crate) fn sidecar_outputs(&self) -> Vec<CcSidecarOutput> {
        if self.mode != CompileMode::Compile || self.family.dialect() != Dialect::Gnu {
            return Vec::new();
        }
        let Some(object) = self.object_output_path() else {
            return Vec::new();
        };
        let has = |flag: &str| self.rest.iter().any(|arg| arg == flag);
        let mut outputs = Vec::new();
        if has("--coverage") || has("-ftest-coverage") {
            outputs.push(CcSidecarOutput {
                role: CcSidecar::CoverageNotes,
                path: object.with_extension("gcno"),
                presence: CcSidecarPresence::Required,
            });
        }
        if self.splits_dwarf() {
            let certain = self.debug_level.is_some_and(|level| level > 0) && !cfg!(target_os = "macos");
            outputs.push(CcSidecarOutput {
                role: CcSidecar::SplitDwarf,
                path: object.with_extension("dwo"),
                presence: if certain {
                    CcSidecarPresence::Required
                } else {
                    CcSidecarPresence::IfWritten
                },
            });
        }
        if let Some(path) = &self.serialize_diagnostics {
            outputs.push(CcSidecarOutput {
                role: CcSidecar::SerializedDiagnostics,
                path: path.clone(),
                presence: CcSidecarPresence::Required,
            });
        }
        if let Some(path) = self.gcc_json_diagnostics_file() {
            outputs.push(CcSidecarOutput {
                role: CcSidecar::JsonDiagnostics,
                path,
                presence: CcSidecarPresence::Required,
            });
        }
        for (flag, role, extension) in [
            ("-fstack-usage", CcSidecar::StackUsage, "su"),
            ("-fsave-optimization-record", CcSidecar::OptimizationRecord, "opt.yaml"),
            ("-fcallgraph-info", CcSidecar::Callgraph, "ci"),
        ] {
            if self.rest.iter().any(|arg| arg == flag || (flag == "-fcallgraph-info" && arg.starts_with("-fcallgraph-info="))) {
                outputs.push(CcSidecarOutput { role, path: object.with_extension(extension), presence: CcSidecarPresence::Required });
            }
        }
        if let Some(path) = self.rest.iter().rev().find_map(|arg| arg.strip_prefix("-Wa,-adhln=")) {
            outputs.push(CcSidecarOutput { role: CcSidecar::AssemblerListing, path: PathBuf::from(path), presence: CcSidecarPresence::Required });
        }
        if let Some(path) = self.rest.windows(2).rev().find_map(|pair| (pair[0] == "-aux-info").then(|| PathBuf::from(&pair[1]))) {
            outputs.push(CcSidecarOutput { role: CcSidecar::AuxInfo, path, presence: CcSidecarPresence::IfWritten });
        }
        outputs
    }

    /// The file GCC writes for `-fdiagnostics-format=json-file`, even when
    /// there is nothing to report. Observed with GCC 15: it lands in the
    /// working directory whatever directory `-o` names, as the object's name
    /// without its extension, then the source's extension, then `.gcc.json`
    /// (`-o out/a.b.o` with `w.c` writes `a.b.c.gcc.json`).
    fn gcc_json_diagnostics_file(&self) -> Option<PathBuf> {
        if self.family != ToolFamily::Gnu || self.mode != CompileMode::Compile {
            return None;
        }
        let format = self.rest.iter().rev().find_map(|arg| arg.strip_prefix("-fdiagnostics-format="));
        if format != Some("json-file") {
            return None;
        }
        let source_ext = self.sources.first()?.extension()?.to_str()?;
        let object = self.object_output_path()?;
        let stem = object.file_stem()?.to_str()?;
        Some(PathBuf::from(format!("{stem}.{source_ext}.gcc.json")))
    }

    /// Coverage instrumentation: the object names its absolute `.gcda`
    /// path and the notes file records the compilation directory.
    fn coverage_instrumented(&self) -> bool {
        self.rest.iter().any(|arg| {
            matches!(arg.as_str(), "--coverage" | "-fprofile-arcs" | "-ftest-coverage")
                || arg.starts_with("-fprofile-generate=")
        })
    }

    fn records_switches(&self) -> bool {
        self.rest.iter().any(|arg| arg == "-frecord-gcc-switches")
    }

    fn emits_llvm(&self) -> bool {
        self.rest.iter().any(|arg| arg == "-emit-llvm")
    }

    fn lists_assembler_source(&self) -> bool {
        self.rest.iter().any(|arg| arg == "-Wa,-adhln" || arg.starts_with("-Wa,-adhln="))
    }

    fn emits_reports(&self) -> bool {
        self.rest.iter().any(|arg| cc_report_flag(arg))
    }

    /// Split DWARF: the object names its `.dwo` as the output was spelled.
    fn splits_dwarf(&self) -> bool {
        self.rest.iter().any(|arg| arg == "-gsplit-dwarf")
    }

    /// Every filesystem output selected by a compile-mode invocation.
    ///
    /// Cacheable invocations have one source, but refused multi-source
    /// invocations still need exact passthrough safety checks for each default
    /// object and dep-info path. `-S` selects `foo.s`; the other modes
    /// deliberately return no object-shaped paths: `-E foo.c` does not
    /// select `foo.o`.
    pub(crate) fn compiler_output_paths(&self) -> Vec<PathBuf> {
        let ext = match (self.mode, self.family.dialect()) {
            (CompileMode::Compile, Dialect::Cl) => "obj",
            (CompileMode::Compile, Dialect::Gnu) if self.emits_llvm() => "bc",
            (CompileMode::Assemble, Dialect::Gnu) if self.emits_llvm() => "ll",
            (CompileMode::Compile, Dialect::Gnu) => "o",
            (CompileMode::Assemble, _) => "s",
            (CompileMode::Precompile, _) => {
                return match &self.output {
                    Some(output) => vec![output.clone()],
                    None => self
                        .sources
                        .iter()
                        .map(|source| PathBuf::from(format!("{}.gch", source.to_string_lossy())))
                        .collect(),
                };
            }
            (CompileMode::Preprocess | CompileMode::Link, _) => return Vec::new(),
        };

        let objects = if let Some(output) = &self.output {
            vec![output.clone()]
        } else {
            self.sources
                .iter()
                .filter_map(|source| {
                    source
                        .file_stem()
                        .map(|stem| PathBuf::from(format!("{}.{ext}", stem.to_string_lossy())))
                })
                .collect()
        };

        let mut paths = objects.clone();
        if let Some(depinfo) = &self.depinfo
            && depinfo.emit
        {
            if let Some(output) = &depinfo.output {
                paths.push(output.clone());
            } else {
                paths.extend(objects.into_iter().map(|mut object| {
                    object.set_extension("d");
                    object
                }));
            }
        }
        paths.extend(self.sidecar_outputs().into_iter().map(|sidecar| sidecar.path));
        paths
    }

    /// Whether an existing output needs the selected compiler's own path
    /// handling instead of cache materialization.
    ///
    /// Ordinary compiler-owned outputs are private, owner-writable regular
    /// files. Kache may replace those on a hit and let the compiler overwrite
    /// them on a miss. Symlinks, hardlinks, read-only files and non-regular
    /// paths still need the selected compiler's exact pathname semantics.
    pub(crate) fn requires_compiler_output_semantics(&self) -> bool {
        self.compiler_output_paths().into_iter().any(|path| {
            self.invocation_env
                .resolve(&path)
                .is_none_or(|path| output_path_requires_compiler_semantics(&path))
        })
    }

    /// Anchor used to relativize/expand C/C++ dep-info target paths.
    pub fn depinfo_anchor(&self) -> Option<PathBuf> {
        self.depinfo_output_path()?;
        let object = self.object_output_path()?;
        Some(
            object
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
        )
    }

    /// Target architecture for cache-key / metadata purposes:
    /// an explicit `-arch X` if present, else the host arch.
    pub fn cache_target_arch(&self) -> String {
        cc_target_arch(self)
    }

    /// True when this clang-cl compile requests debug info that CodeView
    /// records with un-remapped paths. Those objects stay in the local store.
    #[cfg(test)]
    pub fn embeds_codeview_debug(&self) -> bool {
        cl_debug_present(self)
    }

    /// The subset of `rest` that identifies the *compile configuration*
    /// — per-translation-unit noise removed: source files, the `-o`
    /// output path, and (under the Gnu dialect only) dependency-file
    /// flags (`-MF`/`-MT`/`-MQ`) with their values. Under the Cl dialect
    /// those `-M*` spellings are CRT selection (codegen), so they are
    /// kept. The resolved-invocation probe (`cc -###`) is memoized on
    /// this, so every TU of a build that shares a flag set reuses one
    /// probe record instead of re-resolving per file.
    pub fn config_args(&self) -> Vec<String> {
        let mut out = Vec::new();
        let probe_args = self.probe_args();
        let mut iter = probe_args.iter();
        // Per-TU noise to drop from the probe-memo key. GnuDialect drops
        // the dep-target flags (`-MT`/`-MF`/`-MQ`) and their values; the
        // cl dialect must NOT — there `-MT`/`-MD` are CRT-selection
        // codegen (CapturedByProbe) and stripping them from the memo key
        // would collapse distinct CRTs into one record (false hit, #285).
        let drops_value: &[&str] = match self.family.dialect() {
            Dialect::Gnu => &[
                "-o",
                "-MF",
                "-MT",
                "-MQ",
                "--serialize-diagnostics",
                "-serialize-diagnostics",
            ],
            Dialect::Cl => &["-o"],
        };
        while let Some(arg) = iter.next() {
            if drops_value.contains(&arg.as_str()) {
                iter.next(); // also drop the flag's value
            } else if self.family.dialect() == Dialect::Cl
                && (arg.starts_with("-Fo") || arg.starts_with("/Fo"))
            {
                // `-Fo<obj>` / `/Fo<obj>` — concatenated output token,
                // per-TU noise. Strip it from the probe-memo key so the
                // same config with different output paths reuses one probe
                // record. The value is embedded in the token (no next-arg
                // to consume).
            } else if self
                .sources
                .iter()
                .any(|s| s.to_str() == Some(arg.as_str()))
            {
                // source file — per-TU
            } else {
                out.push(arg.clone());
            }
        }
        out
    }

    /// Terminal formatting must not enter the resolved cc1 tokens either.
    /// Keep the original argv for preprocessing and compiler execution.
    fn probe_args(&self) -> Vec<String> {
        let mut args = self.split_diagnostic_format_args().0;
        let mut skip = false;
        args.retain(|arg| {
            if skip { skip = false; return false; }
            if arg == "-aux-info" { skip = true; }
            !cc_report_flag(arg)
        });
        if cc_wp_dependency_spec(self).is_some() {
            args.retain(|arg| cc_wp_dependency_option(arg).is_none());
        }
        args
    }

    /// The formatting flags [`Self::probe_args`] leaves out. They shape the
    /// diagnostics a hit replays.
    fn diagnostic_format_args(&self) -> Vec<String> {
        self.split_diagnostic_format_args().1
    }

    /// The formatting flags in argv order, as stored with an entry's
    /// diagnostics: a warning-free entry serves any formatting, one with
    /// diagnostics only the formatting it was printed under.
    pub(crate) fn diagnostic_format_fingerprint(&self) -> String {
        self.diagnostic_format_args().join("\x1f")
    }

    fn split_diagnostic_format_args(&self) -> (Vec<String>, Vec<String>) {
        let mut kept = Vec::new();
        let mut formatting = Vec::new();
        let mut rest = self.rest.as_slice();
        while let Some((arg, tail)) = rest.split_first() {
            if arg == "-Xclang"
                && let Some((inner, remaining)) = tail.split_first()
            {
                let out = if cc_terminal_diagnostic_flag(inner) { &mut formatting } else { &mut kept };
                out.extend([arg.clone(), inner.clone()]);
                rest = remaining;
                continue;
            }
            let consumed =
                parse_cc_arg_at(rest, 0, self.family.dialect()).map_or(1, |parsed| parsed.consumed);
            let (args, remaining) = rest.split_at(consumed);
            let out = if cc_terminal_diagnostic_flag(arg) { &mut formatting } else { &mut kept };
            out.extend_from_slice(args);
            rest = remaining;
        }
        (kept, formatting)
    }
}

/// `Some(true)` for a switch that turns diagnostic color on. The compiler's
/// stderr is a pipe under kache, so `auto` is off.
fn diagnostic_color_switch(arg: &str) -> Option<bool> {
    match arg {
        "-fcolor-diagnostics" | "-fdiagnostics-color" | "-fdiagnostics-color=always" => Some(true),
        "-fno-color-diagnostics"
        | "-fno-diagnostics-color"
        | "-fdiagnostics-color=never"
        | "-fdiagnostics-color=auto" => Some(false),
        _ => None,
    }
}

/// A formatting fingerprint without its color switches, and whether color
/// ends up on.
fn without_color_switches(fingerprint: &str) -> (Vec<&str>, bool) {
    let mut rest = Vec::new();
    let mut color = false;
    let mut args = fingerprint.split('\x1f').filter(|arg| !arg.is_empty()).peekable();
    while let Some(arg) = args.next() {
        let switch = if arg == "-Xclang" {
            match args.peek().and_then(|next| diagnostic_color_switch(next)) {
                Some(on) => {
                    args.next();
                    Some(on)
                }
                None => None,
            }
        } else {
            diagnostic_color_switch(arg)
        };
        match switch {
            Some(on) => color = on,
            None => rest.push(arg),
        }
    }
    (rest, color)
}

/// Whether diagnostics stored under the `stored` formatting fingerprint can
/// be replayed as they are for an invocation whose fingerprint is `current`:
/// the two differ at most in spellings of the same color setting. Colored
/// text does not serve a colorless invocation. Removing escapes from it
/// would also remove the escapes a source wrote into a message, which the
/// compiler prints as they are under either setting.
pub(crate) fn diagnostic_replay(stored: &str, current: &str) -> bool {
    stored == current || {
        let (stored_rest, stored_color) = without_color_switches(stored);
        let (current_rest, current_color) = without_color_switches(current);
        stored_rest == current_rest && stored_color == current_color
    }
}

fn cc_terminal_diagnostic_flag(arg: &str) -> bool {
    matches!(
        arg,
        "-fcolor-diagnostics" | "-fno-color-diagnostics" | "-fansi-escape-codes"
    ) || arg.starts_with("-fdiagnostics-")
        || arg.starts_with("-fno-diagnostics-")
}

pub(crate) fn output_path_requires_compiler_semantics(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => !meta.file_type().is_file() || !regular_output_is_replaceable(path, &meta),
        Err(err) => err.kind() != std::io::ErrorKind::NotFound,
    }
}

fn regular_output_is_replaceable(path: &Path, meta: &std::fs::Metadata) -> bool {
    regular_output_is_independent(path, meta) && regular_output_is_owner_writable(meta)
}

fn regular_output_is_owner_writable(meta: &std::fs::Metadata) -> bool {
    if meta.permissions().readonly() {
        return false;
    }

    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    // SAFETY: geteuid has no arguments, pointers, or preconditions.
    let current_uid = unsafe { libc::geteuid() };
    meta.uid() == current_uid && meta.permissions().mode() & 0o200 != 0
}

fn regular_output_is_independent(_path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    meta.nlink() == 1
}

fn parse_cc_arg_at(args: &[String], idx: usize, dialect: Dialect) -> Option<ParsedCcArg> {
    let arg = args.get(idx)?;
    CC_ARG_SPECS
        .iter()
        .find_map(|spec| parse_cc_arg_with_spec(spec, args, idx, arg, dialect))
}

fn parse_cc_arg_with_spec(
    spec: &'static CcArgSpec,
    args: &[String],
    idx: usize,
    arg: &str,
    dialect: Dialect,
) -> Option<ParsedCcArg> {
    // Skip rows that are restricted to a different dialect (mirrors the
    // classifier's dialect filter in `flags::classify_against`).
    if let Some(d) = spec.dialect
        && d != dialect
    {
        return None;
    }
    match spec.value_form {
        CcArgValueForm::Flag => cc_arg_spec_matches(spec, arg).then_some(ParsedCcArg {
            spec,
            value: None,
            consumed: 1,
        }),
        CcArgValueForm::Separated => cc_arg_spec_matches(spec, arg).then(|| ParsedCcArg {
            spec,
            value: args.get(idx + 1).cloned(),
            consumed: if args.get(idx + 1).is_some() { 2 } else { 1 },
        }),
        CcArgValueForm::Concatenated { prefix } => {
            arg.strip_prefix(prefix).map(|value| ParsedCcArg {
                spec,
                value: Some(value.to_string()),
                consumed: 1,
            })
        }
        CcArgValueForm::CanBeSeparated { prefix } => {
            if arg == prefix {
                Some(ParsedCcArg {
                    spec,
                    value: args.get(idx + 1).cloned(),
                    consumed: if args.get(idx + 1).is_some() { 2 } else { 1 },
                })
            } else {
                arg.strip_prefix(prefix)
                    .filter(|value| !value.is_empty())
                    .map(|value| ParsedCcArg {
                        spec,
                        value: Some(value.to_string()),
                        consumed: 1,
                    })
            }
        }
    }
}

fn cc_arg_spec_matches(spec: &CcArgSpec, arg: &str) -> bool {
    match spec.matcher {
        Matcher::Exact(s) => arg == s,
        Matcher::Prefix(s) => arg.starts_with(s),
        Matcher::Regex(pat) => Regex::new(&format!("^(?:{pat})$"))
            .map(|re| re.is_match(arg))
            .unwrap_or(false),
    }
}

fn apply_cc_arg(parsed: &mut CcArgs, depinfo: &mut Option<DepInfoSpec>, arg: &ParsedCcArg) {
    match arg.spec.action {
        CcArgAction::SetMode(mode) => parsed.mode = mode,
        CcArgAction::SetOutput => {
            if let Some(value) = &arg.value {
                parsed.output = Some(PathBuf::from(value));
            }
        }
        CcArgAction::SetPic => parsed.pic = true,
        CcArgAction::SetDebugLevel(level) => parsed.debug_level = Some(level),
        CcArgAction::SetOptimization(level) => parsed.optimization = Some(level),
        CcArgAction::SetStd => {
            if let Some(value) = &arg.value {
                parsed.std = Some(value.clone());
            }
        }
        CcArgAction::DepIncludeSystem(include_system) => {
            let d = depinfo.get_or_insert_with(DepInfoSpec::default);
            d.emit = true;
            d.include_system = include_system;
        }
        CcArgAction::DepPhonyTargets => {
            let d = depinfo.get_or_insert_with(DepInfoSpec::default);
            d.phony_targets = true;
        }
        CcArgAction::DepMissingGenerated => {
            let d = depinfo.get_or_insert_with(DepInfoSpec::default);
            d.missing_generated = true;
        }
        CcArgAction::DepOutput => {
            if let Some(value) = &arg.value {
                let d = depinfo.get_or_insert_with(DepInfoSpec::default);
                d.output = Some(PathBuf::from(value));
            }
        }
        CcArgAction::DepTarget => {
            if let Some(value) = &arg.value {
                let d = depinfo.get_or_insert_with(DepInfoSpec::default);
                let quoted = matches!(arg.spec.matcher, Matcher::Exact("-MQ"));
                d.targets.push((value.clone(), quoted));
            }
        }
        CcArgAction::LanguageOverride => {
            if let Some(value) = &arg.value {
                parsed.language_override = Some(value.clone());
            }
        }
        CcArgAction::IncludePch => {
            if let Some(value) = &arg.value {
                parsed.include_pch = Some(PathBuf::from(value));
            }
        }
        CcArgAction::SerializeDiagnostics => {
            if let Some(value) = &arg.value {
                parsed.serialize_diagnostics = Some(PathBuf::from(value));
            }
        }
        CcArgAction::Include => {
            if let Some(value) = &arg.value {
                parsed.includes.push(PathBuf::from(value));
            }
        }
        CcArgAction::Define => {
            if let Some(value) = &arg.value {
                parsed.defines.push(parse_define(value));
            }
        }
        CcArgAction::SystemInclude | CcArgAction::AuxInfo => {}
    }
}

/// Cache key schema version for C-family compiles. Bump when the key
/// composition or restored artifact semantics change in a way that
/// could collide with old entries.
///
/// v4: `.pp` dependency sidecars are restored as dep-info and C/C++
/// dep-info path rewriting uses the common source/object root. Older
/// entries may contain machine-local source paths in `.pp` blobs.
///
/// v5: dep-info blobs use an explicit kache sentinel instead of `./`
/// for stored project-root paths. The old marker collided with ordinary
/// make depfile parent paths such as `../foo.h` during restore.
///
/// v6: C/C++ object compiles now inject prefix maps for the common
/// source/build root, not just the compiler CWD, and the preprocessor
/// stdout is normalized with the same maps before hashing. Older
/// entries may embed clone-local paths in `__FILE__`, debug info, or
/// preprocessor-expanded string literals.
///
/// The cc recipe now shares [`crate::cache_key::CACHE_KEY_VERSION`] with
/// the rustc recipe (one number to bump). The `cc_key_version:` label
/// plus disjoint fields keep cc and rustc entries from ever colliding;
/// the shared number just means one bump invalidates both recipes.
// Prefix-map targets are ABSOLUTE, profiler-resolvable spellings rather than
// angle-bracket sentinels (kunobi-ninja/kache#485). Absolute targets stop a
// DWARF consumer composing a relative target onto `DW_AT_comp_dir`, and the
// `<CC_BUILD>` cwd target uses `/proc/self/cwd` on Linux so samply / gdb / perf
// launched from the build directory resolve C/C++ sources through the kernel
// with no configuration (the Bazel trick). The other roots are ancestors of —
// or unrelated to — the cwd, so they get distinct `/kache/*` roots (a debugger
// `source-map`s them, or a future `/proc/self/cwd/..` scheme resolves the
// common-root case; see #485). These strings are also folded into the cc cache
// key, so changing them is covered by the `CACHE_KEY_VERSION` bump.
const CC_ROOT_SENTINEL: &str = "/kache/cc-root";
#[cfg(target_os = "linux")]
const CC_BUILD_SENTINEL: &str = "/proc/self/cwd";
#[cfg(not(target_os = "linux"))]
const CC_BUILD_SENTINEL: &str = "/kache/cc-build";
const CC_SOURCE_SENTINEL: &str = "/kache/cc-source";
/// Stable store-side name for a C/C++ dependency artifact.
///
/// `-MF` accepts arbitrary filenames. Inferring the artifact kind from that
/// filename loses the parser's knowledge for compound or extensionless names
/// such as OpenSSL's `a_bitstr.d.tmp` (kunobi-ninja/kache#655). The restore
/// target always comes from the current parsed invocation, so the cache entry
/// can use one semantic name ending in `.d`. Besides making every future `-MF`
/// spelling restoreable, this keeps pre-fix `.d.tmp` entries untrusted: their
/// old raw name still fails the coverage gate once, is evicted, and is replaced
/// with a normalized entry without a store-wide cache-version bump.
pub(crate) const CC_DEPINFO_STORE_NAME: &str = "__kache_cc_depinfo.d";
pub(crate) const CC_OBJECT_STORE_NAME: &str = "__kache_cc_object.o";
/// Target for a user-declared `KACHE_BASE_DIR` (ccache `CCACHE_BASEDIR`
/// analog). Shares the spelling with the rustc `<BASE_DIR>` target (same
/// concept, compiler-independent) and stays distinct from the derived roots so
/// an explicit base dir can't collide with a `/kache/cc-root` subtree.
const CC_BASE_SENTINEL: &str = "/kache/base-dir";
/// Sentinel for the Apple SDK root (issue #78). The resolved `cc -###`
/// tokens embed the SDK path (`-isysroot /…/MacOSX14.2.sdk`,
/// `-internal-isystem /…/usr/include`), which differs across Xcode and
/// Command Line Tools installs and between machines. Stripping it to a
/// sentinel lets two builds with the same SDK *contents* at different
/// paths share a key — differing SDK *contents* still diverge via
/// `compiler_version` and the preprocessor expansion, so this only ever
/// merges keys that would otherwise miss, never miscaches. A distinct
/// sentinel so the SDK can't collide with a project root.
const CC_SDKROOT_SENTINEL: &str = "/kache/sdkroot";
/// A build script's `OUT_DIR` and the Cargo target directory above it. Every
/// build directory spells these differently, and a `cc`-crate compile names
/// them in `-I` for generated headers, so without the map the read-set memo
/// is private to one build directory and peers cannot coalesce.
const CC_OUT_DIR_SENTINEL: &str = "/kache/cc-out-dir";
const CC_TARGET_SENTINEL: &str = "/kache/cc-target";
/// Another crate's build-script output directory, named by crate rather
/// than by unit: `-I<target>/debug/build/libz-sys-<hash>/out/include`
/// reaches the memo as `/kache/cc-dep-out/libz-sys/include`. Cargo's
/// metadata hash follows the feature set of the whole build, so `cargo
/// test` and `cargo check` give the same dependency different hashes; with
/// the hash in the name, the two jobs could never share a memo.
const CC_DEP_OUT_DIR_SENTINEL: &str = "/kache/cc-dep-out";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CcPrefixMap {
    from: String,
    to: String,
}

/// Resolve the target architecture for the cache key: an explicit
/// `-arch X` flag if present, else the host arch. (Multi-`-arch` is
/// refused upstream, so at most one value is found here.)
fn cc_target_arch(parsed: &CcArgs) -> String {
    let slices = parsed.arch_slices();
    if slices.is_empty() {
        std::env::consts::ARCH.to_string()
    } else {
        slices.join("+")
    }
}

fn cc_report_flag(arg: &str) -> bool {
    matches!(arg, "-fstack-usage" | "-fsave-optimization-record" | "-aux-info" | "-Wa,-adhln")
        || arg.starts_with("-fopt-info")
        || arg.starts_with("-fcallgraph-info")
        || arg.starts_with("-Wa,-adhln=")
}

fn cc_wp_dependency_option(arg: &str) -> Option<(bool, Option<&str>)> {
    let option = arg.strip_prefix("-Wp,")?;
    let (mode, output) = match option.split_once(',') {
        Some((mode, output)) if !output.is_empty() && !output.contains(',') && output != "-" => {
            (mode, Some(output))
        }
        Some(_) => return None,
        None => (option, None),
    };
    match mode {
        "-MD" => Some((true, output)),
        "-MMD" => Some((false, output)),
        _ => None,
    }
}

fn cc_wp_dependency_spec(parsed: &CcArgs) -> Option<DepInfoSpec> {
    if parsed.family.dialect() != Dialect::Gnu || parsed.mode != CompileMode::Compile {
        return None;
    }
    let family = parsed.probed_family?;
    let mut wp = None;
    let mut idx = 0;
    while idx < parsed.rest.len() {
        let token = &parsed.rest[idx];
        if matches!(token.as_str(), "-Xclang" | "-Xpreprocessor") {
            return None;
        }
        if token.starts_with("-Wp,") {
            if wp.is_some() {
                return None;
            }
            wp = Some(cc_wp_dependency_option(token)?);
        } else if let Some(arg) = parse_cc_arg_at(&parsed.rest, idx, parsed.family.dialect()) {
            if matches!(
                arg.spec.action,
                CcArgAction::DepIncludeSystem(_)
                    | CcArgAction::DepOutput
                    | CcArgAction::DepTarget
                    | CcArgAction::DepPhonyTargets
                    | CcArgAction::DepMissingGenerated
            ) {
                return None;
            }
            idx += arg.consumed;
            continue;
        }
        idx += if takes_separate_value(&parsed.rest, idx, parsed.family.dialect()) {
            2
        } else {
            1
        };
    }
    let (include_system, output) = wp?;
    if family == crate::probe::ProbedFamily::Gnu && output.is_none() {
        return None;
    }
    let targets = if family == crate::probe::ProbedFamily::Gnu {
        vec![(
            format!("{}.o", parsed.sources.first()?.file_stem()?.to_str()?),
            false,
        )]
    } else {
        Vec::new()
    };
    Some(DepInfoSpec {
        emit: true,
        include_system,
        output: output.map(PathBuf::from),
        targets,
        ..DepInfoSpec::default()
    })
}

/// Build the argv for a preprocess-only run, dialect-dependent.
///
/// **Gnu dialect** — the original args with mode/output/dep-info flags
/// stripped and `-E` forced:
/// - `-c` / `-S` removed — we force `-E` (preprocess only).
/// - `-o <arg>` removed — preprocessed output must go to stdout, not
///   a file (we capture and hash it).
/// - `-MMD` / `-MD` / `-MF` / `-MT` / `-MQ` / `-MP` / `-MG` removed —
///   dep-info generation is irrelevant to preprocessor *content* and
///   `-MF` would redirect output.
/// - `-E` prepended, and a caller's `-P` dropped. The line markers
///   (`# 12 "dir/header.h"`) are what place each line, and line positions
///   reach the object's debug line table and the diagnostics a hit
///   replays. [`split_line_markers`] keys them apart from the text so
///   their paths do not bind the key to a checkout.
///
/// **Cl dialect** — `/E` preprocesses to stdout with the same markers
/// (gnu `-E` writes nothing to stdout under clang-cl).
/// Only compile-mode (`-c`/`-S`) and output (`-o`, `-Fo`/`/Fo`) flags are
/// stripped; the `-M*` spellings are CRT-selection codegen in this dialect
/// (they affect `_MT`/`_DLL` defines), so they are KEPT in the expansion.
fn build_preprocess_args(parsed: &CcArgs) -> Vec<String> {
    match parsed.family.dialect() {
        Dialect::Gnu => {
            let mut out = vec!["-E".to_string()];
            let mut iter = parsed.rest.iter();
            let wp_dependency = cc_wp_dependency_spec(parsed).is_some();
            while let Some(arg) = iter.next() {
                if arg == "-aux-info" { iter.next(); continue; }
                if cc_report_flag(arg) { continue; }
                if wp_dependency && cc_wp_dependency_option(arg).is_some() {
                    continue;
                }
                match arg.as_str() {
                    "-c" | "-S" | "-P" | "-emit-llvm" => {}
                    "-o" | "-MF" | "-MT" | "-MQ" | "--serialize-diagnostics"
                    | "-serialize-diagnostics" => {
                        iter.next(); // also drop the flag's value
                    }
                    "-MMD" | "-MD" | "-MP" | "-MG" => {}
                    "-fdiagnostics-format=json-file" => {}
                    _ => out.push(arg.clone()),
                }
            }
            out
        }
        Dialect::Cl => {
            // `/E` = preprocess to stdout with line markers. Drop
            // compile-mode + output flags; keep preprocessor-affecting
            // flags so the hash reflects them.
            let mut out = vec!["/E".to_string()];
            let mut iter = parsed.rest.iter();
            while let Some(arg) = iter.next() {
                match arg.as_str() {
                    "-c" | "-S" => {}
                    "-o" => {
                        iter.next();
                    }
                    // Attached output form (`-Fofoo.obj` / `/Fofoo.obj`).
                    // clang-cl build systems use the attached form
                    // exclusively; a space-separated `/Fo obj` would leave
                    // a stray token, but such an invocation refuses before
                    // this point (output flags are unmodeled until Layer 2).
                    _ if arg.starts_with("-Fo") || arg.starts_with("/Fo") => {}
                    _ => out.push(arg.clone()),
                }
            }
            out
        }
    }
}

/// Hash the preprocessor expansion of the translation unit.
///
/// Runs `<cc> -E …` (gnu) or `<cc> /E …` (clang-cl) — see
/// [`build_preprocess_args`] — with `SOURCE_DATE_EPOCH` pinned so the
/// `__DATE__` / `__TIME__` macros expand deterministically (without
/// this the hash would change every second → ~0% hit rate; support is probed
/// before caching). The expansion includes every `#include`d
/// header transitively, so any header change invalidates the key
/// automatically — no separate dependency tracking needed. Bails (→
/// passthrough) if the preprocessor yields empty stdout.
const CC_PREPROCESS_MEMO_TARGET: &str = "__kache_preprocess_memo";

/// The depfile a compile is asked to write inside `dir`, with the spelling the
/// compiler receives. `None` when that spelling is not UTF-8, which leaves the
/// invocation without dependency capture.
fn capture_depfile(dir: &Path) -> Option<(PathBuf, String)> {
    let path = dir.join("inputs.d");
    let arg = path.to_str()?.to_string();
    Some((path, arg))
}

fn add_preprocess_dep_capture(
    parsed: &CcArgs,
    pp_args: Vec<String>,
    path: String,
) -> Vec<String> {
    let extra = match parsed.family.dialect() {
        Dialect::Gnu => vec![
            "-MD".to_string(),
            "-MF".to_string(),
            path,
            "-MT".to_string(),
            CC_PREPROCESS_MEMO_TARGET.to_string(),
        ],
        Dialect::Cl => vec![
            "-Xclang".to_string(),
            "-dependency-file".to_string(),
            "-Xclang".to_string(),
            path,
            "-Xclang".to_string(),
            "-MT".to_string(),
            "-Xclang".to_string(),
            CC_PREPROCESS_MEMO_TARGET.to_string(),
            // clang-cl excludes system headers from dependency output unless
            // this cc1 option is present. A partial header set is never safe
            // for direct-mode reuse.
            "-Xclang".to_string(),
            "-sys-header-deps".to_string(),
        ],
    };
    compose_cc_args(&pp_args, extra)
}

/// Parse the Make dependency rule emitted into the probe's private file.
/// Continuations, escaped spaces/hash/backslashes, and Make's `$$` spelling
/// are decoded. The target may remain user-selected under clang-cl when its
/// earlier forwarded `-MT` wins; only the private output path is authoritative.
/// Any unfamiliar/malformed shape disables memoization.
fn parse_preprocess_dependencies(raw: &str, cwd: &Path) -> Result<Vec<PathBuf>> {
    let bytes = raw.as_bytes();
    let mut logical = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && bytes.get(index + 1) == Some(&b'\n') {
            logical.push(b' ');
            index = index.saturating_add(2);
        } else if bytes[index] == b'\\'
            && bytes.get(index + 1) == Some(&b'\r')
            && bytes.get(index + 2) == Some(&b'\n')
        {
            logical.push(b' ');
            index = index.saturating_add(3);
        } else {
            logical.push(bytes[index]);
            index = index.saturating_add(1);
        }
    }
    let logical = std::str::from_utf8(&logical).context("cc dependency file is not UTF-8")?;
    let (rule, separator) = logical
        .lines()
        .find_map(|line| {
            line.find(": ")
                .or_else(|| line.find(":\t"))
                .map(|separator| (line, separator))
        })
        .context("cc dependency file has no Make rule")?;
    let dependencies = &rule[separator + 1..];

    let mut paths = Vec::new();
    let mut word = String::new();
    let mut chars = dependencies.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\\' => match chars.peek().copied() {
                Some(next) if matches!(next, ' ' | '\t' | '#' | '\\' | ':') => {
                    word.push(next);
                    chars.next();
                }
                Some(_) => word.push('\\'),
                None => anyhow::bail!("cc dependency rule ends in an escape"),
            },
            '$' if chars.peek() == Some(&'$') => {
                word.push('$');
                chars.next();
            }
            '#' => break,
            whitespace if whitespace.is_whitespace() => {
                if !word.is_empty() {
                    paths.push(PathBuf::from(std::mem::take(&mut word)));
                }
            }
            other => word.push(other),
        }
    }
    if !word.is_empty() {
        paths.push(PathBuf::from(word));
    }
    if paths.is_empty() {
        anyhow::bail!("cc dependency rule contains no inputs");
    }
    for path in &mut paths {
        let text = path.to_string_lossy();
        let bytes = text.as_bytes();
        let windows_absolute = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\');
        if path.is_relative() && !windows_absolute && !text.starts_with("\\\\") {
            *path = cwd.join(&*path);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[derive(Debug)]
struct PreprocessHash {
    hash: String,
    fingerprints: Option<Vec<crate::cache_key::CcPreprocessMemoInput>>,
    /// The expansion spells out a checkout root, so its object does too and
    /// the key is bound to this checkout (see [`hash_cc_expansion`]).
    path_bound: bool,
}

/// The `-E` key probe's argv: the preprocess args plus the same
/// `-ffile-prefix-map` rules the real compile gets.
///
/// With the maps applied, `__FILE__` expands to its sentinel exactly as it
/// will in the object. A checkout root still spelled out in the expansion is
/// then text the compiler copies into the object verbatim, such as a `-D`
/// value or a string in a generated header (kunobi-ninja/kache#1004).
fn key_preprocess_args(parsed: &CcArgs, prefix_maps: &[CcPrefixMap]) -> Vec<String> {
    compose_cc_args(
        &build_preprocess_args(parsed),
        file_prefix_map_args(prefix_maps),
    )
}

#[derive(Debug, PartialEq, Eq)]
struct CcExpansionHash {
    hash: String,
    path_bound: bool,
}

/// Hash a preprocessor expansion for the cc key.
///
/// The probe ran with the compile's prefix maps (see [`key_preprocess_args`]),
/// so any root left in the expansion is one `-ffile-prefix-map` does not
/// rewrite, and the object will carry it. Mapping it to a sentinel would give
/// every checkout the same key for objects that differ. Such an expansion is
/// hashed as it is instead: the key hits in this checkout and misses in every
/// other one. A portable expansion hashes its mapped form, as before.
///
/// A bound hash also folds every mapped root, not only the ones the expansion
/// spells out. `execute` skips the object scan for a bound key, so a root that
/// reaches the object some other way must still separate two checkouts even
/// when the expansion only names a root they share, such as a base dir.
fn hash_cc_expansion(raw: Vec<u8>, prefix_maps: &[CcPrefixMap]) -> CcExpansionHash {
    let mapped = apply_cc_prefix_maps_to_bytes(raw.clone(), prefix_maps);
    if mapped == raw {
        return CcExpansionHash {
            hash: blake3::hash(&mapped).to_hex().to_string(),
            path_bound: false,
        };
    }
    let mut roots: Vec<&str> = prefix_maps
        .iter()
        .map(|map| map.from.as_str())
        .filter(|from| !from.is_empty())
        .collect();
    roots.sort_unstable();
    roots.dedup();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kache.cc.path-bound-expansion.v1\0");
    for root in roots {
        hasher.update(root.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(b"\n");
    hasher.update(&raw);
    CcExpansionHash {
        hash: hasher.finalize().to_hex().to_string(),
        path_bound: true,
    }
}

/// Split a preprocessor expansion into its text and its line layout.
///
/// The text is every line that is not a line marker; it is what the
/// assembler scan and [`hash_cc_expansion`] see. The layout keeps each marker,
/// prefix-mapped like the compile's debug paths, and one newline for each text
/// line, so it fixes where every line sits. `None` when the expansion carries
/// no marker at all.
fn split_line_markers(expansion: &[u8], prefix_maps: &[CcPrefixMap]) -> (Vec<u8>, Option<Vec<u8>>) {
    let mut text = Vec::with_capacity(expansion.len());
    let mut layout = Vec::new();
    let mut markers = false;
    for line in expansion.split_inclusive(|&byte| byte == b'\n') {
        if is_line_marker(line) {
            markers = true;
            layout.extend_from_slice(&apply_cc_prefix_maps_to_bytes(line.to_vec(), prefix_maps));
        } else {
            text.extend_from_slice(line);
            layout.push(b'\n');
        }
    }
    (text, markers.then_some(layout))
}

/// `# 12 "file" 1 3` (GCC, Clang, clang-cl `/E`) or `#line 12 "file"`.
fn is_line_marker(line: &[u8]) -> bool {
    line.strip_prefix(b"# ")
        .or_else(|| line.strip_prefix(b"#line "))
        .is_some_and(|rest| rest.first().is_some_and(u8::is_ascii_digit))
}

fn cc_check_listing_inputs(parsed: &CcArgs, expansion: &[u8], inputs: &[PathBuf]) -> Result<()> {
    if !parsed.lists_assembler_source() {
        return Ok(());
    }
    let literals = cc_string_literals(expansion);
    anyhow::ensure!(
        [expansion, literals.text.as_slice()].into_iter().all(|text| {
            cc_directive_operands(text, b".file").next().is_none()
        }),
        "cc: assembler listing with inline .file — not yet"
    );
    let mut source = None;
    for line in expansion.split(|&byte| byte == b'\n') {
        if is_line_marker(line) {
            let decoded = cc_string_literals(line).text;
            let name = std::str::from_utf8(decoded.strip_prefix(b"\n").context("listing source marker has no file")?)?;
            source = Some(parsed.invocation_env.resolve(Path::new(name)).context("resolving listing source marker")?);
        } else if !line.trim_ascii().is_empty() {
            let source = source.as_ref().context("listing text has no source marker")?;
            anyhow::ensure!(
                inputs.contains(source),
                "cc: assembler listing source outside the read set: {} — not yet", source.display()
            );
        }
    }
    Ok(())
}

/// Fold an expansion's line layout into its text hash. Two expansions whose
/// text matches but whose code sits on other lines compile to objects with
/// different line tables and replay diagnostics at other locations.
fn fold_line_layout(hash: String, layout: Option<&[u8]>) -> String {
    let Some(layout) = layout else {
        return hash;
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kache.cc.expansion-layout.v1\0");
    hasher.update(hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(layout);
    hasher.finalize().to_hex().to_string()
}

/// Assembler directives that read another file at assembly time.
const CC_ASSEMBLER_FILE_DIRECTIVES: [&str; 2] = [".incbin", ".include"];

/// Assembler facilities that substitute into their bodies, and so can build a
/// file directive whose name never appears in the expansion (`.\op "\file"`
/// invoked as `emit incbin, payload.bin`, or `.inc\()bin` inside `.rept`).
const CC_ASSEMBLER_MACRO_DIRECTIVES: [&str; 4] = [".macro", ".irp", ".irpc", ".rept"];

/// Why the assembler may read a file the key cannot see, if it may.
///
/// `.incbin` and assembler `.include` run after preprocessing, so no depfile
/// lists what they read: a changed file would be served from the old object
/// (kunobi-ninja/kache#1015). They reach the expansion from `.S` and `.s`
/// sources and from inline `asm` strings in C, where the compiler joins
/// adjacent literals and decodes escapes only after preprocessing. So the
/// expansion is checked as written and as the compiler reads its strings, and
/// an `asm` whose text is computed rather than written is refused outright.
fn cc_assembler_hidden_input(expansion: &[u8]) -> Option<&'static str> {
    if let Some(found) = cc_assembler_text_hidden_input(expansion) {
        return Some(found);
    }
    let literals = cc_string_literals(expansion);
    if literals.computed_asm {
        return Some("asm((...))");
    }
    if literals.named_escape {
        return Some(r"\N{...}");
    }
    cc_assembler_text_hidden_input(&literals.text)
}

/// The first construct in assembler text that may read an unseen file.
///
/// Directives are matched ignoring case, as the assembler does, and must end
/// at a word boundary. A file directive counts when the next non-blank byte
/// opens its operand: a quote, or a backslash for an escaped quote in a C
/// string or a macro argument. That keeps C++ member access such as
/// `opts.include(` out. A macro facility counts when a blank follows it;
/// `.altmacro` counts anywhere, and the `\()` separator wherever a body is
/// closed, which also catches an opening spelled without a blank (`.rept(1)`).
fn cc_assembler_text_hidden_input(text: &[u8]) -> Option<&'static str> {
    let file = CC_ASSEMBLER_FILE_DIRECTIVES.into_iter().find(|directive| {
        cc_directive_operands(text, directive.as_bytes()).any(|rest| {
            rest.iter()
                .find(|byte| !matches!(byte, b' ' | b'\t'))
                .is_some_and(|byte| matches!(byte, b'"' | b'\\'))
        })
    });
    file.or_else(|| {
        CC_ASSEMBLER_MACRO_DIRECTIVES.into_iter().find(|directive| {
            cc_directive_operands(text, directive.as_bytes()).any(|rest| {
                rest.first()
                    .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            })
        })
    })
    .or_else(|| {
        cc_directive_operands(text, b".altmacro")
            .next()
            .map(|_| ".altmacro")
    })
    .or_else(|| {
        // `\()` acts only inside a macro, `.irp`, `.irpc` or `.rept` body,
        // and every body is closed in the same text. Without a closing
        // directive it is inert, as in libstdc++'s `<regex>`, which spells
        // `".[\\()*+?{|^$"`.
        let closes_a_body = [b".endm".as_slice(), b".endr"]
            .into_iter()
            .any(|end| cc_directive_operands(text, end).next().is_some());
        (closes_a_body && text.windows(3).any(|window| window == br"\()")).then_some(r"\()")
    })
}

/// String literals in a C or C++ expansion, read the way the compiler reads
/// them after preprocessing.
#[derive(Debug, Default, PartialEq, Eq)]
struct CcStringLiterals {
    /// Each run of adjacent literals joined, escapes decoded, one run per line.
    text: Vec<u8>,
    /// An `asm` whose text is an expression rather than a string literal.
    computed_asm: bool,
    /// A C++23 named escape (`\N{LATIN SMALL LETTER B}`). It can spell any
    /// character, and decoding it would need the Unicode name table.
    named_escape: bool,
}

/// Literal prefixes that open a raw string (`R"delim(...)delim"`).
const CC_RAW_STRING_PREFIXES: [&[u8]; 5] = [b"R", b"u8R", b"uR", b"UR", b"LR"];

/// Encoding prefixes of ordinary string and character literals.
const CC_LITERAL_PREFIXES: [&[u8]; 4] = [b"u8", b"u", b"U", b"L"];

/// Read the string literals in `src`, skipping character literals and numbers
/// so a quote inside them cannot throw the reading out of step. Raw strings
/// are taken as written. Assembly sources are not C; for them the plain text
/// check is the one that counts, and a misreading here can only refuse more.
fn cc_string_literals(src: &[u8]) -> CcStringLiterals {
    let mut literals = CcStringLiterals::default();
    let mut joining = false;
    let mut i = 0;
    while let Some(&byte) = src.get(i) {
        let next = if byte.is_ascii_whitespace() {
            i + 1
        } else if byte == b'/' && matches!(src.get(i + 1), Some(b'*' | b'/')) {
            // Comments only survive a `-C` probe. The compiler reads one as a
            // blank, so literals on either side still join.
            cc_skip_comment(src, i)
        } else {
            cc_read_token(src, i, &mut literals, &mut joining)
        };
        debug_assert!(next > i, "string literal reader must advance");
        i = next;
    }
    literals
}

/// Read the token starting at `i`, which is not a blank or a comment, and
/// return the index after it. String literals are decoded into `literals`;
/// `joining` tracks whether the previous token was one.
fn cc_read_token(
    src: &[u8],
    i: usize,
    literals: &mut CcStringLiterals,
    joining: &mut bool,
) -> usize {
    let word_end = cc_word_end(src, i);
    let word = &src[i..word_end];
    let quote = src.get(word_end).copied();
    let plain = word.is_empty() || CC_LITERAL_PREFIXES.contains(&word);
    let raw_paren = (quote == Some(b'"') && CC_RAW_STRING_PREFIXES.contains(&word))
        .then(|| cc_raw_delimiter(src, word_end))
        .flatten();
    if raw_paren.is_some() || (quote == Some(b'"') && plain) {
        if !*joining {
            literals.text.push(b'\n');
        }
        *joining = true;
        return match raw_paren {
            Some(paren) => cc_read_raw_string(src, word_end, paren, &mut literals.text),
            None => cc_read_string(src, word_end, literals),
        };
    }
    *joining = false;
    if quote == Some(b'\'') && plain {
        return cc_skip_char_literal(src, word_end);
    }
    if word.is_empty() {
        return i + 1;
    }
    if matches!(word, b"asm" | b"__asm" | b"__asm__") && !cc_asm_text_is_literal(src, word_end) {
        literals.computed_asm = true;
    }
    word_end
}

/// End of the identifier or number starting at `start`, or `start` if neither
/// starts there. A number takes C++14 digit separators, whose quote would
/// otherwise read as a character literal.
fn cc_word_end(src: &[u8], start: usize) -> usize {
    let is_word = |byte: &u8| byte.is_ascii_alphanumeric() || *byte == b'_';
    if !src.get(start).is_some_and(is_word) {
        return start;
    }
    let number = src[start].is_ascii_digit();
    let mut i = start;
    while let Some(byte) = src.get(i) {
        let separator = number && *byte == b'\'' && src.get(i + 1).is_some_and(is_word);
        if !is_word(byte) && !separator {
            break;
        }
        let next = i + 1;
        debug_assert!(next > i, "word reader must advance");
        i = next;
    }
    i
}

/// Decode the string literal whose opening quote is at `open` into
/// `literals.text`, and return the index after it. A literal cut off by a
/// newline ends there.
fn cc_read_string(src: &[u8], open: usize, literals: &mut CcStringLiterals) -> usize {
    let mut i = open + 1;
    while let Some(&byte) = src.get(i) {
        let next = match byte {
            b'"' | b'\n' => return i + 1,
            b'\\' => cc_decode_escape(src, i + 1, literals),
            _ => {
                literals.text.push(byte);
                i + 1
            }
        };
        debug_assert!(next > i, "string reader must advance");
        i = next;
    }
    i
}

/// Decode the escape whose letter is at `at` into `literals.text`, returning
/// the index after it. Control escapes other than newline and tab become a
/// blank: the directive check only needs to know that they separate words. A
/// named escape is flagged rather than decoded.
fn cc_decode_escape(src: &[u8], at: usize, literals: &mut CcStringLiterals) -> usize {
    let out = &mut literals.text;
    let digits = |from: usize, max: usize, radix: u32| {
        let len = src[from..]
            .iter()
            .take(max)
            .take_while(|byte| char::from(**byte).is_digit(radix))
            .count();
        let value = src[from..from + len].iter().fold(0u32, |value, byte| {
            value
                .wrapping_mul(radix)
                .wrapping_add(char::from(*byte).to_digit(radix).unwrap_or(0))
        });
        (value, from + len)
    };
    let push_code_point = |out: &mut Vec<u8>, value: u32| {
        let mut utf8 = [0; 4];
        let decoded = char::from_u32(value).unwrap_or(' ');
        out.extend_from_slice(decoded.encode_utf8(&mut utf8).as_bytes());
    };
    let Some(&kind) = src.get(at) else {
        return at;
    };
    // C23 and C++23 delimited escapes: `\x{2e}`, `\o{56}`, `\u{62}`. One
    // without its `}` does not compile, so its object is never stored.
    if matches!(kind, b'x' | b'o' | b'u') && src.get(at + 1) == Some(&b'{') {
        let (value, end) = digits(at + 2, usize::MAX, if kind == b'o' { 8 } else { 16 });
        if kind == b'u' {
            push_code_point(out, value);
        } else {
            out.push(value as u8);
        }
        return end + usize::from(src.get(end) == Some(&b'}'));
    }
    match kind {
        b'x' => {
            let (value, end) = digits(at + 1, usize::MAX, 16);
            out.push(value as u8);
            end
        }
        b'0'..=b'7' => {
            let (value, end) = digits(at, 3, 8);
            out.push(value as u8);
            end
        }
        b'u' | b'U' => {
            let (value, end) = digits(at + 1, if kind == b'u' { 4 } else { 8 }, 16);
            push_code_point(out, value);
            end
        }
        b'n' => {
            out.push(b'\n');
            at + 1
        }
        b't' => {
            out.push(b'\t');
            at + 1
        }
        b'r' | b'f' | b'v' | b'a' | b'b' => {
            out.push(b' ');
            at + 1
        }
        b'N' if src.get(at + 1) == Some(&b'{') => {
            literals.named_escape = true;
            let close = src[at..]
                .iter()
                .position(|byte| matches!(byte, b'}' | b'"' | b'\n'));
            close.map_or(src.len(), |offset| {
                at + offset + usize::from(src[at + offset] == b'}')
            })
        }
        other => {
            out.push(other);
            at + 1
        }
    }
}

/// The offset of the `(` that ends a raw string's delimiter, if the quote at
/// `open` starts a valid one: at most 16 characters, none of them a blank,
/// backslash, parenthesis or quote. Otherwise the prefix is an identifier and
/// the quote opens an ordinary string, as the compiler would read it.
fn cc_raw_delimiter(src: &[u8], open: usize) -> Option<usize> {
    let body = src.get(open + 1..)?;
    let paren = body.iter().take(17).position(|byte| *byte == b'(')?;
    body[..paren]
        .iter()
        .all(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'\\' | b')' | b'"'))
        .then_some(paren)
}

/// Copy the raw string whose opening quote is at `open` and whose delimiter
/// ends at `paren` into `out`, returning the index after it. Its text is
/// taken as written, escapes included.
fn cc_read_raw_string(src: &[u8], open: usize, paren: usize, out: &mut Vec<u8>) -> usize {
    let body = &src[open + 1..];
    let mut close = vec![b')'];
    close.extend_from_slice(&body[..paren]);
    close.push(b'"');
    let text = &body[paren + 1..];
    let len = text
        .windows(close.len())
        .position(|window| window == close.as_slice())
        .unwrap_or(text.len());
    out.extend_from_slice(&text[..len]);
    open + 1 + paren + 1 + len + close.len()
}

/// The index after the character literal whose opening quote is at `open`.
fn cc_skip_char_literal(src: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while let Some(&byte) = src.get(i) {
        let next = match byte {
            b'\\' => i + 2,
            b'\'' | b'\n' => return i + 1,
            _ => i + 1,
        };
        debug_assert!(next > i, "character literal reader must advance");
        i = next;
    }
    i
}

/// The index after the `/* */` or `//` comment starting at `start`.
fn cc_skip_comment(src: &[u8], start: usize) -> usize {
    let close: &[u8] = if src.get(start + 1) == Some(&b'*') {
        b"*/"
    } else {
        b"\n"
    };
    let rest = start + 2;
    src[rest.min(src.len())..]
        .windows(close.len())
        .position(|window| window == close)
        .map_or(src.len(), |offset| rest + offset + close.len())
}

/// Whether the `asm` keyword ending at `after` takes a written string. Clang
/// accepts a constant expression in its place, which can build any text, so
/// anything else counts as computed. A word not followed by `(` is not an
/// `asm` statement.
fn cc_asm_text_is_literal(src: &[u8], after: usize) -> bool {
    let skip_blanks = |mut i: usize| loop {
        let next = if src.get(i).is_some_and(u8::is_ascii_whitespace) {
            i + 1
        } else if src.get(i) == Some(&b'/') && matches!(src.get(i + 1), Some(b'*' | b'/')) {
            cc_skip_comment(src, i)
        } else {
            return i;
        };
        debug_assert!(next > i, "asm reader must advance");
        i = next;
    };
    let mut i = skip_blanks(after);
    loop {
        let end = cc_word_end(src, i);
        match &src[i..end] {
            b"volatile" | b"__volatile__" | b"__volatile" | b"inline" | b"__inline__" | b"goto" => {
                i = skip_blanks(end)
            }
            b"" if src.get(i) == Some(&b'(') => break,
            _ => return true,
        }
    }
    let start = skip_blanks(i + 1);
    let end = cc_word_end(src, start);
    let word = &src[start..end];
    src.get(end) == Some(&b'"')
        && (word.is_empty()
            || CC_LITERAL_PREFIXES.contains(&word)
            || CC_RAW_STRING_PREFIXES.contains(&word))
}

/// The text after each whole-word, case-insensitive occurrence of `name`.
fn cc_directive_operands<'a>(
    expansion: &'a [u8],
    name: &'a [u8],
) -> impl Iterator<Item = &'a [u8]> + 'a {
    expansion
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'.')
        .filter_map(move |(start, _)| {
            let candidate = expansion.get(start..start + name.len())?;
            if !candidate.eq_ignore_ascii_case(name) {
                return None;
            }
            let rest = &expansion[start + name.len()..];
            let continues_word = rest
                .first()
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
            (!continues_word).then_some(rest)
        })
}

/// Whether `bytes` contain the raw spelling of any mapped root.
///
/// A plain substring search, unlike [`apply_cc_prefix_maps_to_bytes`]: object
/// files separate strings with NUL bytes, which the configured-root token rule
/// does not treat as a boundary, and a missed root here is a false hit.
fn bytes_embed_mapped_root(bytes: &[u8], prefix_maps: &[CcPrefixMap]) -> bool {
    prefix_maps
        .iter()
        .map(|map| map.from.as_bytes())
        .filter(|from| !from.is_empty())
        .any(|from| {
            bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == from[0])
                .any(|(start, _)| bytes[start..].starts_with(from))
        })
}

/// Whether the object at `path` embeds a raw mapped root.
fn cc_object_embeds_mapped_root(path: &Path, prefix_maps: &[CcPrefixMap]) -> std::io::Result<bool> {
    std::fs::read(path).map(|bytes| bytes_embed_mapped_root(&bytes, prefix_maps))
}

/// Why a successful compile's outputs must not be stored, if they must not.
///
/// Only a key bound to this checkout may hold an object that names it.
/// Anything the scan cannot vouch for is not stored either: an object that
/// cannot be read, or outputs with no object to read (`object_embeds_root`
/// returns `None`). With no artifacts there is nothing to store or scan.
fn cc_unsafe_to_store(
    no_artifacts: bool,
    key_path_bound: bool,
    object_embeds_root: impl FnOnce() -> Option<std::io::Result<bool>>,
) -> Option<String> {
    if no_artifacts || key_path_bound {
        return None;
    }
    match object_embeds_root() {
        None => Some("has no object to check for checkout roots".to_string()),
        Some(Ok(false)) => None,
        Some(Ok(true)) => {
            Some("embeds a checkout root the prefix maps did not rewrite".to_string())
        }
        Some(Err(error)) => Some(format!(
            "could not be read to check for checkout roots: {error}"
        )),
    }
}

/// The key cannot see a file the assembler would read (kunobi-ninja/kache#1015).
///
/// Its own type so the wrapper can tell it from other key failures: this TU
/// is unsafe for any cache keyed on the preprocessor output, so it must run
/// the compiler directly rather than through a configured fallback wrapper.
#[derive(Debug)]
pub(crate) struct CcHiddenInput {
    pub(crate) construct: &'static str,
}

impl std::fmt::Display for CcHiddenInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The fixed text leads so one `known_passthrough` prefix in the bench
        // scenarios covers every construct.
        write!(
            f,
            "cc: the assembler may read a file the cache key cannot see (`{}`)",
            self.construct
        )
    }
}

impl std::error::Error for CcHiddenInput {}

/// Preprocess each slice of a universal object on its own. The driver
/// refuses `-E` with several `-arch` flags, and a compile's own depfile lists
/// only the last slice's headers, so neither can key the whole object.
fn preprocess_universal_hash(
    parsed: &CcArgs,
    prefix_maps: &[CcPrefixMap],
    file_hasher: &crate::cache_key::FileHasher<'_>,
) -> Result<PreprocessHash> {
    let mut slices = Vec::new();
    let mut fingerprints: Vec<crate::cache_key::CcPreprocessMemoInput> = Vec::new();
    let mut path_bound = false;
    for arch in parsed.arch_slices() {
        let slice = preprocess_hash(&parsed.single_slice(&arch)?, prefix_maps, file_hasher, None)?;
        let inputs = slice
            .fingerprints
            .with_context(|| format!("cc: the {arch} slice gave no read set to key"))?;
        slices.push(format!("{arch}={}+{}", cc_direct_inputs_digest(&inputs), slice.hash));
        path_bound |= slice.path_bound;
        for input in inputs {
            if !fingerprints.iter().any(|known| known.local_path() == input.local_path()) {
                fingerprints.push(input);
            }
        }
    }
    Ok(PreprocessHash {
        hash: slices.join(";"),
        fingerprints: Some(fingerprints),
        path_bound,
    })
}

/// What a precompiled header adds to the key, for a PCH build or a
/// `-include-pch` consumer.
///
/// A PCH records the modification time and size of every input it was built
/// from, and clang refuses to use it once either changes, even with the same
/// bytes. Folding them makes a hit mean what a fresh compile would: a PCH
/// build restores a PCH recording the inputs' current stamps, and a consumer
/// hits only where clang would accept the PCH. A consumer's dependency file
/// names the PCH's inputs but not the PCH, so its bytes are folded too.
fn cc_pch_key(parsed: &CcArgs, read_inputs: Option<&[PathBuf]>) -> Result<Option<String>> {
    if parsed.mode != CompileMode::Precompile && parsed.include_pch.is_none() {
        return Ok(None);
    }
    let inputs = read_inputs.context("cc: a precompiled header needs the read set to key its input stamps")?;
    let mut fields = Vec::new();
    if let Some(pch) = &parsed.include_pch {
        let path = parsed
            .invocation_env
            .resolve(pch)
            .context("resolving the precompiled header")?;
        let digest = crate::cache_key::hash_file(&path)
            .with_context(|| format!("hashing precompiled header {}", path.display()))?;
        fields.push(format!("pch={digest}"));
    }
    let mut stamps = Vec::with_capacity(inputs.len());
    for input in inputs {
        let path = match parsed.invocation_env.resolve(input) {
            Some(path) => path,
            None => input.clone(),
        };
        let meta = std::fs::metadata(&path)
            .with_context(|| format!("stating precompiled header input {}", path.display()))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());
        stamps.push(format!("{}@{mtime}:{}", path.display(), meta.len()));
    }
    stamps.sort();
    fields.extend(stamps);
    Ok(Some(fields.join("\x1f")))
}

/// What the module files a C++20 module consumer may load add to the key.
///
/// Neither the preprocessor output nor the dependency file names an imported
/// module, and clang does not revalidate a module file against the interface
/// it was built from, so the bytes of every module file the consumer can
/// reach are the only record of what it imports. `-fmodule-file=[name=]path`
/// names one file, folded as absent when it does not exist because clang
/// fails only if the unit imports it. `-fprebuilt-module-path=dir` makes
/// clang open `dir/<name>.pcm` by module name, so every `.pcm` entry is
/// folded by name, and adding, renaming or removing one changes the key.
fn cc_module_inputs_key(
    parsed: &CcArgs,
    file_hasher: &crate::cache_key::FileHasher<'_>,
) -> Result<Option<String>> {
    let mut fields = Vec::new();
    let hash = |path: &Path| -> Result<String> {
        let resolved = parsed
            .invocation_env
            .resolve(path)
            .context("resolving a module file")?;
        match std::fs::metadata(&resolved) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("absent".to_string()),
            _ => file_hasher
                .hash(&resolved)
                .with_context(|| format!("hashing module file {}", resolved.display())),
        }
    };
    for arg in &parsed.rest {
        if let Some(value) = arg.strip_prefix("-fmodule-file=") {
            let path = value.split_once('=').map_or(value, |(_, path)| path);
            fields.push(format!("file:{value}={}", hash(Path::new(path))?));
        } else if let Some(dir) = arg.strip_prefix("-fprebuilt-module-path=") {
            let resolved = parsed
                .invocation_env
                .resolve(Path::new(dir))
                .context("resolving a prebuilt module directory")?;
            let entries = match std::fs::read_dir(&resolved) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fields.push(format!("dir:{dir}:absent"));
                    continue;
                }
                entries => entries.with_context(|| format!("listing prebuilt modules in {}", resolved.display()))?,
            };
            let mut names = Vec::new();
            for entry in entries {
                let name = entry?.file_name();
                if Path::new(&name).extension().is_some_and(|ext| ext == "pcm") {
                    names.push(name);
                }
            }
            names.sort();
            fields.push(format!("dir:{dir}"));
            for name in names {
                let digest = hash(&Path::new(dir).join(&name))?;
                fields.push(format!("{}={digest}", name.to_string_lossy()));
            }
        }
    }
    Ok((!fields.is_empty()).then(|| fields.join("\x1f")))
}

/// The operands of `-include`, `--include` and `--include=`, in order.
fn cc_forced_includes(rest: &[String]) -> Vec<&str> {
    let mut names = Vec::new();
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--include=") {
            names.push(value);
        } else if (arg == "-include" || arg == "--include")
            && let Some(value) = args.next()
        {
            names.push(value.as_str());
        }
    }
    names
}

/// Whether `directive` (the text after `#`) is `keyword` as a whole word.
fn cc_directive_is(directive: &[u8], keyword: &[u8]) -> bool {
    directive
        .strip_prefix(keyword)
        .is_some_and(|rest| rest.first().is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_'))
}

/// The header names the main file includes before its first token, where
/// GCC may load a precompiled header in place of the header. `None` when
/// one of them is spelled through a macro, or when text inside a
/// conditional leaves the first token unknown.
fn cc_leading_include_names(text: &[u8]) -> Option<Vec<String>> {
    let text = cc_splice_lines(text);
    let mut names = Vec::new();
    let mut in_comment = false;
    let mut conditional_depth = 0usize;
    for raw in text.split(|&byte| byte == b'\n') {
        let mut line = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            if in_comment {
                match memchr::memmem::find(&raw[i..], b"*/") {
                    Some(end) => {
                        in_comment = false;
                        line.push(b' ');
                        i += end + 2;
                    }
                    None => i = raw.len(),
                }
            } else if raw[i..].starts_with(b"/*") {
                in_comment = true;
                i += 2;
            } else if raw[i..].starts_with(b"//") {
                break;
            } else {
                line.push(raw[i]);
                i += 1;
            }
        }
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        let Some(directive) = line.strip_prefix(b"#").or_else(|| line.strip_prefix(b"%:")) else {
            // Text inside a conditional may be skipped, and then it is not the
            // main file's first token: which includes still lead is unknown.
            if conditional_depth > 0 {
                return None;
            }
            break;
        };
        let directive = directive.trim_ascii_start();
        if [b"ifdef".as_slice(), b"ifndef", b"if"].iter().any(|keyword| cc_directive_is(directive, keyword)) {
            conditional_depth += 1;
            continue;
        }
        if cc_directive_is(directive, b"endif") {
            conditional_depth = conditional_depth.saturating_sub(1);
            continue;
        }
        let Some(operand) = [b"include_next".as_slice(), b"include", b"import"]
            .iter()
            .find_map(|keyword| directive.strip_prefix(*keyword))
        else {
            continue;
        };
        let operand = operand.trim_ascii_start();
        let close = match operand.first() {
            Some(b'"') => b'"',
            Some(b'<') => b'>',
            _ => return None,
        };
        let end = memchr::memchr(close, &operand[1..])?;
        names.push(String::from_utf8_lossy(&operand[1..=end]).into_owned());
    }
    Some(names)
}

/// What the precompiled headers a compile may load without naming them add
/// to the key.
///
/// GCC loads `<dir>/<name>.gch`, a file or a directory of variants, in place
/// of a header included before the main file's first token, looking in each
/// search directory before the header itself. clang's driver turns
/// `-include X` into `-include-pch X.pch` (or `X.gch`) when that file exists;
/// those candidates are folded for every driver, since one named `cc` may be
/// clang.
/// Neither the expansion nor the dependency file names the precompiled
/// header, and once GCC loads one its dependency file drops the header and
/// everything it includes, so the key folds each candidate that exists, by
/// name and directory position. A compile with none keys as it did before.
fn cc_implicit_pch_key(
    parsed: &CcArgs,
    file_hasher: &crate::cache_key::FileHasher<'_>,
) -> Result<Option<String>> {
    if !matches!(parsed.family, ToolFamily::Gnu | ToolFamily::Clang) {
        return Ok(None);
    }
    let cwd = parsed
        .invocation_env
        .cwd()
        .context("cc invocation has no working directory")?;
    let forced = cc_forced_includes(&parsed.rest);
    let mut fields = Vec::new();
    let mut fold = |label: String, path: PathBuf| -> Result<()> {
        let meta = match std::fs::metadata(&path) {
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
                return Ok(());
            }
            meta => meta.with_context(|| format!("stating precompiled header {}", path.display()))?,
        };
        if !meta.is_dir() {
            let digest = file_hasher
                .hash(&path)
                .with_context(|| format!("hashing precompiled header {}", path.display()))?;
            fields.push(format!("{label}={digest}"));
            return Ok(());
        }
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&path)
            .with_context(|| format!("listing precompiled headers in {}", path.display()))?
        {
            let entry = entry?;
            if entry.path().is_file() {
                entries.push(entry.file_name());
            }
        }
        entries.sort();
        fields.push(format!("{label}/"));
        for name in entries {
            let digest = file_hasher.hash(&path.join(&name)).with_context(|| {
                format!("hashing precompiled header {}", path.join(&name).display())
            })?;
            fields.push(format!("{label}/{}={digest}", name.to_string_lossy()));
        }
        Ok(())
    };
    // A driver named `cc` or `c++` may be clang whatever the family its name
    // suggests, so every driver folds clang's `-include` candidates.
    for name in &forced {
        for ext in ["pch", "gch"] {
            fold(format!("{name}.{ext}"), absolutize_path(cwd, Path::new(&format!("{name}.{ext}"))))?;
        }
    }
    if parsed.family == ToolFamily::Clang {
        return Ok((!fields.is_empty()).then(|| fields.join("\x1f")));
    }
    let mut dirs = vec![cwd.to_path_buf()];
    for dir in cc_user_include_dirs(parsed, cwd).into_iter().chain(cc_system_include_dirs(parsed, cwd)) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let leading = match parsed.sources.first() {
        Some(source) => {
            let path = absolutize_path(cwd, source);
            let text = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            cc_leading_include_names(&text)
        }
        None => Some(Vec::new()),
    };
    let Some(leading) = leading else {
        // The name a macro spells, or which includes lead past a conditional,
        // is unknown, so every precompiled header in the search directories
        // is a candidate. The marker keys this path apart from the narrower
        // scan, which once let an object built against a `.gch` be stored
        // under the key of a compile without one.
        for (index, dir) in dirs.iter().enumerate() {
            let entries = match std::fs::read_dir(dir) {
                Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => continue,
                entries => entries.with_context(|| format!("listing {}", dir.display()))?,
            };
            let mut names = Vec::new();
            for entry in entries {
                let name = entry?.file_name();
                if Path::new(&name).extension().is_some_and(|ext| ext == "gch") {
                    names.push(name);
                }
            }
            names.sort();
            for name in names {
                fold(format!("{}@{index}", name.to_string_lossy()), dir.join(&name))?;
            }
        }
        for name in forced {
            fold(format!("{name}.gch"), absolutize_path(cwd, Path::new(&format!("{name}.gch"))))?;
        }
        fields.push("leading=unknown".to_string());
        return Ok(Some(fields.join("\x1f")));
    };
    for name in forced.into_iter().map(str::to_string).chain(leading) {
        let candidate = format!("{name}.gch");
        if Path::new(&candidate).is_absolute() {
            fold(candidate.clone(), PathBuf::from(&candidate))?;
            continue;
        }
        for (index, dir) in dirs.iter().enumerate() {
            fold(format!("{candidate}@{index}"), dir.join(&candidate))?;
        }
    }
    Ok((!fields.is_empty()).then(|| fields.join("\x1f")))
}

fn cc_may_reference_time_macro(bytes: &[u8]) -> bool {
    // Escapes, trigraphs and digraphs can hide an identifier or paste operator.
    bytes.iter().any(|b| !b.is_ascii() || matches!(b, 0 | b'\\'))
        || [b"__DATE__".as_slice(), b"__TIME__", b"__TIMESTAMP__", b"##", b"%:", b"??"]
            .iter()
            .any(|needle| memchr::memmem::find(bytes, needle).is_some())
}

fn cc_time_macro_free_inputs(
    parsed: &CcArgs,
    resolved: Option<&crate::probe::ResolvedConfig>,
    dep_path: Option<&Path>,
    file_hasher: &crate::cache_key::FileHasher<'_>,
) -> Option<BTreeMap<PathBuf, String>> {
    let resolved = resolved.filter(|driver| driver.native_driver)?;
    let tokens = resolved.resolved_tokens.as_ref()?;
    let elided_macro = tokens.iter().enumerate().any(|(index, arg)| {
        (arg.starts_with("-D") || (index > 0 && tokens[index - 1] == "-D"))
            && arg.contains('<')
    });
    if parsed.family.dialect() != Dialect::Gnu
        || parsed.mode == CompileMode::Precompile
        || parsed.sources.is_empty()
        || parsed.include_pch.is_some()
        || elided_macro
        || parsed.rest.iter().chain(tokens).any(|arg| {
            cc_may_reference_time_macro(arg.as_bytes()) || arg.starts_with('@')
                || [
                    "-fmodule", "-fprebuilt-module", "-include", "-imacros", "-fpch",
                    "-fplugin", "-plugin", "-load", "-specs", "--specs",
                    "-finput-charset", "-traditional", "-Xclang", "-Xpreprocessor", "-Wp,",
                    "-add-plugin", "-wrapper",
                ].iter().any(|prefix| arg.starts_with(prefix))
        })
    {
        return None;
    }
    let cwd = parsed.invocation_env.cwd()?;
    let dependencies = parse_preprocess_dependencies(&fs::read_to_string(dep_path?).ok()?, cwd).ok()?;
    if !parsed.sources.iter().all(|source| {
        parsed.invocation_env.resolve(source).is_some_and(|source| dependencies.contains(&source))
    }) {
        return None;
    }
    let mut scanned = BTreeMap::new();
    for path in dependencies {
        let bytes = fs::read(&path).ok()?;
        if cc_may_reference_time_macro(&bytes) {
            return None;
        }
        scanned.insert(path, blake3::hash(&bytes).to_hex().to_string());
    }
    if cc_implicit_pch_key(parsed, file_hasher).ok()?.is_some() {
        return None;
    }
    Some(scanned)
}

fn preprocess_hash(
    parsed: &CcArgs,
    prefix_maps: &[CcPrefixMap],
    file_hasher: &crate::cache_key::FileHasher<'_>,
    resolved: Option<&crate::probe::ResolvedConfig>,
) -> Result<PreprocessHash> {
    let pp_args = key_preprocess_args(parsed, prefix_maps);
    let dep_temp = match tempfile::Builder::new()
        .prefix("kache-cc-preprocess-")
        .tempdir_in(parsed.invocation_env.temp_dir())
    {
        Ok(temp) => Some(temp),
        Err(error) => {
            log::debug!("cc preprocess dependency tempdir unavailable: {error}");
            None
        }
    };
    let dep_path = dep_temp
        .as_ref()
        .and_then(|dir| capture_depfile(dir.path()));
    let mut pp_args = dep_path.as_ref().map_or(pp_args.clone(), |(_, arg)| {
        add_preprocess_dep_capture(parsed, pp_args.clone(), arg.clone())
    });
    let run = |args: &[String]| {
        crate::opcounts::record_preprocessor_run();
        let mut command = parsed.invocation_env.command(&parsed.program)?;
        command.args(args);
        // If `program` is a kache shim, this probe must not re-enter the
        // cache (it would try to key another -E to stdout). The wrapper
        // passthroughs when this is set.
        command.env("KACHE_CC_KEY_PROBE", "1");
        // Pin the build timestamp so `__DATE__` / `__TIME__` expand
        // deterministically. The real compile uses the same value.
        if let Some(epoch) = effective_source_date_epoch_for(&parsed.invocation_env) {
            command.env("SOURCE_DATE_EPOCH", epoch);
        }
        command
            .invocation_output()
            .with_context(|| format!("running preprocessor `{}`", parsed.program))
    };
    let mut output = run(&pp_args)?;
    let captured_dependencies = output.status.success();
    if !output.status.success() && dep_path.is_some() {
        // An otherwise supported compiler may not implement the private
        // dependency flags. Preserve the existing cache-key behavior and just
        // leave this invocation unmemoized.
        log::debug!("cc preprocess dependency capture failed; retrying without memo capture");
        pp_args = key_preprocess_args(parsed, prefix_maps);
        output = run(&pp_args)?;
    }
    if !output.status.success() {
        // Preprocess failed — the real compile would also fail.
        // Bail so the wrapper falls back to passthrough, which runs
        // the real compiler and surfaces the real diagnostic. Worded as the
        // `uncacheable` passthrough detail, not a failure: often a configure
        // probe that is *meant* to fail.
        anyhow::bail!(
            "cc -E key probe exited {}",
            output
                .status
                .code()
                .map_or_else(|| "by signal".to_string(), |c| c.to_string())
        );
    }
    if output.stdout.is_empty() {
        // Zero preprocessor output: a mis-detected family ran the wrong
        // preprocess flags (gnu `-E` under clang-cl writes to a file),
        // or a degenerate empty TU. Either way refuse rather than hash
        // nothing → passthrough.
        anyhow::bail!("cc -E key probe produced no output");
    }
    let unpinned = !cc_source_date_epoch_is_pinned(parsed);
    let time_macro_free = unpinned.then(|| {
        cc_time_macro_free_inputs(
            parsed, resolved, dep_path.as_ref().map(|(path, _)| path.as_path()).filter(|_| captured_dependencies), file_hasher,
        )
    }).flatten();
    if unpinned && time_macro_free.is_none() {
        let check_args = compose_cc_args(
            &pp_args,
            vec![
                "-Wno-builtin-macro-redefined".into(),
                "-D__DATE__=\"kache-unpinned-date\"".into(),
                "-D__TIME__=\"kache-unpinned-time\"".into(),
                "-D__TIMESTAMP__=\"kache-unpinned-timestamp\"".into(),
            ],
        );
        let checked = run(&check_args)?;
        anyhow::ensure!(
            checked.status.success()
                && checked.stdout == output.stdout
                && checked.stderr == output.stderr,
            "cc: unpinned time macros (not yet supported)"
        );
    }
    let (text, layout) = split_line_markers(&output.stdout, prefix_maps);
    if let Some(construct) = cc_assembler_hidden_input(&text) {
        // Refuse before the memo can record this TU: a memo hit skips the
        // probe, and the file the assembler reads is in no fingerprint.
        return Err(CcHiddenInput { construct }.into());
    }
    let CcExpansionHash { hash, path_bound } = hash_cc_expansion(text, prefix_maps);
    let hash = fold_line_layout(hash, layout.as_deref());
    // A path-bound expansion is never memoized. The memo is read from other
    // checkouts through mapped names, and a hit skips the probe that would
    // have noticed the root.
    // Fingerprinted for a path-bound expansion too: the memo carries the
    // binding, and the key folds the reading checkout's roots.
    let fingerprints = dep_path.as_ref().and_then(|(path, _)| {
        let dependencies = std::fs::read_to_string(path)
            .context("reading cc preprocess dependency file")
            .and_then(|raw| {
                let cwd = parsed
                    .invocation_env
                    .cwd()
                    .context("reading cc compiler directory")?;
                parse_preprocess_dependencies(&raw, cwd)
            });
        match dependencies {
            Ok(paths) => {
                // Record each input under its prefix-mapped spelling, the form
                // the expansion above was hashed in. That is what lets another
                // checkout read the memo: the path a second worktree resolves
                // an input to differs, the mapped name does not.
                let mapped: Vec<(String, PathBuf)> = paths
                    .into_iter()
                    .map(|path| (cc_mapped_path(&path, prefix_maps), path))
                    .collect();
                file_hasher.cc_preprocess_fingerprints(
                    &mapped,
                    &cc_prefix_maps_key(prefix_maps),
                    &|path| cc_mapped_content_hash(path, prefix_maps),
                )
            }
            Err(error) => {
                log::debug!("cc preprocess dependency capture unavailable: {error:#}");
                None
            }
        }
    });
    if let Some(scanned) = time_macro_free {
        anyhow::ensure!(
            fingerprints.as_ref().is_some_and(|inputs| {
                inputs.len() == scanned.len() && inputs.iter().all(|input| {
                    scanned.get(Path::new(input.local_path())) == Some(&input.content)
                })
            }),
            "cc: inputs changed during the time macro scan"
        );
    }
    if parsed.lists_assembler_source() {
        let inputs = fingerprints.as_ref().context("listing requires a dependency read set")?
            .iter().map(|input| PathBuf::from(input.local_path())).collect::<Vec<_>>();
        cc_check_listing_inputs(parsed, &output.stdout, &inputs)?;
    }
    Ok(PreprocessHash {
        hash,
        fingerprints,
        path_bound,
    })
}

/// Whether a positional argument looks like a C-family source file
/// (matches one of the recognized extensions in [`SOURCE_EXTENSIONS`]).
/// Conservative: extensionless files or unknown extensions are NOT
/// treated as sources, even if they happen to be C code in practice.
fn looks_like_header(arg: &str) -> bool {
    Path::new(arg)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "h" | "hh" | "hpp" | "hxx" | "H" | "h++"))
}

fn looks_like_source(arg: &str) -> bool {
    Path::new(arg)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| SOURCE_EXTENSIONS.contains(&e))
        .unwrap_or(false)
}

/// Parse a `-D NAME` or `-D NAME=VALUE` argument value.
fn parse_define(s: &str) -> (String, Option<String>) {
    match s.split_once('=') {
        Some((name, value)) => (name.to_string(), Some(value.to_string())),
        None => (s.to_string(), None),
    }
}

/// Cc flag classification table — the declarative source of truth
/// for "how does kache treat this argument?".
///
/// Each row pairs a [`Matcher`] with a [`FlagClass`] and a `source`
/// reference. See [`crate::compiler::flags`] for the matcher /
/// classification vocabulary and for the audit / extensibility
/// guarantees this shape delivers.
///
/// **Adding a flag**: drop a row in the appropriate `class`
/// section, point `source` at the issue / PR that introduced it,
/// and write a test for the new pattern. Done.
///
/// **Reading the table**: `class` answers "why is this safe?".
/// `ModeledInKey` = the parser extracts it into a typed field.
/// `ParserHandled` = the parser routes it to a structural field used
/// for refusal / execution flow rather than object-content keying.
/// `CapturedByProbe` = `cc -###` resolves it into `-cc1` tokens
/// the cache key already hashes. `RawKeyed` = the argument is folded
/// directly into the cache key. `PreprocessorCaptured` = the
/// preprocessor expansion hash subsumes its effect.
/// `NoObjectEffect` = it doesn't change the resulting object.
///
/// **Anything not in the table** is refused with `cc: unsupported
/// flag(s): …` — see [`CcArgs::refuse_reasons`]. The omission is
/// the safety signal: a flag kache has never seen could miscache,
/// so the conservative default is to passthrough.
pub static CC_FLAGS: &[FlagSpec] = &[
    FlagSpec { matcher: Matcher::Exact("-fstack-usage"), class: FlagClass::RawKeyed, source: "GCC object-derived .su; reports key literal argv/cwd/maps and original input content; LTO and dump overrides refuse", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fsave-optimization-record"), class: FlagClass::RawKeyed, source: "Clang object-derived .opt.yaml; source locations bind reports to literal paths and input content; format/file overrides remain unmodeled", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Regex(r"-fopt-info(-optimized)?(=.*)?"), class: FlagClass::RawKeyed, source: "GCC bare/optimized stderr only; bounded modes because other verbosity is unverified; =file variants refuse because GCC appends instead of replacing existing output", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Regex(r"-fcallgraph-info(=(su|da)(,(su|da))*)?"), class: FlagClass::RawKeyed, source: "GCC object-derived .ci; bounded marker lists only; literal paths and original input content are keyed; LTO/dump overrides refuse", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Regex(r"-Wa,-adhln(=[^,]+)?"), class: FlagClass::RawKeyed, source: "GCC/GAS stdout or explicit listing; h reads debug source files, so line-marker targets outside the dependency read set and inline .file refuse; only literal, local, validated read sets may reuse the memo; mixed forwarding and LTO refuse", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-aux-info"), class: FlagClass::RawKeyed, source: "GCC explicit auxiliary declarations output, if written (C++ ignores it); literal argv/cwd/maps and original input content keyed", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Regex(r"-fsanitize-coverage=(trace-pc|trace-pc-guard|inline-8bit-counters|pc-table|trace-cmp)(,(trace-pc|trace-pc-guard|inline-8bit-counters|pc-table|trace-cmp))*"), class: FlagClass::RawKeyed, source: "argv-only sanitizer instrumentation, including fuzzer-no-link combinations; other modes remain unmodeled", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-frecord-gcc-switches"), class: FlagClass::RawKeyed, source: "Clang records input/output/include paths and prefix-map operands; key the full literal argv, cwd and injected maps, bind to this checkout, and disable private dependency capture", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fno-delete-null-pointer-checks"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fno-strict-overflow"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fno-ident"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fno-lifetime-dse"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-flto"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-emit-llvm"), class: FlagClass::RawKeyed, source: "verified Clang -c/-S only; source_filename and textual ModuleID retain literal input spelling, so key argv/cwd/maps and bind to the checkout; disable private dependency capture; LTO flags retain their existing raw keying; universal and other modes refuse", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Regex(r"-flto=(thin|full)"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fsplit-lto-unit"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fwhole-program-vtables"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fcomplete-member-pointers"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-mno-outline"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-no-canonical-prefixes"), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-mmacos-version-min="), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fsanitize="), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fsanitize-trap="), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fsanitize-ignore-for-ubsan-feature="), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fmodule-name="), class: FlagClass::RawKeyed, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fprofile-use="), class: FlagClass::ParserHandled, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("--warning-suppression-mappings="), class: FlagClass::ParserHandled, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fsanitize-ignorelist="), class: FlagClass::ParserHandled, source: "clang sanitizer ignorelist, keyed by content", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fcrash-diagnostics-dir="), class: FlagClass::NoObjectEffect, source: "clang compile flag", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-Xarch_"), class: FlagClass::RawKeyed, source: "CCP-07 — clang per-slice operand; keyed with its operand, and each slice is preprocessed with its own operands", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fmodule-file="), class: FlagClass::ParserHandled, source: "CCP-06 — clang explicit module consumer; the key folds the module file's bytes", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Prefix("-fprebuilt-module-path="), class: FlagClass::ParserHandled, source: "CCP-06 — clang prebuilt module directory; the key folds every module file in it", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-include-pch"), class: FlagClass::ParserHandled, source: "CCP-06 — clang PCH consumer; the key folds the PCH bytes and its inputs' stamps", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("--coverage"), class: FlagClass::RawKeyed, source: "CCP-05 — gcov instrumentation and notes; the sidecar key folds the paths the outputs embed", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-fprofile-arcs"), class: FlagClass::RawKeyed, source: "CCP-05 — gcov arc instrumentation; the object names its absolute .gcda path", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-ftest-coverage"), class: FlagClass::RawKeyed, source: "CCP-05 — gcov notes file", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Regex(r"-fprofile-dir=.+"), class: FlagClass::RawKeyed, source: "profile directory keyed verbatim; coverage keys the exact cwd and object spelling, retaining GCC path mangling; -fprofile-prefix-path remains unmodeled and GCC profile-use directories refuse", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Regex(r"-fprofile-generate=.+"), class: FlagClass::RawKeyed, source: "generation directory keyed verbatim and bound to cwd/object like gcov; notes, when requested, use the coverage sidecar model; bare/empty generation and profile-prefix-path remain unmodeled", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-gsplit-dwarf"), class: FlagClass::RawKeyed, source: "CCP-05 — split DWARF; the object names its .dwo as the output was spelled", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("--serialize-diagnostics"), class: FlagClass::NoObjectEffect, source: "CCP-05 — clang serialized diagnostics output path", dialect: Some(Dialect::Gnu) },
    FlagSpec { matcher: Matcher::Exact("-serialize-diagnostics"), class: FlagClass::NoObjectEffect, source: "CCP-05 — clang serialized diagnostics output path", dialect: Some(Dialect::Gnu) },
    // ── ModeledInKey: parser extracts into a typed field ──
    FlagSpec {
        // `-O` family: bare, digit (`-O0`..`-O3`), `-Os`/`-Oz`, `-Og`.
        // The regex names the family in one row; an out-of-set value
        // (`-Ofast`) deliberately falls through to refusal because the
        // parser doesn't model it. See `CcArgs::parse`.
        matcher: Matcher::Regex(r"-O[0-3sz]?|-Og"),
        class: FlagClass::ModeledInKey,
        source: "PR #94 — opt level. Regex captures family; -Ofast/+others fall through to refuse.",
        dialect: None,
    },
    FlagSpec {
        // `-g` family: bare or with a level digit (`-g0`..`-g3`). The
        // parser extracts the level into `debug_level`. Variants like
        // `-gdwarf-5` / `-ggdb` / `-gline-tables-only` change debug
        // info but aren't modeled, so they're not on this row.
        matcher: Matcher::Regex(r"-g[0-3]?"),
        class: FlagClass::ModeledInKey,
        source: "PR #94 — debug level. Regex captures `-g`/`-g0..3`; -gdwarf-* etc. refuse.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fPIC"),
        class: FlagClass::ModeledInKey,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fpic"),
        class: FlagClass::ModeledInKey,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-std="),
        class: FlagClass::ModeledInKey,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        // Single `-arch <value>`. The parser sets `cache_target_arch`
        // from the resolved arch; multi-`-arch X -arch Y` is refused
        // separately in the procedural pass of `refuse_reasons`.
        matcher: Matcher::Exact("-arch"),
        class: FlagClass::ModeledInKey,
        source: "PR #94",
        dialect: None,
    },
    // ── ParserHandled: parser routes to structural invocation state ──
    FlagSpec {
        matcher: Matcher::Exact("-c"),
        class: FlagClass::ParserHandled,
        source: "PR #94 — compile mode marker parsed into CompileMode.",
        dialect: None,
    },
    FlagSpec {
        // End-of-options marker. Does not change object bytes; Firefox's
        // Windows clang-cl nightly still refused it as an unclassified flag.
        matcher: Matcher::Exact("--"),
        class: FlagClass::NoObjectEffect,
        source: "Firefox Windows 0.20 nightly — end-of-options marker.",
        dialect: None,
    },
    FlagSpec {
        // MSVC `/c` slash spelling of the compile-mode marker (cl only).
        // The parser already routes `/c` to CompileMode::Compile via
        // CC_ARG_SPECS; this row tells the unsupported-flag classifier the
        // token is known (ParserHandled) so it isn't rejected. Without it,
        // a `/c` clang-cl compile is refused as `unsupported flag(s): /c`
        // (box-confirmed). Mirrors the `-c` row above. (#312)
        matcher: Matcher::Exact("/c"),
        class: FlagClass::ParserHandled,
        source: "Issue #312 — MSVC /c compile-mode marker, cl dialect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-E"),
        class: FlagClass::ParserHandled,
        source: "Flag audit — preprocessor mode marker parsed into CompileMode.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-S"),
        class: FlagClass::ParserHandled,
        source: "Flag audit — assembly mode marker parsed into CompileMode.",
        dialect: None,
    },
    FlagSpec {
        // `--driver-mode=<mode>` selects the driver dialect clang speaks —
        // `cl` turns a plain `clang` into clang-cl (a real cross-/Linux way
        // to get MSVC-driver behavior, and how kache's own Linux e2e drives
        // the clang-cl path). The token is consumed structurally by
        // `ToolFamily::detect`; it carries no object effect of its own, and
        // the dialect's actual codegen/preprocessor consequences are already
        // keyed (the `-E`/`/EP` preprocessor hash sees the MSVC predefined
        // macros, the `-###` probe sees the resolved cc1 line). Without this
        // row every `clang --driver-mode=cl` compile refused on the token
        // itself. Prefix-matched so `=gcc`/`=g++`/`=cpp` are covered too.
        matcher: Matcher::Prefix("--driver-mode="),
        class: FlagClass::ParserHandled,
        source: "Issue #411 — driver-mode selector; consumed by ToolFamily::detect, effects keyed via preprocessor + -### probe.",
        dialect: None,
    },
    // ── CapturedByProbe: `cc -###` resolved tokens differentiate ──
    //
    // Each row's effect on the resulting object is captured by the
    // resolved `cc -###` `-cc1` token stream that the cache key
    // already hashes (see `cache_key`'s `resolved:` tokens). Identical
    // user-facing flags → identical resolved tokens → same key;
    // different values → different tokens → different key. Safety holds
    // only when the probe resolves on the host compiler. If it does
    // not, `cache_key` refuses probe-keyed flags before preprocessing
    // so these rows cannot silently under-key.
    //
    // Initial population sourced from the Firefox/Gecko Darwin
    // baseline (kunobi-ninja/kache#114): ~4,476 single-source compiles
    // per Firefox build that previously passed through unnecessarily.
    FlagSpec {
        matcher: Matcher::Prefix("-mmacosx-version-min="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #114 — Darwin deployment target.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-fstrict-flex-arrays="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #114 — strict-flex-arrays codegen knob.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-ffp-contract="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #114 — fp-contract codegen knob.",
        dialect: None,
    },
    FlagSpec {
        // Clang's automatic-variable hardening mode changes generated code and
        // is forwarded verbatim to `-cc1`, so the resolved-token hash safely
        // distinguishes it from the default mode. Keep the row exact on the
        // Firefox-reported spelling; adjacent modes remain fail-closed until
        // there is workload evidence for them.
        matcher: Matcher::Exact("-ftrivial-auto-var-init=pattern"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #849 — Firefox automatic-variable pattern initialization; Apple clang forwards the value to -cc1 verbatim (verified -###).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-ftrivial-auto-var-init=zero"),
        class: FlagClass::CapturedByProbe,
        source: "Chromium official macOS build; Apple clang forwards the value to -cc1 verbatim (verified -###).",
        dialect: None,
    },
    // ── Codegen knobs: one sorted stem list, BOTH polarities ──
    //
    // `-f(?:no-)?<stem>` matches `-f<stem>` AND `-fno-<stem>` for every stem
    // below. Each is a codegen knob clang/gcc forward to `-cc1`, so the
    // resolved-token hash captures whichever polarity is passed — both are
    // equally safe (CapturedByProbe; the probe resolves the actual codegen).
    // Listing the STEM once instead of each `-f`/`-fno-` spelling structurally
    // prevents the "modeled one polarity, missed the other" passthrough class
    // that recurred across the nightly benches: `-ftrapping-math` (#422) and
    // `-fomit-frame-pointer` (#426) each slipped through because only the
    // opposite polarity was listed. Add a knob = add one sorted stem; both
    // polarities are then covered, and a stray knob on every TU can't silently
    // void the cache (the #411 class).
    //
    // The matcher is anchored `^(?:…)$` by `cc_arg_spec_matches`, so this never
    // partial-matches a longer flag. Math stems: `-ffast-math` defines
    // `__FAST_MATH__`/`__FINITE_MATH_ONLY__` (seen by the `-E` hash), but its
    // optimizer assumptions (reassociation, no-inf/no-nan, FP contraction) are
    // invisible to `-E` — only the `-cc1` stream captures them, so these are
    // `CapturedByProbe`, not `PreprocessorCaptured`. Verified against
    // `clang -###`. Stems are kept ALPHABETICAL for maintenance.
    FlagSpec {
        matcher: Matcher::Regex(concat!(
            r"-f(?:no-)?(?:",
            "associative-math|asynchronous-unwind-tables|common|cxx-exceptions|data-sections|",
            "fast-math|finite-math-only|freestanding|function-sections|math-errno|",
            "merge-all-constants|omit-frame-pointer|reciprocal-math|rounding-math|",
            "semantic-interposition|signaling-nans|signed-zeros|strict-aliasing|",
            "trapping-math|unsafe-math-optimizations|unroll-loops|unwind-tables|wrapv",
            ")",
        )),
        class: FlagClass::CapturedByProbe,
        source: "#114/#245/#418/#422/#426/#580/#856 + 0.20 nightly — codegen knobs, both polarities, resolved into -cc1 tokens. One sorted stem per knob covers -f<stem> AND -fno-<stem> (prevents the missed-polarity passthrough class). unroll-loops / asynchronous-unwind-tables from lance/Firefox passthroughs. common: libtool adds -fno-common to every Darwin shared-object compile (expat 2.7.1); clang forwards -fcommon to -cc1 and drops its default -fno-common, GCC forwards both (verified -###). cxx-exceptions / freestanding from Firefox Windows nightly.",
        dialect: None,
    },
    FlagSpec {
        // `-mrecip=<value>` (x86 reciprocal-estimate codegen). Prefix-matched so
        // `=none`/`=all`/`=default,...` are all covered; the value rides through
        // to `-cc1` so different settings key differently.
        matcher: Matcher::Prefix("-mrecip="),
        class: FlagClass::CapturedByProbe,
        source: "Firefox nightly bench — reciprocal-estimate codegen selector; value forwarded to -cc1 (verified clang -###).",
        dialect: None,
    },
    FlagSpec {
        // `-mno-omit-leaf-frame-pointer` (clang): forces frame pointers for
        // leaf functions too. Current cc-rs emits it when Rust requests
        // forced frame pointers, and in debug-mode tool setup alongside
        // `-fno-omit-frame-pointer` — so it lands on every aws-lc-sys TU of
        // a macOS debug build (#839). Real codegen effect: clang's `-###`
        // resolves it to `-mframe-pointer=all` against the default
        // `-mframe-pointer=non-leaf`, which the resolved-token hash
        // separates. gcc spells the equivalent knob `-momit-leaf-frame-
        // pointer` (opposite default, same `-m` shape); a driver whose
        // `-###` doesn't resolve keeps refusing fail-closed.
        matcher: Matcher::Exact("-mno-omit-leaf-frame-pointer"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #839 — cc-rs forced-frame-pointer flag (aws-lc-sys debug builds); clang -### resolves to -mframe-pointer=all.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-pthread"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #114 — pthread feature switch (also visible via _REENTRANT in preprocessor).",
        dialect: None,
    },
    FlagSpec {
        // Firefox nightly still passed `-fno-stack-protector` through because
        // only `-fstack-protector-strong` was listed. Cover the family
        // (bare / -strong / -all, both polarities); clang forwards the
        // resulting -stack-protector* token to -cc1.
        matcher: Matcher::Regex(r"-f(?:no-)?stack-protector(?:-strong|-all)?"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #114 + Firefox 0.20 nightly — stack-protector family, both polarities.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fstack-clash-protection"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #245 — stack-clash-protection codegen hardening (Firefox).",
        dialect: None,
    },
    // `--param <name>=<value>` / `--param=<name>=<value>` — backend tuning
    // knobs (`ssp-buffer-size`, `max-inline-insns-auto`, …). Real codegen
    // effect under gcc, which forwards the pair verbatim to cc1, so the
    // resolved-token hash separates one value from another. Classifying the
    // flag `CapturedByProbe` also *forces* the probe
    // (`cc_flags_need_resolved_invocation`), and the key bails when it can't
    // resolve — the separated value token is inert on its own, so without
    // that forcing it would under-key. clang accepts the option and drops it
    // before cc1: no token, and no object difference either, so collapsing
    // those keys is correct rather than lossy.
    //
    // aws-lc-sys passes `--param ssp-buffer-size=4` on its 6 jitterentropy
    // TUs (#580). Those live in the same archive as the `--include=` TUs
    // above, so both rows are needed for the `.a` to converge cross-clone.
    FlagSpec {
        matcher: Matcher::Exact("--param"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #580 — aws-lc-sys jitterentropy `--param ssp-buffer-size=4`; gcc forwards the pair to cc1, clang drops it.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("--param="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #580 — joined spelling of --param; value rides through to cc1.",
        dialect: None,
    },
    // `-fsanitize-undefined-strip-path-components=-1` (clang-only). aws-lc-sys
    // 0.44 adds it to the same jitterentropy TUs as the `--param` rows above
    // once a compile probe confirms support (`builder/cc_builder.rs`): Clang's
    // UBSan check metadata at -O0 can embed the full source path, and the
    // option strips leading path components from it. Path metadata rather
    // than codegen, but it still changes object bytes, so it must be keyed —
    // clang forwards the option, value included, verbatim into the resolved
    // `-cc1` line, and the resolved-token hash captures it. Exact-matched on
    // the one observed value: every integer value clang accepts rides through
    // to cc1 (verified `=0`/`=2` on Apple clang 21), so widening is safe when
    // a workload needs it, but `=-1` is the only spelling in evidence and the
    // exact row keeps unobserved values refusing — the `-gdwarf-4` precedent.
    FlagSpec {
        matcher: Matcher::Exact("-fsanitize-undefined-strip-path-components=-1"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #840 — aws-lc-sys 0.44 jitterentropy UBSan path-stripping; Apple clang forwards the value to -cc1 verbatim (verified -###).",
        dialect: None,
    },
    // (math-errno, strict-aliasing, omit-frame-pointer, unwind-tables —
    // #114/#426 — are now covered by the sorted codegen-knob stem list above,
    // both polarities.)
    // `-ffile-reproducible` / `-fno-file-reproducible` (clang). Firefox's
    // Windows build passes `-ffile-reproducible` (it also `-Werror`-probes
    // for it — see `clang_cl_invocation_injects_no_flags_issue_299`). The
    // flag controls how clang renders embedded paths (`__FILE__`, debug
    // info) for reproducibility; clang forwards it to `-cc1`, so its full
    // effect is captured by the resolved-token hash. `CapturedByProbe`,
    // not `NoObjectEffect`: it can change object bytes (the `__FILE__`
    // path separator), so it must be keyed. Dialect-agnostic — the flag is
    // gnu-spelled and accepted by both the default driver and clang-cl;
    // under a driver whose `-###` doesn't resolve, the probe contract
    // refuses rather than under-keys. (Issue #411 — Firefox/Windows.)
    FlagSpec {
        matcher: Matcher::Exact("-ffile-reproducible"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #411 — clang reproducible embedded paths; keyed via -### resolved tokens.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fno-file-reproducible"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #411 — clang reproducible embedded paths (negation); keyed via -### resolved tokens.",
        dialect: None,
    },
    // Firefox debug-info & clang argument-wrapper flags
    // (kunobi-ninja/kache#117). The debug-info flags affect DWARF
    // sections of the object; clang's `-###` expands them into
    // `-cc1 -dwarf-version=4` / `-dwarf-linkage-names=Abstract` / etc.,
    // so the resolved-tokens hash differentiates them per-value.
    //
    // The DWARF version digit is *not* wildcarded on purpose:
    // `-gdwarf-5` produces a different (larger, newer-toolchain-
    // dependent) object and isn't part of any observed evidence. Each
    // version is listed exactly as workloads surface it; the probe's
    // resolved `-dwarf-version=N` token keeps the keys distinct.
    FlagSpec {
        matcher: Matcher::Exact("-gdwarf-4"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #117 — DWARF v4 emission (Firefox baseline).",
        dialect: None,
    },
    // cc-rs selects `-gdwarf-2` for every Apple target when debug info is
    // enabled, so every Rust workspace with native deps hits this on macOS
    // debug/test profiles (aws-lc-sys was the sampled root).
    FlagSpec {
        matcher: Matcher::Exact("-gdwarf-2"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #838 — DWARF v2 emission; cc-rs default on Apple targets.",
        dialect: None,
    },
    // ring's Darwin build.rs requires full debug information so Apple's
    // dead strip can see which symbols are used. Apple clang's `-###`
    // resolves `-gfull` to `-debug-info-kind=standalone -dwarf-version=5`,
    // the same cc1 tokens as ordinary `-g` on that compiler; a no-debug
    // invocation has neither. Exact, not a `-g*` prefix: neighbouring
    // Apple spellings such as `-gused` stay refused until a workload
    // needs them. (Issue #857)
    FlagSpec {
        matcher: Matcher::Exact("-gfull"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #857 — Darwin full debug info; ring 0.17 dead-strip contract. Apple clang -### resolves it to standalone DWARF; keyed via those tokens.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-gsimple-template-names"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #117 — clang template-name compression in debug info.",
        dialect: None,
    },
    FlagSpec {
        // `-mllvm=` passes through to LLVM. Different `-mllvm`
        // values can do arbitrary codegen things, so a `Prefix("-mllvm=")`
        // wildcard would silently accept unmodeled codegen flags. List
        // specific values that workloads need; `-Mllvm=…` etc. still
        // refuse.
        matcher: Matcher::Exact("-mllvm=-dwarf-linkage-names=Abstract"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #117 — LLVM debug-info abstraction (Firefox baseline). Listed by exact value rather than `-mllvm=*` wildcard so unmodeled LLVM flags still refuse.",
        dialect: None,
    },
    // Compiler path-remapping flags: `-ffile-prefix-map` (= `-fdebug-prefix-map`
    // + `-fmacro-prefix-map`). Build systems pass these to make the OBJECT
    // path-portable — e.g. Firefox's `--enable-path-remapping` emits
    // `-fdebug-prefix-map=<objdir>=/topobjdir/`, a `<srcdir>` map, and an SDK
    // map. Each is `<flag>=<from>=<to>`. Clang's `-###` captures them in the
    // resolved invocation, and kache normalizes every resolved token through
    // its own cc prefix maps before hashing — so a per-checkout `<from>`
    // (the objdir/srcdir) collapses to a sentinel (two clones → one key),
    // while a genuinely different `<to>`, or an unrelated `<from>` like the
    // SDK path (identical across clones), still differentiates correctly.
    //
    // Without these rows the entire compile refused ("unsupported flag(s):
    // -fdebug-prefix-map=…"), so a build enabling its OWN path remapping
    // silently disabled all cc caching (kunobi-ninja/kache: Firefox bench saw
    // 4090+ TUs pass through uncached). `CapturedByProbe`, not
    // `CapturedByPreprocessor`: `-fdebug-prefix-map` only rewrites debug-info
    // paths in the object (not the preprocessed text), so the preprocessor
    // hash would under-key it — the resolved `-###` token stream is what
    // captures the flag's full effect.
    FlagSpec {
        matcher: Matcher::Prefix("-ffile-prefix-map="),
        class: FlagClass::CapturedByProbe,
        source: "Build-system path remapping (e.g. Firefox --enable-path-remapping). Resolved-token hash captures it; per-checkout `from` normalized via cc prefix maps.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-fdebug-prefix-map="),
        class: FlagClass::CapturedByProbe,
        source: "Build-system debug-info path remapping. Resolved-token hash captures it; per-checkout `from` normalized via cc prefix maps.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-fmacro-prefix-map="),
        class: FlagClass::CapturedByProbe,
        source: "Build-system __FILE__ path remapping. Resolved-token hash captures it; per-checkout `from` normalized via cc prefix maps.",
        dialect: None,
    },
    FlagSpec {
        // `aws-lc-sys` passes this spelling for assembler sources because
        // `-fdebug-prefix-map` does not reach GNU as. GCC's `-###` output puts
        // it only on the separate `as` subprocess, while kache's probe hashes
        // the `cc1`/`cc1plus` subprocess. Key it directly rather than claiming
        // it is probe-captured. The anchored regex admits exactly one `-Wa`
        // sub-option: GCC splits commas into additional assembler arguments,
        // which must remain unsupported until modeled separately.
        matcher: Matcher::Regex(r"-Wa,--debug-prefix-map=[^,=]+=[^,]*"),
        class: FlagClass::RawKeyed,
        source: "Issue #644 — GNU assembler debug path remapping; raw-keyed because the cc1 probe omits the separate assembler subprocess.",
        dialect: Some(Dialect::Gnu),
    },
    // C++ ABI, RTTI, and exception flags (kunobi-ninja/kache#116).
    // Each row affects the resulting object materially — `-fno-rtti`
    // omits RTTI tables, `-fno-exceptions` skips exception-handling
    // tables, `-stdlib=libc++` vs `libstdc++` selects a different C++
    // standard library with different ABI defaults. Clang's `-###`
    // captures all of them in the resolved `-cc1` invocation, so the
    // cache key differentiates per-value via the resolved-tokens hash.
    //
    // Both the positive and negative forms are listed (`-frtti` /
    // `-fno-rtti`, `-fexceptions` / `-fno-exceptions`) because a build
    // may explicitly request either mode — they're conflicting and the
    // cache must distinguish them, which is automatic via the probe.
    FlagSpec {
        // `-stdlib=libc++` (clang default on macOS), `-stdlib=libstdc++`
        // (typical on Linux). Values are a small fixed set; the probe
        // resolves each into a distinct `-cc1` form.
        matcher: Matcher::Prefix("-stdlib="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #116 — C++ standard-library selector (libc++ / libstdc++).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fno-exceptions"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #116 — C++ exception mode (off).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fexceptions"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #116 — C++ exception mode (on).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fno-rtti"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #116 — C++ RTTI mode (off).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-frtti"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #116 — C++ RTTI mode (on).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fno-sized-deallocation"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #116 — C++ sized-deallocation (disabled).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-faligned-new"),
        class: FlagClass::CapturedByProbe,
        source: "RocksDB C++17 aligned new/delete (enabled); resolved cc1 tokens distinguish this from the default and -fno-aligned-new.",
        dialect: Some(Dialect::Gnu),
    },
    FlagSpec {
        matcher: Matcher::Exact("-fno-aligned-new"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #116 — C++ aligned new/delete (disabled).",
        dialect: None,
    },
    // ELF symbol-visibility defaults (Firefox bench evidence, post-#146).
    // `-fvisibility=hidden` and `-fvisibility-inlines-hidden` are pure-
    // codegen knobs that change the object's exported symbol table; same
    // source + same flag pair → same object bytes. Clang's `cc -###`
    // resolves each into a distinct `-cc1 -fvisibility hidden` /
    // `-fvisibility-inlines-hidden` token, so the resolved-tokens hash
    // differentiates them. Single highest-volume passthrough on a
    // Firefox warm build: 2987 of 3475 refused compiles came from this
    // pair (86% of the cc passthrough wall).
    //
    // Listed by `Exact` value (not `Prefix("-fvisibility=")`) so
    // unmodeled visibility modes (`default`, `protected`, `internal`)
    // still refuse — same conservative convention as the #116 cluster.
    FlagSpec {
        matcher: Matcher::Exact("-fvisibility=hidden"),
        class: FlagClass::CapturedByProbe,
        source: "Firefox bench evidence (post-#146) — symbol visibility default = hidden.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fvisibility-inlines-hidden"),
        class: FlagClass::CapturedByProbe,
        source: "Firefox bench evidence (post-#146) — inline-function visibility default = hidden.",
        dialect: None,
    },
    // (`-f[no-]semantic-interposition` — the SINGLE flag that degraded the first
    // LLVM CMake/Ninja bench to a 0% hit rate, on every Release TU — is now in
    // the sorted codegen-knob stem list above, both polarities.)
    // Target / arch / WASM / ObjC / section flags
    // (kunobi-ninja/kache#115). Each row affects the resulting object
    // materially — `--target=` changes the entire output architecture,
    // `-march=` picks a CPU baseline, `-msimd128` enables WASM SIMD,
    // section flags reshape the object layout. Clang's `cc -###`
    // resolves each into the `-cc1` token stream (target triple,
    // target-cpu, target-feature list, language mode, section options),
    // so the resolved-tokens hash differentiates per-value and a
    // cross-target hit can't serve a foreign object.
    //
    // These flags were previously in the refuse-list (catch-all "would
    // serve a foreign object" guard); the explicit classification
    // makes them safe via the probe, with the boundary tests pinning
    // adjacent / unmodeled cases.
    FlagSpec {
        // Sticky `--target=arm64-apple-macosx` / `--target=wasm32-wasi`
        // / `--target=aarch64-linux-gnu`. The probe resolves the
        // triple into a `-cc1 -triple <value>` token, so different
        // targets produce different keys.
        matcher: Matcher::Prefix("--target="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — cross-compilation target triple (sticky form).",
        dialect: None,
    },
    FlagSpec {
        // Separate-arg form: `-target <triple>`. The value classifies
        // as a positional (no leading `-`), so this row only needs to
        // accept the flag itself.
        matcher: Matcher::Exact("-target"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — cross-compilation target triple (separate-arg form).",
        dialect: None,
    },
    FlagSpec {
        // `-march=` family: `native`, `armv8-a`, `armv8.2-a+dotprod`,
        // `armv8.2-a+i8mm`, etc. The probe captures the resolved
        // `-target-cpu` and `-target-feature` list, so `native` on
        // host A vs host B produces different keys (correct: they're
        // different objects).
        matcher: Matcher::Prefix("-march="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — architecture selection. `Prefix` is safe because the probe resolves the value into target-cpu/target-feature tokens.",
        dialect: None,
    },
    // ── Cross-target ABI / ISA-state selectors (issue #823) ──
    //
    // The Rust `cc` crate injects these on its own for cross targets:
    // `-mabi=` (riscv), `-mfloat-abi=` + `-mfpu=` (arm), `-mthumb` (thumbv*)
    // — so refusing any one of them makes every cross-compiled C TU on that
    // architecture uncacheable, not just hand-written invocations.
    //
    // Each is `RawKeyed`, deliberately NOT `CapturedByProbe` like their
    // `-march=` neighbour. Every value in these sets is a closed enumeration
    // with no host-relative member (there is no `-mabi=native` the way
    // `-mtune=`/`-mcpu=` have one), so the flag text alone determines the
    // object — the exact property a verbatim fold keys. Clang does lower
    // them into the resolved cc1 line (verified: `-### --target=
    // riscv64-unknown-linux-gnu -mabi=lp64d` yields `"-target-abi" "lp64d"`
    // against `"lp64"` for `-mabi=lp64`), so the probe would key them on
    // clang — but probe-captured soundness would rest on every supported
    // driver spelling the effect into that line, and a driver that omitted
    // it would serve one ABI's object to the other: a broken binary, not a
    // missed hit. The verbatim fold cannot do that.
    FlagSpec {
        // Target ABI: `lp64d`/`lp64`/`ilp32` on riscv, `ms`/`sysv` on
        // x86-64, `aapcs-linux` on arm.
        matcher: Matcher::Prefix("-mabi="),
        class: FlagClass::RawKeyed,
        source: "Issue #823 — target ABI selection; closed value set, keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        // Float ABI: `soft` / `softfp` / `hard`. cc-rs injects
        // `-mfloat-abi=hard` for every *eabihf target.
        matcher: Matcher::Prefix("-mfloat-abi="),
        class: FlagClass::RawKeyed,
        source: "Issue #823 — arm float ABI; closed value set (soft/softfp/hard), keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        // Code model: `tiny`/`small`/`kernel`/`medium`/`large` (x86,
        // aarch64), `medlow`/`medany` (riscv). Common in riscv firmware
        // and kernel builds.
        matcher: Matcher::Prefix("-mcmodel="),
        class: FlagClass::RawKeyed,
        source: "Issue #823 — code model; closed value set, keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        // Concrete arm FPU names — the standardized set gcc and clang
        // both document. cc-rs injects `vfpv3-d16` / `vfp` / `neon`.
        //
        // `-mfpu=auto` is DELIBERATELY not matched and keeps refusing: it
        // resolves from `-march`/`-mcpu` (and the toolchain's configured
        // default when those are absent) inside cc1, so its text does not
        // determine the object, and gcc's driver passes the literal `auto`
        // to cc1 — the probe cannot capture the resolution either.
        matcher: Matcher::Regex(
            r"-mfpu=(?:none|vfp|vfpv2|vfpv3(?:-fp16|-d16(?:-fp16)?|xd(?:-fp16)?)?|vfpv4(?:-d16)?|fpv4-sp-d16|fpv5-(?:sp-)?d16|fp-armv8(?:-fullfp16)?|neon(?:-fp16|-vfpv3|-vfpv4|-fp-armv8)?|crypto-neon-fp-armv8)",
        ),
        class: FlagClass::RawKeyed,
        source: "Issue #823 — concrete arm FPU selection; enumerated so `-mfpu=auto` (resolved inside cc1, not text-deterministic) still refuses.",
        dialect: None,
    },
    FlagSpec {
        // Instruction-set state: Thumb vs ARM encoding, both polarities —
        // conflicting occurrences are last-one-wins, which the raw fold
        // preserves because it hashes in argv order with duplicates kept.
        // Valueless and text-deterministic; cc-rs injects `-mthumb` for
        // thumbv* targets.
        matcher: Matcher::Regex(r"-m(?:(?:no-)?thumb|arm)"),
        class: FlagClass::RawKeyed,
        source: "Issue #823 — arm/thumb instruction-set state, keyed verbatim in argv order.",
        dialect: None,
    },
    FlagSpec {
        // x87 vs SSE math. Closed set; no `native`/`auto`.
        matcher: Matcher::Regex(r"-mfpmath=(?:sse|387|both)"),
        class: FlagClass::RawKeyed,
        source: "Issue #826 — x87/SSE fp math; enumerated so unknown values still refuse.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Regex(r"-mtls-dialect=(?:gnu2?|trad|desc)"),
        class: FlagClass::RawKeyed,
        source: "Issue #826 — TLS dialect; closed value set, keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        // aarch64 pointer-auth / BTI / GCS. Combinations gcc documents;
        // `native`/`auto` are not in this set and keep refusing.
        matcher: Matcher::Regex(
            r"-mbranch-protection=(?:none|standard|pac-ret(?:\+leaf)?(?:\+b-key)?(?:\+bti)?|bti|gcs)",
        ),
        class: FlagClass::RawKeyed,
        source: "Issue #826 — aarch64 branch protection; enumerated closed set.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-mgeneral-regs-only"),
        class: FlagClass::RawKeyed,
        source: "Issue #826 — restrict to general registers; valueless, keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Regex(r"-mstack-protector-guard=(?:tls|global|sysreg)"),
        class: FlagClass::RawKeyed,
        source: "Issue #826 — stack-protector guard location; enumerated so host-relative values refuse.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-mstack-protector-guard-offset="),
        class: FlagClass::RawKeyed,
        source: "Issue #826 — stack-protector guard offset; numeric, keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-mstack-protector-guard-reg="),
        class: FlagClass::RawKeyed,
        source: "Issue #826 — stack-protector guard register name; keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-msimd128"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — WASM SIMD128 enable.",
        dialect: None,
    },
    FlagSpec {
        // x86 width / SIMD / ISA feature flags seen in Firefox passthroughs
        // (#375, extended). These materially change the object by selecting the
        // target width or enabled ISA features. The resolved `cc -###` stream
        // records the resulting target triple / `-target-feature` set, so the
        // cache key differentiates each value (and a `-mno-` form flips it).
        // The nightly Firefox bench passed media/codec TUs through on
        // `-mavx -mbmi2 -mf16c`; the set below covers the x86 codec ISA family
        // (libvpx/dav1d/aom). Enumerated explicitly (with optional `no-`)
        // rather than opening `-m*`, so the remaining value-taking knobs
        // (`-mtune=`, `-mcpu=`, `-mcmodel=`) and unmodeled `-m` flags still
        // refuse. `-mabi=` is modeled separately above.
        matcher: Matcher::Regex(
            r"^-m(?:no-)?(?:32|64|mmx|sse|sse2|sse3|ssse3|sse4|sse4\.1|sse4\.2|sse4a|avx|avx2|avxvnni|avx512[a-z0-9]+|fma|fma4|f16c|bmi|bmi2|abm|popcnt|lzcnt|aes|vaes|pclmul|vpclmulqdq|gfni|sha|movbe|rdrnd|rdseed|adx|fsgsbase|xsave|xsaveopt|xsavec|xsaves|prfchw|clflushopt|clwb|cldemote|fxsr)$",
        ),
        class: FlagClass::CapturedByProbe,
        source: "Issue #375 (extended, Firefox nightly bench) — x86 width + SIMD/ISA codec feature flags; resolved into target-cpu/target-feature tokens.",
        dialect: None,
    },
    // (`-f[no-]function-sections` / `-f[no-]data-sections` — #115 — are now in
    // the sorted codegen-knob stem list above, both polarities.)
    FlagSpec {
        // `-Wa,*` passes through to the assembler. Different `-Wa,*`
        // values do arbitrary assembler things — listed as `Exact` for
        // the specific Firefox value (per #115's evidence) so a wildcard
        // `Prefix("-Wa,")` doesn't silently accept unmodeled assembler
        // flags. `--noexecstack` sets a section flag on the object.
        matcher: Matcher::Exact("-Wa,--noexecstack"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — assembler: non-executable stack section flag. Listed by exact value rather than `-Wa,*` wildcard so unmodeled assembler flags still refuse.",
        dialect: None,
    },
    FlagSpec {
        // Separate-arg form: `-x <lang>`. Value is positional. The
        // probe resolves the language mode into the `-cc1` invocation.
        matcher: Matcher::Exact("-x"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — language override (separate-arg form).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Regex(r"-Wp,-M(?:M)?D(?:,[^,]+)?"),
        class: FlagClass::NoObjectEffect,
        source: "isolated depfile modelled by DepInfoSpec using the verified compiler family; mixed lists/options and unknown families refuse; bare GCC overwrites its source",
        dialect: Some(Dialect::Gnu),
    },
    FlagSpec {
        // Sticky language override forms. The parser records the
        // language for invocation shape, and the probe resolves the
        // language mode into the `-cc1` invocation. One regex row
        // covers the sticky forms while `-x <lang>` stays an exact
        // row because its language value is a separate argv token.
        matcher: Matcher::Regex(r"-x(?:c|c\+\+|objective-c|objective-c\+\+)"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 / flag audit — sticky language override forms.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fobjc-exceptions"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — Objective-C exception model.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fobjc-arc"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #115 — Objective-C ARC mode.",
        dialect: None,
    },
    // ── PreprocessorCaptured: cc -E -P expansion hash subsumes effect ──
    FlagSpec {
        matcher: Matcher::Prefix("-D"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-U"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-I"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("--sysroot"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-include"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    // `--include=<file>` / `--include <file>` — the long spellings of
    // `-include`. Same forced-include semantics, so the header's bytes land
    // in the `-E` expansion the key already hashes. `Prefix("--include=")`
    // (not a bare `--include` prefix) so the neighbouring `--include-*`
    // options (`--include-directory=`, `--include-with-prefix=`, …) keep
    // refusing until they are modeled on their own terms.
    //
    // aws-lc-sys drives its BoringSSL symbol-prefixing through the `=` form
    // (`builder/cc_builder.rs`: `--include=` for non-cl, `/FI` for cl), so
    // before this row 56 of its TUs passed through uncached, the `.a`
    // diverged per checkout, and the `extern:` content hash re-keyed the
    // whole rustls/TLS subtree above it (#580).
    FlagSpec {
        matcher: Matcher::Prefix("--include="),
        class: FlagClass::PreprocessorCaptured,
        source: "Issue #580 — aws-lc-sys boringssl_prefix_symbols forced include; `=` form of -include.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("--include"),
        class: FlagClass::PreprocessorCaptured,
        source: "Issue #580 — separated form of --include=<file>.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-imacros"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        // Separated (`-isystem <dir>`) and joined (`-isystem<dir>`) forms:
        // GCC and clang read both, and build scripts pass the joined one
        // through `cc::Build::flag`. Not a plain prefix: clang's
        // `-isystem-after <dir>` is another flag. GCC expands leading `=`
        // and `$SYSROOT` directory values relative to its sysroot; joined
        // values starting `=` or `$` stay unsupported until that resolution
        // is modeled by `cc_flag_dir_values`.
        matcher: Matcher::Regex(r"-isystem(?:[^-=$].*)?"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94; joined form excluding -isystem-* and sysroot-indirect values.",
        dialect: None,
    },
    FlagSpec {
        // clang-cl's `-isystem`: `/imsvc <dir>` or joined. The `/EP`
        // expansion keeps it, so the headers it finds are in the key.
        matcher: Matcher::Prefix("/imsvc"),
        class: FlagClass::PreprocessorCaptured,
        source: "clang-cl system include directory (cargo-xwin CRT and SDK headers).",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("-imsvc"),
        class: FlagClass::PreprocessorCaptured,
        source: "clang-cl system include directory, dash spelling.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("--rtlib="),
        class: FlagClass::RawKeyed,
        source: "runtime selection and compile diagnostics",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("--unwindlib="),
        class: FlagClass::RawKeyed,
        source: "unwind runtime selection and compile diagnostics",
        dialect: None,
    },
    FlagSpec {
        // No linker runs for `-c` or `-E`, but an unused-argument warning
        // can fail under `-Werror`. Key the spelling so a successful entry
        // without it cannot hide that failure. cargo-xwin puts
        // `-fuse-ld=lld-link` in every CFLAGS.
        matcher: Matcher::Prefix("-fuse-ld="),
        class: FlagClass::RawKeyed,
        source: "linker selection; unused-argument diagnostics can change compile success.",
        dialect: None,
    },
    FlagSpec {
        // Separated and joined (`-iquote<dir>`) forms, as for `-isystem`.
        matcher: Matcher::Regex(r"-iquote(?:[^=$].*)?"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94; joined form excluding sysroot-indirect values.",
        dialect: None,
    },
    FlagSpec {
        // Separated and joined (`-idirafter<dir>`) forms, as for `-isystem`.
        matcher: Matcher::Regex(r"-idirafter(?:[^=$].*)?"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94; joined form excluding sysroot-indirect values.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-isysroot"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-nostdinc"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-nostdinc++"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-undef"),
        class: FlagClass::PreprocessorCaptured,
        source: "PR #94",
        dialect: None,
    },
    // ── NoObjectEffect: diagnostics / dep-info / build mechanics ──
    //
    // Outcome gates come FIRST: classification is first-match, so the
    // `-Werror` / `-pedantic-errors` rows below must precede the general
    // diagnostics rows they would otherwise be shadowed by.
    FlagSpec {
        // `-Werror[=spec]` and `-Wno-error[=spec]`: escalate (or de-escalate)
        // warnings into/out of hard errors. They cannot change the object
        // bytes of a *successful* compile, but they change whether the
        // compile succeeds at all — and a hit replays success. Two
        // invocations differing only here must not share a key: an entry
        // stored under `-Wno-error=foo` must not serve green to a `-Werror`
        // build that `foo` should have failed (review finding #2). Both
        // spellings are RawKeyed so any combination keys distinctly.
        //
        // The tail is deliberately open-ended: every spelling that starts
        // `-Werror` / `-Wno-error` is about turning diagnostics into errors,
        // including the dashed legacy aliases GCC and clang still accept
        // (`-Werror-implicit-function-declaration`). Keying one of these
        // unnecessarily would only cost a hit; missing one serves a green
        // hit to a build that should have failed, so the row errs wide.
        matcher: Matcher::Regex(r"-W(no-)?error.*"),
        class: FlagClass::RawKeyed,
        source: "review #2 — outcome gate: -Werror/-Wno-error change success vs failure; keyed verbatim.",
        dialect: None,
    },
    FlagSpec {
        // `-pedantic-errors` is the error-flavored member of the -pedantic
        // family: unlike plain `-pedantic` (diagnostics-only) it makes
        // non-conforming code fail to compile. Keyed for the same reason as
        // `-Werror` above; the plain `-pedantic` prefix row further down
        // keeps its NoObjectEffect treatment.
        matcher: Matcher::Exact("-pedantic-errors"),
        class: FlagClass::RawKeyed,
        source: "review #2 — outcome gate: -pedantic-errors changes success vs failure.",
        dialect: None,
    },
    FlagSpec {
        // `-W*` warnings — the remaining, genuinely diagnostics-only
        // warning flags (`-Werror`/`-Wno-error` are keyed above). The
        // regex EXCLUDES `-Wl,*` / `-Wa,*` / `-Wp,*` (linker /
        // assembler / preprocessor passthrough forms that change the
        // resulting object); they need separate handling and aren't
        // covered here.
        matcher: Matcher::Regex(r"-W[^,]*"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94 — warnings. Regex excludes `-Wl,*`/`-Wa,*`/`-Wp,*` passthrough forms.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-w"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-pedantic"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-fdiagnostics-"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Prefix("-fno-diagnostics-"),
        class: FlagClass::NoObjectEffect,
        source: "CCP-05 — negated diagnostic formatting; keyed with the formatting flags",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fcolor-diagnostics"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-fno-color-diagnostics"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        // Forces ANSI color escapes in diagnostics regardless of TTY
        // detection — terminal coloring only, no object effect. A real
        // clang driver flag in both dialects (Firefox's Windows clang-cl
        // build passes it bare, not just `-Xclang` forwarded as #411
        // covered), with no clang-cl spelling collision. Same family as
        // `-fcolor-diagnostics` above. Re-added after #430's codegen-knob
        // refactor dropped it (issue #438; originally #425/#424).
        matcher: Matcher::Exact("-fansi-escape-codes"),
        class: FlagClass::NoObjectEffect,
        source: "Issue #424/#438 — Firefox/Windows bare diagnostics flag; ANSI color escapes only, no object effect.",
        dialect: None,
    },
    FlagSpec {
        // Dep-info generation: -MD, -MMD, -MF, -MT, -MQ, -MP, -MG.
        // All write the `.d` sidecar; none affect the object. Regex
        // captures the family; alternatives are equally tight in this
        // table layout but the row stays declarative this way.
        matcher: Matcher::Regex(r"-MM?D|-M[FTQPG]"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94 — gcc dep-info flags. Gnu-dialect ONLY: in cl mode -MD/-MT/-MTd/-MDd are CRT selection (codegen), -MP is multi-process; they must not classify as inert dep-info (issue #285).",
        dialect: Some(Dialect::Gnu),
    },
    FlagSpec {
        matcher: Matcher::Exact("-o"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-P"),
        class: FlagClass::NoObjectEffect,
        source: "Flag audit — preprocessor line-marker suppression has no compile-mode object effect.",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("-pipe"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec { matcher: Matcher::Exact("-fno-lto"), class: FlagClass::RawKeyed, source: "LTO mode", dialect: None },
    FlagSpec {
        matcher: Matcher::Exact("-v"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("--verbose"),
        class: FlagClass::NoObjectEffect,
        source: "PR #94",
        dialect: None,
    },
    // Clang argument-wrapper flags (kunobi-ninja/kache#117). These
    // bracket a section of the command line where clang suppresses
    // unused-argument warnings; they only affect diagnostics, never
    // the resulting object. Listed as `Exact` (not a paired/regional
    // matcher) because each flag classifies independently for caching
    // purposes — kache doesn't care whether they appear together.
    FlagSpec {
        matcher: Matcher::Exact("--start-no-unused-arguments"),
        class: FlagClass::NoObjectEffect,
        source: "Issue #117 — clang unused-argument warning region (open).",
        dialect: None,
    },
    FlagSpec {
        matcher: Matcher::Exact("--end-no-unused-arguments"),
        class: FlagClass::NoObjectEffect,
        source: "Issue #117 — clang unused-argument warning region (close).",
        dialect: None,
    },
    // ── clang-cl flag classification (#285) ──────────────────────
    //
    // All rows below carry `dialect: Some(Dialect::Cl)` — they apply
    // exclusively to clang-cl invocations. Gnu/clang dialect behaviour
    // is unchanged.

    // NoObjectEffect — output path and conformance flags that do not
    // affect the resulting object bytes.
    FlagSpec {
        // `-Fo<obj>` / `/Fo<obj>` — object output path. Classified here
        // so the classifier gate doesn't refuse it; the parser extracts
        // it into `CcArgs.output` (Artifact bucket).
        matcher: Matcher::Prefix("-Fo"),
        class: FlagClass::NoObjectEffect,
        source: "Issue #285 — clang-cl object output path, no object-content effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/Fo"),
        class: FlagClass::NoObjectEffect,
        source: "Issue #285 — clang-cl object output path, no object-content effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        // `-Zc:inline` / `/Zc:inline` — clang-cl ignores this flag
        // entirely in cc1; confirmed via -### that it generates no cc1
        // token and produces no object-content difference.
        matcher: Matcher::Exact("-Zc:inline"),
        class: FlagClass::NoObjectEffect,
        source: "Issue #285 — clang-cl ignores -Zc:inline (no cc1 token, no object effect).",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Zc:inline"),
        class: FlagClass::NoObjectEffect,
        source: "Issue #285 — clang-cl ignores /Zc:inline (no cc1 token, no object effect).",
        dialect: Some(Dialect::Cl),
    },
    // ModeledInKey — language standard is extracted by the parser into
    // `CcArgs.std` and folded directly into the cache key.
    FlagSpec {
        matcher: Matcher::Prefix("-std:"),
        class: FlagClass::ModeledInKey,
        source: "Issue #285 — clang-cl language standard (-std:c++NN); modeled in key via CcArgs.std.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/std:"),
        class: FlagClass::ModeledInKey,
        source: "Issue #285 — clang-cl language standard (/std:c++NN); modeled in key via CcArgs.std.",
        dialect: Some(Dialect::Cl),
    },
    // PreprocessorCaptured — forced-include headers enter the /EP
    // preprocessor hash, so their content is already in the key.
    FlagSpec {
        matcher: Matcher::Prefix("-FI"),
        class: FlagClass::PreprocessorCaptured,
        source: "Issue #285 — clang-cl forced include; content captured by /EP preprocessor hash.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/FI"),
        class: FlagClass::PreprocessorCaptured,
        source: "Issue #285 — clang-cl forced include; content captured by /EP preprocessor hash.",
        dialect: Some(Dialect::Cl),
    },
    // CapturedByProbe — keyed via the clang-cl -### resolved-token
    // stream. Different values produce different cc1 tokens → different
    // keys. Safe only when the probe resolves; `cc_flags_need_resolved_invocation`
    // ensures the key refuses if the probe is unavailable.
    FlagSpec {
        // `-guard:cf` / `-guard:cf,nochecks` / `/guard:cf` etc. The
        // `Prefix` wildcard is intentional and safe here BECAUSE this is
        // CapturedByProbe: clang-cl -### resolves each guard variant into
        // a distinct -cc1 token (e.g. `-cfguard`) so the key differentiates
        // per-value automatically. An unrecognized guard variant still
        // produces a distinct -### token → distinct key (no miscache risk).
        matcher: Matcher::Prefix("-guard:"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl Control Flow Guard. Prefix wildcard safe: -### resolves each variant to a distinct -cc1 token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/guard:"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl Control Flow Guard (/guard: spelling). Prefix wildcard safe: -### resolves each variant to a distinct -cc1 token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        // `-fms-compatibility-version=<ver>` — MSVC version emulation.
        // `Prefix` wildcard is safe here BECAUSE this is CapturedByProbe:
        // clang-cl -### reflects the exact version number into a -cc1
        // token, so different versions produce different keys.
        matcher: Matcher::Prefix("-fms-compatibility-version="),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — MSVC compatibility version. Prefix wildcard safe: -### reflects the exact version into a -cc1 token.",
        dialect: Some(Dialect::Cl),
    },
    // Function/global inlining and frame-pointer optimizations.
    FlagSpec {
        matcher: Matcher::Exact("-Gy"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl function-level linking (COMDAT). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Gy"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl function-level linking (COMDAT). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-Gw"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl global data optimization (COMDAT). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Gw"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl global data optimization (COMDAT). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-Oy"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl omit frame pointer (MSVC spelling of -fomit-frame-pointer). Keyed via -### resolved tokens. -Oy- was modeled in #285; Firefox Windows uses the enable polarity.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Oy"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl omit frame pointer (slash spelling). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-Oy-"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl frame-pointer omission disabled. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-Zl"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl omit default library name from the object (.drectve). Firefox Windows nightly. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Zl"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl omit default library name from the object (.drectve). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("-fp:"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl floating-point model (-fp:fast/precise/strict). Firefox Windows nightly. Prefix safe: -### reflects the model into cc1.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/fp:"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl floating-point model (/fp:fast/precise/strict). Prefix safe: -### reflects the model into cc1.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/clang:"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl /clang:<flag> forwards a clang driver flag. Firefox Windows uses /clang:-fno-finite-math-only. Prefix safe: -### captures the forwarded token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("-clang:"),
        class: FlagClass::CapturedByProbe,
        source: "clang-cl -clang:<flag> dash spelling of /clang:. Prefix safe: -### captures the forwarded token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/WX"),
        class: FlagClass::RawKeyed,
        source: "clang-cl warnings-as-errors (/WX). Outcome gate, same contract as -Werror.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-WX"),
        class: FlagClass::RawKeyed,
        source: "clang-cl warnings-as-errors (-WX). Outcome gate, same contract as -Werror.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Regex(r"/W(all|[0-4])"),
        class: FlagClass::NoObjectEffect,
        source: "clang-cl warning level (/W0-/W4, /Wall). Diagnostics only. Dash -Wall is already covered by the dialect-agnostic -W* row.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Oy-"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl frame-pointer omission disabled. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    // CRT (C Runtime) selection flags. Under Cl these are codegen flags
    // (they define `_MT`/`_DLL` macros and link the appropriate CRT),
    // not dep-info markers (cf. the Gnu-dialect row above which tags
    // the same spellings as NoObjectEffect).
    FlagSpec {
        matcher: Matcher::Exact("-MD"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded DLL (dynamic). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-MDd"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded DLL debug. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-MT"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded static. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-MTd"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded static debug. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/MD"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded DLL (dynamic). Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/MDd"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded DLL debug. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/MT"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded static. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/MTd"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl CRT: multithreaded static debug. Keyed via -### resolved tokens.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        // clang-cl optimization levels, `-` and `/` spellings, keyed via
        // the cc1 level in `-###`. NOTE: the bare `-O1`/`-O2` spellings
        // are already matched by the earlier dialect-agnostic `-O[0-3sz]?`
        // ModeledInKey row (→ `parsed.optimization`), so this row only
        // fires for `/O1`/`/O2`/`-Od`/`/Od`/`-Ox`/`/Ox`. Both paths key
        // the level, just by different mechanisms. `-Os`/`-Oz`/`-Ofast`
        // are not in this set and still refuse.
        matcher: Matcher::Regex(r"[-/]O[12dx]"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #285 — clang-cl optimization levels. Bare -O1/-O2 are caught earlier as ModeledInKey; this row covers /Onn and -Od/-Ox. -### resolves each to a distinct cc1 level. -Ofast/-Os/-Oz not matched → still refuse.",
        dialect: Some(Dialect::Cl),
    },
    // ── clang-cl Layer 4: Firefox-corpus flag classification (#285) ──
    //
    // All rows carry `dialect: Some(Dialect::Cl)`. Gnu/clang dialect
    // behaviour is unchanged.

    // ── CapturedByProbe: exception / RTTI / stack-protector / misc codegen ──
    //
    // Each row's effect is reflected in the clang-cl -### token stream:
    //   -EHsc   → -fexceptions + -fcxx-exceptions
    //   -GR-    → -fno-rtti
    //   -GS-    → removes -stack-protector from cc1
    //   -Brepro → removes -mincremental-linker-compatible from cc1
    //   -utf-8  → clang-cl is UTF-8 by default; the flag is inert (no cc1 token)
    //             but remains CapturedByProbe — keyed if it ever produces a token,
    //             inert if not; the probe requirement is always met for clang-cl.
    //   -Zc:*   → various conformance knobs reflected into cc1 tokens per value.
    //             NOTE: `-Zc:inline` / `/Zc:inline` are listed BEFORE these Prefix
    //             rows (exact rows appear earlier in the table) so they continue to
    //             resolve as NoObjectEffect via first-match.
    // `Prefix` wildcards on CapturedByProbe are safe: the -### stream captures the
    // exact value (or the flag is inert), so an unknown suffix still produces a
    // distinct key (no miscache risk).
    // clang-cl source-language override: `-TP`/`/TP` force every input to
    // compile as C++, `-TC`/`/TC` force C (MSVC `/TP` / `/TC`). This
    // changes the language the front end uses (and thus the object), but
    // clang-cl resolves it into the `-cc1 -x c++` / `-x c` token, so the
    // resolved-token hash differentiates it — the same treatment as the
    // gnu `-x <lang>` override. (Issue #411 — Firefox/Windows compiles
    // `Unified_cpp_*.cpp` with `-TP`.)
    FlagSpec {
        matcher: Matcher::Exact("-TP"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #411 — clang-cl force-C++ source mode (-TP). -### resolves it into the -cc1 -x c++ token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/TP"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #411 — clang-cl force-C++ source mode (/TP). -### resolves it into the -cc1 -x c++ token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-TC"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #411 — clang-cl force-C source mode (-TC). -### resolves it into the -cc1 -x c token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/TC"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #411 — clang-cl force-C source mode (/TC). -### resolves it into the -cc1 -x c token.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("-EH"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl exception model (-EHsc, -EHs-c-, …). Prefix safe: -### resolves each variant into distinct -fexceptions/-fcxx-exceptions tokens (or their negations).",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/EH"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl exception model (/EH* spelling). Prefix safe: -### resolves each variant into distinct -fexceptions/-fcxx-exceptions tokens (or their negations).",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-GR"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl RTTI enabled (-GR). -### reflects -frtti.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-GR-"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl RTTI disabled (-GR-). -### reflects -fno-rtti.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/GR"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl RTTI enabled (/GR). -### reflects -frtti.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/GR-"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl RTTI disabled (/GR-). -### reflects -fno-rtti.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-GS"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl stack-buffer-security-check enabled (-GS). -### reflects -stack-protector.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-GS-"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl stack-buffer-security-check disabled (-GS-). -### removes -stack-protector.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/GS"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl stack-buffer-security-check enabled (/GS). -### reflects -stack-protector.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/GS-"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl stack-buffer-security-check disabled (/GS-). -### removes -stack-protector.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-Brepro"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl reproducible build (-Brepro). -### removes -mincremental-linker-compatible.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Brepro"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl reproducible build (/Brepro). -### removes -mincremental-linker-compatible.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("-utf-8"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl UTF-8 source/execution charset (-utf-8). clang-cl is UTF-8 by default; the flag produces no cc1 token but is inert — CapturedByProbe is safe (keyed if token present, inert if not; probe always resolves for clang-cl).",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/utf-8"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl UTF-8 source/execution charset (/utf-8). Same rationale as -utf-8.",
        dialect: Some(Dialect::Cl),
    },
    // -Zc: conformance flags. The Exact rows for -Zc:inline / /Zc:inline
    // appear EARLIER in the table and resolve first (NoObjectEffect), so
    // only non-inline -Zc: values reach these Prefix rows.
    FlagSpec {
        matcher: Matcher::Prefix("-Zc:"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl conformance flags (-Zc:wchar_t, -Zc:forScope, …). Prefix safe: -### captures the exact value (or flag is inert); placed AFTER the -Zc:inline Exact row so that spelling resolves NoObjectEffect first.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/Zc:"),
        class: FlagClass::CapturedByProbe,
        source: "#285 Layer 4 — clang-cl conformance flags (/Zc:* spelling). Prefix safe: -### captures the exact value (or flag is inert); placed AFTER the /Zc:inline Exact row so that spelling resolves NoObjectEffect first.",
        dialect: Some(Dialect::Cl),
    },
    // ── PreprocessorCaptured: -FC makes __FILE__ expand to the full path ──
    //
    // clang accepts `-FC` and ignores it: `__FILE__` keeps the argv
    // spelling (checked with clang 23), and the key folds that spelling.
    FlagSpec {
        matcher: Matcher::Exact("-FC"),
        class: FlagClass::PreprocessorCaptured,
        source: "#285 Layer 4 — clang-cl full-path __FILE__ (-FC). MSVC makes __FILE__ absolute; clang accepts the flag and ignores it.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/FC"),
        class: FlagClass::PreprocessorCaptured,
        source: "#285 Layer 4 — clang-cl full-path __FILE__ (/FC). MSVC makes __FILE__ absolute; clang accepts the flag and ignores it.",
        dialect: Some(Dialect::Cl),
    },
    // ── NoObjectEffect: diagnostics / build mechanics ──
    //
    // None of these flags change the resulting object bytes.
    FlagSpec {
        matcher: Matcher::Exact("-nologo"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl suppress banner (-nologo). Pure build-output mechanic; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/nologo"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl suppress banner (/nologo). Pure build-output mechanic; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        // `-wdNNNN` / `/wdNNNN` — disable a specific warning by number.
        // Warnings only affect diagnostics, never the object.
        matcher: Matcher::Prefix("-wd"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl disable warning (-wdNNNN). Diagnostics only; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/wd"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl disable warning (/wdNNNN). Diagnostics only; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        // `-FS` / `/FS` — force synchronous PDB writes (serializes access
        // to the shared .pdb across parallel compilations). Pure build
        // mechanic; has no effect on the object file content.
        matcher: Matcher::Exact("-FS"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl force synchronous PDB writes (-FS). Build mechanic; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/FS"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl force synchronous PDB writes (/FS). Build mechanic; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        // `-Gm-` / `/Gm-` — disable minimal rebuild (deprecated MSVC flag).
        // Has no effect on the object content.
        matcher: Matcher::Exact("-Gm-"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl minimal rebuild disabled (-Gm-, deprecated). No object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Exact("/Gm-"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl minimal rebuild disabled (/Gm-, deprecated). No object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        // `-external:W<n>` / `/external:W<n>` and similar external-header
        // warning-level flags. These only affect diagnostics for headers
        // treated as "external" (system headers); no object effect.
        matcher: Matcher::Prefix("-external:"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl external-header warning level (-external:*). Diagnostics only; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    FlagSpec {
        matcher: Matcher::Prefix("/external:"),
        class: FlagClass::NoObjectEffect,
        source: "#285 Layer 4 — clang-cl external-header warning level (/external:*). Diagnostics only; no object effect.",
        dialect: Some(Dialect::Cl),
    },
    // ── clang-cl debug-info flags (#312) ─────────────────────────
    //
    // MSVC `/Z7`/`/Zi`/`/ZI`/`/Zd` and their `-` spellings embed
    // CodeView debug info in the object. Their codegen effect is in the
    // `-cc1` line (`-gcodeview`, `-debug-info-kind`, edit-and-continue),
    // so the variant is keyed via the `cc -###` resolved tokens —
    // `CapturedByProbe`. This also enforces the safety contract: if the
    // probe is unavailable the compile bails rather than under-keying
    // (box-confirmed: `/Z7` and `/Zi` resolve identically and produce
    // identical objects, but `/ZI` differs). The PATH inputs the debug
    // object embeds (source/output/compilation-dir) are a separate
    // concern, folded into the key by `cl_debug_path_inputs`.
    FlagSpec {
        matcher: Matcher::Regex(r"[-/]Z[7iId]"),
        class: FlagClass::CapturedByProbe,
        source: "Issue #312 — clang-cl CodeView debug-info flags; variant captured via cc -### resolved tokens; embedded paths folded by cl_debug_path_inputs.",
        dialect: Some(Dialect::Cl),
    },
];

#[derive(Debug, Default)]
struct FlagClassificationSummary {
    modeled_in_key: usize,
    raw_keyed: usize,
    captured_by_probe: usize,
    preprocessor_captured: usize,
    no_object_effect: usize,
    parser_handled: usize,
    /// Unmodeled by the built-in table but opted into caching via the
    /// user's `[cc] extra_allowlist_flags` allow-list (issue #95).
    user_allowed: usize,
    unmodeled: usize,
}

impl FlagClassificationSummary {
    fn record(&mut self, class: Option<FlagClass>) {
        match class {
            Some(FlagClass::ModeledInKey) => self.modeled_in_key += 1,
            Some(FlagClass::RawKeyed) => self.raw_keyed += 1,
            Some(FlagClass::CapturedByProbe) => self.captured_by_probe += 1,
            Some(FlagClass::PreprocessorCaptured) => self.preprocessor_captured += 1,
            Some(FlagClass::NoObjectEffect) => self.no_object_effect += 1,
            Some(FlagClass::ParserHandled) => self.parser_handled += 1,
            None => self.unmodeled += 1,
        }
    }
}

/// Classify the parsed flags, emitting per-flag and per-compile traces,
/// and return the tokens that should *refuse* (force passthrough).
///
/// `extra_allowlist_flags` is the user's allow-list (issue #95): a flag the
/// built-in table doesn't model is normally rejected, but if it exactly
/// matches an allow-list entry it is accepted instead (logged as
/// `user-allowed (config)`) and folded verbatim into the cache key by
/// [`CcCompiler::cache_key`].
fn classify_and_trace_cc_flags<'a>(
    parsed: &'a CcArgs,
    extra_allowlist_flags: &[String],
) -> Vec<&'a str> {
    let subject = parsed
        .sources
        .first()
        .map(|source| source.display().to_string())
        .unwrap_or_else(|| parsed.program.clone());
    let mut summary = FlagClassificationSummary::default();
    let mut rejected = Vec::new();

    let dialect = parsed.family.dialect();
    let mut args = parsed.rest.iter().enumerate();
    while let Some((idx, arg)) = args.next() {
        // `-Xclang <tok>` forwards `<tok>` straight to the cc1 front end,
        // so the driver-level table can't classify it — `<tok>` means
        // whatever cc1 makes of it (cc1 `-MP` is dep-info phony targets,
        // not the clang-cl driver's `/MP`). Classify the forwarded token
        // against the cc1 allow-list and consume the pair, so its value
        // (itself a separate `-Xclang <value>`) isn't reprocessed.
        // Issue #411 (Firefox/Windows).
        if arg == "-Xclang" {
            // The marker only forwards; it has no object effect itself.
            summary.record(Some(FlagClass::NoObjectEffect));
            let Some((_, inner)) = args.next() else {
                // Trailing `-Xclang` with no operand: nothing to forward.
                log::trace!("[cc:{subject}] flag -Xclang (no operand) -> NoObjectEffect");
                continue;
            };
            let class = classify_xclang_forwarded(inner, dialect);
            summary.record(class);
            match class {
                Some(class) => log::trace!(
                    "[cc:{subject}] flag -Xclang {inner} -> {class:?} [forwarded cc1]"
                ),
                None if extra_allowlist_flags.iter().any(|f| f == inner) => {
                    summary.user_allowed += 1;
                    log::trace!("[cc:{subject}] flag -Xclang {inner} -> user-allowed (config)");
                }
                None => {
                    log::trace!(
                        "[cc:{subject}] flag -Xclang {inner} -> unmodeled [forwarded cc1]"
                    );
                    rejected.push(inner.as_str());
                }
            }
            continue;
        }

        if arg == "-mllvm" {
            let value = args.next().map(|(_, value)| value.as_str());
            if value.is_some_and(cc_supported_mllvm_value) {
                summary.record(Some(FlagClass::RawKeyed));
            } else {
                summary.record(None);
                rejected.push(arg.as_str());
            }
            continue;
        }

        let analysis = analyze_cc_arg(arg, dialect);
        summary.record(analysis.class);
        match analysis.class {
            Some(class) => log::trace!(
                "[cc:{subject}] flag {arg} -> {class:?} [{:?}]",
                analysis.bucket
            ),
            None if extra_allowlist_flags.iter().any(|f| f == arg) => {
                summary.user_allowed += 1;
                log::trace!(
                    "[cc:{subject}] flag {arg} -> user-allowed (config) [verbatim-keyed]"
                );
            }
            None => {
                log::trace!(
                    "[cc:{subject}] flag {arg} -> unmodeled [{:?}]",
                    analysis.bucket
                );
                rejected.push(arg.as_str());
            }
        }
        if dialect == Dialect::Cl && arg == "--" {
            // clang-cl reads everything after `--` as an input file, which
            // the `cc` crate relies on for sources with absolute Unix paths
            // (`/home/...`) that would otherwise spell flags.
            // Only the parsed sources are tracked by the compile key.
            // An extra input such as LLVM IR must not bypass the gate.
            for (_, input) in args.by_ref() {
                if !parsed
                    .sources
                    .iter()
                    .any(|source| source.as_os_str() == OsStr::new(input))
                {
                    summary.record(None);
                    log::trace!("[cc:{subject}] untracked input after --: {input}");
                    rejected.push(input.as_str());
                }
            }
            break;
        }
        if takes_separate_value(&parsed.rest, idx, dialect)
            && let Some((_, value)) = args.next()
        {
            log::trace!("[cc:{subject}] value {value} of {arg} -> consumed by the flag");
        }
    }

    if !parsed.rest.is_empty() {
        log::debug!(
            "[cc:{subject}] flag classify: {} modeled / {} raw-keyed / {} probe / {} preprocessor / {} no-effect / {} parser-handled / {} user-allowed / {} unmodeled",
            summary.modeled_in_key,
            summary.raw_keyed,
            summary.captured_by_probe,
            summary.preprocessor_captured,
            summary.no_object_effect,
            summary.parser_handled,
            summary.user_allowed,
            summary.unmodeled
        );
    }

    rejected
}

/// Select the user-declared flags (issue #95) to fold verbatim into the
/// cache key: the command-line tokens that (a) match an allow-list entry
/// exactly and (b) the built-in table does NOT model — i.e. exactly the
/// "user-allowed" set from [`classify_and_trace_cc_flags`]. Sorted +
/// deduped so argv order and repeated flags never perturb the key, and a
/// configured-but-absent flag is excluded (it has no codegen effect).
fn cc_extra_flags_for_key<'a>(
    parsed: &'a CcArgs,
    extra_allowlist_flags: &[String],
) -> Vec<&'a str> {
    if extra_allowlist_flags.is_empty() {
        return Vec::new();
    }
    let dialect = parsed.family.dialect();
    let mut matched: Vec<&str> = parsed
        .rest
        .iter()
        .map(String::as_str)
        .filter(|arg| {
            classify_cc_flag(arg, dialect).is_none()
                && extra_allowlist_flags.iter().any(|f| f == arg)
        })
        .collect();
    matched.sort_unstable();
    matched.dedup();
    matched
}

const WA_DEBUG_PREFIX_MAP_PREFIX: &str = "-Wa,--debug-prefix-map=";

/// Built-in flags whose object effect is not present in the resolved `cc1`
/// stream and therefore must be folded directly into the cache key.
///
/// Preserve argv order and duplicates: assembler option precedence may be
/// order-sensitive. For GNU as's `--debug-prefix-map=OLD=NEW`, normalize only
/// the location-dependent `OLD`; `NEW` is object material and stays verbatim.
fn cc_supported_mllvm_value(value: &str) -> bool {
    ["-instcombine-lower-dbg-declare=", "-split-threshold-for-reg-with-hint=", "-inlinehint-threshold="]
        .iter().any(|prefix| value.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
}

fn cc_raw_flags_for_key(parsed: &CcArgs, prefix_maps: &[CcPrefixMap]) -> Vec<Vec<u8>> {
    let dialect = parsed.family.dialect();
    let mut flags = Vec::new();
    if !parsed.response_file_digests.is_empty() {
        flags.push(format!("response-files.v1={:?}", parsed.response_file_digests).into_bytes());
    }
    if parsed.records_switches() || parsed.emits_llvm() || parsed.emits_reports() {
        flags.push(format!("recorded.argv.v1={:?}", (&parsed.program, &parsed.rest, parsed.invocation_env.cwd(), file_prefix_map_args(prefix_maps))).into_bytes());
    }
    let mut args = parsed.rest.iter();
    while let Some(arg) = args.next() {
        if arg == "-mllvm" || arg.starts_with("-Xarch_") {
            if let Some(value) = args.next() {
                flags.push(arg.as_bytes().to_vec());
                flags.push(value.as_bytes().to_vec());
            }
        } else if classify_cc_flag(arg, dialect) == Some(FlagClass::RawKeyed) {
            flags.push(normalize_raw_keyed_cc_flag(arg, prefix_maps));
        }
    }
    let mut trailing_language_flags = Vec::new();
    let mut idx = 0;
    while idx < parsed.rest.len() {
        if let Some(arg) = parse_cc_arg_at(&parsed.rest, idx, dialect) {
            if matches!(arg.spec.action, CcArgAction::LanguageOverride) {
                trailing_language_flags.extend(parsed.rest[idx..idx + arg.consumed].iter().map(|token| token.as_bytes().to_vec()));
            }
            idx += arg.consumed;
        } else {
            if !parsed.rest[idx].starts_with('-') || parsed.rest[idx] == "-" {
                trailing_language_flags.clear();
            }
            idx += if takes_separate_value(&parsed.rest, idx, dialect) { 2 } else { 1 };
        }
    }
    flags.extend(trailing_language_flags);
    flags
}

fn normalize_raw_keyed_cc_flag(arg: &str, prefix_maps: &[CcPrefixMap]) -> Vec<u8> {
    let Some(mapping) = arg.strip_prefix(WA_DEBUG_PREFIX_MAP_PREFIX) else {
        return arg.as_bytes().to_vec();
    };
    let Some((from, to)) = mapping.split_once('=') else {
        // The classifier rejects this malformed spelling, but keeping the
        // fallback lossless makes this helper safe if called independently.
        return arg.as_bytes().to_vec();
    };

    let from = apply_cc_prefix_maps_to_bytes(from.as_bytes().to_vec(), prefix_maps);
    let mut normalized = Vec::with_capacity(arg.len());
    normalized.extend_from_slice(WA_DEBUG_PREFIX_MAP_PREFIX.as_bytes());
    normalized.extend_from_slice(&from);
    normalized.push(b'=');
    normalized.extend_from_slice(to.as_bytes());
    normalized
}

fn normalize_resolved_cc_token(
    token: &str,
    previous: Option<&str>,
    prefix_maps: &[CcPrefixMap],
) -> Vec<u8> {
    let mapping = if previous.is_some_and(crate::probe::is_prefix_map_flag) {
        Some(("", token))
    } else {
        token.split_once('=').filter(|(flag, _)| crate::probe::is_prefix_map_flag(flag))
    };
    if let Some((flag, mapping)) = mapping {
        let Some((from, to)) = mapping.split_once('=') else {
            return token.as_bytes().to_vec();
        };
        let mut normalized = Vec::with_capacity(token.len());
        if !flag.is_empty() {
            normalized.extend_from_slice(flag.as_bytes());
            normalized.push(b'=');
        }
        normalized.extend_from_slice(&apply_cc_prefix_maps_to_bytes(from.as_bytes().to_vec(), prefix_maps));
        normalized.push(b'=');
        normalized.extend_from_slice(to.as_bytes());
        normalized
    } else {
        apply_cc_prefix_maps_to_bytes(token.as_bytes().to_vec(), prefix_maps)
    }
}

/// Whether `rest[idx]` is a flag that [`CcArgs::parse`] reads together with
/// the next token as its value (`-I <dir>`, `-o <path>`, `-D <name>`).
///
/// That token is the flag's value and is never classified as a flag of its
/// own. Under clang-cl a token starting with `/` is a flag spelling, so an
/// absolute Unix path in a separate `-I` value (cargo-xwin and other
/// clang-cl cross builds) would otherwise read as an unknown flag and refuse
/// the compile.
fn takes_separate_value(rest: &[String], idx: usize, dialect: Dialect) -> bool {
    parse_cc_arg_at(rest, idx, dialect).is_some_and(|arg| arg.consumed == 2)
}

fn analyze_cc_arg(arg: &str, dialect: Dialect) -> CcArgAnalysis<'_> {
    let class = classify_cc_flag(arg, dialect);
    let spec = cc_arg_spec_for_token(arg, dialect);
    CcArgAnalysis {
        arg,
        class,
        bucket: cc_arg_bucket(class, spec),
        normalized: normalize_cc_arg(arg, dialect),
        refusal: class.is_none().then_some("cc: unsupported flag"),
        source: spec.map(|spec| spec.source),
    }
}

fn cc_arg_bucket(class: Option<FlagClass>, spec: Option<&'static CcArgSpec>) -> CcArgBucket {
    if class.is_none() {
        return CcArgBucket::TooHard;
    }
    if let Some(spec) = spec {
        return spec.bucket;
    }
    match class {
        Some(FlagClass::ModeledInKey) => CcArgBucket::ModeledInKey,
        Some(FlagClass::RawKeyed) => CcArgBucket::RawKeyed,
        Some(FlagClass::ParserHandled) => CcArgBucket::Structural,
        Some(FlagClass::CapturedByProbe) => CcArgBucket::ProbeKeyed,
        Some(FlagClass::PreprocessorCaptured) => CcArgBucket::Preprocessor,
        Some(FlagClass::NoObjectEffect) => CcArgBucket::NoObjectEffect,
        None => CcArgBucket::TooHard,
    }
}

fn normalize_cc_arg(arg: &str, dialect: Dialect) -> Vec<String> {
    let Some(spec) = cc_arg_spec_for_token(arg, dialect) else {
        return vec![arg.to_string()];
    };
    match spec.value_form {
        CcArgValueForm::Flag | CcArgValueForm::Separated => vec![arg.to_string()],
        CcArgValueForm::Concatenated { prefix } => arg
            .strip_prefix(prefix)
            .map(|value| vec![prefix.to_string(), value.to_string()])
            .unwrap_or_else(|| vec![arg.to_string()]),
        CcArgValueForm::CanBeSeparated { prefix } => {
            if arg == prefix {
                vec![prefix.to_string()]
            } else {
                arg.strip_prefix(prefix)
                    .filter(|value| !value.is_empty())
                    .map(|value| vec![prefix.to_string(), value.to_string()])
                    .unwrap_or_else(|| vec![arg.to_string()])
            }
        }
    }
}

fn cc_arg_spec_for_token(arg: &str, dialect: Dialect) -> Option<&'static CcArgSpec> {
    CC_ARG_SPECS.iter().find(|spec| {
        // Skip rows restricted to a different dialect, so a token shared
        // across dialects (e.g. `-MT`) resolves to the row for the active
        // dialect — not whichever appears first in the table.
        if spec.dialect.is_some_and(|d| d != dialect) {
            return false;
        }
        match spec.value_form {
            CcArgValueForm::Flag | CcArgValueForm::Separated => cc_arg_spec_matches(spec, arg),
            CcArgValueForm::Concatenated { prefix } => arg.starts_with(prefix),
            CcArgValueForm::CanBeSeparated { prefix } => {
                arg == prefix
                    || arg
                        .strip_prefix(prefix)
                        .is_some_and(|value| !value.is_empty())
            }
        }
    })
}

/// Classify a cc argument. Wraps [`crate::compiler::flags::classify_against`]
/// over [`CC_FLAGS`] with a lazy regex cache. Returns `None` for any
/// argument no row matches — the caller treats that as "unsupported
/// flag, refuse to cache".
fn classify_cc_flag(arg: &str, dialect: Dialect) -> Option<FlagClass> {
    static CACHE: OnceLock<crate::compiler::flags::RegexCache> = OnceLock::new();
    crate::compiler::flags::classify_against(
        arg,
        CC_FLAGS,
        CACHE.get_or_init(|| crate::compiler::flags::build_regex_cache(CC_FLAGS)),
        dialect,
    )
}

/// cc1 frontend flags that clang's driver forwards verbatim via
/// `-Xclang <flag>`. Each is inert for object-content caching — dep-info
/// sidecar emission or terminal diagnostics — so an `-Xclang`-wrapped one
/// is safe to cache past. A forwarded flag NOT on this list still refuses
/// (see [`classify_xclang_forwarded`]), so an `-Xclang`-wrapped *codegen*
/// flag can never slip through. Sourced from Firefox's Windows build
/// (issue #411).
///
/// These are deliberately classified only in forwarded (`-Xclang`)
/// position, NOT as bare driver flags: e.g. cc1 `-MP` means "emit phony
/// dep targets", whereas the clang-cl driver's `/MP` is multi-process
/// compilation — different flags that happen to share a spelling.
const XCLANG_INERT_CC1_FLAGS: &[&str] = &[
    // ── dep-info sidecar family ──────────────────────────────────────
    // Every one of these only shapes the `.d`/`.pp` dependency sidecar;
    // none changes a byte of the object. They are listed as a *family*,
    // not just the two flags issue #411 happened to surface, because they
    // co-occur: cc1 rejects `-dependency-file` unless a `-MT`/`-MQ` target
    // accompanies it, so a build that forwards one forwards several. (cc1
    // spellings — distinct from the clang-cl driver's `/MT` CRT-selection
    // flag, which is why these are recognized only in forwarded position.)
    "-dependency-file", // write the dep sidecar (path is a separate -Xclang value)
    "-MT",              // dependency target name
    "-MQ",              // dependency target name, quoted for make
    "-MP",              // emit a phony target per header
    "-MG",              // tolerate missing (generated) headers
    "-MV",              // NMake/Visual Studio style dependency output
    "-sys-header-deps", // include system headers in the dep output
    "-module-file-deps", // include module files in the dep output
    "-dependency-dot",  // write DOT-format header deps (path is a separate value)
    // ── terminal diagnostics ─────────────────────────────────────────
    "-fansi-escape-codes", // emit ANSI color escapes regardless of TTY detection
];

/// Classify a single cc1 token forwarded to the front end via `-Xclang`.
///
/// Returns `NoObjectEffect` for an allow-listed inert cc1 flag
/// ([`XCLANG_INERT_CC1_FLAGS`]) or for a bare value token — e.g. the
/// dependency-file path, itself forwarded as its own `-Xclang <value>`
/// pair.
///
/// A forwarded flag that matches a modeled `CapturedByProbe` codegen knob
/// (`-Xclang -ffp-contract=off`, the Firefox/Windows clang-cl shape of #428)
/// is allowed and keyed via the probe: `-Xclang` forwards it to cc1, where the
/// `cc -###` resolved cc1 line records it (verified: `-Xclang -ffp-contract=off`
/// appends `-ffp-contract=off` to the dump), so the cache key already
/// differentiates it from the default. This is safe, NOT "blind": the bare
/// operand token also classifies as `ProbeKeyed` at the driver level, so
/// `cc_flags_need_resolved_invocation` forces the probe and refuses to cache if
/// it cannot resolve — there is no under-keying path. An UNMODELED `-Xclang`
/// codegen flag (not in `CC_FLAGS`) still returns `None` and refuses.
fn classify_xclang_forwarded(inner: &str, dialect: Dialect) -> Option<FlagClass> {
    if !inner.starts_with('-') {
        // A value token (e.g. the dependency-file path). No object effect;
        // its content is irrelevant to the resulting `.obj`.
        return Some(FlagClass::NoObjectEffect);
    }
    // Inert cc1 flags FIRST: a forwarded `-MT`/`-MP`/… is the cc1 dep-info flag,
    // which must NOT be confused with the clang-cl driver flag of the same
    // spelling (`/MT` = CRT selection, a CapturedByProbe codegen knob). The
    // driver-level `classify_cc_flag` below would misread the cc1 spelling as
    // the driver flag, so the forwarded-position allow-list wins.
    if XCLANG_INERT_CC1_FLAGS.contains(&inner) {
        return Some(FlagClass::NoObjectEffect);
    }
    if classify_cc_flag(inner, dialect) == Some(FlagClass::CapturedByProbe) {
        return Some(FlagClass::CapturedByProbe);
    }
    None
}

fn cc_flags_need_resolved_invocation(parsed: &CcArgs) -> bool {
    let dialect = parsed.family.dialect();
    // Any driver-level probe-keyed flag forces the resolved `cc -###`
    // invocation so the key captures its codegen effect.
    if parsed
        .rest
        .iter()
        .any(|arg| analyze_cc_arg(arg, dialect).bucket == CcArgBucket::ProbeKeyed)
    {
        return true;
    }
    // A `-Xclang`-forwarded CapturedByProbe knob is keyed via the resolved cc1
    // line too (#428), so it must force the probe as well. The flat scan above
    // already catches self-contained shapes whose operand also matches a driver
    // row (e.g. `-ffp-contract=`), but a cc1-only forwarded knob would slip
    // past it — without the probe its codegen effect would not be keyed.
    parsed.rest.windows(2).any(|w| {
        w[0] == "-Xclang"
            && classify_xclang_forwarded(&w[1], dialect) == Some(FlagClass::CapturedByProbe)
    })
}

/// MSVC debug-info markers that embed absolute CodeView paths into the
/// object. clang-cl puts debug info in the `.obj` for all of these (no
/// compile-time PDB — box-confirmed), so each is a single cacheable
/// artifact once the embedded path inputs are keyed.
const CL_DEBUG_FLAGS: &[&str] = &["/Z7", "/Zi", "/ZI", "/Zd", "-Z7", "-Zi", "-ZI", "-Zd"];

/// Whether this clang-cl invocation requests debug info (native MSVC
/// spelling or a `-g` form parsed into `debug_level`). Only meaningful
/// for `Dialect::Cl`; the caller gates on dialect.
///
/// NOTE: `cl_debug_present` does NOT imply the `-###` probe is forced.
/// The native `/Z*` spellings are modeled `CapturedByProbe` (the
/// `/Z7`-vs-`/ZI` variant split is keyed via resolved tokens, bailing if
/// the probe is absent). The bare `-g` form has no such variant — it is
/// `ModeledInKey` via `debug_level` and its embedded paths are folded
/// here — so a `-g`-only clang-cl compile keys correctly without the
/// probe. Don't assume `cl_debug_present ⇒ probe required`.
fn cl_debug_present(parsed: &CcArgs) -> bool {
    parsed.family.dialect() == Dialect::Cl
        && (parsed.debug_level.is_some_and(|d| d > 0)
            || parsed
                .rest
                .iter()
                .any(|a| CL_DEBUG_FLAGS.contains(&a.as_str())))
}

/// The per-TU path inputs a clang-cl debug object embeds in CodeView that
/// the cache key would otherwise miss: the source file path(s) as spelled
/// on the command line, the output object name from `-Fo`/`-o`, and the
/// effective compilation directory (an explicit `-fdebug-compilation-dir`
/// or `-ffile-compilation-dir` value if present, otherwise the OS cwd).
/// These are exactly the tokens `config_args()` strips (source, output)
/// plus the compilation directory. `-I` dirs and flags are already in the
/// key via the resolved tokens. Capture, not remap — clang-cl stays
/// path-literal (#299/#312). `None` when this is not a clang-cl debug
/// compile (no fold; non-debug objects don't embed these, so
/// cross-CWD/name hits stay correct).
/// Per-TU path strings that can appear verbatim in the resolved `cc -###`
/// token stream and MUST be kept out of the cache key.
///
/// The resolved-invocation probe is memoized per *flag set* — `config_args`
/// strips the source, `-o`/`-Fo` output, and dep-file values so ONE probe
/// record serves every TU of a build. Absolute paths in the resolved tokens
/// are already blanked to a sentinel ([`crate::probe`]), and the trailing
/// source input token too — but a RELATIVE per-TU path that appears as a
/// flag *value* (`-main-file-name u00.c`, `-o build/u00.o`) survives,
/// leaving the shared record's tokens TU-specific. Serially the leak is
/// consistent (cold==warm → still hits); under `make -j` the TUs race over
/// whose paths the first-probing TU wrote into the shared record, leaking
/// one TU's source/output into another TU's key, so the key is
/// non-deterministic and the warm rebuild intermittently MISSES.
///
/// Blanking these tokens (to the path sentinel, inside the shared probe
/// record before it is stored) is safe in the never-miscache direction: the
/// source CONTENT is already captured by the preprocessor-expansion hash and
/// the output path has no object-content effect, so this only ever merges
/// keys that differ solely in a per-TU path. The set mirrors what
/// `config_args` removes from the probe-memo key, and is handed to the probe
/// so [`crate::probe`] sentinels these values out of the resolved tokens.
fn cc_resolved_per_tu_paths(parsed: &CcArgs) -> Vec<String> {
    let mut set = HashSet::new();
    // Insert a path both as written AND by basename: the cc1 line spells
    // the same file differently per token — `-o build/u00.o` keeps the
    // path, but `-main-file-name u00.c` uses only the basename. Blanking
    // must catch every spelling or a residual per-TU token still races.
    let mut add = |p: &Path| {
        set.insert(p.to_string_lossy().into_owned());
        if let Some(name) = p.file_name() {
            set.insert(name.to_string_lossy().into_owned());
        }
    };
    for src in &parsed.sources {
        add(src);
    }
    if let Some(o) = &parsed.output {
        add(o);
    }
    if let Some(o) = parsed.object_output_path() {
        add(&o);
    }
    if let Some(d) = parsed.depinfo_output_path() {
        add(&d);
    }
    for sidecar in parsed.sidecar_outputs() {
        add(&sidecar.path);
    }
    for (target, _) in parsed.depinfo.iter().flat_map(|d| &d.targets) {
        set.insert(target.clone());
    }
    // Never blank an empty token (a no-op path would blank real tokens).
    set.remove("");
    set.into_iter().collect()
}

/// The sidecar set this invocation stores, and the location facts its
/// outputs embed, which the probe blanks as per-TU paths. `None` when there
/// are neither, so other keys keep their value.
///
/// Coverage outputs embed the compilation directory and the output as
/// spelled; a split-DWARF object embeds only the latter, and its
/// compilation directory is prefix-mapped like any debug object's.
fn cc_sidecar_key(parsed: &CcArgs) -> Option<String> {
    let mut fields: Vec<String> = parsed
        .sidecar_outputs()
        .into_iter()
        .map(|sidecar| {
            let presence = match sidecar.presence {
                CcSidecarPresence::Required => "required",
                CcSidecarPresence::IfWritten => "if-written",
            };
            format!("{}={presence}", sidecar.role.store_name())
        })
        .collect();
    if parsed.coverage_instrumented() {
        let cwd = parsed.invocation_env.cwd().map(|cwd| cwd.to_string_lossy().into_owned());
        fields.push(format!("cwd={}", cwd.unwrap_or_default()));
    }
    if parsed.coverage_instrumented() || parsed.splits_dwarf() {
        let object = parsed.object_output_path().map(|path| path.to_string_lossy().into_owned());
        fields.push(format!("object={}", object.unwrap_or_default()));
    }
    (!fields.is_empty()).then(|| fields.join("\x1f"))
}

fn cl_debug_path_inputs(parsed: &CcArgs) -> Option<Vec<String>> {
    if !cl_debug_present(parsed) {
        return None;
    }
    let mut out = Vec::new();
    // Paths are encoded lossily; on Windows (where clang-cl runs) paths
    // are always valid UTF-16 → UTF-8, so no two distinct paths collapse.
    for src in &parsed.sources {
        out.push(format!("src={}", src.to_string_lossy()));
    }
    if let Some(o) = &parsed.output {
        out.push(format!("out={}", o.to_string_lossy()));
    }
    // NOTE (#312 follow-up): the compilation-dir spellings below are
    // NOT yet modeled in CC_FLAGS, so a clang-cl debug compile that
    // passes one EXPLICITLY currently hits the unmodeled-flag refusal
    // (passthrough — safe, not a miscache) before reaching this fold.
    // The common case (clang auto-injects -fdebug-compilation-dir at
    // -cc1, not on the driver line) is unaffected. Modeling these flags
    // to also cache the explicit-dir case is a deferred follow-up.
    let dir = parsed
        .rest
        .iter()
        .find_map(|a| {
            [
                "-fdebug-compilation-dir=",
                "-ffile-compilation-dir=",
                "/fdebug-compilation-dir=",
                "/ffile-compilation-dir=",
            ]
            .iter()
            .find_map(|p| a.strip_prefix(p))
            .map(str::to_string)
        })
        .or_else(|| {
            parsed
                .invocation_env
                .cwd()
                .map(|p| p.to_string_lossy().into_owned())
        });
    if let Some(d) = dir {
        out.push(format!("dir={d}"));
    }
    Some(out)
}

thread_local! {
    /// The prefix maps of the last invocation keyed in this process. One
    /// wrapper keys the same invocation up to three times (memo lookup, the
    /// re-check after a discovery flight, the key after a deferred compile)
    /// and the maps only depend on the arguments, the invocation snapshot
    /// and the configured base dirs.
    static PREFIX_MAPS_MEMO: std::cell::RefCell<Option<PrefixMapsMemo>> =
        const { std::cell::RefCell::new(None) };
}

struct PrefixMapsMemo {
    identity: Vec<String>,
    base_dirs: Vec<String>,
    maps: Vec<CcPrefixMap>,
}

/// Prefix maps that make C/C++ objects path-stable across worktrees.
///
/// A `-g` compile bakes paths into DWARF (`DW_AT_comp_dir`) and
/// `__FILE__` expansions. Firefox also exposes this through headers
/// whose macros stringify absolute include paths after preprocessing.
/// Mapping only the compiler CWD misses sibling objdir/source paths
/// like `<checkout>/obj/dist/include`, so derive the common root of
/// the source and build directories and map that instead.
///
/// The fallback split roots handle out-of-tree builds where source and
/// object directories do not share a useful project root. Distinct
/// sentinels avoid collapsing unrelated paths to the same spelling.
fn cc_prefix_maps(parsed: &CcArgs, configured_base_dirs: &[String]) -> Vec<CcPrefixMap> {
    // Everything the uncached computation reads besides its arguments.
    let env = |name: &str| {
        parsed
            .invocation_env
            .var_os(name)
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut identity = Vec::with_capacity(parsed.rest.len() + 6);
    identity.push(parsed.program.clone());
    identity.extend(parsed.rest.iter().cloned());
    identity.push(
        parsed
            .invocation_env
            .cwd()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    identity.push(
        parsed
            .invocation_env
            .temp_dir()
            .to_string_lossy()
            .into_owned(),
    );
    for name in [
        "KACHE_CC_PATH_NORMALIZE",
        "KACHE_BASE_DIR",
        "SDKROOT",
        "OUT_DIR",
    ] {
        identity.push(env(name));
    }
    let memoized = PREFIX_MAPS_MEMO.with(|memo| {
        memo.borrow().as_ref().and_then(|last| {
            (last.identity == identity && last.base_dirs == configured_base_dirs)
                .then(|| last.maps.clone())
        })
    });
    if let Some(maps) = memoized {
        return maps;
    }
    let maps = cc_prefix_maps_uncached(parsed, configured_base_dirs);
    PREFIX_MAPS_MEMO.with(|memo| {
        *memo.borrow_mut() = Some(PrefixMapsMemo {
            identity,
            base_dirs: configured_base_dirs.to_vec(),
            maps: maps.clone(),
        });
    });
    maps
}

fn cc_prefix_maps_uncached(parsed: &CcArgs, configured_base_dirs: &[String]) -> Vec<CcPrefixMap> {
    // `KACHE_CC_PATH_NORMALIZE=0` disables cc path normalization entirely:
    // no maps → the key hashes raw paths AND `execute` injects no
    // `-ffile-prefix-map`. The conservative escape hatch — cc keys become
    // path-literal (no cross-machine cc sharing, but zero normalization
    // miscache risk). Default on.
    if !cc_path_normalize_enabled_for(&parsed.invocation_env) {
        return Vec::new();
    }
    let Some(cwd) = parsed.invocation_env.cwd() else {
        return Vec::new();
    };
    let base = parsed
        .invocation_env
        .var_os("KACHE_BASE_DIR")
        .filter(|v| !v.is_empty());
    // `SDKROOT` is the Apple-clang env that pins the SDK when no explicit
    // `-isysroot` is on the command line; read here (the only env access)
    // and threaded into the deterministic core for testability.
    let sdkroot = parsed
        .invocation_env
        .var_os("SDKROOT")
        .filter(|v| !v.is_empty());
    let mut maps = cc_prefix_maps_cfg(
        parsed,
        cwd,
        base.as_deref().map(Path::new),
        sdkroot.as_deref().map(Path::new),
        configured_base_dirs,
    );
    if !maps.is_empty()
        && let Some(out_dir) = parsed
            .invocation_env
            .var_os("OUT_DIR")
            .filter(|v| !v.is_empty())
    {
        push_cargo_out_dir_maps(&mut maps, cwd, Path::new(&out_dir));
        let mut include_dirs = cc_user_include_dirs(parsed, cwd);
        include_dirs.extend(
            cc_flag_dir_values(&parsed.rest, "-isystem")
                .into_iter()
                .map(|dir| absolutize_path(cwd, Path::new(dir))),
        );
        push_cargo_dep_out_dir_maps(&mut maps, cwd, Path::new(&out_dir), &include_dirs);
        maps.sort_by_key(|m| std::cmp::Reverse(m.from.len()));
    }
    maps
}

/// Map the build-script output directories of the other crates this compile
/// includes from (`-I<target>/debug/build/libz-sys-<hash>/out/include`, or
/// `.../build/libz-sys/<hash>/out/include` from Cargo 1.100) to
/// [`CC_DEP_OUT_DIR_SENTINEL`] plus the crate name, dropping Cargo's
/// metadata hash. The compile's own `OUT_DIR` keeps its map. A crate name
/// that resolves to two units in one compile keeps the hash: the sentinel
/// would no longer name one directory.
fn push_cargo_dep_out_dir_maps(
    maps: &mut Vec<CcPrefixMap>,
    cwd: &Path,
    out_dir: &Path,
    include_dirs: &[PathBuf],
) {
    let own = absolutize_path(cwd, out_dir);
    let Some(target) = crate::build_script::target_dir(&own) else {
        return;
    };
    let mut units: rustc_hash::FxHashMap<String, Vec<PathBuf>> =
        rustc_hash::FxHashMap::default();
    for dir in include_dirs {
        let Some((unit_out, name)) = crate::cargo_layout::unit_out_dir_under(&target, dir) else {
            continue;
        };
        if unit_out == own {
            continue;
        }
        let dirs = units.entry(name).or_default();
        if !dirs.contains(&unit_out) {
            dirs.push(unit_out);
        }
    }
    // Sorted by unit name: `maps` order must not depend on the map's iteration
    // order.
    let mut units: Vec<(String, Vec<PathBuf>)> = units.into_iter().collect();
    units.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    for (name, dirs) in units {
        let [unit_out] = dirs.as_slice() else {
            continue;
        };
        let to = format!("{CC_DEP_OUT_DIR_SENTINEL}/{name}");
        for root in [canonicalize_or_self(unit_out), unit_out.clone()] {
            let from = root.to_string_lossy().to_string();
            if !from.is_empty() && !maps.iter().any(|m| m.from == from) {
                maps.push(CcPrefixMap {
                    from,
                    to: to.clone(),
                });
            }
        }
    }
}

/// Map a build script's `OUT_DIR`, and the target directory it sits in, to
/// sentinels shared by every build directory. Appended after the derived
/// roots; the caller re-sorts so the longest prefix still wins.
fn push_cargo_out_dir_maps(maps: &mut Vec<CcPrefixMap>, cwd: &Path, out_dir: &Path) {
    let out_abs = absolutize_path(cwd, out_dir);
    let mut roots: Vec<(PathBuf, &'static str)> = Vec::new();
    if let Some(target) = crate::build_script::target_dir(&out_abs) {
        roots.push((canonicalize_or_self(&target), CC_TARGET_SENTINEL));
        roots.push((target, CC_TARGET_SENTINEL));
    }
    roots.push((canonicalize_or_self(&out_abs), CC_OUT_DIR_SENTINEL));
    roots.push((out_abs, CC_OUT_DIR_SENTINEL));
    for (root, to) in roots {
        let from = root.to_string_lossy().to_string();
        if !from.is_empty() && !maps.iter().any(|m| m.from == from) {
            maps.push(CcPrefixMap {
                from,
                to: to.to_string(),
            });
        }
    }
}

/// The Apple SDK path this invocation pins, for the `<SDKROOT>` map.
///
/// Prefers an explicit `-isysroot <path>` (cargo's `cc` crate passes it on
/// Apple targets via `apple_sdk_root()`; mozbuild and CMake toolchains do
/// too), falling back to the `SDKROOT` env value the env-reading wrapper
/// threads in. Returns `None` when neither is present — a bare `cc -c`
/// that lets clang resolve the SDK via `xcrun` internally is not
/// normalized yet (issue #78).
fn cc_sdk_root(parsed: &CcArgs, sdkroot_env: Option<&Path>) -> Option<PathBuf> {
    let mut iter = parsed.rest.iter();
    while let Some(arg) = iter.next() {
        if arg == "-isysroot"
            && let Some(path) = iter.next()
            && !path.is_empty()
        {
            return Some(PathBuf::from(path));
        }
    }
    sdkroot_env.map(Path::to_path_buf)
}

/// Deterministic core of [`cc_prefix_maps`] (reads no env) — the derived
/// roots plus an optional user `base_dir` (`KACHE_BASE_DIR`) and the
/// Apple SDK root (explicit `-isysroot`, else `sdk_root` from `SDKROOT`).
fn cc_prefix_maps_cfg(
    parsed: &CcArgs,
    cwd: &Path,
    base_dir: Option<&Path>,
    sdk_root: Option<&Path>,
    configured_base_dirs: &[String],
) -> Vec<CcPrefixMap> {
    // clang-cl ignores `-ffile-prefix-map`, so prefix-mapping the key over
    // an object that still embeds raw paths would miscache. Until Layer 3
    // proves a cl path-remap, cl keys stay path-literal (unnormalised
    // `-###` abs paths make them per-machine — misses, never miscache) and
    // `execute` injects nothing. (The MSVC dialect doesn't use
    // `-isysroot`/`SDKROOT` anyway, on any host.)
    if parsed.family.dialect() == Dialect::Cl {
        return Vec::new();
    }
    let mut maps: Vec<CcPrefixMap> = Vec::new();

    // User-declared base dir (ccache `CCACHE_BASEDIR` analog). An explicit
    // root stripped to `<CC_BASE>` — covers paths the derived roots miss,
    // e.g. objdir-built TUs whose `__FILE__` points into the source tree
    // *above* the (narrow) derived root. A distinct sentinel so it can't
    // collide with a derived `<CC_ROOT>` subtree.
    if let Some(base) = base_dir {
        let base_abs = absolutize_path(cwd, base);
        for root in [base_abs.clone(), canonicalize_or_self(&base_abs)] {
            let from = root.to_string_lossy().to_string();
            if !from.is_empty() && !maps.iter().any(|m| m.from == from) {
                maps.push(CcPrefixMap {
                    from,
                    to: CC_BASE_SENTINEL.to_string(),
                });
            }
        }
    }

    // File-configured extra roots. Share PathNormalizer's lexical-first alias
    // reservation and Windows path variants so Rust and C/C++ assign the same
    // stable target on every host.
    for (from, to) in crate::path_normalizer::configured_base_dir_prefix_maps(configured_base_dirs)
    {
        if !from.is_empty() && !maps.iter().any(|m| m.from == from) {
            maps.push(CcPrefixMap { from, to });
        }
    }

    // Explicit roots win exact-prefix ties; derived roots cover everything
    // else. Longest-prefix sorting below handles overlapping configured roots.
    for map in cc_prefix_maps_for(parsed, cwd) {
        if !maps.iter().any(|existing| existing.from == map.from) {
            maps.push(map);
        }
    }

    // Apple SDK root (issue #78). The SDK path leaks into the key via the
    // resolved `cc -###` tokens; map it to `<SDKROOT>` so the same SDK at
    // a different install path (Xcode vs Command Line Tools, a teammate's
    // machine, a CI runner) keys identically. An explicit `-isysroot`
    // wins over the `SDKROOT` env value (`cc_sdk_root`). Distinct sentinel
    // — never a project root.
    if let Some(sdk) = cc_sdk_root(parsed, sdk_root) {
        let sdk_abs = absolutize_path(cwd, &sdk);
        for root in [sdk_abs.clone(), canonicalize_or_self(&sdk_abs)] {
            let from = root.to_string_lossy().to_string();
            if !from.is_empty() && !maps.iter().any(|m| m.from == from) {
                maps.push(CcPrefixMap {
                    from,
                    to: CC_SDKROOT_SENTINEL.to_string(),
                });
            }
        }
    }

    // Longest `from` first so the most specific prefix wins in the byte
    // normalizer (covers the derived roots, the base dir, and the SDK).
    maps.sort_by_key(|m| std::cmp::Reverse(m.from.len()));
    maps
}

/// Whether cc path normalization is active. `KACHE_CC_PATH_NORMALIZE` set
/// to `0` / `false` / `off` / `no` disables it; default on.
fn cc_path_normalize_enabled_for(env: &InvocationEnv) -> bool {
    parse_cc_normalize_toggle(env.var("KACHE_CC_PATH_NORMALIZE").ok().as_deref())
}

fn parse_cc_normalize_toggle(value: Option<&str>) -> bool {
    match value {
        Some(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        None => true,
    }
}

/// The effective `SOURCE_DATE_EPOCH` kache pins on *both* the `-E` key probe
/// and the real compile, so a translation unit that bakes
/// `__DATE__` / `__TIME__` into its object produces bytes
/// that match the time-stable cache key — no stale-timestamp false hit (#423).
///
/// Resolution (see [`resolve_source_date_epoch`]):
/// - the build's own `SOURCE_DATE_EPOCH` if it exported one (honored as-is, so
///   the key reflects the date the object actually bakes);
/// - otherwise `"0"`, kache's default pin that makes the key time-independent
///   so warm rebuilds hit;
/// - `None` only when the build set nothing *and* the user opted out via
///   `KACHE_CC_SOURCE_DATE_EPOCH=passthrough` — then kache pins nothing and the
///   object bakes wall-clock. Without a verified pin, preprocessing refuses
///   inputs whose expansion depends on time macros.
fn effective_source_date_epoch_for(env: &InvocationEnv) -> Option<std::ffi::OsString> {
    resolve_source_date_epoch(
        env.var_os("SOURCE_DATE_EPOCH"),
        source_date_epoch_passthrough_for(env),
    )
}

/// Pure resolution of the effective `SOURCE_DATE_EPOCH` (env read separately so
/// this stays unit-testable). A build-exported value is honored **verbatim** —
/// the raw bytes, untrimmed — so kache never normalizes a value the compiler
/// would otherwise reject (e.g. `" 123 "`, `""`, non-UTF-8) into a different,
/// accepted one, which would turn a failing compile into a cached success. Only
/// when the build set nothing does kache pin its default `"0"`, unless the
/// caller opted out.
pub(super) fn resolve_source_date_epoch(
    build_value: Option<std::ffi::OsString>,
    passthrough: bool,
) -> Option<std::ffi::OsString> {
    match build_value {
        Some(v) => Some(v),
        None if passthrough => None,
        None => Some(std::ffi::OsString::from("0")),
    }
}

/// Whether the build opted out of kache's default `SOURCE_DATE_EPOCH=0` pin via
/// `KACHE_CC_SOURCE_DATE_EPOCH=passthrough` (aliases: `wallclock` / `off`), for
/// the rare project that must bake the real wall-clock time into its objects.
/// Ignored when the build exports its own `SOURCE_DATE_EPOCH` (always honored).
fn source_date_epoch_passthrough_for(env: &InvocationEnv) -> bool {
    env.var("KACHE_CC_SOURCE_DATE_EPOCH")
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "passthrough" || v == "wallclock" || v == "off"
        })
        .unwrap_or(false)
}

fn fold_cc_memo_field(hasher: &mut blake3::Hasher, label: &[u8], value: &[u8]) {
    hasher.update(&(label.len() as u64).to_le_bytes());
    hasher.update(label);
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
    // Values are logged as digests only: an environment value may be a secret.
    log::trace!(
        target: "kache::cc_memo_key",
        "memo-field {}={}",
        String::from_utf8_lossy(label),
        &blake3::hash(value).to_hex()[..12]
    );
}

/// Driver/preprocessor inputs from GCC's Environment-Variables.html and
/// Clang's Driver.cpp, ToolChains/{Clang,Darwin,MSVC}.cpp, tools/driver/driver.cpp.
/// https://gcc.gnu.org/onlinedocs/gcc/Environment-Variables.html
/// https://github.com/llvm/llvm-project/tree/main/clang/lib/Driver
/// Shared with the native driver probe; vendor/loader families over-key.
/// Names are matched exactly; [`cc_memo_env_pattern_is_keyed`]
/// widens this to families of names.
const CC_MEMO_KEYED_ENV: &[&str] = &[
    // GCC "Environment Variables Affecting GCC"; Clang honours the same set.
    "CPATH",                         // Common preprocessor include directories.
    "C_INCLUDE_PATH",                // C system include directories.
    "CPLUS_INCLUDE_PATH",            // C++ system include directories.
    "OBJC_INCLUDE_PATH",             // Objective-C system include directories.
    "OBJCPLUS_INCLUDE_PATH",         // Objective-C++ system include directories.
    "GCC_EXEC_PREFIX",               // GCC subprogram and toolchain prefix.
    "COMPILER_PATH",                 // GCC/Clang subprogram directories.
    "LIBRARY_PATH",                  // GCC/Clang library directories.
    "LIB",                           // Clang Windows library directories (ToolChains/MSVC.cpp).
    "PATH",                          // Driver subprogram lookup.
    "PATHEXT",                       // Executable suffix lookup.
    "HOME",                          // Clang user configuration directory.
    "USERPROFILE",                   // Windows user configuration directory.
    "XDG_CONFIG_HOME",               // Vendor/user configuration lookup.
    "PWD",                           // Clang debug compilation directory.
    "DEVELOPER_DIR",                 // Apple developer tools selection.
    "TOOLCHAINS",                    // Apple toolchain selection.
    "OBJECT_MODE",                   // Clang AIX 32/64-bit target selection (Driver.cpp).
    "LIBPATH",                       // AIX library lookup.
    "TZ",                            // Timestamp/diagnostic time zone.
    "TMPDIR",                        // GCC/Clang temporary files.
    "TMP",                           // Platform temporary directory fallback.
    "TEMP",                          // Platform temporary directory fallback.
    "LANGUAGE",                      // GNU diagnostic message language.
    "TERM",                          // Diagnostic rendering capabilities.
    "NO_COLOR",                      // Diagnostic color selection.
    "EXPERIMENTAL_SARIF_SOCKET",     // GCC diagnostic transport.
    "AS_SECURE_LOG_FILE",            // Clang assembler secure log (ToolChains/Clang.cpp).
    "FORCE_CLANG_DIAGNOSTICS_CRASH", // Clang diagnostic behavior (driver.cpp).
    "COLORTERM",                     // Diagnostic color capabilities.
    "ZERO_AR_DATE",                  // Darwin archive timestamps.
    "SystemRoot",                    // Windows system tools and libraries.
    "ComSpec",                       // Windows command processor selection.
    "LANG",                          // Default diagnostic locale.
    "LC_ALL",                        // Locale override (pinned for probes, inherited by compiles).
    "LC_CTYPE",                      // Character classification locale.
    "LC_MESSAGES",                   // Diagnostic message locale.
    "DEPENDENCIES_OUTPUT",           // GCC non-system dependency output.
    "SUNPRO_DEPENDENCIES",           // GCC system dependency output.
    "SOURCE_DATE_EPOCH",             // Reproducible time macros.
    // Clang driver.
    "CCC_OVERRIDE_OPTIONS",         // Clang argument rewriting.
    "CLANG_CONFIG_FILE_SYSTEM_DIR", // Clang system configuration directory.
    "CLANG_CONFIG_FILE_USER_DIR",   // Clang user configuration directory.
    "CLANG_NO_DEFAULT_CONFIG",      // Clang configuration suppression.
    "SDKROOT",                      // Apple SDK selection.
    "MACOSX_DEPLOYMENT_TARGET",     // macOS ABI/deployment selection.
    "IPHONEOS_DEPLOYMENT_TARGET",   // iOS ABI/deployment selection.
    "TVOS_DEPLOYMENT_TARGET",       // tvOS ABI/deployment selection.
    "WATCHOS_DEPLOYMENT_TARGET",    // watchOS ABI/deployment selection.
    "XROS_DEPLOYMENT_TARGET",       // visionOS ABI/deployment selection.
    "DRIVERKIT_DEPLOYMENT_TARGET",  // DriverKit ABI/deployment selection.
    // MSVC.
    "INCLUDE",           // MSVC-compatible include directories.
    "EXTERNAL_INCLUDE",  // MSVC-compatible external include directories.
    "CL",                // clang-cl prepended arguments.
    "_CL_",              // clang-cl appended arguments.
    "VCINSTALLDIR",      // Visual C++ installation.
    "VCToolsInstallDir", // Visual C++ toolchain directory.
    "VCToolsVersion",    // Visual C++ toolchain version.
    "WindowsSdkDir",     // Windows SDK directory.
    "WindowsSDKVersion", // Windows SDK version.
    "UCRTVersion",       // Windows runtime version.
    "Platform",          // Windows target platform.
];

/// Families of names that can reach a preprocessor without being on the
/// documented list: toolchain wrappers and vendor drivers read `*_INCLUDE*`,
/// `*SYSROOT*`, `*SDK*` and `*FLAGS` variables of their own. Keying a whole
/// family costs at most a spurious miss.
fn cc_memo_env_pattern_is_keyed(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "INCLUDE",           // Vendor include directories.
        "SYSROOT",           // Vendor target roots.
        "SDK",               // Vendor SDK selection.
        "DEPLOYMENT_TARGET", // Apple and vendor deployment selection.
        "CLANG",             // Clang configuration, diagnostics and module controls.
        "GCC",               // GCC configuration, diagnostics and subprocess controls.
        "CCACHE",            // Compiler wrapper configuration.
    ]
    .iter()
    .any(|needle| upper.contains(needle))
        || upper.ends_with("FLAGS")
        || [
            "CCC_",         // Clang driver overrides and tests (Driver.cpp).
            "CC_",          // Clang diagnostic/output controls (driver.cpp).
            "LC_",          // GCC/Clang locale categories.
            "LLVM_",        // LLVM driver/tool configuration.
            "LD_",          // ELF loader, library and plugin selection.
            "DYLD_",        // Mach-O loader and library selection.
            "COLLECT_",     // GCC collect2/LTO subprocess configuration.
            "COMPILER_RT_", // Clang runtime library lookup.
            "RC_",          // Apple driver debug/environment options.
            "XCODE_",       // Apple developer tool configuration.
            "XCRUN_",       // Apple tool lookup, cache and diagnostic controls.
            "VC",           // Clang Windows toolchain selection.
            "UCRT",         // Clang Windows runtime selection.
            "UNIVERSALCRT", // Clang Windows runtime root.
            "CUDA",         // Clang CUDA toolchain lookup.
            "HIP",          // Clang HIP toolchain lookup.
            "ROC",          // Clang ROCm toolchain and runtime lookup.
            "HSA_",         // Clang GPU toolchain configuration.
            "AMD",          // Clang AMD GPU target overrides.
            "OMP_",         // OpenMP offload toolchain configuration.
            "OFFLOAD_",     // Offload target selection.
            "SYCL_",        // Clang SYCL toolchain selection.
            "SPIRV_",       // Clang SPIR-V toolchain selection.
            "_CC_",         // Clang z/OS driver options.
            "_CEE_",        // z/OS runtime configuration.
        ]
        .iter()
        .any(|prefix| upper.starts_with(prefix))
}

/// Is this a variable the memo key folds?
///
/// The memo identifies a preprocessor run, so the environment it keys is the
/// environment a preprocessor reads: the documented include, locale, SDK and
/// driver variables, plus families of names that vendor wrappers use. Folding
/// everything else made the memo unreachable in practice: a CI runner sets
/// per-run variables (`GITHUB_RUN_ID`, per-job temporary paths, tokens), so
/// no run could ever match the previous one and every warm build preprocessed
/// every translation unit again. kache's own `KACHE_*` settings steer the
/// wrapper, never the preprocessor, and are never keyed.
pub(crate) fn cc_memo_env_is_keyed(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    // Checked before the allow list below: `MAKEFLAGS` would otherwise match
    // its `*FLAGS` family.
    if name.starts_with("KACHE_") || crate::invocation::PER_COMMAND_ENV.contains(&name) {
        return false;
    }
    // `DEP_<links>_<KEY>` is Cargo's `links` metadata for build scripts. No
    // compiler reads it; the build script turns it into argv, which is
    // keyed. Folding it split the memo between jobs whose dependency graphs
    // differ only in crates the C compile never sees.
    if name.starts_with("DEP_") {
        return false;
    }
    CC_MEMO_KEYED_ENV.contains(&name) || cc_memo_env_pattern_is_keyed(name)
}

/// The schema line of every C/C++ compile key ([`CcCompiler::cache_key_with`]).
///
/// 2: older entries can retain donor paths outside the source/object root.
/// 3: older entries can hold an object built from other bytes than their
///    key names: keyed from a memo stamp or a mapped hash taken from a racy
///    read, or from an input saved before a key-first compile read it.
/// 4: older entries replay compiler diagnostics with the producer's paths.
const CC_ARTIFACT_SCHEMA: &[u8] = b"cc_artifact_schema:4\n";

/// The schema of [`cc_preprocess_memo_key`].
///
/// v4: the probe now runs with the compile's prefix maps, and path-bound
/// expansions are never recorded. A v3 record may hold the mapped hash of
/// one of those, which would give another checkout its key (#1004).
/// v5: TUs whose expansion `.incbin`s or `.include`s a file are refused. A
/// v4 record of one would skip the probe that refuses it (#1015).
/// v6: inputs, mapped hashes and assembler verdicts follow the settle and
/// single-read rules. A release that records v5 memos rewrites a memo row
/// in place when it records the same key again, and nothing in the row says
/// which release wrote it, so the two never share a key.
/// v7: read-set digests pair mapped input names with content hashes; a v6
/// memo stores a content-only digest that can alias exchanged headers.
/// v8: memo reuse also validates include resolution, including negative
/// header availability probes; older read sets have no such proof.
/// v9: drivers that ignore SOURCE_DATE_EPOCH may only memoize expansions
/// proven independent of time macros.
/// v10: the driver and preprocessing memos share expanded environment inputs.
const CC_PREPROCESS_MEMO_SCHEMA: &[u8] = b"cc-preprocess-memo-v10";

/// Local identity for a preprocessor invocation. The environment is included
/// as selected by [`cc_memo_env_is_keyed`].
fn cc_preprocess_memo_key(
    parsed: &CcArgs,
    prefix_maps: &[CcPrefixMap],
    compiler_version: &str,
) -> Option<String> {
    cc_preprocess_memo_key_as(
        CC_PREPROCESS_MEMO_SCHEMA,
        parsed,
        prefix_maps,
        compiler_version,
    )
}

/// [`cc_preprocess_memo_key`] under `schema`.
fn cc_preprocess_memo_key_as(
    schema: &[u8],
    parsed: &CcArgs,
    prefix_maps: &[CcPrefixMap],
    compiler_version: &str,
) -> Option<String> {
    let epoch = effective_source_date_epoch_for(&parsed.invocation_env)?;
    let cwd = parsed.invocation_env.cwd()?;
    let compiler_path = parsed
        .invocation_env
        .resolve_program(Path::new(&parsed.program))?;
    let compiler_metadata = std::fs::metadata(&compiler_path).ok()?;
    let mut hasher = blake3::Hasher::new();
    fold_cc_memo_field(&mut hasher, b"schema", schema);
    if parsed.lists_assembler_source() {
        // A relocated or older memo has not checked this invocation's listing sources.
        fold_cc_memo_field(&mut hasher, b"listing-inputs-v1",
            format!("{:?}", (&parsed.rest, cwd, file_prefix_map_args(prefix_maps))).as_bytes());
    }
    // The name alone: it selects the driver mode, and `compiler-path` below
    // says which binary it found, so `cc` and `/usr/bin/cc` share a memo.
    let compiler_name = Path::new(&parsed.program)
        .file_name()
        .unwrap_or(OsStr::new(&parsed.program));
    fold_cc_memo_field(
        &mut hasher,
        b"compiler-program",
        crate::cache_key::env_os_key_bytes(compiler_name).as_slice(),
    );
    fold_cc_memo_field(
        &mut hasher,
        b"compiler-path",
        crate::cache_key::env_os_key_bytes(compiler_path.as_os_str()).as_slice(),
    );
    fold_cc_memo_field(
        &mut hasher,
        b"compiler-size",
        &compiler_metadata.len().to_le_bytes(),
    );
    fold_cc_memo_field(
        &mut hasher,
        b"compiler-mtime",
        &crate::cache_key::metadata_mtime_ns(&compiler_metadata).to_le_bytes(),
    );
    fold_cc_memo_field(
        &mut hasher,
        b"compiler-ctime",
        &crate::cache_key::metadata_ctime_ns(&compiler_metadata).to_le_bytes(),
    );
    fold_cc_memo_field(
        &mut hasher,
        b"compiler-inode",
        &crate::cache_key::metadata_inode(&compiler_metadata).to_le_bytes(),
    );
    fold_cc_memo_field(
        &mut hasher,
        b"compiler-version",
        compiler_version.as_bytes(),
    );
    // The mapped cwd, not the raw one: two checkouts of the same tree differ
    // here and nowhere the expansion can see, so folding the raw path gave
    // every worktree its own memo.
    fold_cc_memo_field(
        &mut hasher,
        b"cwd",
        cc_mapped_path(cwd, prefix_maps).as_bytes(),
    );
    fold_cc_memo_field(
        &mut hasher,
        b"source-date-epoch",
        crate::cache_key::env_os_key_bytes(&epoch).as_slice(),
    );
    // Likewise the argv: `-I` and the source path carry the checkout root.
    // The maps are folded below, so two trees agree here only when they agree
    // about what the mapping means.
    // Compile-first lookup also needs the same read-set memo when only
    // terminal formatting changes; otherwise it compiles before finding a hit.
    let mut memo_parsed = parsed.clone();
    memo_parsed.rest = parsed.probe_args();
    for arg in build_preprocess_args(&memo_parsed) {
        let mapped = apply_cc_prefix_maps_to_bytes(arg.into_bytes(), prefix_maps);
        fold_cc_memo_field(&mut hasher, b"arg", &mapped);
    }
    // The source's own bytes, so two units that spell their source the same
    // way share nothing. Every tree-sitter grammar compiles `src/parser.c`
    // from its crate directory with the same flags, and with that directory
    // mapped to a sentinel their memos collided: each grammar found the
    // previous one's record, failed to validate it, and then ran the
    // preprocessor the memo exists to avoid. A source that cannot be read
    // keys as such; its compile fails on its own.
    for source in &parsed.sources {
        let content = crate::cache_key::hash_file(&absolutize_path(cwd, source))
            .unwrap_or_else(|_| "unreadable".to_string());
        fold_cc_memo_field(&mut hasher, b"source-content", content.as_bytes());
    }
    // Targets only. The sources are the per-checkout roots this whole change
    // exists to keep out; the targets are the sentinels both trees share, and
    // they are what decides whether two mappings mean the same thing.
    // Sorted: the maps come ordered by source length, which is a property
    // of one checkout's paths, not of what the mapping means.
    let mut targets: Vec<&str> = prefix_maps.iter().map(|map| map.to.as_str()).collect();
    targets.sort_unstable();
    targets.dedup();
    for target in targets {
        fold_cc_memo_field(&mut hasher, b"prefix-to", target.as_bytes());
    }

    // Values through the same maps as the argv: a `cc`-crate build hands a
    // dependency's include directory down as `DEP_<links>_INCLUDE`, a path
    // under this build directory's target, and it must not give every build
    // directory its own memo any more than `-I` does.
    let mut environment: Vec<(Vec<u8>, Vec<u8>)> = parsed
        .invocation_env
        .vars_os()
        .filter(|(name, _)| cc_memo_env_is_keyed(name))
        .map(|(name, value)| {
            // Mapped as text, then encoded like every other memo field: on
            // Windows the field encoding is UTF-16 and the maps are UTF-8.
            let mapped = match value.to_str() {
                Some(text) => {
                    let mapped =
                        apply_cc_prefix_maps_to_bytes(text.as_bytes().to_vec(), prefix_maps);
                    crate::cache_key::env_os_key_bytes(OsStr::new(
                        String::from_utf8_lossy(&mapped).as_ref(),
                    ))
                }
                None => crate::cache_key::env_os_key_bytes(&value),
            };
            (crate::cache_key::env_os_key_bytes(&name), mapped)
        })
        .collect();
    environment.sort();
    for (name, value) in environment {
        fold_cc_memo_field(&mut hasher, b"env-name", &name);
        fold_cc_memo_field(&mut hasher, b"env-value", &value);
    }
    Some(hasher.finalize().to_hex().to_string())
}

fn cc_prefix_maps_for(parsed: &CcArgs, cwd: &Path) -> Vec<CcPrefixMap> {
    let cwd_abs = absolutize_path(cwd, cwd);
    let Some(source) = parsed.sources.first() else {
        return prefix_maps_from_roots([(cwd_abs, CC_BUILD_SENTINEL)]);
    };
    let source_abs = absolutize_path(cwd, source);
    let source_parent = source_abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| source_abs.clone());

    let mut roots: Vec<(PathBuf, &'static str)> = Vec::new();
    push_cwd_source_roots(&mut roots, &cwd_abs, &source_parent);

    let cwd_canon = canonicalize_or_self(&cwd_abs);
    let source_canon = canonicalize_or_self(&source_abs);
    let source_canon_parent = source_canon
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(source_canon);
    if let Some(common) = common_ancestor(&cwd_canon, &source_canon_parent)
        && stable_cc_common_root_for(
            &common,
            &cwd_canon,
            &source_canon_parent,
            parsed.invocation_env.temp_dir(),
        )
    {
        roots.push((common, CC_ROOT_SENTINEL));
    }

    // Objdir-generated TUs (`Unified_cpp_*`, generated `.cpp`) live in the
    // build dir, so cwd ≈ source-dir and the roots above collapse to a
    // narrow objdir subdir — missing `__FILE__` paths into `dist/include`
    // and the source tree. The `-I` dirs span the repo, so fold them in:
    // the common ancestor of cwd and each include reaches the repo root.
    // `stable_cc_common_root`/`useful_cc_prefix` already drop the out-of-
    // tree ones — a system `-I` gives `/` (0 components) and `$HOME`-rooted
    // toolchain dirs give a 2-component ancestor, both below the ≥3 bound —
    // so only genuine in-tree roots survive. This is what makes
    // cross-checkout cc caching work automatically (no `KACHE_BASE_DIR`).
    for include in &parsed.includes {
        let include_abs = absolutize_path(cwd, include);
        for (a, b) in [
            (&cwd_abs, include_abs.clone()),
            (&cwd_canon, canonicalize_or_self(&include_abs)),
        ] {
            if let Some(common) = common_ancestor(a, &b)
                && stable_cc_common_root_for(&common, a, &b, parsed.invocation_env.temp_dir())
            {
                roots.push((common, CC_ROOT_SENTINEL));
            }
        }
    }

    prefix_maps_from_roots(roots)
}

/// Push the build (cwd) and source-dir roots, choosing the sentinel scheme by
/// TOPOLOGY rather than absolute location, so the same build converges
/// regardless of where its tree lives — the fix for out-of-tree build trees
/// (kunobi-ninja/kache#304, #394).
///
/// - **Nested** (one dir is under the other — the ordinary under-source build,
///   where the common ancestor IS the build or source dir): a single
///   `<CC_ROOT>` at the common ancestor is stable and preserves the
///   build↔source relative structure. Unchanged from before.
/// - **Sibling / out-of-tree** (the common ancestor is a *strict* ancestor of
///   both): the single-root scheme is fragile — whether the common ancestor is
///   judged "useful" depends on absolute properties (its depth, whether it is a
///   temp dir), so the SAME logical build picks `<CC_ROOT>` at one location and
///   split `<CC_BUILD>`/`<CC_SOURCE>` at another, and the cc key diverges.
///   Always use the split sentinels here (location-independent), and still fold
///   the shared root for paths under it (sibling includes) when it is a stable,
///   non-degenerate prefix. Prefix maps apply longest-first, so the split maps
///   (more specific) win for the build/source paths while `<CC_ROOT>` covers the
///   rest.
fn push_cwd_source_roots(
    roots: &mut Vec<(PathBuf, &'static str)>,
    cwd_abs: &Path,
    source_parent: &Path,
) {
    let common = common_ancestor(cwd_abs, source_parent);
    let nested = matches!(&common, Some(c) if c == cwd_abs || c == source_parent);
    if nested {
        if let Some(common) = common {
            roots.push((common, CC_ROOT_SENTINEL));
        }
    } else {
        roots.push((cwd_abs.to_path_buf(), CC_BUILD_SENTINEL));
        // Prefer the shared root over the source file's immediate parent. A
        // `<CC_ROOT>` at the common ancestor already strips the (clone-varying)
        // absolute prefix while PRESERVING the relative path below it
        // (e.g. `security/sandbox/chromium/base/location.cc`). The more-specific
        // `<CC_SOURCE>` at `source_parent` would win under longest-match and
        // collapse that parent directory to a flat sentinel — which gains no
        // cross-clone stability (the relative path is clone-invariant either
        // way) but BREAKS code that `static_assert`s on `__FILE__`, e.g.
        // Chromium's `base/location.cc`
        // (`StrEndsWith(__FILE__, …, "base/location.cc")`), failing the cold
        // Firefox build. Folding `<CC_ROOT>` keeps `__FILE__` ending in
        // `…/base/location.cc` so the assert holds.
        //
        // Gate on the common having a normal component — i.e. anything but a
        // bare filesystem root (`/`, a drive root) that would over-map unrelated
        // absolutes. This is a LOCATION-INDEPENDENT test, so the prefix-map
        // sentinel SET (hashed into the key) does not flip with absolute
        // location and an out-of-tree build still converges across machines / a
        // relocate (kunobi-ninja/kache#304, #394). Fall back to `<CC_SOURCE>`
        // only when there is no usable shared root, so a sibling source's
        // absolute path still cannot leak into the key.
        //
        // Do not anchor that fallback at `source_parent` when the parent is
        // nested under the filesystem root (`/obj` + `/src/base/location.cc`).
        // Mapping `/src/base` drops every directory below the first and
        // `__FILE__` becomes `<CC_SOURCE>/location.cc`, which fails Chromium's
        // `base/location.cc` assert (kunobi-ninja/kache#1755). A configured
        // base dir of `/src` does not win either: the more specific parent map
        // is applied last. Anchor at the root plus the source's first normal
        // component (`/src`, or `C:\\src` with the Windows prefix kept) so the
        // relative path below it survives. A file whose parent is already that
        // anchor (`/src/foo.c`) still maps the anchor. Never map the bare root.
        // If the canonical paths still share a directory (`/var` and
        // `/private/var` on macOS), keep `source_parent`: widening to `/var`
        // would leave the checkout id in the key.
        match common {
            Some(common) if has_normal_component(&common) => {
                roots.push((common, CC_ROOT_SENTINEL));
            }
            _ => {
                if let Some(anchor) = cc_source_anchor_for(cwd_abs, source_parent) {
                    roots.push((anchor, CC_SOURCE_SENTINEL));
                }
            }
        }
    }
}

/// Whether `path` has at least one normal component — true for any real
/// directory, false only for a bare filesystem root (`/`) or a drive/UNC root.
/// Folding a bare root to a sentinel would collapse unrelated absolute paths.
fn has_normal_component(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::Normal(_)))
}

/// The filesystem root plus `source_parent`'s first normal component.
///
/// `/src/base` is `/src`. A Windows path keeps its prefix (`C:\\src\\base` is
/// `C:\\src`; a UNC path keeps the server and share). `None` when
/// `source_parent` is itself a bare root, so that root is never a map source.
/// Lexical only: the caller passes the absolute parent it already computed.
fn cc_source_anchor(source_parent: &Path) -> Option<&Path> {
    source_parent
        .ancestors()
        .take_while(|path| has_normal_component(path))
        .last()
}

/// Where `<CC_SOURCE>` anchors when the lexical common ancestor is the
/// filesystem root.
///
/// Widen to the first directory only when the canonical paths also fail to
/// meet. `/obj` and `/src/base` stay apart, so `/src` is the anchor and
/// `base/location.cc` survives. `/private/var/.../clone-a` and
/// `/var/.../clone-a/src` are the same directory once the `/var` symlink is
/// resolved; mapping `/var` there would leave the clone id in the key.
fn cc_source_anchor_for(cwd: &Path, source_parent: &Path) -> Option<PathBuf> {
    let widened = cc_source_anchor(source_parent)?;
    let shares_directory = common_ancestor(
        &canonicalize_or_self(cwd),
        &canonicalize_or_self(source_parent),
    )
    .is_some_and(|common| has_normal_component(&common));
    if shares_directory {
        Some(source_parent.to_path_buf())
    } else {
        Some(widened.to_path_buf())
    }
}

fn prefix_maps_from_roots<I>(roots: I) -> Vec<CcPrefixMap>
where
    I: IntoIterator<Item = (PathBuf, &'static str)>,
{
    let mut maps = Vec::new();
    for (root, to) in roots {
        let from = root.to_string_lossy().to_string();
        if from.is_empty() || maps.iter().any(|m: &CcPrefixMap| m.from == from) {
            continue;
        }
        maps.push(CcPrefixMap {
            from,
            to: to.to_string(),
        });
    }
    maps.sort_by_key(|m| std::cmp::Reverse(m.from.len()));
    maps
}

fn absolutize_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn canonicalize_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn common_ancestor(a: &Path, b: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for (left, right) in a.components().zip(b.components()) {
        if left != right {
            break;
        }
        out.push(left.as_os_str());
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn useful_cc_prefix(path: &Path) -> bool {
    path.components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .count()
        >= 3
}

fn stable_cc_common_root_for(common: &Path, cwd: &Path, source_parent: &Path, temp: &Path) -> bool {
    if common == cwd || common == source_parent {
        return true;
    }
    if common_is_temp_dir(common, temp) {
        return false;
    }
    useful_cc_prefix(common) || common_is_below_temp_dir(common, temp)
}

fn common_is_below_temp_dir(common: &Path, temp: &Path) -> bool {
    let temp_dir = canonicalize_or_self(temp);
    let common = canonicalize_or_self(common);
    common != temp_dir && common.starts_with(temp_dir)
}

fn common_is_temp_dir(common: &Path, temp: &Path) -> bool {
    canonicalize_or_self(common) == canonicalize_or_self(temp)
}

/// Substitute build-path prefixes with their targets in a byte buffer (resolved
/// `-###` tokens, preprocessor stdout) for the cache key.
///
/// SINGLE left-to-right pass: at each position the most-specific matching map
/// wins, its target is emitted, and the cursor skips past the source WITHOUT
/// re-scanning the emitted target. This is deliberate now that targets are real
/// absolute paths (`/proc/self/cwd`, `/kache/*`, kunobi-ninja/kache#485): a
/// naive per-map sequential replace could re-match an earlier map's target with
/// a later map's source (e.g. a pathological `KACHE_BASE_DIR=/proc/self`
/// rewriting the `/proc/self/cwd` just written), diverging the key from the
/// compiler's single-application `-ffile-prefix-map`. Single-pass matches the
/// compiler's semantics and is identical to the old behavior for the normal case
/// of non-overlapping absolute source prefixes.
/// One path in the spelling the prefix maps give it, lossily for non-UTF-8.
///
/// The same rewrite the expansion and the resolved tokens already get, so a
/// path recorded here reads the same from any checkout the maps cover.
/// The maps of declared base dirs (`KACHE_BASE_DIR`, `[paths].base_dirs`)
/// as `(root, target)` pairs, for keying the compiler probe.
fn cc_declared_base_dirs(prefix_maps: &[CcPrefixMap]) -> Vec<(String, String)> {
    prefix_maps
        .iter()
        .filter(|map| {
            map.to == CC_BASE_SENTINEL
                || map
                    .to
                    .strip_prefix(CC_BASE_SENTINEL)
                    .is_some_and(|index| index.starts_with('-'))
        })
        .map(|map| (map.from.clone(), map.to.clone()))
        .collect()
}

fn cc_mapped_path(path: &Path, prefix_maps: &[CcPrefixMap]) -> String {
    // Text, not `env_os_key_bytes`: that encodes UTF-16 on Windows, and the
    // map sources and targets are UTF-8, so nothing would ever match and
    // every Windows memo missed. The maps are applied to argv strings the
    // same way.
    let text = path.to_string_lossy().into_owned().into_bytes();
    String::from_utf8_lossy(&apply_cc_prefix_maps_to_bytes(text, prefix_maps)).into_owned()
}

/// Identity of a map set for the mapped-hash memo: every `from => to` pair,
/// ordered by source. Two invocations with the same pairs rewrite bytes the
/// same way, whatever order the pairs were derived in.
fn cc_prefix_maps_key(prefix_maps: &[CcPrefixMap]) -> String {
    let mut pairs: Vec<(&str, &str)> = prefix_maps
        .iter()
        .filter(|map| !map.from.is_empty())
        .map(|map| (map.from.as_str(), map.to.as_str()))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kache.cc.maps.v1\0");
    for (from, to) in pairs {
        hasher.update(from.as_bytes());
        hasher.update(b"\0");
        hasher.update(to.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// blake3 of a file's contents with the prefix maps applied, and of the
/// contents as read, from one read.
///
/// The expansion is hashed this way, so an input has to be compared this way
/// too. A `-sys` crate's generated config header names its own build
/// directory: the raw bytes differ between two checkouts, the mapped bytes do
/// not, and the expansion each produces is identical. Comparing raw bytes
/// alone therefore made the memo stricter than the key it feeds. The raw hash
/// tells the caller which bytes the mapped one is of. `None` when the file
/// cannot be read, which the caller treats as "cannot reuse".
fn cc_mapped_content_hash(
    path: &Path,
    prefix_maps: &[CcPrefixMap],
) -> Option<crate::cache_key::CcContentHashes> {
    let bytes = std::fs::read(path).ok()?;
    let raw = blake3::hash(&bytes).to_hex().to_string();
    let mapped = apply_cc_prefix_maps_to_bytes(bytes, prefix_maps);
    Some(crate::cache_key::CcContentHashes {
        raw,
        mapped: blake3::hash(&mapped).to_hex().to_string(),
    })
}

/// Candidate real paths a mapped spelling could name in this invocation.
///
/// The inverse of [`cc_mapped_path`], and inexact by nature: two roots can
/// share one sentinel, so this yields every prefix that could have produced
/// the recorded name, most specific first. Picking wrong is not a
/// correctness problem — the caller compares contents, so a candidate whose
/// bytes differ is a miss and the expansion is recomputed. A mapped path
/// with no matching sentinel yields nothing and the memo is skipped.
fn cc_unmapped_path_candidates(mapped: &str, prefix_maps: &[CcPrefixMap]) -> Vec<PathBuf> {
    let mut maps: Vec<&CcPrefixMap> = prefix_maps
        .iter()
        .filter(|map| !map.from.is_empty() && !map.to.is_empty())
        .collect();
    maps.sort_by_key(|map| std::cmp::Reverse(map.to.len()));

    let mut candidates: Vec<PathBuf> = Vec::new();
    for map in maps {
        if let Some(rest) = mapped.strip_prefix(map.to.as_str()) {
            let candidate = PathBuf::from(format!("{}{rest}", map.from));
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    // An absolute path outside every mapped root (a system header, the
    // toolchain) was recorded verbatim and needs no inverse.
    if candidates.is_empty() && Path::new(mapped).is_absolute() {
        candidates.push(PathBuf::from(mapped));
    }
    candidates
}

fn apply_cc_prefix_maps_to_bytes(bytes: Vec<u8>, prefix_maps: &[CcPrefixMap]) -> Vec<u8> {
    // Most-specific (longest source) first so it wins at any position where two
    // sources overlap. (`cc_prefix_maps` already sorts this way; re-sort here so
    // callers passing ad-hoc maps get the same precedence.)
    let mut maps: Vec<&CcPrefixMap> = prefix_maps.iter().filter(|m| !m.from.is_empty()).collect();
    maps.sort_by_key(|m| std::cmp::Reverse(m.from.len()));

    // A map can only start at a byte its `from` starts with, and every
    // `from` is an absolute path. Skipping to those bytes in bulk keeps a
    // 10 MB generated parser from costing sixty million prefix tests.
    let mut leading: Vec<u8> = maps.iter().filter_map(|m| m.from.bytes().next()).collect();
    leading.sort_unstable();
    leading.dedup();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i..].iter().position(|byte| leading.contains(byte)) {
            Some(0) => {}
            Some(offset) => {
                // The slice is taken from `next`, so a step that does not
                // move forward fails here instead of looping.
                let next = i + offset;
                out.extend_from_slice(&bytes[i..next]);
                i = next;
            }
            None => {
                out.extend_from_slice(&bytes[i..]);
                break;
            }
        }
        let matched = maps.iter().find(|m| {
            let from = m.from.as_bytes();
            bytes[i..].starts_with(from)
                && (!is_configured_cc_prefix_map(m)
                    || configured_cc_prefix_starts_path_token(&bytes, i, from))
        });
        if let Some(m) = matched {
            out.extend_from_slice(m.to.as_bytes());
            i += m.from.len();
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn is_configured_cc_prefix_map(map: &CcPrefixMap) -> bool {
    map.to
        .strip_prefix("/kache/base-dir-")
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
}

fn configured_cc_prefix_starts_path_token(input: &[u8], start: usize, prefix: &[u8]) -> bool {
    if !cc_windows_absolute_prefix(prefix) && cc_follows_windows_drive_prefix(input, start) {
        return false;
    }
    let before = &input[..start];
    start == 0
        || before.ends_with(b"-I")
        || before.ends_with(b"-L")
        || before.ends_with(b"-F")
        || before.ends_with(b"-B")
        || before.last().is_some_and(|byte| {
            byte.is_ascii_whitespace()
                || matches!(
                    byte,
                    b'=' | b':' | b';' | b',' | b'"' | b'\'' | b'(' | b'[' | b'{' | b'@'
                )
        })
}

fn cc_windows_absolute_prefix(prefix: &[u8]) -> bool {
    (prefix.len() >= 3
        && prefix[0].is_ascii_alphabetic()
        && prefix[1] == b':'
        && matches!(prefix[2], b'/' | b'\\'))
        || prefix.starts_with(b"//")
        || prefix.starts_with(b"\\\\")
}

fn cc_follows_windows_drive_prefix(input: &[u8], start: usize) -> bool {
    if start < 2 || input[start - 1] != b':' || !input[start - 2].is_ascii_alphabetic() {
        return false;
    }
    let lead = &input[..start - 2];
    lead.is_empty()
        || lead.ends_with(b"-I")
        || lead.ends_with(b"-L")
        || lead.ends_with(b"-F")
        || lead.ends_with(b"-B")
        || lead.last().is_some_and(|byte| {
            byte.is_ascii_whitespace()
                || matches!(
                    byte,
                    b'=' | b':' | b';' | b',' | b'"' | b'\'' | b'(' | b'[' | b'{' | b'@'
                )
        })
}

/// The compiler receives the broadest map first and the most specific
/// last, mirroring rustc remap ordering. The byte-normalizer above
/// applies most-specific first.
fn file_prefix_map_args(prefix_maps: &[CcPrefixMap]) -> Vec<String> {
    prefix_maps
        .iter()
        .rev()
        .map(|m| format!("-ffile-prefix-map={}={}", m.from, m.to))
        .collect()
}

/// Compose the final argv for an `execute` invocation: the original
/// args with kache's `appended` flags placed *before* the `--`
/// end-of-options separator if one is present, otherwise at the end.
///
/// The clang / clang-cl driver treats every token after `--` as an
/// input file, not a flag — and cc-rs emits `--` before the source on
/// clang-cl invocations. Appending `-ffile-prefix-map=…` after that
/// separator makes the driver see the flags as extra source files,
/// producing `clang-cl: error: cannot specify '-Fo…' when compiling
/// multiple source files` (#300). Splicing them in ahead of `--` keeps
/// them classified as options. With no `--` present this is a plain
/// append, identical to the prior behaviour.
///
/// Splices before the *first* bare `--` — the only token clang/clang-cl/
/// gcc treat as the end-of-options marker (later `--` are inputs). It
/// matches `rest` literally, so a `--` that is some option's separated
/// value, or one hidden inside an `@response-file`, is not recognised;
/// both are out of scope for the cc-rs `-c` compiles that reach here.
fn compose_cc_args(rest: &[String], appended: Vec<String>) -> Vec<String> {
    if appended.is_empty() {
        return rest.to_vec();
    }
    match rest.iter().position(|a| a == "--") {
        Some(sep) => {
            let mut out = Vec::with_capacity(rest.len() + appended.len());
            out.extend_from_slice(&rest[..sep]);
            out.extend(appended);
            out.extend_from_slice(&rest[sep..]);
            out
        }
        None => {
            let mut out = rest.to_vec();
            out.extend(appended);
            out
        }
    }
}

fn cc_trace_name(parsed: &CcArgs) -> String {
    parsed
        .sources
        .first()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "cc".to_string())
}

/// Both halves of the shadowing digest, as the key computes them.
#[cfg(test)]
fn digest_cc_include_shadowing(parsed: &CcArgs, read_inputs: &[PathBuf]) -> Result<String> {
    digest_cc_shadowing_names(
        parsed,
        &cc_shadowing_names(parsed, read_inputs, &[], &crate::cache_key::FileHasher::new())?,
    )
}

/// Names that can change include resolution without changing a read file.
/// Ordinary headers track the first user directory providing their name.
/// Availability probes track every candidate, including the enclosing
/// header's parent for quoted names. Local lookup paths stay out of the key.
#[derive(Default, Clone)]
struct CcShadowingNames {
    names: rustc_hash::FxHashSet<PathBuf>,
    probes: Vec<CcProbedHeader>,
}

#[derive(Clone)]
struct CcProbedHeader {
    name: PathBuf,
    // Paths are local lookup locations, never folded into the portable key.
    candidates: Vec<PathBuf>,
}

fn cc_shadowing_names(
    parsed: &CcArgs,
    read_inputs: &[PathBuf],
    prefix_maps: &[CcPrefixMap],
    file_hasher: &crate::cache_key::FileHasher<'_>,
) -> Result<CcShadowingNames> {
    let cwd = parsed
        .invocation_env
        .cwd()
        .context("cc invocation has no working directory")?;
    let dirs = cc_user_include_dirs(parsed, cwd);
    let mut probe_dirs = dirs.clone();
    probe_dirs.extend(cc_system_include_dirs(parsed, cwd));
    for flag in ["-idirafter", "/imsvc", "-imsvc"] {
        probe_dirs.extend(
            cc_flag_dir_values(&parsed.rest, flag)
                .into_iter()
                .map(|value| absolutize_path(cwd, Path::new(value))),
        );
    }
    for variable in [
        "CPATH",
        "C_INCLUDE_PATH",
        "CPLUS_INCLUDE_PATH",
        "OBJC_INCLUDE_PATH",
        "OBJCPLUS_INCLUDE_PATH",
        "INCLUDE",
        "EXTERNAL_INCLUDE",
    ] {
        if let Some(value) = parsed.invocation_env.var_os(variable) {
            probe_dirs
                .extend(std::env::split_paths(&value).map(|path| absolutize_path(cwd, &path)));
        }
    }
    let read_inputs = read_inputs
        .iter()
        .map(|path| absolutize_path(cwd, path))
        .collect::<Vec<_>>();
    let definitions = parsed
        .defines
        .iter()
        .map(|(name, value)| format!("#define {name} {}\n", value.as_deref().unwrap_or("1")))
        .collect::<String>();
    let contents = file_hasher.cc_content_hashes(&read_inputs)?;
    let read_set = cc_probe_read_set(&contents, &definitions);
    let resolved = match file_hasher
        .cc_probe_resolution(&read_set)
        .and_then(|recorded| deser_json::from_str::<CcReadSetProbes>(&recorded).ok())
    {
        Some(resolved) => resolved,
        None => {
            let resolved =
                cc_resolve_read_set_probes(&read_inputs, &contents, &definitions, file_hasher)?;
            file_hasher.record_cc_probe_resolution(&read_set, &deser_json::to_string(&resolved)?);
            resolved
        }
    };
    let by_content = resolved
        .inputs
        .iter()
        .map(|(content, probes)| (content.as_str(), probes.as_slice()))
        .collect::<std::collections::HashMap<_, _>>();
    // Mapped, so a root spelled two ways (a base dir through `..`) gives an
    // input the same relative name whichever spelling named it.
    let mapped_dirs = dirs
        .iter()
        .map(|dir| PathBuf::from(cc_mapped_path(dir, prefix_maps)))
        .collect::<Vec<_>>();
    let mut result = CcShadowingNames::default();
    for (input, content) in read_inputs.iter().zip(&contents) {
        let absolute = absolutize_path(cwd, input);
        let mapped = PathBuf::from(cc_mapped_path(&absolute, prefix_maps));
        // Keep the ordinary shadowing names as before: the most specific
        // relative path and the basename can both be used by an include.
        if let Some(relative) = mapped_dirs
            .iter()
            .filter_map(|dir| mapped.strip_prefix(dir).ok())
            .min_by_key(|relative| relative.components().count())
        {
            result.names.insert(relative.to_path_buf());
        }
        if let Some(file_name) = absolute.file_name() {
            result.names.insert(PathBuf::from(file_name));
        }
        let probes = by_content.get(content.as_str()).copied().unwrap_or_default();
        for probe in probes.iter().chain(&resolved.arguments) {
            let mut candidates = Vec::new();
            if probe.quoted
                && let Some(parent) = absolute.parent()
            {
                candidates.push(parent.join(&probe.name));
            }
            candidates.extend(probe_dirs.iter().map(|dir| dir.join(&probe.name)));
            result.probes.push(CcProbedHeader {
                name: probe.name.clone(),
                candidates,
            });
        }
    }
    // The digest folds probes in this order. A probe's availability is a
    // function of its name and candidates alone, so a canonical order over
    // them describes the same lookups whichever order the read set arrived
    // in: preprocessing reports inclusion order, a memo reports its rows'.
    // Mapped spellings keep the order the same on every checkout.
    result.probes.sort_by_cached_key(|probe| {
        (
            probe.name.clone(),
            probe
                .candidates
                .iter()
                .map(|candidate| cc_mapped_path(candidate, prefix_maps))
                .collect::<Vec<_>>(),
        )
    });
    result
        .probes
        .dedup_by(|a, b| a.name == b.name && a.candidates == b.candidates);
    Ok(result)
}

/// Version of the header scans [`cc_resolve_read_set_probes`] reads.
const CC_HEADER_SCAN_VERSION: i64 = 6;

/// The probes a read set's headers make once every header's aliases are
/// known. A function of the headers' bytes and the command-line
/// definitions alone, so it is recorded under [`cc_probe_read_set`] and a
/// unit that reads the same bytes again skips the scans: parsing them was
/// most of a memo hit on headers such as the macOS SDK's, whose scans run
/// to megabytes.
#[derive(Default, deser::Serialize, deser::Deserialize)]
struct CcReadSetProbes {
    arguments: Vec<CcHeaderProbe>,
    /// Each distinct content hash that probes, with its probes.
    inputs: Vec<(String, Vec<CcHeaderProbe>)>,
}

/// Identity of a read set for [`CcReadSetProbes`]: its distinct contents
/// and the command-line definitions, under the scan version and the
/// resolution rules.
fn cc_probe_read_set(contents: &[String], definitions: &str) -> String {
    let mut distinct = contents.iter().map(String::as_str).collect::<Vec<_>>();
    distinct.sort_unstable();
    distinct.dedup();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kache.cc.probe-read-set.v1\0");
    hasher.update(&CC_HEADER_SCAN_VERSION.to_le_bytes());
    fold_cc_memo_field(&mut hasher, b"definitions", definitions.as_bytes());
    hasher.update(&(distinct.len() as u64).to_le_bytes());
    for content in distinct {
        fold_cc_memo_field(&mut hasher, b"content", content.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// Content-addressed header analysis, parsed.
type CcParsedHeaderScan = std::sync::Arc<CcHeaderAnalysis>;

#[derive(deser::Serialize, deser::Deserialize)]
struct CcHeaderAnalysis {
    probes: Option<CcHeaderProbeScan>,
    timestamp: String,
}

/// Parsed scans by content hash, kept for the life of the process. A scan
/// depends on the header's bytes alone, and the store holds the macOS SDK's
/// as megabytes of JSON: parsing them again was most of the include check on
/// every memo miss in the daemon.
struct CcHeaderScanCache {
    scans: rustc_hash::FxHashMap<String, CcParsedHeaderScan>,
    bytes: usize,
}

impl CcHeaderScanCache {
    /// Keep `scan`, whose serialized form is `len` bytes, starting over
    /// first when it would take the cache past `budget`.
    fn insert(&mut self, content: String, scan: CcParsedHeaderScan, len: usize, budget: usize) {
        if self.bytes + len > budget {
            self.scans.clear();
            self.bytes = 0;
        }
        if self.scans.insert(content, scan).is_none() {
            self.bytes += len;
        }
    }
}

/// Serialized size of the scans [`CcHeaderScanCache`] holds before it starts
/// over.
const CC_HEADER_SCAN_CACHE_BYTES: usize = 32 << 20;

fn cc_header_scan_cache() -> &'static std::sync::Mutex<CcHeaderScanCache> {
    static CACHE: OnceLock<std::sync::Mutex<CcHeaderScanCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        std::sync::Mutex::new(CcHeaderScanCache {
            scans: Default::default(),
            bytes: 0,
        })
    })
}

/// The parsed probe scan of each input, whose content hash is in
/// `contents`: from memory, else from the store's scan table, else scanned.
fn cc_parsed_header_scans(
    read_inputs: &[PathBuf],
    contents: &[String],
    file_hasher: &crate::cache_key::FileHasher<'_>,
) -> Result<Vec<CcParsedHeaderScan>> {
    let mut scans = {
        let cache = cc_header_scan_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        contents
            .iter()
            .map(|content| cache.scans.get(content).cloned())
            .collect::<Vec<_>>()
    };
    let missing = scans
        .iter()
        .enumerate()
        .filter_map(|(index, scan)| scan.is_none().then_some(index))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(scans.into_iter().flatten().collect());
    }
    let paths = missing
        .iter()
        .map(|&index| read_inputs[index].clone())
        .collect::<Vec<_>>();
    let scan = |path: &Path| -> Result<(String, String)> {
        let bytes = fs::read(path)
            .with_context(|| format!("reading header availability input {}", path.display()))?;
        let content = blake3::hash(&bytes).to_hex().to_string();
        let analysis = CcHeaderAnalysis {
            probes: cc_header_probe_scan(&cc_splice_lines(&bytes)),
            timestamp: cc_timestamp_scan_record(&content, cc_timestamp_may_expand(&bytes)),
        };
        Ok((content, deser_json::to_string(&analysis)?))
    };
    let mut serialized = file_hasher.cc_header_scans(&paths, CC_HEADER_SCAN_VERSION, &scan)?;
    let parsed = serialized
        .iter_mut()
        .zip(&missing)
        .map(|(text, &index)| -> Result<CcParsedHeaderScan> {
            let parsed = deser_json::from_str::<CcHeaderAnalysis>(text).ok()
                .filter(|analysis| cc_timestamp_scan_valid(&contents[index], &analysis.timestamp));
            let analysis = match parsed {
                Some(analysis) => analysis,
                None => {
                    let (content, fresh) = scan(&read_inputs[index])?;
                    anyhow::ensure!(content == contents[index], "cc: input changed while repairing header analysis");
                    let analysis = deser_json::from_str(&fresh)?;
                    *text = fresh;
                    analysis
                }
            };
            Ok(std::sync::Arc::new(analysis))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut cache = cc_header_scan_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for ((index, scan), text) in missing.into_iter().zip(parsed).zip(&serialized) {
        cache.insert(
            contents[index].clone(),
            std::sync::Arc::clone(&scan),
            text.len(),
            CC_HEADER_SCAN_CACHE_BYTES,
        );
        scans[index] = Some(scan);
    }
    Ok(scans.into_iter().flatten().collect())
}

/// Scan every input (memoised by content) and resolve the probes each makes
/// against the aliases all of them and the command line define.
fn cc_resolve_read_set_probes(
    read_inputs: &[PathBuf],
    contents: &[String],
    definitions: &str,
    file_hasher: &crate::cache_key::FileHasher<'_>,
) -> Result<CcReadSetProbes> {
    let scans = cc_parsed_header_scans(read_inputs, contents, file_hasher)?;
    let argument_scan = cc_header_probe_scan(&cc_splice_lines(definitions.as_bytes()))
        .context("cc: unresolved header availability probe in command-line definitions")?;
    let all_scans = || {
        scans
            .iter()
            .filter_map(|scan| scan.probes.as_ref())
            .chain(std::iter::once(&argument_scan))
    };
    let aliases = cc_header_probe_aliases(all_scans());
    anyhow::ensure!(
        cc_header_probe_pastes_are_resolved(all_scans(), &aliases)?,
        "cc: token pasting may construct a header availability probe"
    );
    let mut resolved = CcReadSetProbes {
        arguments: cc_resolve_header_probes(&argument_scan, &aliases)
            .context("cc: unresolved header availability probe in command-line definitions")?,
        inputs: Vec::new(),
    };
    let mut seen = HashSet::new();
    for ((input, content), scan) in read_inputs.iter().zip(contents).zip(scans) {
        let Some(probes) = scan
            .probes.as_ref()
            .and_then(|scan| cc_resolve_header_probes(scan, &aliases))
        else {
            anyhow::bail!(
                "cc: a header availability probe (__has_include) in {} cannot be resolved \
                 from literal source text",
                input.display()
            );
        };
        if !probes.is_empty() && seen.insert(content.as_str()) {
            resolved.inputs.push((content.clone(), probes));
        }
    }
    Ok(resolved)
}

/// A header name an availability probe tests for.
#[derive(Debug, Clone, PartialEq, Eq, deser::Serialize, deser::Deserialize)]
struct CcHeaderProbe {
    name: PathBuf,
    /// `"name"`, searched from the probing file's directory first; else
    /// `<name>`.
    quoted: bool,
}

/// Only transparent macro forwarding is modeled. Keep uses symbolic until
/// every read header has been scanned: an alias can be defined in one header
/// and used in another. Conditional definitions are an overapproximation;
/// every possible literal probe participates in the availability guard.
#[derive(Default, deser::Serialize, deser::Deserialize)]
struct CcHeaderProbeScan {
    aliases: Vec<(String, String)>,
    uses: Vec<(String, Option<CcHeaderProbe>)>,
    macro_uses: Vec<(String, Vec<String>, Vec<CcHeaderMacroUse>)>,
    conditionals: Vec<CcHeaderMacroUse>,
    pastes: Vec<(String, String)>,
    paste_forwarders: Vec<CcHeaderPasteForwarder>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, deser::Serialize, deser::Deserialize)]
struct CcHeaderMacroUse {
    name: String,
    arguments: Option<Vec<Vec<String>>>,
}

#[derive(deser::Serialize, deser::Deserialize)]
struct CcHeaderPasteForwarder {
    name: String,
    parameters: Option<Vec<String>>,
    parts: Vec<String>,
}

fn cc_header_probe_aliases<'a>(
    scans: impl Iterator<Item = &'a CcHeaderProbeScan>,
) -> HashSet<String> {
    let mut edges = std::collections::HashMap::<&str, Vec<&str>>::new();
    for (alias, target) in scans.flat_map(|scan| &scan.aliases) {
        edges.entry(target).or_default().push(alias);
    }
    let mut aliases = HashSet::new();
    let mut pending = vec!["__has_include", "__has_include_next"];
    while let Some(target) = pending.pop() {
        if aliases.insert(target.to_owned())
            && let Some(forwarders) = edges.get(target)
        {
            pending.extend(forwarders);
        }
    }
    aliases
}

fn cc_resolve_header_probes(
    scan: &CcHeaderProbeScan,
    aliases: &HashSet<String>,
) -> Option<Vec<CcHeaderProbe>> {
    scan.uses
        .iter()
        .filter(|(name, _)| aliases.contains(name))
        .map(|(_, probe)| probe.clone())
        .collect()
}

/// A macro can manufacture an operator or alias with `##`. Refuse when a
/// conditional can reach such a constructor. Fixed pasted prefixes, such as
/// glibc's `__GLIBC_USE_ ## feature`, cannot spell unrelated probe aliases.
fn cc_header_probe_pastes_are_resolved<'a>(
    scans: impl Iterator<Item = &'a CcHeaderProbeScan>,
    aliases: &HashSet<String>,
) -> Result<bool> {
    let scans = scans.collect::<Vec<_>>();
    let mut definitions = std::collections::HashMap::<&str, usize>::new();
    let mut forwarders = std::collections::HashMap::<&str, Vec<&CcHeaderPasteForwarder>>::new();
    for (name, _, _) in scans.iter().flat_map(|scan| &scan.macro_uses) {
        *definitions.entry(name).or_default() += 1;
    }
    for forwarder in scans.iter().flat_map(|scan| &scan.paste_forwarders) {
        forwarders
            .entry(&forwarder.name)
            .or_default()
            .push(forwarder);
    }
    let mut edges = std::collections::HashMap::<&str, Vec<&str>>::new();
    for (name, parameters, uses) in scans.iter().flat_map(|scan| &scan.macro_uses) {
        for used in uses {
            if cc_paste_use_can_name_probe(used, parameters, &definitions, &forwarders, aliases)? {
                edges.entry(&used.name).or_default().push(name);
            }
        }
    }
    let mut pending = Vec::new();
    for (name, pattern) in scans.iter().flat_map(|scan| &scan.pastes) {
        let pattern = Regex::new(pattern)?;
        if aliases.iter().any(|alias| pattern.is_match(alias)) {
            pending.push(name.as_str());
        }
    }
    let mut constructors = HashSet::new();
    while let Some(name) = pending.pop() {
        if constructors.insert(name)
            && let Some(users) = edges.get(name)
        {
            pending.extend(users);
        }
    }
    for used in scans.iter().flat_map(|scan| &scan.conditionals) {
        if constructors.contains(used.name.as_str())
            && cc_paste_use_can_name_probe(used, &[], &definitions, &forwarders, aliases)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Bind literal arguments to a pure concatenation helper. Caller parameters
/// and macros that may expand stay unknown; every alternative definition
/// must support the same proof before an edge can be discarded.
fn cc_paste_use_can_name_probe(
    used: &CcHeaderMacroUse,
    parameters: &[String],
    definitions: &std::collections::HashMap<&str, usize>,
    forwarders: &std::collections::HashMap<&str, Vec<&CcHeaderPasteForwarder>>,
    aliases: &HashSet<String>,
) -> Result<bool> {
    let Some(candidates) = forwarders.get(used.name.as_str()) else {
        return Ok(true);
    };
    // An alternative definition may construct a name by another route.
    if definitions.get(used.name.as_str()) != Some(&candidates.len()) {
        return Ok(true);
    }
    for forwarder in candidates {
        let arguments = match &forwarder.parameters {
            Some(formals) => match &used.arguments {
                Some(arguments) if arguments.len() == formals.len() => Some((formals, arguments)),
                _ => return Ok(true),
            },
            None => None,
        };
        let mut pattern = String::from("^");
        for part in &forwarder.parts {
            if let Some((formals, actuals)) = arguments
                && let Some(index) = formals.iter().position(|formal| formal == part)
            {
                let argument = &actuals[index];
                match argument.as_slice() {
                    [literal]
                        if !definitions.contains_key(literal.as_str())
                            && !parameters.contains(literal) =>
                    {
                        pattern.push_str(&regex_lite::escape(literal))
                    }
                    _ => pattern.push_str(".*"),
                }
            } else {
                pattern.push_str(&regex_lite::escape(part));
            }
        }
        pattern.push('$');
        let pattern = Regex::new(&pattern)?;
        if aliases.iter().any(|alias| pattern.is_match(alias)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `bytes` with every backslash-newline removed, the line splice the
/// preprocessor applies before it sees tokens. CRLF line ends included.
fn cc_splice_lines(bytes: &[u8]) -> Vec<u8> {
    let mut spliced = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some(at) = memchr::memchr(b'\\', rest) {
        let (before, backslash) = rest.split_at(at);
        spliced.extend_from_slice(before);
        let after = &backslash[1..];
        let line_end = after.strip_prefix(b"\r").unwrap_or(after);
        match line_end.strip_prefix(b"\n") {
            Some(next) => rest = next,
            None => {
                spliced.push(b'\\');
                rest = after;
            }
        }
    }
    spliced.extend_from_slice(rest);
    spliced
}

/// Resolve a self-contained source fragment, also used by scanner tests.
#[cfg(test)]
fn cc_header_probes(text: &[u8]) -> Option<Vec<CcHeaderProbe>> {
    let scan = cc_header_probe_scan(text)?;
    let aliases = cc_header_probe_aliases(std::iter::once(&scan));
    cc_header_probe_pastes_are_resolved(std::iter::once(&scan), &aliases)
        .ok()?
        .then(|| cc_resolve_header_probes(&scan, &aliases))?
}

fn cc_header_probe_scan(text: &[u8]) -> Option<CcHeaderProbeScan> {
    // A trigraph may splice an otherwise invisible operator or alias.
    if memchr::memmem::find(text, b"??/").is_some() || memchr::memmem::find(text, b"??=").is_some()
    {
        return None;
    }
    let mut scan = CcHeaderProbeScan::default();
    let mut line = Vec::new();
    let mut scanned_to = 0;
    for (i, &byte) in text.iter().enumerate() {
        if i < scanned_to {
            continue;
        }
        if byte.is_ascii_whitespace() {
            if byte == b'\n' {
                cc_scan_header_probe_line(text, &line, &mut scan)?;
                line.clear();
            }
            continue;
        }
        if byte == b'/' && matches!(text.get(i + 1), Some(b'*' | b'/')) {
            let end = cc_skip_comment(text, i);
            if text.get(i + 1) == Some(&b'/') {
                cc_scan_header_probe_line(text, &line, &mut scan)?;
                line.clear();
            }
            // Do not discard a directive when its block comment spans lines.
            // Keeping its tokens together can only add conservative uses.
            scanned_to = end;
            continue;
        }
        let end = cc_read_token(text, i, &mut CcStringLiterals::default(), &mut false);
        line.push(i..end);
        scanned_to = end;
    }
    cc_scan_header_probe_line(text, &line, &mut scan)?;
    Some(scan)
}

fn cc_scan_header_probe_line(
    text: &[u8],
    spans: &[std::ops::Range<usize>],
    scan: &mut CcHeaderProbeScan,
) -> Option<()> {
    let tokens = spans
        .iter()
        .map(|span| &text[span.clone()])
        .collect::<Vec<_>>();
    // `%:` is the standard digraph spelling of a preprocessing `#`.
    let offset = match tokens.as_slice() {
        [b"#", ..] => 1,
        [b"%", b":", ..] => 2,
        _ => 0,
    };
    let directive = &tokens[offset..];
    let definition = offset != 0 && directive.first() == Some(&b"define".as_slice());
    if offset != 0 && matches!(directive.first(), Some(&b"ifdef" | &b"ifndef" | &b"undef")) {
        return Some(());
    }
    let identifier = cc_header_probe_identifier;
    if definition {
        let name = directive.get(1)?;
        if !identifier(name) {
            return None;
        }
        if let Some(next) = spans.get(offset + 2)
            && spans[offset + 1].end == next.start
            && &text[next.clone()] != b"("
        {
            // Extensions such as `$` in macro names are not tokens this
            // scanner can resolve. Do not mistake their suffix for a body.
            return None;
        }
    }
    if definition {
        let name = String::from_utf8_lossy(directive[1]).into_owned();
        let function = directive.get(2) == Some(&b"(".as_slice())
            && spans[offset + 1].end == spans[offset + 2].start;
        let body_at = if function {
            directive
                .iter()
                .position(|token| *token == b")")
                .map_or(directive.len(), |end| end + 1)
        } else {
            2
        };
        let parameters = if function {
            directive
                .get(3..body_at.saturating_sub(1))
                .unwrap_or_default()
        } else {
            &[]
        };
        let body = &directive[body_at..];
        let mut uses = body
            .iter()
            .enumerate()
            .filter(|(_, word)| identifier(word) && !parameters.contains(word))
            .map(|(index, _)| cc_header_macro_use(body, index))
            .collect::<Vec<_>>();
        uses.sort_unstable();
        uses.dedup();
        let target = match directive {
            // `uses` already excludes literal tokens; an object alias has
            // exactly one replacement token and at most one identifier use.
            [_, _, _] => uses.first().map(|used| used.name.clone()),
            [_, _, b"(", param, b")", target, b"(", arg, b")"]
                if function && identifier(param) && param == arg && identifier(target) =>
            {
                Some(String::from_utf8_lossy(target).into_owned())
            }
            _ => None,
        };
        let mut parameter_names = parameters
            .iter()
            .filter(|word| **word != b",")
            .map(|word| String::from_utf8_lossy(word).into_owned())
            .collect::<Vec<_>>();
        if parameters.contains(&b".".as_slice()) {
            parameter_names.push("__VA_ARGS__".into());
        }
        scan.macro_uses.push((name.clone(), parameter_names, uses));
        if let Some(target) = target {
            scan.aliases.push((name, target));
            return Some(());
        }
        cc_header_macro_pastes(&name, body, parameters, function, scan);
    }
    // Operators are valid in preprocessor expressions and macro bodies.
    // Retain symbolic identifiers there so cross-header aliases are resolved
    // without storing the C/C++ declarations in every shared header scan.
    let conditional = offset != 0 && matches!(directive.first(), Some(&b"if" | &b"elif"));
    for (index, word) in tokens.iter().enumerate() {
        if !identifier(word) || (definition && index == offset + 1) {
            continue;
        }
        let before = &tokens[..index];
        let feature_test = before.last() == Some(&b"defined".as_slice())
            || before.ends_with(&[b"defined".as_slice(), b"(".as_slice()]);
        if feature_test {
            continue;
        }
        if conditional {
            scan.conditionals.push(cc_header_macro_use(&tokens, index));
        }
        if conditional || definition || matches!(*word, b"__has_include" | b"__has_include_next") {
            scan.uses.push((
                String::from_utf8_lossy(word).into_owned(),
                // A quoted probe in a macro searches beside its expansion
                // site, which this definition scan cannot locate.
                cc_literal_header_probe(text, spans[index].end)
                    .filter(|probe| !definition || !probe.quoted),
            ));
        }
    }
    Some(())
}

fn cc_header_macro_pastes(
    name: &str,
    body: &[&[u8]],
    parameters: &[&[u8]],
    function: bool,
    scan: &mut CcHeaderProbeScan,
) {
    let operand = |word: &[u8]| {
        if parameters.contains(&word) || word == b"__VA_ARGS__" {
            ".*".to_owned()
        } else {
            regex_lite::escape(&String::from_utf8_lossy(word))
        }
    };
    let mut scanned_to = 0;
    for (index, word) in body.iter().enumerate() {
        if index < scanned_to {
            continue;
        }
        let mut end = index + 1;
        let mut pattern = format!("^{}", operand(word));
        let mut parts = vec![String::from_utf8_lossy(word).into_owned()];
        loop {
            let paste_len = match body.get(end..) {
                Some([b"#", b"#", ..]) => 2,
                Some([b"%", b":", b"%", b":", ..]) => 4,
                _ => break,
            };
            let Some(word) = body.get(end + paste_len) else {
                break;
            };
            pattern.push_str(&operand(word));
            parts.push(String::from_utf8_lossy(word).into_owned());
            end += paste_len + 1;
        }
        if end > index + 1 {
            pattern.push('$');
            scan.pastes.push((name.to_owned(), pattern));
            if index == 0
                && end == body.len()
                && parameters
                    .iter()
                    .all(|word| *word == b"," || cc_header_probe_identifier(word))
            {
                scan.paste_forwarders.push(CcHeaderPasteForwarder {
                    name: name.to_owned(),
                    parameters: function.then(|| {
                        parameters
                            .iter()
                            .filter(|word| **word != b",")
                            .map(|word| String::from_utf8_lossy(word).into_owned())
                            .collect()
                    }),
                    parts,
                });
            }
        }
        scanned_to = end;
    }
}

fn cc_header_probe_identifier(word: &[u8]) -> bool {
    word.first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && word.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn cc_header_macro_use(tokens: &[&[u8]], index: usize) -> CcHeaderMacroUse {
    let mut used = CcHeaderMacroUse {
        name: String::from_utf8_lossy(tokens[index]).into_owned(),
        arguments: None,
    };
    if tokens.get(index + 1) != Some(&b"(".as_slice()) {
        return used;
    }
    let mut depth = 1;
    let mut argument = Vec::new();
    let mut arguments = Vec::new();
    for token in &tokens[index + 2..] {
        match *token {
            b"(" => depth += 1,
            b")" => {
                depth -= 1;
                if depth == 0 {
                    arguments.push(argument);
                    used.arguments = Some(arguments);
                    return used;
                }
            }
            b"," if depth == 1 => {
                arguments.push(std::mem::take(&mut argument));
                continue;
            }
            _ => {}
        }
        argument.push(String::from_utf8_lossy(token).into_owned());
    }
    used
}

fn cc_literal_header_probe(text: &[u8], word_end: usize) -> Option<CcHeaderProbe> {
    let open = cc_skip_probe_trivia(text, word_end);
    if text.get(open) != Some(&b'(') {
        return None;
    }
    let arg = cc_skip_probe_trivia(text, open + 1);
    let (close, quoted) = match text.get(arg) {
        Some(b'<') => (b'>', false),
        Some(b'"') => (b'"', true),
        _ => return None,
    };
    let start = arg + 1;
    let end = start + memchr::memchr(close, &text[start..])?;
    let name = std::str::from_utf8(&text[start..end]).ok()?;
    if name.is_empty() || name.contains('\n') {
        return None;
    }
    let after = cc_skip_probe_trivia(text, end + 1);
    (text.get(after) == Some(&b')')).then(|| CcHeaderProbe {
        name: PathBuf::from(name),
        quoted,
    })
}

fn cc_skip_probe_trivia(text: &[u8], mut i: usize) -> usize {
    while let Some(&byte) = text.get(i) {
        let next = if byte.is_ascii_whitespace() {
            i + 1
        } else if byte == b'/' && matches!(text.get(i + 1), Some(b'*' | b'/')) {
            cc_skip_comment(text, i)
        } else {
            break;
        };
        debug_assert!(next > i, "header probe trivia reader must advance");
        i = next;
    }
    i
}

/// Digest where each of `names` is first provided in the include search
/// order, read from the directories as they are now.
fn digest_cc_shadowing_names(parsed: &CcArgs, names: &CcShadowingNames) -> Result<String> {
    digest_cc_shadowing_names_stamped(parsed, names, None).map(|(digest, _)| digest)
}

/// [`digest_cc_shadowing_names`], with the stamps of every directory it
/// listed when all of them can vouch for their listing (see
/// [`CcListingStamp`]), so a later check can confirm the digest without
/// listing again.
fn digest_cc_shadowing_names_stamped(
    parsed: &CcArgs,
    names: &CcShadowingNames,
    file_hasher: Option<&crate::cache_key::FileHasher<'_>>,
) -> Result<(String, Option<CcListingStamps>)> {
    let cwd = parsed
        .invocation_env
        .cwd()
        .context("cc invocation has no working directory")?;
    let dirs = cc_user_include_dirs(parsed, cwd);
    let mut hasher = blake3::Hasher::new();
    let mut listings = CcDirectoryListings::default();
    // Sorted: the names are folded into the key in this order, so it must come
    // from the content rather than the set's iteration order.
    let mut ordered_names: Vec<&std::path::PathBuf> = names.names.iter().collect();
    ordered_names.sort_unstable();
    for name in ordered_names {
        hasher.update(name.as_os_str().as_encoded_bytes());
        hasher.update(b"\x1f");
        match cc_first_include_dir_providing_cached(&dirs, name, &mut listings)? {
            Some(index) => {
                hasher.update(b"@");
                hasher.update(index.to_string().as_bytes());
            }
            None => {
                hasher.update(b"-");
            }
        }
        hasher.update(b"\n");
    }
    // Probe every candidate, rather than stopping at the first match:
    // __has_include_next may depend on a later search directory. Quoted
    // probes also carry their enclosing header's actual parent, even when
    // it lies outside all user include directories.
    let mut availability = foldhash::HashMap::default();
    for probe in &names.probes {
        hasher.update(b"probe\0");
        fold_cc_memo_field(
            &mut hasher,
            b"name",
            probe.name.as_os_str().as_encoded_bytes(),
        );
        for candidate in &probe.candidates {
            let present = match availability.get(candidate) {
                Some(present) => *present,
                None => {
                    let present = cc_header_probe_available(candidate, file_hasher)?;
                    availability.insert(candidate.clone(), present);
                    present
                }
            };
            hasher.update(&[u8::from(present)]);
        }
        hasher.update(b"\n");
    }
    Ok((
        hasher.finalize().to_hex().to_string(),
        // A directory stamp cannot prove that an existing symlink still
        // resolves. Probe candidates are stat'ed again at publication.
        if names.probes.is_empty() {
            listings.stamps(&dirs)
        } else {
            None
        },
    ))
}

/// A probe follows symlinks, but a newly created link can make an old
/// target available. Observe both stamps before allowing a memo publication.
fn cc_header_probe_available(
    path: &Path,
    file_hasher: Option<&crate::cache_key::FileHasher<'_>>,
) -> Result<bool> {
    let read = |follow| {
        let metadata = if follow {
            fs::metadata(path)
        } else {
            fs::symlink_metadata(path)
        };
        match metadata {
            Ok(metadata) => {
                if let Some(file_hasher) = file_hasher {
                    file_hasher.note_too_new(&crate::cache_key::FileFingerprint::from_metadata(
                        path, &metadata,
                    ));
                }
                Ok(Some(metadata))
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                Ok(None)
            }
            Err(error) => anyhow::bail!(
                "cc header availability candidate {} is unreadable ({error})",
                path.display()
            ),
        }
    };
    let Some(metadata) = read(false)? else {
        return Ok(false);
    };
    if metadata.is_symlink() {
        Ok(read(true)?.is_some_and(|metadata| metadata.is_file()))
    } else {
        Ok(metadata.is_file())
    }
}

/// What a directory looked like just before its listing was read: absent, or
/// present with this fingerprint. Creating, removing or renaming an entry
/// sets a directory's mtime to the current time, so an unchanged fingerprint
/// means an unchanged listing, provided the mtime was already older than the
/// read by more than any filesystem's timestamp granularity. A directory
/// modified more recently than that has no stamp and is listed again. The
/// fingerprint also holds the ctime, which moves when anything resets the
/// mtime by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CcListingStamp {
    Absent,
    Present(crate::cache_key::FileFingerprint),
}

/// Each listed directory with its stamp.
type CcListingStamps = Vec<(PathBuf, CcListingStamp)>;

/// Two seconds covers the coarsest directory timestamps in use (FAT's).
const CC_LISTING_SETTLED_NS: i64 = 2_000_000_000;

/// Whether a directory modified at `mtime_ns` had settled by `read_at`:
/// modified more than [`CC_LISTING_SETTLED_NS`] before it.
fn cc_listing_settled(mtime_ns: i64, read_at: i64) -> bool {
    mtime_ns < read_at - CC_LISTING_SETTLED_NS
}

impl CcListingStamp {
    /// The stamp of `directory` now, or `None` when it cannot vouch for a
    /// listing read right after it.
    fn take(directory: &Path) -> Option<Self> {
        let read_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let read_at = i64::try_from(read_at).ok()?;
        match crate::cache_key::FileFingerprint::from_path(directory) {
            Ok(fingerprint) if cc_listing_settled(fingerprint.mtime_ns, read_at) => {
                Some(Self::Present(fingerprint))
            }
            Ok(_) => None,
            Err(_) => match std::fs::metadata(directory) {
                Err(error) if error.kind() == ErrorKind::NotFound => Some(Self::Absent),
                _ => None,
            },
        }
    }

    /// Whether `directory` still looks exactly as stamped.
    fn holds(&self, directory: &Path) -> bool {
        match self {
            Self::Absent => {
                std::fs::metadata(directory).is_err_and(|error| error.kind() == ErrorKind::NotFound)
            }
            Self::Present(fingerprint) => crate::cache_key::FileFingerprint::from_path(directory)
                .is_ok_and(|now| now == *fingerprint),
        }
    }
}

/// Index of the first user include directory that provides `name`, stopping
/// at the first hit. `None` when no directory does, which is itself a fact
/// worth keying: a directory later gaining the name changes it.
#[cfg(test)]
fn cc_first_include_dir_providing(dirs: &[PathBuf], name: &Path) -> Result<Option<usize>> {
    cc_first_include_dir_providing_cached(dirs, name, &mut CcDirectoryListings::default())
}

/// Directory listings read once per invocation. Resolving a few hundred
/// header names against a dozen include directories one `stat` at a time was
/// the largest cost of a warm C hit; listing each directory once answers the
/// same question from memory.
///
/// An include directory's own listing is found by its position in the search
/// order: most header names have no directory part, and hashing the joined
/// path for each (name, directory) pair was most of a C miss's CPU on
/// libgit2. A name with a directory part (`sys/types.h`) is listed by path.
#[derive(Default)]
struct CcDirectoryListings {
    listings: foldhash::HashMap<PathBuf, CcStampedListing>,
    by_position: Vec<Option<CcStampedListing>>,
}

struct CcDirectoryListing {
    names: foldhash::HashSet<std::ffi::OsString>,
    /// Lower-cased names, so a case-insensitive filesystem's answer can be
    /// confirmed with one `stat` instead of assumed from the exact spelling.
    /// `None` for a directory shown to be case-sensitive, where only the
    /// exact spelling resolves.
    folded: Option<foldhash::HashSet<String>>,
}

/// A listing, or its absence, with the stamp taken just before it was read.
struct CcStampedListing {
    listing: Option<CcDirectoryListing>,
    stamp: Option<CcListingStamp>,
}

impl CcStampedListing {
    fn read(directory: &Path) -> Result<Self> {
        let stamp = CcListingStamp::take(directory);
        Ok(Self {
            listing: CcDirectoryListing::read(directory)?,
            stamp,
        })
    }
}

impl CcDirectoryListing {
    /// `None` when the directory cannot be listed as a directory: absent, or
    /// an intermediate component is a file, which the compiler skips too.
    fn read(directory: &Path) -> Result<Option<Self>> {
        match std::fs::read_dir(directory) {
            Ok(entries) => {
                let mut names = foldhash::HashSet::default();
                for entry in entries {
                    names.insert(entry?.file_name());
                }
                let folded = (!Self::case_sensitive(directory, &names)).then(|| {
                    names
                        .iter()
                        .map(|name| name.to_string_lossy().to_lowercase())
                        .collect()
                });
                Ok(Some(Self { names, folded }))
            }
            Err(error)
                if error.kind() == ErrorKind::NotFound
                    || error.kind() == ErrorKind::NotADirectory =>
            {
                Ok(None)
            }
            Err(error) => anyhow::bail!(
                "cc include directory {} is unreadable ({error})",
                directory.display()
            ),
        }
    }

    /// Whether `directory` resolves only exact spellings: an entry's name
    /// with its ASCII letters case-swapped is not in the listing and does not
    /// resolve. Anything else, including a listing with no ASCII letter to
    /// swap, counts as case-insensitive.
    fn case_sensitive(directory: &Path, names: &foldhash::HashSet<std::ffi::OsString>) -> bool {
        let Some(swapped) = names.iter().find_map(|name| {
            let name = name.to_str()?;
            name.bytes().any(|b| b.is_ascii_alphabetic()).then(|| {
                name.chars()
                    .map(|c| {
                        if c.is_ascii_lowercase() {
                            c.to_ascii_uppercase()
                        } else {
                            c.to_ascii_lowercase()
                        }
                    })
                    .collect::<String>()
            })
        }) else {
            return false;
        };
        !names.contains(OsStr::new(&swapped))
            && std::fs::symlink_metadata(directory.join(&swapped))
                .is_err_and(|error| error.kind() == ErrorKind::NotFound)
    }

    /// Whether `directory`, which this lists, has an entry named `file_name`.
    fn provides(&self, directory: &Path, file_name: &OsStr) -> Result<bool> {
        if self.names.contains(file_name) {
            return Ok(true);
        }
        let Some(folded) = &self.folded else {
            return Ok(false);
        };
        // A case-insensitive filesystem (macOS, Windows, but also a casefold
        // ext4 or a mounted share on Linux) resolves `foo.h` to `Foo.h`; the
        // listing does not. When only the case differs, ask the filesystem,
        // which is what the compiler does.
        if folded.contains(&file_name.to_string_lossy().to_lowercase()) {
            return match std::fs::symlink_metadata(directory.join(file_name)) {
                Ok(_) => Ok(true),
                Err(error)
                    if error.kind() == ErrorKind::NotFound
                        || error.kind() == ErrorKind::NotADirectory =>
                {
                    Ok(false)
                }
                Err(error) => anyhow::bail!(
                    "cc include candidate {} is unreadable ({error})",
                    directory.join(file_name).display()
                ),
            };
        }
        Ok(false)
    }
}

impl CcDirectoryListings {
    /// Whether `directory` contains an entry named `file_name`, of any kind.
    fn contains(&mut self, directory: &Path, file_name: &OsStr) -> Result<bool> {
        let listing = match self.listings.entry(directory.to_path_buf()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(CcStampedListing::read(directory)?)
            }
        };
        match &listing.listing {
            Some(listing) => listing.provides(directory, file_name),
            None => Ok(false),
        }
    }

    /// [`Self::contains`] for the include directory at `position` in the
    /// search order, found without hashing its path.
    fn contains_at(
        &mut self,
        position: usize,
        directory: &Path,
        file_name: &OsStr,
    ) -> Result<bool> {
        if self.by_position.len() <= position {
            self.by_position.resize_with(position + 1, || None);
        }
        let slot = &mut self.by_position[position];
        if slot.is_none() {
            *slot = Some(CcStampedListing::read(directory)?);
        }
        match slot.as_ref().and_then(|stamped| stamped.listing.as_ref()) {
            Some(listing) => listing.provides(directory, file_name),
            None => Ok(false),
        }
    }

    /// Every directory listed so far with its stamp; `None` when any of them
    /// has none. `dirs` is the search order `by_position` indexes.
    fn stamps(&self, dirs: &[PathBuf]) -> Option<CcListingStamps> {
        let positioned = self
            .by_position
            .iter()
            .zip(dirs)
            .filter_map(|(slot, dir)| slot.as_ref().map(|stamped| (dir, stamped)));
        positioned
            .chain(self.listings.iter())
            .map(|(dir, stamped)| Some((dir.clone(), stamped.stamp.clone()?)))
            .collect()
    }
}

fn cc_first_include_dir_providing_cached(
    dirs: &[PathBuf],
    name: &Path,
    listings: &mut CcDirectoryListings,
) -> Result<Option<usize>> {
    let Some(file_name) = name.file_name() else {
        return Ok(None);
    };
    let parent = name
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    for (index, dir) in dirs.iter().enumerate() {
        let provided = match parent {
            Some(parent) => listings.contains(&dir.join(parent), file_name)?,
            None => listings.contains_at(index, dir, file_name)?,
        };
        if provided {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// Maximum file names walked across every user include dir. A partial
/// listing could miss the shadowing header the digest exists to catch,
/// so overflow is fail-closed passthrough rather than a truncated key.
const CC_INCLUDE_DIR_NAME_CAP: usize = 8192;

const CC_INCLUDE_DIR_NAME_EXTENSIONS: &[&str] = &[
    "h", "hh", "hpp", "hxx", "h++", "cuh", "c", "cc", "cpp", "cxx", "c++", "m", "mm", "i", "ii",
    "inl", "inc", "def", "pch", "gch",
];

fn digest_cc_include_dir_names(parsed: &CcArgs) -> Result<String> {
    digest_cc_include_dir_names_capped(parsed, CC_INCLUDE_DIR_NAME_CAP)
}

fn digest_cc_include_dir_names_capped(parsed: &CcArgs, cap: usize) -> Result<String> {
    let cwd = parsed
        .invocation_env
        .cwd()
        .context("cc invocation has no working directory")?;
    let exempt = cc_system_include_dirs(parsed, cwd);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"include_dir_names.v1\n");
    let mut seen = 0usize;
    for dir in cc_user_include_dirs(parsed, cwd) {
        if exempt.iter().any(|root| dir.starts_with(root)) {
            continue;
        }
        hasher.update(b"dir\n");
        collect_include_dir_names(&dir, Path::new(""), &mut hasher, &mut seen, cap)?;
        hasher.update(b"enddir\n");
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn cc_user_include_dirs(parsed: &CcArgs, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |path: PathBuf| {
        let abs = absolutize_path(cwd, &path);
        if seen.insert(abs.clone()) {
            dirs.push(abs);
        }
    };
    if let Some(source) = parsed.sources.first() {
        let parent = source.parent().filter(|p| !p.as_os_str().is_empty());
        push(parent.map_or_else(|| cwd.to_path_buf(), Path::to_path_buf));
    }
    for value in cc_flag_dir_values(&parsed.rest, "-iquote") {
        push(PathBuf::from(value));
    }
    for include in &parsed.includes {
        push(include.clone());
    }
    dirs
}

fn cc_system_include_dirs(parsed: &CcArgs, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |path: PathBuf| {
        let abs = absolutize_path(cwd, &path);
        if seen.insert(abs.clone()) {
            dirs.push(abs);
        }
    };
    for value in cc_flag_dir_values(&parsed.rest, "-isystem") {
        push(PathBuf::from(value));
    }
    for value in cc_flag_dir_values(&parsed.rest, "-isysroot") {
        push(PathBuf::from(value));
        push(PathBuf::from(value).join("usr/include"));
    }
    if let Some(path) = parsed
        .invocation_env
        .var_os("SDKROOT")
        .filter(|sdk| !sdk.is_empty())
        .and_then(|sdk| parsed.invocation_env.resolve(Path::new(&sdk)))
        && path.is_dir()
    {
        push(path.clone());
        push(path.join("usr/include"));
    }
    dirs
}

fn cc_flag_dir_values<'a>(rest: &'a [String], flag: &'a str) -> Vec<&'a str> {
    let mut values = Vec::new();
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        let Some(suffix) = arg.strip_prefix(flag) else {
            continue;
        };
        if suffix.is_empty() {
            if let Some(value) = args.next() {
                values.push(value.as_str());
            }
        } else if let Some(value) = suffix.strip_prefix('=') {
            if !value.is_empty() {
                values.push(value);
            }
        } else if flag != "-isystem" || !suffix.starts_with('-') {
            // `-isystem-after` is another flag, but `-iquote-headers`
            // names the ordinary relative directory `-headers`.
            values.push(suffix);
        }
    }
    values
}

fn collect_include_dir_names(
    dir: &Path,
    rel: &Path,
    hasher: &mut blake3::Hasher,
    seen: &mut usize,
    cap: usize,
) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            anyhow::bail!("cc include dir {} is unreadable ({error})", dir.display())
        }
    };

    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", entry.path().display()))?;
        let name = entry.file_name();
        if file_type.is_symlink() {
            // Hash the name (it can shadow) but do not recurse; a
            // symlink dir can loop.
            if cc_include_name_counts(&name) {
                files.push(name);
            }
            continue;
        }
        if file_type.is_dir() {
            subdirs.push(name);
            continue;
        }
        if cc_include_name_counts(&name) {
            files.push(name);
        }
    }
    files.sort();
    subdirs.sort();

    for name in files {
        *seen += 1;
        if *seen > cap {
            anyhow::bail!("cc include dir name walk exceeded {cap} entries");
        }
        let path = rel.join(&name);
        hasher.update(path.as_os_str().as_encoded_bytes());
        hasher.update(b"\n");
    }
    for name in subdirs {
        *seen += 1;
        if *seen > cap {
            anyhow::bail!("cc include dir name walk exceeded {cap} entries");
        }
        collect_include_dir_names(&dir.join(&name), &rel.join(&name), hasher, seen, cap)?;
    }
    Ok(())
}

fn cc_include_name_counts(name: &OsStr) -> bool {
    let Some(ext) = Path::new(name).extension() else {
        return true;
    };
    let ext = ext.to_string_lossy();
    CC_INCLUDE_DIR_NAME_EXTENSIONS
        .iter()
        .any(|keep| ext.eq_ignore_ascii_case(keep))
}

struct PendingCcPreprocessMemo {
    memo_key: String,
    preprocessed_hash: String,
    fingerprints: Vec<crate::cache_key::CcPreprocessMemoInput>,
    /// The maps the expansion and the inputs were hashed under. Revalidation
    /// at commit time has to use the same ones.
    prefix_maps: Vec<CcPrefixMap>,
    /// The inputs came from a compile's own read set, fingerprinted after
    /// it ran, rather than from a preprocess that preceded the compile.
    captured: bool,
}

/// How the key learns what the translation unit reads.
///
/// The memo answers first in every mode. Without a memo the classic path
/// preprocesses (`-E`) to learn the read set and hash the expansion. The
/// deferrable path instead hands the decision back to the wrapper, which
/// compiles once with dependency capture and keys from what that compile
/// read; the captured read set then comes back through `Captured`.
pub(crate) enum CcKeyDiscovery {
    Expansion,
    Deferrable,
    Captured(CcCapturedInputs),
}

pub(crate) enum CcKeyOutcome {
    Key(String),
    /// No memo for this invocation: compile first, then key from the read
    /// set. Carries the memo identity so peers of the same unit coalesce.
    Deferred(CcDeferredKey),
}

pub(crate) struct CcDeferredKey {
    pub(crate) memo_key: String,
}

/// The files a compile with dependency capture read, fingerprinted the way
/// the memo records them (mapped name, content hash under the prefix maps).
pub(crate) struct CcCapturedInputs {
    fingerprints: Vec<crate::cache_key::CcPreprocessMemoInput>,
    /// The object spells a checkout root the prefix maps did not rewrite,
    /// so its key is bound to this checkout (see `hash_cc_expansion`).
    path_bound: bool,
    /// An input was written after the invocation started, as seen by the
    /// hasher that fingerprinted the captured read set. The wrapper's
    /// re-entry keys with a fresh hasher, so the bit travels here; an entry
    /// or memo must not describe what the compiler may not have read.
    inputs_changed: bool,
}

impl CcCapturedInputs {
    pub(crate) fn inputs_changed(&self) -> bool {
        self.inputs_changed
    }
}

/// Memo hash prefix for a path-bound read set: the digest is portable, the
/// key that uses it folds the reading checkout's roots as well.
const CC_MEMO_PATH_BOUND_PREFIX: &str = "pb:";

/// Whether the memo identity of this invocation is spelled entirely in
/// mapped or system paths. If a checkout root reaches the identity raw, a
/// record for the same unit may exist under another checkout's spelling, and
/// the miss is not certain enough to compile first.
fn cc_memo_identity_portable(parsed: &CcArgs, cwd: &Path, prefix_maps: &[CcPrefixMap]) -> bool {
    let portable = |path: &Path| {
        let absolute = absolutize_path(cwd, path);
        let raw = absolute.to_string_lossy().into_owned();
        let mapped = cc_mapped_path(&absolute, prefix_maps);
        // The build and source sentinels stand for "wherever this invocation
        // runs": which one a path lands under depends on the checkout's
        // layout, so a spelling under them is not one other checkouts share.
        (mapped != raw
            && !mapped.starts_with(CC_BUILD_SENTINEL)
            && !mapped.starts_with(CC_SOURCE_SENTINEL))
            || cc_system_path(&raw)
    };
    portable(cwd)
        && parsed.sources.iter().all(|source| portable(source))
        && parsed.includes.iter().all(|include| portable(include))
}

/// A path the toolchain owns, the same on every checkout of one machine.
fn cc_system_path(path: &str) -> bool {
    ["/usr/", "/opt/", "/Library/", "/Applications/", "/nix/"]
        .iter()
        .any(|root| path.starts_with(root))
}

/// Hash each mapped input name with its content, preserving their association
/// when headers exchange bytes. Mapped names and project content retain reuse
/// across equivalent checkouts; physical paths and timestamps stay out.
fn cc_direct_inputs_digest(fingerprints: &[crate::cache_key::CcPreprocessMemoInput]) -> String {
    // A toolchain file is the same bytes on every checkout and can spell a
    // directory a checkout happens to be under (glibc's `P_tmpdir`); its raw
    // hash is the portable one. A project file may spell its own checkout
    // root, which the maps make portable.
    let mut contents: Vec<(&str, &str)> = fingerprints
        .iter()
        .map(|input| {
            let content = if cc_system_path(input.local_path()) {
                input.content.as_str()
            } else {
                input.mapped.as_str()
            };
            (input.name.as_str(), content)
        })
        .collect();
    contents.sort_unstable();
    contents.dedup();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kache.cc.direct-inputs.v3\0");
    hasher.update(&(contents.len() as u64).to_le_bytes());
    for (name, content) in contents {
        fold_cc_memo_field(&mut hasher, b"name", name.as_bytes());
        fold_cc_memo_field(&mut hasher, b"content", content.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn cc_timestamp_may_expand(bytes: &[u8]) -> bool {
    const NAME: &[u8] = b"__TIMESTAMP__";
    memchr::memchr_iter(b'_', bytes).any(|start| {
        bytes[start..].iter().copied()
            .filter(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            .take(NAME.len())
            .eq(NAME.iter().copied())
    })
}

fn cc_timestamp_scan_record(content: &str, sensitive: bool) -> String {
    let value = if sensitive { "1" } else { "0" };
    let mut hash = blake3::Hasher::new();
    hash.update(b"kache.cc.timestamp-scan.v1\0");
    hash.update(content.as_bytes());
    hash.update(value.as_bytes());
    format!("{value}:{}", hash.finalize().to_hex())
}

fn cc_timestamp_scan_valid(content: &str, record: &str) -> bool {
    match record.as_bytes().first() {
        Some(b'0') => record == cc_timestamp_scan_record(content, false),
        Some(b'1') => record == cc_timestamp_scan_record(content, true),
        _ => false,
    }
}

fn cc_timestamp_inputs_sensitive(
    inputs: &[PathBuf],
    file_hasher: Option<&crate::cache_key::FileHasher<'_>>,
) -> Result<bool> {
    let scan = |path: &Path| -> Result<(String, String)> {
        let bytes = fs::read(path)
            .with_context(|| format!("reading timestamp input {}", path.display()))?;
        let content = blake3::hash(&bytes).to_hex().to_string();
        let record = cc_timestamp_scan_record(&content, cc_timestamp_may_expand(&bytes));
        Ok((content, record))
    };
    if let Some(file_hasher) = file_hasher {
        let analyses = file_hasher.cc_content_hashes(inputs)
            .and_then(|contents| cc_parsed_header_scans(inputs, &contents, file_hasher));
        match analyses {
            Ok(scans) => return Ok(scans.iter().any(|scan| scan.timestamp.starts_with('1'))),
            Err(error) => log::debug!("cc timestamp analysis unavailable: {error}"),
        }
    }
    let mut sensitive = false;
    for path in inputs {
        sensitive |= scan(path)?.1.starts_with('1');
    }
    Ok(sensitive)
}

fn cc_timestamp_key(
    parsed: &CcArgs,
    inputs: &[PathBuf],
    prefix_maps: &[CcPrefixMap],
    file_hasher: Option<&crate::cache_key::FileHasher<'_>>,
) -> Result<Option<String>> {
    let sensitive = parsed.rest.iter().any(|arg| cc_timestamp_may_expand(arg.as_bytes()))
        || cc_timestamp_inputs_sensitive(inputs, file_hasher)?;
    if !sensitive {
        return Ok(None);
    }
    let mut stamps = Vec::with_capacity(inputs.len());
    for path in inputs {
        let metadata = fs::metadata(path)?;
        stamps.push((cc_mapped_path(path, prefix_maps), crate::cache_key::metadata_mtime_ns(&metadata)));
    }
    // A macro defined in a header expands with the invoking file's mtime.
    stamps.sort_unstable();
    let mut hasher = blake3::Hasher::new();
    for (name, mtime) in stamps {
        fold_cc_memo_field(&mut hasher, b"name", name.as_bytes());
        fold_cc_memo_field(&mut hasher, b"mtime", &mtime.to_le_bytes());
    }
    Ok(Some(hasher.finalize().to_hex().to_string()))
}

/// The memo's hash column for a read-set digest, flagged when path-bound.
fn cc_memo_hash(digest: &str, path_bound: bool) -> String {
    if path_bound {
        format!("{CC_MEMO_PATH_BOUND_PREFIX}{digest}")
    } else {
        digest.to_string()
    }
}

/// The digest and the path-bound flag a memo hash column carries.
fn cc_memo_hash_parts(hash: &str) -> (&str, bool) {
    match hash.strip_prefix(CC_MEMO_PATH_BOUND_PREFIX) {
        Some(digest) => (digest, true),
        None => (hash, false),
    }
}

/// Make target the compile-first dependency capture writes; never a real file.
const CC_DIRECT_CAPTURE_TARGET: &str = "__kache_direct_capture";

/// Whether the key may come from a compile-first read set instead of the
/// expansion: the invocation has the right shape and the driver is known
/// to accept the flags the capture adds.
pub(crate) fn cc_direct_key_eligible(parsed: &CcArgs) -> bool {
    cc_direct_key_shape_ok(parsed)
        && !parsed.is_universal()
        && cc_source_date_epoch_is_pinned(parsed)
        && crate::config::probe_memo_dir_for(&parsed.invocation_env).is_ok_and(|memo| {
            cc_driver_captures_dependencies_for(&parsed.program, &memo, &parsed.invocation_env)
        })
}

/// Only the GNU dialect adds a private `-MD -MF`; a caller that asked for
/// `-MMD` gets a depfile without system headers, which is not a complete
/// read set, so it keeps the classic path.
fn cc_direct_key_shape_ok(parsed: &CcArgs) -> bool {
    parsed.family.dialect() == Dialect::Gnu
        && !parsed.records_switches()
        && !parsed.emits_llvm()
        && !parsed.emits_reports()
        && parsed.mode == CompileMode::Compile
        && parsed.sources.len() == 1
        && parsed
            .depinfo
            .as_ref()
            .is_none_or(|depinfo| !depinfo.emit || depinfo.include_system)
}

/// Whether `program` compiles an object while writing a usable dependency
/// file under the flags a compile-first run adds. The `-E` probe used to
/// vouch for the driver before any flag was injected; without it, this one
/// compile of an empty unit does, once per driver binary, remembered in
/// the probe memo directory.
#[cfg(all(test, unix))]
fn cc_driver_captures_dependencies(program: &str, memo_dir: &Path) -> bool {
    cc_driver_captures_dependencies_for(program, memo_dir, &InvocationEnv::capture())
}

fn cc_driver_captures_dependencies_for(
    program: &str,
    memo_dir: &Path,
    env: &InvocationEnv,
) -> bool {
    let Some(digest) = cc_direct_probe_material(program, env) else {
        return false;
    };
    let path = crate::probe_memo::memo_path(memo_dir, "cc-direct-capture", "txt", &digest);
    if let Some(body) = crate::probe_memo::read_verified(&path, &digest) {
        return body.trim() == "ok";
    }
    let supported = {
        let _trace = crate::phase_trace::phase("cc_direct_probe");
        cc_direct_probe(program, env)
    };
    log::debug!("cc: probed {program} for compile-first capture: {supported}");
    crate::probe_memo::write_verified(&path, &digest, if supported { "ok" } else { "no" });
    supported
}

fn cc_source_date_epoch_is_pinned(parsed: &CcArgs) -> bool {
    effective_source_date_epoch_for(&parsed.invocation_env).is_some()
        && cc_driver_honors_source_date_epoch(parsed)
}

fn cc_driver_honors_source_date_epoch(parsed: &CcArgs) -> bool {
    let env = &parsed.invocation_env;
    let Ok(memo_dir) = crate::config::probe_memo_dir_for(env) else {
        return false;
    };
    let Some(digest) = cc_direct_probe_material(&parsed.program, env) else {
        return false;
    };
    let path = crate::probe_memo::memo_path(&memo_dir, "cc-source-date-epoch-v1", "txt", &digest);
    if let Some(body) = crate::probe_memo::read_verified(&path, &digest) {
        return body.trim() == "ok";
    }
    let supported = cc_source_date_epoch_probe(parsed).unwrap_or(false);
    crate::probe_memo::write_verified(&path, &digest, if supported { "ok" } else { "no" });
    supported
}

fn cc_source_date_epoch_probe(parsed: &CcArgs) -> Result<bool> {
    let env = &parsed.invocation_env;
    let dir = tempfile::Builder::new()
        .prefix("kache-cc-epoch-probe-")
        .tempdir_in(env.temp_dir())?;
    fs::write(dir.path().join("epoch.c"), "__DATE__ __TIME__\n")?;
    let mut command = env.command(&parsed.program)?;
    match parsed.family.dialect() {
        Dialect::Gnu => {
            command.args(["-E", "-P", "epoch.c"]);
        }
        Dialect::Cl => {
            command.args(["/EP", "epoch.c"]);
        }
    }
    let output = command
        .current_dir(dir.path())
        .env("KACHE_CC_KEY_PROBE", "1")
        .env("SOURCE_DATE_EPOCH", "946684800")
        .invocation_output()?;
    Ok(output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .collect::<Vec<_>>()
            == ["\"Jan", "1", "2000\"", "\"00:00:00\""])
}

fn cc_direct_probe_material(program: &str, env: &InvocationEnv) -> Option<String> {
    let resolved = env.resolve_program(Path::new(program))?;
    let canonical = std::fs::canonicalize(&resolved).ok()?;
    let metadata = std::fs::metadata(&canonical).ok()?;
    let mut material = crate::probe_memo::Material::new("cc-direct-capture-v3");
    material.push(program.as_bytes());
    material.push(canonical.as_os_str().as_encoded_bytes());
    material.push(&metadata.len().to_le_bytes());
    material.push(&crate::cache_key::metadata_mtime_ns(&metadata).to_le_bytes());
    // Driver support may depend on its selected toolchain and environment.
    let mut environment: Vec<_> = env
        .vars_os()
        .filter(|(name, _)| {
            cc_memo_env_is_keyed(name)
                || name.to_str().is_some_and(|name| {
                    matches!(
                        name.to_ascii_uppercase().as_str(),
                        "PATH" | "PATHEXT" | "HOME" | "USERPROFILE" | "TMPDIR" | "TMP" | "TEMP"
                    )
                })
        })
        .collect();
    environment.sort();
    for (name, value) in environment {
        material.push(name.as_encoded_bytes());
        material.push(value.as_encoded_bytes());
    }
    Some(material.digest())
}

fn cc_direct_probe(program: &str, env: &InvocationEnv) -> bool {
    let Ok(dir) = tempfile::Builder::new()
        .prefix("kache-cc-direct-probe-")
        .tempdir_in(env.temp_dir())
    else {
        return false;
    };
    let root = dir.path();
    if fs::write(
        root.join("probe.c"),
        "int kache_direct_probe(void) { return 0; }\n",
    )
    .is_err()
    {
        return false;
    }
    let Some(prefix_map) = root
        .to_str()
        .map(|root| format!("-ffile-prefix-map={root}=/kache/probe"))
    else {
        return false;
    };
    let Ok(mut command) = env.command(program) else {
        return false;
    };
    let output = command
        .args([
            "-c",
            "probe.c",
            "-o",
            "probe.o",
            "-MD",
            "-MF",
            "probe.d",
            "-MT",
            CC_DIRECT_CAPTURE_TARGET,
            prefix_map.as_str(),
        ])
        .current_dir(root)
        // A kache shim standing in for the compiler must run it, not key it.
        .env("KACHE_CC_KEY_PROBE", "1")
        .invocation_output();
    let Ok(output) = output else {
        return false;
    };
    let depfile = fs::read_to_string(root.join("probe.d")).unwrap_or_default();
    let ok =
        output.status.success() && root.join("probe.o").is_file() && depfile.contains("probe.c");
    log::debug!(
        "cc: {program} compile-first capture probe: exit {} object {} depfile {}: {}",
        output.status,
        root.join("probe.o").is_file(),
        depfile.contains("probe.c"),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    ok
}

/// The first construct in any of `paths` that could make the assembler read
/// a file no fingerprint covers. The classic path scans the expansion; a
/// compile-first key never sees one, so it scans what the compile read.
#[cfg(test)]
fn cc_inputs_hide_assembler_input(paths: &[PathBuf]) -> Option<&'static str> {
    paths.iter().find_map(|path| {
        let bytes = fs::read(path).ok()?;
        cc_raw_assembler_hidden_input(&bytes)
    })
}

/// [`cc_assembler_hidden_input`] for unexpanded source text. The directive
/// and named-escape checks carry over: a directive the expansion would
/// contain is spelled in some input, macro pieces included, as the escaped
/// quote a stringised operand leaves. The computed-operand check does not:
/// in raw text an `asm` whose operand is not a literal is a macro
/// definition, as in every libc's symbol-aliasing headers, not a hidden
/// string.
fn cc_raw_assembler_hidden_input(text: &[u8]) -> Option<&'static str> {
    if let Some(found) = cc_assembler_text_hidden_input(text) {
        return Some(found);
    }
    let literals = cc_string_literals(text);
    if literals.named_escape {
        return Some(r"\N{...}");
    }
    cc_assembler_text_hidden_input(&literals.text)
}

pub struct CcCompiler {
    invocation_env: InvocationEnv,
    /// User-declared flags (issue #95) that kache's built-in allow-list
    /// doesn't model but the user opted into caching. A flag here stops
    /// refusing and is folded verbatim into the cache key. Empty in the
    /// common case (and for every existing `CcCompiler::new()` caller).
    extra_allowlist_flags: Vec<String>,
    /// Deterministically ordered `[paths].base_dirs` roots applied to both the
    /// cc key probes and the real compiler invocation.
    base_dirs: Vec<String>,
    pending_preprocess_memo: RefCell<Option<PendingCcPreprocessMemo>>,
    /// The include digest folded into the key, so the publish-time recheck
    /// can reproduce it exactly.
    pending_include_dir_digest: RefCell<Option<PendingIncludeDirDigest>>,
    /// The last key bound itself to this checkout's roots. `execute` stores an
    /// object that embeds a raw root only under such a key.
    key_path_bound: Cell<bool>,
}

/// See [`CcCompiler::include_dir_names_still_match`].
struct PendingIncludeDirDigest {
    digest: String,
    /// The shadowing names the digest was computed over; `None` when the
    /// directories were walked.
    names: Option<CcShadowingNames>,
    /// Stamps of the directories the digest listed, when all could vouch
    /// for their listing.
    stamps: Option<CcListingStamps>,
}

fn named_tool_family(name: &str) -> Option<ToolFamily> {
    Some(match kache_shims::compiler_driver(name)? {
        "clang-cl" => ToolFamily::ClangCl,
        "clang" | "clang++" => ToolFamily::Clang,
        _ => ToolFamily::Gnu,
    })
}

fn is_unresolvable_bare_program(program: &str, env: &InvocationEnv) -> bool {
    if program.contains('/') {
        return false;
    }
    if program.contains('\\') {
        return false;
    }
    env.resolve_program(Path::new(program)).is_none()
}

impl Default for CcCompiler {
    fn default() -> Self {
        Self::with_extra_allowlist_flags(Vec::new())
    }
}

impl CcCompiler {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a user-declared cc flag allow-list (issue #95),
    /// typically `config.cc_extra_allowlist_flags`.
    pub fn with_extra_allowlist_flags(extra_allowlist_flags: Vec<String>) -> Self {
        Self::for_invocation(extra_allowlist_flags, &InvocationEnv::capture())
    }

    pub(crate) fn for_invocation(extra_allowlist_flags: Vec<String>, env: &InvocationEnv) -> Self {
        Self {
            invocation_env: env.clone(),
            extra_allowlist_flags,
            base_dirs: Vec::new(),
            pending_preprocess_memo: RefCell::new(None),
            pending_include_dir_digest: RefCell::new(None),
            key_path_bound: Cell::new(false),
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn with_invocation_env(mut self, env: &InvocationEnv) -> Self {
        self.invocation_env = env.clone();
        self
    }


    /// clang-cl's `-###` dump uses Windows path tokens; GNU/clang do not.
    fn cc_link_probe_is_windows_aware(family: ToolFamily) -> bool {
        family.dialect() != Dialect::Cl
    }

    fn cache_key_for_link(&self, parsed: &CcArgs, ctx: &KeyCtx<'_, '_>) -> Result<String> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"cc_link_key_version:2\n");
        hasher.update(CC_ARTIFACT_SCHEMA);
        let program_name = Path::new(&parsed.program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(parsed.program.as_str());
        hasher.update(b"compiler:");
        hasher.update(program_name.as_bytes());
        hasher.update(b"\n");
        let config_args = parsed.config_args();
        let resolved = crate::probe::probe_for(
            ctx.cache_dir,
            &crate::probe::CcProber,
            &crate::probe::ProbeRequest {
                key_env_vars: ctx.key_env_vars,
                compiler: &parsed.program,
                args: &parsed.rest,
                key_args: &config_args,
                per_tu_paths: &[],
                windows_aware: Self::cc_link_probe_is_windows_aware(parsed.family),
                base_dirs: &[],
            },
            &parsed.invocation_env,
        )?;
        hasher.update(b"compiler_version:");
        hasher.update(resolved.version_line.as_bytes());
        hasher.update(b"\n");

        let expanded = expand_cc_response_files(&parsed.rest, &parsed.invocation_env)?;
        hasher.update(b"argv:");
        for arg in &expanded {
            hasher.update(ctx.path_normalizer.normalize(arg).as_bytes());
            hasher.update(b"\x1f");
        }
        hasher.update(b"\n");

        let mut inputs = parsed.sources.clone();
        inputs.extend(cc_link_file_inputs(&expanded));
        inputs.sort();
        inputs.dedup();
        hasher.update(b"inputs:\n");
        for input in &inputs {
            let digest = ctx
                .file_hasher
                .hash(
                    &parsed
                        .invocation_env
                        .resolve(input)
                        .context("resolving cc link input")?,
                )
                .with_context(|| format!("hashing link input {}", input.display()))?;
            hasher.update(digest.as_bytes());
            hasher.update(b" ");
            hasher.update(
                ctx.path_normalizer
                    .normalize(input.to_string_lossy())
                    .as_bytes(),
            );
            hasher.update(b"\n");
        }
        let key = hasher.finalize().to_hex().to_string();
        let key = crate::cache_key::apply_key_env_vars_in(
            key,
            ctx.key_env_vars,
            "cc-link",
            ctx.invocation_env,
        );
        Ok(crate::cache_key::apply_key_salt(
            key,
            ctx.key_salt,
            "cc-link",
        ))
    }

    pub fn with_base_dirs(mut self, base_dirs: Vec<String>) -> Self {
        self.base_dirs = base_dirs;
        self.base_dirs.sort();
        self.base_dirs.dedup();
        self
    }

    /// Publish the dependency snapshot from a full preprocess only after a
    /// successful compile or cache restore revalidated every input.
    pub(crate) fn commit_preprocess_memo(&self, file_hasher: &crate::cache_key::FileHasher<'_>) {
        let Some(pending) = self.pending_preprocess_memo.borrow_mut().take() else {
            return;
        };
        let _trace = crate::phase_trace::phase("memo_commit");
        file_hasher.cc_preprocess_memo_record_if_unchanged(
            &pending.memo_key,
            &pending.preprocessed_hash,
            &pending.fingerprints,
            &|path| cc_mapped_content_hash(path, &pending.prefix_maps),
        );
    }

    /// The memo a deferred compile would publish, for a daemon to record in
    /// the wrapper's stead. The inputs were fingerprinted after the compile
    /// under the too-new guard, so a file written since the invocation
    /// started already kept the entry from being stored; nothing is left
    /// to revalidate. Only meaningful when the memo came from a capture:
    /// an expansion's inputs predate the compile and stay on the
    /// revalidating [`Self::commit_preprocess_memo`] path.
    ///
    /// Only a memo whose every stamp had settled when observed travels. A
    /// daemon from an older release records whatever stamp it is given; the
    /// wrapper's own store records an unsettled input by content alone.
    pub(crate) fn captured_preprocess_memo(&self) -> Option<crate::daemon_publish::CcMemoHandoff> {
        let pending = self.pending_preprocess_memo.borrow();
        let pending = pending.as_ref()?;
        if !pending.captured
            || !pending
                .fingerprints
                .iter()
                .all(crate::cache_key::CcPreprocessMemoInput::stamp_settled)
        {
            return None;
        }
        Some(crate::daemon_publish::CcMemoHandoff {
            memo_key: pending.memo_key.clone(),
            preprocessed_hash: pending.preprocessed_hash.clone(),
            inputs: pending
                .fingerprints
                .iter()
                .map(crate::daemon_publish::memo_input_for_wire)
                .collect(),
        })
    }

    /// Forget the pending memo: a daemon took it.
    pub(crate) fn discard_preprocess_memo(&self) {
        self.pending_preprocess_memo.borrow_mut().take();
    }

    /// True when user include-dir names still match the digest folded into
    /// the key. A new shadowing header during compile must not be published
    /// under the old key.
    pub(crate) fn include_dir_names_still_match(&self, parsed: &CcArgs) -> bool {
        let pending = self.pending_include_dir_digest.borrow();
        let Some(pending) = pending.as_ref() else {
            return false;
        };
        // Directories whose stamps still hold list as they did for the key,
        // so the digest over them is unchanged.
        if let Some(stamps) = &pending.stamps
            && stamps.iter().all(|(dir, stamp)| stamp.holds(dir))
        {
            return true;
        }
        // Recompute the way the key did, over the same names. Resolving
        // against a different set of names than the key used would compare
        // two unrelated digests and refuse every store.
        let now = match &pending.names {
            Some(names) => digest_cc_shadowing_names(parsed, names),
            None => digest_cc_include_dir_names(parsed),
        };
        now.is_ok_and(|now| now == pending.digest)
    }

    /// Does this argv invoke a C-family compiler?
    ///
    /// Matches `cc`, `c++`, `gcc`, `g++`, `clang`, `clang++`, their
    /// versioned variants (`gcc-13`, `clang++-17`), and target-prefixed
    /// cross compilers (`arm-linux-gnueabihf-gcc`). Path-prefixed forms
    /// (`/usr/bin/cc`, `C:\path\clang.exe`) and Windows `.exe` suffixes
    /// are accepted.
    ///
    /// Owns its own detection rule; `super::detect_compiler` reaches it
    /// through this module's [`ADAPTER`] descriptor.
    #[cfg(test)]
    pub fn recognizes(args: &[String]) -> bool {
        Self::recognizes_for(args, &InvocationEnv::capture())
    }

    /// Compiler names accepted without executing a family probe.
    pub(crate) fn recognizes_name(program: &str) -> bool {
        kache_shims::is_cc_compiler_name(program)
    }

    pub(crate) fn recognizes_for(args: &[String], env: &InvocationEnv) -> bool {
        if super::is_workspace_wrapper_chain_for(args, env) {
            return false;
        }
        let Some(arg0) = args.first() else {
            return false;
        };
        let Some(name) = super::command_basename(arg0) else {
            return false;
        };
        let name = super::strip_windows_exe_suffix(name).to_ascii_lowercase();

        if Self::recognizes_name(arg0) {
            return true;
        }

        // ── Slow path: `-E` probe for unknown binaries ──
        if super::is_kache_subcommand_or_flag(&name) {
            return false;
        }
        if is_unresolvable_bare_program(arg0, env) {
            return false;
        }
        // A version/info query (e.g. Kani's `kani-compiler -vV`, #656) compiles
        // nothing, so there is nothing to cache — and *running* an unknown
        // program just to sniff its family would add a spurious invocation to a
        // pure passthrough. Leave it unrecognized so it passes through untouched.
        if super::is_version_or_info_query(&args[1..]) {
            return false;
        }

        crate::probe::probe_compiler_family_for(arg0, env).is_some()
    }

    /// Does this argv match the `cc` Rust crate's compiler-family
    /// probe shape, `kache -E <file>`?
    ///
    /// The cc crate uses this probe to detect compiler family
    /// (gcc / clang / MSVC) by reading `__VERSION__` from preprocessor
    /// output. It hardcodes `Command::new(program).arg("-E").arg(file)`,
    /// dropping any trailing args from `CC="kache cc"` — so without
    /// explicit passthrough kache would clap-error and the probe
    /// would silently fall back to a default family guess. Today
    /// that's a logged warning; once C/C++ caching lands and family
    /// identifies the cache key, it becomes silent miscaching across
    /// machines.
    ///
    /// Match is intentionally tight (`-E` + at least one more arg).
    /// Other probe shapes (`-?`, `-dumpmachine`, `-dumpversion`) can
    /// land here when their absence becomes a real symptom —
    /// over-broad matching would mask legitimate CLI typos.
    ///
    /// **Not a compiler adapter.** A probe is a non-compiler invocation
    /// pattern that happens to need passthrough. The dispatch in
    /// `run_wrapper_mode` checks this *before* the compiler match.
    pub fn recognizes_family_probe(args: &[String]) -> bool {
        args.len() >= 2 && args[0] == "-E"
    }
}

/// Does `key` name a `CC`/`CXX` compiler variable the `cc` crate reads?
///
/// Mirrors the crate's `getenv_with_target_prefixes("CC"|"CXX")`: the
/// bare name, a `<target>` suffix (`CC_aarch64_pc_windows_msvc`), or a
/// `TARGET_`/`HOST_` prefix. Deliberately excludes neighbours like
/// `CFLAGS`, `CXXFLAGS`, and `CCACHE_*` whose values are not
/// `<wrapper> <compiler>` pairs.
fn is_cc_family_env_key(key: &str) -> bool {
    let base = key
        .strip_prefix("TARGET_")
        .or_else(|| key.strip_prefix("HOST_"))
        .unwrap_or(key);
    base == "CC" || base == "CXX" || base.starts_with("CC_") || base.starts_with("CXX_")
}

/// Is `key` a C++ (`CXX`) compiler variable, as opposed to C (`CC`)?
fn is_cxx_env_key(key: &str) -> bool {
    let base = key
        .strip_prefix("TARGET_")
        .or_else(|| key.strip_prefix("HOST_"))
        .unwrap_or(key);
    base == "CXX" || base.starts_with("CXX_")
}

/// Does `token` (a path or bare name) refer to the kache binary itself?
fn probe_token_is_self(token: &str, self_stem: &str) -> bool {
    super::command_basename(token)
        .map(super::strip_windows_exe_suffix)
        .is_some_and(|name| name.eq_ignore_ascii_case(self_stem))
}

/// Recover the real compiler the `cc` crate dropped from a family probe.
///
/// When `CC="kache <compiler>"` the cc crate mis-parses it — kache is
/// not in the crate's hard-coded known-wrapper allowlist (`ccache`,
/// `sccache`, `distcc`, …), so it treats kache as the *compiler* and
/// `<compiler>` as a leading argument, then drops that argument when it
/// runs the family probe (`Command::new(path).arg("-E").arg(file)`).
/// kache therefore receives `kache -E <file>` with no compiler to
/// forward to.
///
/// The compiler is still recoverable: the very `CC`/`CXX` variable the
/// cc crate read still holds `kache <compiler>` in our environment.
/// Scan those variables, find the one whose first whitespace token is
/// us, and return `<compiler>` so the probe can forward to the real
/// thing — yielding the genuine compiler family instead of a wrong
/// default guess (issue #286: `cc` is absent on Windows MSVC, so the
/// old hard-coded `cc` forward failed and the build fell back to an
/// unsupported GNU family).
///
/// Selection mirrors the cc crate's own `getenv_with_target_prefixes`
/// precedence so kache forwards to the exact variable the crate read
/// when several are kache-wrapped (mozbuild sets a host *and* a target
/// compiler): for a given `<name>` in `CC`, then `CXX`, the order is
/// `<name>_<target>`, `<name>_<target-underscored>`, `TARGET_<name>`,
/// `<name>`, `HOST_<name>`. `target` comes from cargo's `TARGET` env
/// var (set for build scripts). When `target` is `None`, selection
/// falls back to a deterministic order (CC before CXX, then the
/// lexicographically smallest key) so it never depends on environment
/// iteration order.
///
/// `CC` is preferred over `CXX` because the probe file is C and kache
/// cannot tell from `-E <file>` alone whether the cc crate's probe
/// belongs to a C or C++ `Build`. When `CC` and `CXX` are kache-wrapped
/// with *different* compiler families this can mislabel a C++ probe —
/// harmless in practice (the cc crate treats GNU and Clang identically;
/// only MSVC diverges, and a kache-wrapped MSVC `CXX` paired with a
/// non-MSVC `CC` does not occur in real toolchains).
///
/// Returns `None` when no kache-wrapped compiler variable is present.
pub(crate) fn resolve_probe_compiler<I>(
    self_stem: &str,
    target: Option<&str>,
    env_vars: I,
) -> Option<String>
where
    I: IntoIterator<Item = (String, String)>,
{
    // Collect every kache-wrapped CC/CXX variable: key -> real compiler.
    let mut wrapped: HashMap<String, String> = HashMap::new();
    for (key, value) in env_vars {
        if !is_cc_family_env_key(&key) {
            continue;
        }
        let mut tokens = value.split_whitespace();
        let Some(first) = tokens.next() else { continue };
        // The first token must be us; otherwise this is a plain
        // compiler, not a kache-wrapped one.
        if !probe_token_is_self(first, self_stem) {
            continue;
        }
        let Some(real) = tokens.next() else { continue };
        // Guard against a degenerate `CC="kache kache"`.
        if probe_token_is_self(real, self_stem) {
            continue;
        }
        wrapped.entry(key).or_insert_with(|| real.to_string());
    }
    if wrapped.is_empty() {
        return None;
    }

    // cc-crate precedence: most-specific target var first, CC before CXX.
    for name in ["CC", "CXX"] {
        if let Some(t) = target {
            if let Some(c) = wrapped.get(&format!("{name}_{t}")) {
                return Some(c.clone());
            }
            let underscored = t.replace('-', "_");
            if underscored != t
                && let Some(c) = wrapped.get(&format!("{name}_{underscored}"))
            {
                return Some(c.clone());
            }
            if let Some(c) = wrapped.get(&format!("TARGET_{name}")) {
                return Some(c.clone());
            }
        }
        if let Some(c) = wrapped.get(name) {
            return Some(c.clone());
        }
        if let Some(c) = wrapped.get(&format!("HOST_{name}")) {
            return Some(c.clone());
        }
    }

    // No precedence key matched (e.g. only a target-suffixed var for an
    // unknown target): deterministic fallback — CC family before CXX,
    // then the lexicographically smallest key.
    let mut keys: Vec<&String> = wrapped.keys().collect();
    keys.sort_by(|a, b| {
        is_cxx_env_key(a)
            .cmp(&is_cxx_env_key(b))
            .then_with(|| a.cmp(b))
    });
    keys.first().map(|k| wrapped[*k].clone())
}

impl Compiler for CcCompiler {
    type Parsed = CcArgs;

    fn id(&self) -> CompilerId {
        CC_ID
    }

    fn parse(&self, args: &[String]) -> Result<CcArgs> {
        let expanded = cc_args_with_response_files(args, &self.invocation_env);
        let mut parsed = CcArgs::parse_for(expanded.as_ref().map(|value| value.0.as_slice()).unwrap_or(args), &self.invocation_env)?;
        if let Some((_, digests)) = expanded {
            parsed.response_file_digests = digests;
            parsed.original_args = Some(args[1..].to_vec());
        }
        parsed.depinfo_roots =
            crate::path_normalizer::PathNormalizer::from_invocation(None, &self.invocation_env)
                .with_base_dirs(&self.base_dirs)
                .depinfo_source_roots()
                .into_iter()
                .map(|root| (root.root, root.depinfo_sentinel, root.priority))
                .collect();
        Ok(parsed)
    }

    fn refuse_reasons(&self, parsed: &CcArgs) -> Vec<RefuseReason> {
        // Per-case detection from the parsed shape. The skeleton
        // catch-all is gone — single-source `-c` compiles with no
        // unsafe flags now produce an EMPTY refuse list, which is the
        // signal to the wrapper that this invocation is cacheable.
        // Link mode is always refused: whole-program link caching was
        // removed, so links run uncached.
        parsed.refuse_reasons(&self.extra_allowlist_flags)
    }

    fn execute(&self, parsed: &CcArgs) -> Result<CompileResult> {
        self.execute_with_extra_args(parsed, Vec::new(), false)
            .map(|(result, _)| result)
    }

    fn classify_output(&self, _parsed: &CcArgs, name: &str) -> ArtifactKind {
        // Caching is not active; classification only matters once outputs
        // get stored. Delegate to the shared filename-based classifier so
        // when the cc store path lands, the kinds it produces are already
        // consistent with the rustc table for shared extensions (.o, .a,
        // .dylib, etc.).
        classify_by_filename(name)
    }
}

impl CcCompiler {
    #[cfg(test)]
    pub(crate) fn key_for_test(&self, parsed: &CcArgs, ctx: &KeyCtx<'_, '_>) -> String {
        match self.cache_key_with(parsed, ctx, CcKeyDiscovery::Expansion) {
            Ok(CcKeyOutcome::Key(key)) => key,
            _ => panic!("expected a cache key"),
        }
    }

    /// The key, or the request to compile first when `discovery` allows it
    /// and no memo describes this invocation's read set.
    pub(crate) fn cache_key_with(
        &self,
        parsed: &CcArgs,
        ctx: &KeyCtx<'_, '_>,
        discovery: CcKeyDiscovery,
    ) -> Result<CcKeyOutcome> {
        self.cache_key_under(CC_ARTIFACT_SCHEMA, parsed, ctx, discovery)
    }

    /// [`Self::cache_key_with`], with the compile key under `artifact_schema`.
    fn cache_key_under(
        &self,
        artifact_schema: &[u8],
        parsed: &CcArgs,
        ctx: &KeyCtx<'_, '_>,
        discovery: CcKeyDiscovery,
    ) -> Result<CcKeyOutcome> {
        if parsed.mode == CompileMode::Link {
            return self.cache_key_for_link(parsed, ctx).map(CcKeyOutcome::Key);
        }
        // Preconditions (guaranteed by the wrapper checking
        // refuse_reasons first): `-c` mode, exactly one source.
        self.pending_preprocess_memo.borrow_mut().take();
        self.key_path_bound.set(false);
        let _trace = crate::phase_trace::phase("key");
        let mut hasher = blake3::Hasher::new();
        let trace_name = cc_trace_name(parsed);
        let prefix_maps = {
            let _trace = crate::phase_trace::phase("cc_prefix_maps");
            cc_prefix_maps(parsed, &self.base_dirs)
        };
        for map in &prefix_maps {
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] prefix_map {} => {}",
                trace_name,
                map.from,
                map.to
            );
        }

        hasher.update(b"cc_key_version:");
        hasher.update(crate::cache_key::CACHE_KEY_VERSION.to_string().as_bytes());
        hasher.update(b"\n");
        hasher.update(artifact_schema);
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] cc_key_version={}",
            trace_name,
            crate::cache_key::CACHE_KEY_VERSION
        );

        if !self.base_dirs.is_empty() {
            hasher.update(b"configured_base_dirs.v1:");
            hasher.update(self.base_dirs.len().to_string().as_bytes());
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] configured_base_dirs={}",
                trace_name,
                self.base_dirs.len()
            );
        }

        let mut prefix_sentinels: Vec<&str> = Vec::new();
        for map in &prefix_maps {
            if !prefix_sentinels.contains(&map.to.as_str()) {
                prefix_sentinels.push(map.to.as_str());
            }
        }
        prefix_sentinels.sort_unstable();

        // Expansions that spell out a checkout root are hashed raw (see
        // `hash_cc_expansion`). Naming the scheme retires entries stored before
        // it, which mapped those roots to a sentinel and could hand another
        // checkout an object naming this one (kunobi-ninja/kache#1004).
        hasher.update(b"expansion_roots:literal-bound.v1\n");

        hasher.update(b"prefix_maps:");
        for sentinel in prefix_sentinels {
            hasher.update(sentinel.as_bytes());
            hasher.update(b"\x1f");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] cc_prefix_map={}",
                trace_name,
                sentinel
            );
        }
        hasher.update(b"\n");

        // Compiler identity: family name (cc / gcc / clang — affects
        // codegen defaults) + the version string.
        let program_name = Path::new(&parsed.program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(parsed.program.as_str());
        hasher.update(b"compiler:");
        hasher.update(program_name.as_bytes());
        hasher.update(b"\n");
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] compiler={}",
            trace_name,
            program_name
        );
        // Compiler probe, memoized: the version line (`cc --version`,
        // compiler identity) and the resolved invocation (`cc -###`,
        // the driver's fully-expanded `-cc1` line). One probe per build
        // per flag set; the rest of the build reads the record.
        let config_args = parsed.config_args();
        let probe_args = parsed.probe_args();
        // Per-TU paths to blank from the shared probe record's resolved
        // tokens, so the record is invariant across the build's TUs and
        // parallel builds don't race on whose paths it holds (#keyrace).
        let per_tu_paths = cc_resolved_per_tu_paths(parsed);
        let trace_probe = crate::phase_trace::phase("cc_probe");
        let resolved = crate::probe::probe_for(
            ctx.cache_dir,
            &crate::probe::CcProber,
            &crate::probe::ProbeRequest {
                key_env_vars: ctx.key_env_vars,
                compiler: &parsed.program,
                args: &probe_args,
                key_args: &config_args,
                per_tu_paths: &per_tu_paths,
                // Sentinel Windows paths only for gnu/clang (objects are
                // remapped via -ffile-prefix-map). clang-cl keeps raw
                // native paths → key stays path-literal (#299/#312).
                windows_aware: parsed.family.dialect() != Dialect::Cl,
                base_dirs: &cc_declared_base_dirs(&prefix_maps),
            },
            &parsed.invocation_env,
        )?;
        drop(trace_probe);
        if resolved.resolved_tokens.is_none() && cc_flags_need_resolved_invocation(parsed) {
            anyhow::bail!("cc: resolved invocation unavailable for probe-captured flags");
        }
        hasher.update(b"compiler_version:");
        hasher.update(resolved.version_line.as_bytes());
        hasher.update(b"\n");
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] compiler_version={}",
            trace_name,
            resolved.version_line
        );

        // Resolved compiler invocation: the `cc -###` `-cc1` line with
        // host-local paths sentinelled. Captures codegen the modeled
        // flags below miss — compiler defaults (`-mrelocation-model`,
        // `-ffp-contract`, the resolved `-target-cpu` and feature set).
        // If `-###` cannot be resolved, we can only proceed when no
        // accepted flag relies on those resolved tokens for safety.
        //
        // Tokens are hashed IN ORDER, and order is significant — that
        // is correct, not an oversight. `cc -###` is deterministic, so
        // the same (compiler, flags, env) always yields the same token
        // order: the key is stable, with no spurious misses. The tokens
        // must NOT be sorted — they interleave flag/value pairs as
        // adjacent elements (`-target-cpu`, `apple-m1`), so sorting the
        // flat list would scramble those pairs. The only cost of
        // order-significance is that two *different* flag invocations
        // that happen to resolve to the same object (same tokens,
        // different order) get different keys — a cache miss, never a
        // miscache. That is the safe direction.
        if let Some(tokens) = &resolved.resolved_tokens {
            hasher.update(b"resolved:");
            for (index, tok) in tokens.iter().enumerate() {
                // Resolved `cc -###` tokens carry absolute build paths —
                // `-I` dirs, `-D NAME="/abs/.../foo.ico"` defines, input /
                // `-o` paths — that embed the build directory. Hashing them
                // raw makes the key path-dependent, so two builds of the
                // same TU at different paths (a teammate's checkout, a CI
                // runner, the bench's cross-clone warm phase) miss. Run them
                // through the SAME prefix maps as the preprocessor stdout so
                // the build root collapses to `<CC_ROOT>`/`<CC_BUILD>` and
                // the key is path-portable. Mapping only ever merges keys
                // that differ solely in build path (same object, remapped at
                // compile time via `-ffile-prefix-map`) — never a miscache.
                if tok.starts_with("-crash-diagnostics-dir=") {
                    continue;
                }
                let mut token = tok.clone();
                for (flag, resolved_prefix) in [
                    ("-fprofile-use=", "-fprofile-instrument-use-path="),
                    ("--warning-suppression-mappings=", "--warning-suppression-mappings="),
                    ("-fsanitize-ignorelist=", "-fsanitize-ignorelist="),
                ] {
                    if parsed.rest.iter().filter_map(|arg| arg.strip_prefix(flag))
                        .any(|path| !path.is_empty() && token.strip_prefix(resolved_prefix) == Some(path))
                    {
                        token = format!("{resolved_prefix}<cc-file-input>");
                    }
                }
                let mapped = normalize_resolved_cc_token(
                    &token,
                    index.checked_sub(1).map(|previous| tokens[previous].as_str()),
                    &prefix_maps,
                );
                hasher.update(&mapped);
                hasher.update(b"\x1f");
                log::trace!(
                    target: "kache::cache_key",
                    "[key:{}] resolved_token={}",
                    trace_name,
                    String::from_utf8_lossy(&mapped)
                );
            }
            hasher.update(b"\n");
        }

        // Target architecture.
        let arch = cc_target_arch(parsed);
        hasher.update(b"arch:");
        hasher.update(arch.as_bytes());
        hasher.update(b"\n");
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] arch={}",
            trace_name,
            arch
        );

        // Codegen-affecting flags. These are partly redundant with
        // the preprocessor hash (defines affect macro expansion,
        // -std gates language features) but the redundancy is cheap
        // and defends against e.g. -std affecting codegen without
        // changing the expanded text.
        if let Some(opt) = parsed.optimization {
            hasher.update(b"opt:");
            hasher.update(format!("{opt:?}").as_bytes());
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] opt={opt:?}",
                trace_name
            );
        }
        if let Some(dbg) = parsed.debug_level {
            hasher.update(b"debug:");
            hasher.update(&[dbg]);
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] debug={dbg}",
                trace_name
            );
        }
        // clang-cl debug objects embed per-TU path inputs the rest of
        // the key misses: the source path/filename and output (-Fo) name
        // (both stripped from config_args, so the memoized `-###` tokens
        // can't be trusted for them) and the compilation dir (CWD, not in
        // the key at all). Fold them so distinct objects never share a
        // key — capture, not remap (clang-cl is path-literal; #299/#312).
        if let Some(paths) = cl_debug_path_inputs(parsed) {
            hasher.update(b"cl_debug_paths:");
            for p in &paths {
                hasher.update(p.as_bytes());
                hasher.update(b"\x1f");
                log::trace!(
                    target: "kache::cache_key",
                    "[key:{}] cl_debug_path={}",
                    trace_name,
                    p
                );
            }
            hasher.update(b"\n");
        }
        // The read-set digest below hashes contents only, so two
        // byte-identical sources at different paths would share a key. Their
        // objects differ wherever the compiler spells the source path:
        // `__FILE__`, `assert`, the debug info's file name. Fold the path in
        // the spelling the object gets, so checkouts the maps cover still
        // agree.
        hasher.update(b"source_paths:");
        for source in &parsed.sources {
            let mapped = cc_mapped_path(source, &prefix_maps);
            hasher.update(mapped.as_bytes());
            hasher.update(b"\x1f");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] source_path={}",
                trace_name,
                mapped
            );
        }
        hasher.update(b"\n");
        if let Some(std) = &parsed.std {
            hasher.update(b"std:");
            hasher.update(std.as_bytes());
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] std={}",
                trace_name,
                std
            );
        }
        hasher.update(b"pic:");
        hasher.update(&[parsed.pic as u8]);
        hasher.update(b"\n");
        // `-S` writes the assembly `-c` would assemble; the expansion and
        // flags are otherwise the same, so the mode must separate the keys.
        if parsed.mode == CompileMode::Assemble {
            hasher.update(b"mode:assemble\n");
        }
        if parsed.mode == CompileMode::Precompile {
            hasher.update(b"mode:precompile\n");
        }
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] pic={}",
            trace_name,
            parsed.pic
        );

        for arg in &parsed.rest {
            for prefix in ["-fprofile-use=", "--warning-suppression-mappings=", "-fsanitize-ignorelist="] {
                if let Some(path) = arg.strip_prefix(prefix) {
                    // GCC reads `<path>/<mangled object>.gcda`, not `path` itself.
                    anyhow::ensure!(
                        !(prefix == "-fprofile-use=" && parsed.family == ToolFamily::Gnu),
                        "gcc -fprofile-use= names a profile directory whose per-object files the key cannot see"
                    );
                    let resolved = parsed.invocation_env.resolve(Path::new(path))
                        .context("resolving cc flag input")?;
                    let digest = ctx.file_hasher.hash(&resolved)
                        .with_context(|| format!("hashing cc flag input {}", resolved.display()))?;
                    hasher.update(b"cc_flag_input:");
                    hasher.update(prefix.as_bytes());
                    hasher.update(digest.as_bytes());
                    hasher.update(b"\n");
                }
            }
        }

        if let Some(modules) = cc_module_inputs_key(parsed, ctx.file_hasher)? {
            hasher.update(b"modules.v1:");
            hasher.update(modules.as_bytes());
            hasher.update(b"\n");
            log::trace!(target: "kache::cache_key", "[key:{}] modules={}", trace_name, modules);
        }
        if let Some(pch) = cc_implicit_pch_key(parsed, ctx.file_hasher)? {
            hasher.update(b"implicit-pch.v1:");
            hasher.update(pch.as_bytes());
            hasher.update(b"\n");
            log::trace!(target: "kache::cache_key", "[key:{}] implicit_pch={}", trace_name, pch);
        }

        // Built-in raw-keyed flags have object effects the resolved compiler
        // probe does not expose (notably GNU assembler-only `-Wa` options).
        // Hash their normalized argument bytes in argv order; a length prefix
        // keeps adjacent arguments unambiguous without assuming any character
        // cannot appear in an argv element.
        let raw_flags = cc_raw_flags_for_key(parsed, &prefix_maps);
        if !raw_flags.is_empty() {
            hasher.update(b"cc_raw_flags:");
            for flag in raw_flags {
                hasher.update(&(flag.len() as u64).to_le_bytes());
                hasher.update(&flag);
                log::trace!(
                    target: "kache::cache_key",
                    "[key:{}] cc_raw_flag={}",
                    trace_name,
                    String::from_utf8_lossy(&flag)
                );
            }
            hasher.update(b"\n");
        }

        // Diagnostic formatting stays out of the probe so the read-set memo
        // is shared. Color and display flags only change diagnostics, so the
        // entry records them instead (`diagnostic_format_fingerprint`). An
        // output format can print even when there is nothing to report
        // (GCC's JSON prints `[]`), so it keys the entry.
        let diagnostic_format: Vec<String> = parsed
            .diagnostic_format_args()
            .into_iter()
            .filter(|arg| arg.starts_with("-fdiagnostics-format"))
            .collect();
        if !diagnostic_format.is_empty() {
            hasher.update(b"diagnostic_format.v1:");
            for arg in &diagnostic_format {
                hasher.update(&(arg.len() as u64).to_le_bytes());
                hasher.update(arg.as_bytes());
            }
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] diagnostic_format={}",
                trace_name,
                diagnostic_format.join(" ")
            );
        }

        // User-declared cc flags (issue #95). The built-in table doesn't
        // model these; the user opted them into caching via
        // `[cc] extra_allowlist_flags`. kache can't know how each affects
        // codegen, so it folds the flag string *verbatim* — a different
        // flag (or value) is a different string, hence a different key
        // (never a miscache by value). Only flags actually present on the
        // command line are folded (an unused allow-list entry has no
        // codegen effect and must not move the key), sorted + deduped so
        // argv order and repeats don't perturb the key.
        let matched = cc_extra_flags_for_key(parsed, &self.extra_allowlist_flags);
        if !matched.is_empty() {
            hasher.update(b"cc_extra_flags:");
            for flag in matched {
                hasher.update(flag.as_bytes());
                hasher.update(b"\x1f");
                log::trace!(
                    target: "kache::cache_key",
                    "[key:{}] cc_extra_flag={}",
                    trace_name,
                    flag
                );
            }
            hasher.update(b"\n");
        }

        // The object bytes do not depend on dep-info flags, but the cached
        // artifact set now can include a `.d` sidecar. Key the dep-info
        // content shape so an object-only entry never satisfies an invocation
        // that expects dependency output, and so flags like `-MD` vs `-MMD`
        // or `-MT` do not share incompatible sidecars.
        // Every field folded here is also emitted to the `kache::cache_key`
        // trace target so `KACHE_E2E_KEYTRACE` can attribute a cross-clone miss
        // to dep-info (these were previously folded but untraced, leaving such
        // divergences invisible in the keytrace diff).
        hasher.update(b"depinfo:");
        if let Some(depinfo) = parsed.depinfo.as_ref().filter(|d| d.emit) {
            hasher.update(b"1\n");
            hasher.update(b"depinfo_include_system:");
            hasher.update(&[depinfo.include_system as u8]);
            hasher.update(b"\n");
            hasher.update(b"depinfo_phony_targets:");
            hasher.update(&[depinfo.phony_targets as u8]);
            hasher.update(b"\n");
            hasher.update(b"depinfo_missing_generated:");
            hasher.update(&[depinfo.missing_generated as u8]);
            hasher.update(b"\n");
            // The targets the file names. An object-only target is stored as
            // a placeholder and restored as this invocation's object, so it
            // keys as the placeholder; any other target is in the file as
            // spelled.
            let depinfo_target = match cc_depinfo_object_target(parsed) {
                Some(_) => CC_DEPINFO_OBJECT_PLACEHOLDER.to_string(),
                None if depinfo.targets.is_empty() => parsed
                    .object_output_path()
                    .map(|object| format!("T{}", object.to_string_lossy()))
                    .unwrap_or_default(),
                None => depinfo
                    .targets
                    .iter()
                    .map(|(target, quoted)| format!("{}{target}", if *quoted { 'Q' } else { 'T' }))
                    .collect::<Vec<_>>()
                    .join("\x1f"),
            };
            hasher.update(b"depinfo_target.v2:");
            hasher.update(depinfo_target.as_bytes());
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] depinfo=1 include_system={} phony_targets={} missing_generated={} target={}",
                trace_name,
                depinfo.include_system,
                depinfo.phony_targets,
                depinfo.missing_generated,
                depinfo_target
            );
        } else {
            hasher.update(b"0\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] depinfo=0",
                trace_name
            );
        }

        if let Some(sidecars) = cc_sidecar_key(parsed) {
            hasher.update(b"sidecars.v1:");
            hasher.update(sidecars.as_bytes());
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] sidecars={}",
                trace_name,
                sidecars.replace('\x1f', " ")
            );
        }

        // Preprocessor expansion — the load-bearing input. Captures
        // the source plus every transitively-included header plus
        // macro expansion, with line positions keyed apart from the
        // text (see `split_line_markers`); SOURCE_DATE_EPOCH pins
        // __DATE__/__TIME__ (stable across builds).
        let trace_memo_key = crate::phase_trace::phase("cc_memo_key");
        let memo_key = ctx
            .file_hasher
            .supports_cc_preprocess_memo()
            .then(|| cc_preprocess_memo_key(parsed, &prefix_maps, &resolved.version_line))
            .filter(|_| !parsed.is_universal())
            .flatten();
        drop(trace_memo_key);
        // A content-matching read set is not enough: a formerly absent
        // header, or a new shadowing header, changes what preprocessing reads.
        // Validate resolution before accepting the memo and reuse this work
        // for the artifact key below.
        let mut memo_shadowing = None;
        let memo_hit = if matches!(&discovery, CcKeyDiscovery::Captured(_)) {
            None
        } else {
            memo_key.as_ref().and_then(|key| {
                let _trace = crate::phase_trace::phase("cc_memo_lookup");
                ctx.file_hasher.cc_preprocess_memo_lookup(
                    key,
                    |name| cc_unmapped_path_candidates(name, &prefix_maps),
                    &|path| (!parsed.lists_assembler_source()).then(|| cc_mapped_content_hash(path, &prefix_maps)).flatten(),
                )
            })
        };
        let memo_hit = memo_hit
            .map(|(hash, inputs)| -> Result<_> {
                let Some((hash, expected_resolution)) = hash.split_once('@') else {
                    return Ok(None);
                };
                let names = cc_shadowing_names(parsed, &inputs, &prefix_maps, ctx.file_hasher)?;
                let (resolution, stamps) =
                    digest_cc_shadowing_names_stamped(parsed, &names, Some(ctx.file_hasher))?;
                if resolution != expected_resolution {
                    log::debug!("cc preprocess memo: include resolution differs from the recorded one");
                    return Ok(None);
                }
                memo_shadowing = Some((names, resolution, stamps));
                Ok(Some((hash.to_string(), inputs)))
            })
            .transpose()?
            .flatten();
        // The read set comes back with the expansion, from the memo when it
        // answers and from the dependency capture otherwise. It is what makes
        // the shadowing resolution below possible.
        let (pp_hash, read_inputs) = if let CcKeyDiscovery::Captured(captured) = discovery {
            // The compile already ran and this is what it read: the digest
            // of that read set stands where the expansion hash would, and
            // the memo records it under the same identity.
            let digest = cc_direct_inputs_digest(&captured.fingerprints);
            self.key_path_bound.set(captured.path_bound);
            let read_inputs = captured
                .fingerprints
                .iter()
                .map(|input| PathBuf::from(input.local_path()))
                .collect::<Vec<_>>();
            if let Some(memo_key) = memo_key {
                self.pending_preprocess_memo
                    .borrow_mut()
                    .replace(PendingCcPreprocessMemo {
                        memo_key,
                        preprocessed_hash: cc_memo_hash(&digest, captured.path_bound),
                        fingerprints: captured.fingerprints,
                        prefix_maps: prefix_maps.clone(),
                        captured: true,
                    });
            }
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] preprocessed_memo=captured",
                trace_name
            );
            (digest, Some(read_inputs))
        } else if let Some((memo_hash, satisfied)) = memo_hit {
            let (memo_hash, path_bound) = cc_memo_hash_parts(&memo_hash);
            self.key_path_bound.set(path_bound);
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] preprocessed_memo=hit path_bound={path_bound}",
                trace_name
            );
            (memo_hash.to_string(), Some(satisfied))
        } else if let (CcKeyDiscovery::Deferrable, Some(memo_key)) = (&discovery, &memo_key)
            && cc_direct_key_eligible(parsed)
            && parsed
                .invocation_env
                .cwd()
                .is_some_and(|cwd| cc_memo_identity_portable(parsed, cwd, &prefix_maps))
            && !ctx.file_hasher.cc_preprocess_memo_recorded(memo_key)
        {
            // Nothing recorded for this invocation: the compile has to run,
            // so let it run first and key from what it read. A memo that
            // exists but no longer matches keeps the preprocess below, which
            // can still find the entry an earlier state of the tree left.
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] preprocessed_memo=none deferred",
                trace_name
            );
            return Ok(CcKeyOutcome::Deferred(CcDeferredKey {
                memo_key: memo_key.clone(),
            }));
        } else {
            let _trace = crate::phase_trace::phase("cc_preprocess");
            let preprocessed = if parsed.is_universal() {
                preprocess_universal_hash(parsed, &prefix_maps, ctx.file_hasher)?
            } else {
                preprocess_hash(parsed, &prefix_maps, ctx.file_hasher, Some(&resolved))?
            };
            self.key_path_bound.set(preprocessed.path_bound);
            let read_inputs = preprocessed.fingerprints.as_ref().map(|inputs| {
                inputs
                    .iter()
                    .map(|input| PathBuf::from(input.local_path()))
                    .collect::<Vec<_>>()
            });
            // With the read set in hand the key is its digest, the same key a
            // compile-first run derives, so the two paths find each other's
            // entries. The expansion cannot key alone: it spells tokens apart
            // by one space whatever their columns were, and debug info and
            // diagnostics carry those columns. Without a memo key the build
            // does not pin `__DATE__` and `__TIME__`, which only the expansion
            // shows, so the key folds both.
            let Some(fingerprints) = &preprocessed.fingerprints else {
                anyhow::bail!(
                    "cc: dependency capture gave no read set to key; the expansion alone loses column positions"
                );
            };
            let digest = cc_direct_inputs_digest(fingerprints);
            let hash = if memo_key.is_some() {
                digest
            } else {
                format!("{digest}+{}", preprocessed.hash)
            };
            if let (Some(memo_key), Some(fingerprints)) = (memo_key, preprocessed.fingerprints) {
                self.pending_preprocess_memo
                    .borrow_mut()
                    .replace(PendingCcPreprocessMemo {
                        memo_key,
                        preprocessed_hash: cc_memo_hash(&hash, preprocessed.path_bound),
                        fingerprints,
                        prefix_maps: prefix_maps.clone(),
                        captured: false,
                    });
            }
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] preprocessed_memo=miss",
                trace_name
            );
            (hash, read_inputs)
        };
        hasher.update(b"preprocessed:");
        hasher.update(pp_hash.as_bytes());
        hasher.update(b"\n");
        if parsed.emits_reports() {
            let inputs = read_inputs.as_deref().context("report key requires the original read set")?;
            let contents = ctx.file_hasher.cc_content_hashes(inputs)?;
            let mut fields = inputs.iter().zip(contents).map(|(path, content)| {
                (parsed.invocation_env.resolve(path).unwrap_or_else(|| path.clone()), content)
            }).collect::<Vec<_>>();
            fields.sort_unstable();
            hasher.update(b"cc_report_inputs.v1:");
            hasher.update(format!("{fields:?}").as_bytes());
            hasher.update(b"\n");
            log::trace!(target: "kache::cache_key", "[key:{trace_name}] report_inputs={fields:?}");
        }
        if let Some(timestamp) = cc_timestamp_key(
            parsed,
            read_inputs.as_deref().unwrap_or_default(),
            &prefix_maps,
            Some(ctx.file_hasher),
        )? {
            hasher.update(b"timestamp_mtimes.v1:");
            hasher.update(timestamp.as_bytes());
            hasher.update(b"\n");
        }
        if let Some(pch) = cc_pch_key(parsed, read_inputs.as_deref())? {
            hasher.update(b"pch.v1:");
            hasher.update(pch.as_bytes());
            hasher.update(b"\n");
            log::trace!(target: "kache::cache_key", "[key:{}] pch={}", trace_name, pch);
        }
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] preprocessed={}",
            trace_name,
            pp_hash
        );
        // PCH and coverage outputs can record absolute paths that prefix maps
        // do not rewrite, even when the preprocessor expansion is portable.
        if parsed.mode == CompileMode::Precompile || parsed.coverage_instrumented() || parsed.records_switches() || parsed.emits_llvm() || parsed.emits_reports() {
            self.key_path_bound.set(true);
        }
        // A read set whose object spells a checkout root is keyed to that
        // checkout: the digest is portable, the roots folded here are not,
        // which is the point. Another checkout reading the same memo folds
        // its own roots and misses.
        if self.key_path_bound.get() {
            let mut roots: Vec<&str> = prefix_maps
                .iter()
                .map(|map| map.from.as_str())
                .filter(|from| !from.is_empty())
                .collect();
            roots.sort_unstable();
            roots.dedup();
            hasher.update(b"path_bound_roots:");
            for root in &roots {
                hasher.update(root.as_bytes());
                hasher.update(b"\0");
            }
            hasher.update(b"\n");
            log::trace!(
                target: "kache::cache_key",
                "[key:{}] path_bound_roots={}",
                trace_name,
                roots.len()
            );
        }

        // Shadowing. A header appearing in an earlier `-I`/`-iquote` dir (or
        // next to the source) can take the place of one that was read without
        // changing any fingerprint, so the key has to notice it. With the read
        // set in hand this resolves only the names that could actually be
        // shadowed; without it (a compiler whose dependency capture failed)
        // fall back to enumerating the directories, which is what the capped
        // walk has always done.
        let trace_shadowing = crate::phase_trace::phase("cc_shadowing");
        let (names, include_dir_digest, stamps, include_dir_mode) = match memo_shadowing {
            Some((names, digest, stamps)) => (Some(names), digest, stamps, "resolved"),
            None => {
                let names = read_inputs
                    .as_deref()
                    .map(|inputs| cc_shadowing_names(parsed, inputs, &prefix_maps, ctx.file_hasher))
                    .transpose()?;
                let (digest, stamps, mode) = match &names {
                    Some(names) => {
                        let (digest, stamps) = digest_cc_shadowing_names_stamped(
                            parsed,
                            names,
                            Some(ctx.file_hasher),
                        )?;
                        (digest, stamps, "resolved")
                    }
                    None => (digest_cc_include_dir_names(parsed)?, None, "walked"),
                };
                (names, digest, stamps, mode)
            }
        };
        if let Some(pending) = self.pending_preprocess_memo.borrow_mut().as_mut() {
            pending.preprocessed_hash.push('@');
            pending.preprocessed_hash.push_str(&include_dir_digest);
        }
        drop(trace_shadowing);
        self.pending_include_dir_digest
            .borrow_mut()
            .replace(PendingIncludeDirDigest {
                digest: include_dir_digest.clone(),
                names,
                stamps,
            });
        hasher.update(b"include_dir_names:");
        hasher.update(include_dir_digest.as_bytes());
        hasher.update(b"\n");
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] include_dir_names={} mode={}",
            trace_name,
            include_dir_digest,
            include_dir_mode
        );

        let key = hasher.finalize().to_hex().to_string();
        // A cc-rs crate's C sources can carry the same out-of-band inputs as
        // its Rust siblings; the crate dir is the source file's nearest
        // enclosing `Cargo.toml`. Reaching cache_key means refuse_reasons
        // already gated this invocation, so it is exactly one source and
        // unconditionally cacheable — pass `is_primary = true`. The assert
        // pins that precondition so a future caller that bypasses the gate
        // fails loudly instead of silently anchoring extra_inputs to the
        // first of several sources.
        debug_assert_eq!(
            parsed.sources.len(),
            1,
            "cc cache_key expects a single-source compile (refuse_reasons gates the rest)"
        );
        let key = crate::extra_inputs::apply_extra_inputs(
            key,
            parsed.sources.first().map(|p| p.as_path()),
            &trace_name,
            true,
            ctx.file_hasher,
        );
        let key = crate::cache_key::apply_key_env_vars_in(
            key,
            ctx.key_env_vars,
            &trace_name,
            ctx.invocation_env,
        );
        let key = crate::cache_key::apply_key_salt(key, ctx.key_salt, &trace_name);
        log::trace!(
            target: "kache::cache_key",
            "[key:{}] final={}",
            trace_name,
            &key[..16]
        );
        Ok(CcKeyOutcome::Key(key))
    }

    /// Compile first, capturing what the compiler read, for a unit whose key
    /// was deferred. The read set comes from the caller's own `-MD` depfile
    /// when there is one, else from a private one the compile is asked to
    /// write. `None` inputs mean the compile ran but cannot be keyed: the
    /// capture failed, an input is unreadable, or a file the assembler reads
    /// hides from every fingerprint.
    pub(crate) fn execute_capturing_inputs(
        &self,
        parsed: &CcArgs,
        file_hasher: &crate::cache_key::FileHasher<'_>,
    ) -> Result<(CompileResult, Option<CcCapturedInputs>)> {
        let caller_depfile = parsed.depinfo_output_path();
        let private = if caller_depfile.is_none() {
            match tempfile::Builder::new()
                .prefix("kache-cc-capture-")
                .tempdir_in(parsed.invocation_env.temp_dir())
            {
                Ok(dir) => Some(dir),
                Err(error) => {
                    log::debug!("cc capture tempdir unavailable: {error}");
                    None
                }
            }
        } else {
            None
        };
        let private_path = private
            .as_ref()
            .and_then(|dir| capture_depfile(dir.path()));
        let extra = match &private_path {
            Some((_, arg)) => vec![
                "-MD".to_string(),
                "-MF".to_string(),
                arg.clone(),
                "-MT".to_string(),
                CC_DIRECT_CAPTURE_TARGET.to_string(),
            ],
            None => Vec::new(),
        };
        let (result, path_bound) = self.execute_with_extra_args(parsed, extra, true)?;
        if result.exit_code != 0 {
            return Ok((result, None));
        }
        let depfile = match (caller_depfile, private_path) {
            (Some(path), _) => path,
            (None, Some((path, _))) => path,
            (None, None) => return Ok((result, None)),
        };
        let inputs = {
            let _trace = crate::phase_trace::phase("cc_capture");
            self.captured_inputs(parsed, &depfile, file_hasher)
        };
        let inputs_changed = file_hasher.too_new();
        Ok((
            result,
            inputs.map(|fingerprints| CcCapturedInputs {
                fingerprints,
                path_bound,
                inputs_changed,
            }),
        ))
    }

    fn captured_inputs(
        &self,
        parsed: &CcArgs,
        depfile: &Path,
        file_hasher: &crate::cache_key::FileHasher<'_>,
    ) -> Option<Vec<crate::cache_key::CcPreprocessMemoInput>> {
        let depfile = parsed.invocation_env.resolve(depfile)?;
        let raw = match fs::read_to_string(&depfile) {
            Ok(raw) => raw,
            Err(error) => {
                log::debug!("cc capture depfile unreadable: {error}");
                return None;
            }
        };
        let cwd = parsed.invocation_env.cwd()?;
        let mut paths = match parse_preprocess_dependencies(&raw, cwd) {
            Ok(paths) => paths,
            Err(error) => {
                log::debug!("cc capture depfile unusable: {error:#}");
                return None;
            }
        };
        // The depfile lists the source itself; adding it again is harmless,
        // the fingerprints are deduplicated by name.
        for source in &parsed.sources {
            paths.push(if source.is_absolute() {
                source.clone()
            } else {
                cwd.join(source)
            });
        }
        let prefix_maps = cc_prefix_maps(parsed, &self.base_dirs);
        let mapped: Vec<(String, PathBuf)> = paths
            .into_iter()
            .map(|path| (cc_mapped_path(&path, &prefix_maps), path))
            .collect();
        let inputs = file_hasher.cc_preprocess_fingerprints(
            &mapped,
            &cc_prefix_maps_key(&prefix_maps),
            &|path| cc_mapped_content_hash(path, &prefix_maps),
        )?;
        // After the fingerprints: the scan is memoised by the content hash
        // they carry, so a header shared by every unit is read once.
        if let Some(found) = file_hasher.cc_inputs_hide_assembler_input(&inputs, &|path| {
            let bytes = fs::read(path).ok()?;
            Some((
                blake3::hash(&bytes).to_hex().to_string(),
                cc_raw_assembler_hidden_input(&bytes),
            ))
        }) {
            let why = if found == crate::cache_key::CC_INPUT_CHANGED_WHILE_SCANNED {
                found
            } else {
                format!("the assembler may read a file the key cannot see (`{found}`)")
            };
            log::debug!(
                "cc: {} not keyed from its read set: {why}",
                cc_trace_name(parsed)
            );
            return None;
        }
        Some(inputs)
    }

    /// Run the compiler. With `bind_on_embedded_root`, an object that spells
    /// a checkout root is kept and reported as path-bound instead of being
    /// dropped: the caller keys it to this checkout.
    fn execute_with_extra_args(
        &self,
        parsed: &CcArgs,
        extra: Vec<String>,
        bind_on_embedded_root: bool,
    ) -> Result<(CompileResult, bool)> {
        let mut path_bound = false;
        // Invoke the underlying compiler with the original argv, plus a
        // set of `-ffile-prefix-map` rules so the object doesn't embed
        // clone-local build/source roots. Spliced in before any `--`
        // separator (see `compose_cc_args`) so the driver still reads
        // them as flags, then last among the flags so they win over any
        // user-supplied map for the same prefix.
        crate::opcounts::record_compiler_run();
        let _trace = crate::phase_trace::phase("cc_compile");
        let mut command = parsed.invocation_env.command(&parsed.program)?;
        let prefix_maps = cc_prefix_maps(parsed, &self.base_dirs);
        let mut appended = file_prefix_map_args(&prefix_maps);
        appended.extend(extra);
        let args = compose_cc_args(&parsed.rest, appended);
        let _args_snapshot = match cc_response_file_snapshot(parsed, &args)? {
            Some((snapshot, command_args)) => {
                command.args(&command_args);
                Some(snapshot)
            }
            None => {
                command.args(&args);
                None
            }
        };
        // Pin the same effective SOURCE_DATE_EPOCH the `-E` key probe used, so a
        // TU baking __DATE__/__TIME__ produces an object whose date
        // matches its time-stable cache key. Mirrors the existing __FILE__
        // normalization (we already rewrite paths via -ffile-prefix-map); pinning
        // the date is the same stance. Opt out with
        // KACHE_CC_SOURCE_DATE_EPOCH=passthrough (#423).
        if let Some(epoch) = effective_source_date_epoch_for(&parsed.invocation_env) {
            command.env("SOURCE_DATE_EPOCH", epoch);
        }
        let sidecars_before = snapshot_cc_sidecars(parsed);
        let output = command
            .invocation_output()
            .with_context(|| format!("executing {}", parsed.program))?;
        let exit_code = output.status.code().unwrap_or(1);

        // Output discovery: on a successful compile the named output is the
        // cacheable artifact. Skip on failure, and skip for the modes that
        // are still refused upstream, where discovery would guess at a file
        // the invocation never wrote. `-E` writing to a named file reaches
        // here now, and its expansion is discovered the same way an object
        // is. Objects retain their basename; dep-info gets a semantic store
        // name because `-MF` permits arbitrary suffixes and restore already
        // takes its real destination from the current invocation
        // (kunobi-ninja/kache#655).
        let discovers_outputs = matches!(parsed.mode, CompileMode::Compile)
            || (parsed.emits_llvm() && parsed.mode == CompileMode::Assemble)
            || (matches!(
                parsed.mode,
                CompileMode::Preprocess | CompileMode::Assemble | CompileMode::Precompile
            ) && parsed.output.is_some());
        let mut keepalive = Vec::new();
        let mut artifacts = if exit_code == 0 && discovers_outputs {
            with_cc_sidecars(parsed, discover_cc_output_artifacts(parsed), sidecars_before)
        } else {
            ArtifactSet::empty()
        };
        if exit_code == 0 && cc_expansion_is_stdout(parsed) {
            match stage_cc_stdout_artifact(&output.stdout, &parsed.invocation_env) {
                Ok((artifact, temp)) => {
                    artifacts = ArtifactSet::new(vec![artifact]);
                    keepalive.push(temp);
                }
                Err(error) => {
                    log::warn!("cc: staging -E stdout failed: {error:#}; not caching it");
                }
            }
        } else {
            // Safety net for #1004. The key probe catches roots in the expansion.
            // A root that reaches the object some other way would still be stored
            // under a key every checkout shares, so keep the object for this build
            // and store nothing. It only sees roots spelled out as plain bytes.
            let embeds = parsed
                .object_output_path()
                .and_then(|path| parsed.invocation_env.resolve(&path))
                .map(|object| {
                    let sidecars = artifacts
                        .outputs()
                        .iter()
                        .filter(|artifact| CcSidecar::from_store_name(&artifact.store_name).is_some())
                        .map(|artifact| artifact.path.clone());
                    std::iter::once(object).chain(sidecars).try_fold(false, |found, path| {
                        Ok(found || cc_object_embeds_mapped_root(&path, &prefix_maps)?)
                    })
                });
            if bind_on_embedded_root && !artifacts.is_empty() && matches!(embeds, Some(Ok(true))) {
                path_bound = true;
            } else {
                let unsafe_to_store =
                    cc_unsafe_to_store(artifacts.is_empty(), self.key_path_bound.get(), || embeds);
                artifacts = match unsafe_to_store {
                    None => artifacts,
                    Some(reason) => {
                        log::warn!("cc: {} {reason}; not caching it", cc_trace_name(parsed));
                        ArtifactSet::empty()
                    }
                };
            }
        }

        Ok((
            CompileResult {
                exit_code,
                signaled: output.status.code().is_none(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                pending_stderr: None,
                artifacts,
                keepalive,
            },
            path_bound,
        ))
    }
}

fn stage_cc_stdout_artifact(
    bytes: &[u8],
    env: &InvocationEnv,
) -> Result<(Artifact, tempfile::TempPath)> {
    let mut staged = tempfile::Builder::new()
        .prefix("kache-cc-stdout-")
        .suffix(".i")
        .tempfile_in(env.temp_dir())
        .context("cc: creating -E stdout staging file")?;
    staged
        .write_all(bytes)
        .context("cc: writing -E stdout staging file")?;
    staged
        .flush()
        .context("cc: flushing -E stdout staging file")?;
    let temp = staged.into_temp_path();
    Ok((
        Artifact {
            path: temp.to_path_buf(),
            store_name: CC_STDOUT_STORE_NAME.to_string(),
            kind: ArtifactKind::Other("stdout"),
            required: true,
        },
        temp,
    ))
}

/// Replace each `@file` argument with the arguments the file holds.
///
/// GCC (libiberty `buildargv`) and Clang (`TokenizeGNUCommandLine`) split a
/// response file the same way for everything accepted here: whitespace
/// separates arguments, a backslash takes the next character literally, and
/// single or double quotes group text. Where they differ the file is refused:
/// an empty quoted argument (GCC keeps it, Clang drops it), a backslash
/// before a line break or at the end, and carriage returns. Both drivers
/// resolve nested `@file` paths from cwd in GNU mode, not the containing file.
/// Clang config/quoting overrides, hidden `--`, cycles, excessive expansion, and missing
/// or non-UTF-8 files are refused.
fn expand_cc_response_files(args: &[String], env: &InvocationEnv) -> Result<Vec<String>> {
    expand_cc_response_files_tracked(args, env).map(|value| value.0)
}

fn expand_cc_response_files_tracked(args: &[String], env: &InvocationEnv) -> Result<(Vec<String>, Vec<String>)> {
    fn visit(args: &[String], env: &InvocationEnv, active: &mut Vec<PathBuf>, expanded: &mut Vec<String>, digests: &mut Vec<String>, bytes: &mut usize) -> Result<()> {
        for arg in args {
            anyhow::ensure!(expanded.len() < 16_384, "cc response argument limit");
            anyhow::ensure!(!arg.starts_with("--rsp-quoting") && !arg.starts_with("--config"), "cc response config/quoting override — not yet");
            let Some(path) = arg.strip_prefix('@').filter(|path| !path.is_empty()) else {
                anyhow::ensure!(active.is_empty() || arg != "--", "cc response file hides --");
                expanded.push(arg.clone());
                continue;
            };
            anyhow::ensure!(active.len() < 32 && digests.len() < 4096, "cc response nesting/file limit");
            let path = env.resolve(Path::new(path)).context("resolving cc response file")?;
            let canonical = fs::canonicalize(&path).context("resolving cc response identity")?;
            anyhow::ensure!(!active.contains(&canonical), "cc response file cycle");
            anyhow::ensure!(fs::metadata(&path)?.len() <= 1_048_576, "cc response file size limit");
            let body = fs::read_to_string(&path).with_context(|| format!("reading cc response file {}", path.display()))?;
            *bytes = bytes.saturating_add(body.len());
            anyhow::ensure!(*bytes <= 1_048_576 && !body.contains('\0'), "cc response byte limit or NUL");
            let tokens = gnu_response_file_arguments(&body)?;
            digests.push(blake3::hash(body.as_bytes()).to_hex().to_string());
            active.push(canonical);
            visit(&tokens, env, active, expanded, digests, bytes)?;
            active.pop();
        }
        Ok(())
    }
    let (mut expanded, mut digests) = (Vec::new(), Vec::new());
    visit(args, env, &mut Vec::new(), &mut expanded, &mut digests, &mut 0)?;
    Ok((expanded, digests))
}

/// `args` with its response files expanded, when it has any and they can be
/// expanded safely. GNU drivers only: clang-cl splits them the Windows way.
/// `None` leaves the `@file` arguments in place, and they refuse.
fn cc_args_with_response_files(args: &[String], env: &InvocationEnv) -> Option<(Vec<String>, Vec<String>)> {
    let (program, rest) = args.split_first()?;
    if !rest.iter().any(|arg| arg.len() > 1 && arg.starts_with('@'))
        || ToolFamily::detect(program, rest).dialect() != Dialect::Gnu
    {
        return None;
    }
    match expand_cc_response_files_tracked(rest, env) {
        Ok((expanded, digests)) => Some((std::iter::once(program.clone()).chain(expanded).collect(), digests)),
        Err(error) => {
            log::debug!("cc response file not expanded: {error:#}");
            None
        }
    }
}

/// The keyed arguments of an invocation that named response files, written to
/// a private response file so the compiler cannot read a body edited after
/// keying, while argv stays as compact as the caller's. A top-level `--` and
/// the arguments after it stay on the command line. `None` passes `args`
/// directly: no response file was expanded, or an empty argument would
/// tokenize differently under GCC and Clang.
fn cc_response_file_snapshot(
    parsed: &CcArgs,
    args: &[String],
) -> Result<Option<(tempfile::NamedTempFile, Vec<std::ffi::OsString>)>> {
    if parsed.response_file_digests.is_empty() || args.iter().any(String::is_empty) {
        return Ok(None);
    }
    let split = args.iter().position(|arg| arg == "--").unwrap_or(args.len());
    let (flags, tail) = args.split_at(split);
    let mut body = String::new();
    for arg in flags {
        body.push('\'');
        for c in arg.chars() {
            if matches!(c, '\\' | '\'') {
                body.push('\\');
            }
            body.push(c);
        }
        body.push_str("'\n");
    }
    let mut snapshot = tempfile::Builder::new()
        .prefix("kache-cc-args-")
        .suffix(".rsp")
        .tempfile_in(parsed.invocation_env.temp_dir())
        .context("creating cc response file snapshot")?;
    snapshot
        .write_all(body.as_bytes())
        .and_then(|()| snapshot.flush())
        .context("writing cc response file snapshot")?;
    let mut at_file = std::ffi::OsString::from("@");
    at_file.push(snapshot.path());
    let command_args = std::iter::once(at_file)
        .chain(tail.iter().map(std::ffi::OsString::from))
        .collect();
    Ok(Some((snapshot, command_args)))
}

pub(crate) fn gnu_response_file_arguments(body: &str) -> Result<Vec<String>> {
    anyhow::ensure!(!body.contains('\r'), "carriage returns split differently between GCC and Clang");
    anyhow::ensure!(!body.contains('\x0B'), "GCC splits on vertical tab and Clang does not");
    let mut tokens = Vec::new();
    let mut chars = body.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_ascii_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return Ok(tokens);
        }
        let mut token = String::new();
        let mut quote = None;
        while let Some(c) = chars.next() {
            match (c, quote) {
                ('\\', _) => match chars.next() {
                    Some('\n') | None => anyhow::bail!("a backslash ends a line or the file"),
                    Some(escaped) => token.push(escaped),
                },
                (c, Some(open)) if c == open => quote = None,
                (c, Some(_)) => token.push(c),
                ('\'' | '"', None) => quote = Some(c),
                (c, None) if c.is_ascii_whitespace() => break,
                (c, None) => token.push(c),
            }
        }
        anyhow::ensure!(!token.is_empty(), "an empty quoted argument is kept by GCC and dropped by Clang");
        tokens.push(token);
    }
}

fn cc_link_file_inputs(args: &[String]) -> Vec<PathBuf> {
    let mut inputs = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "-o" || arg == "-I" || arg == "-L" || arg == "-include" {
            skip_next = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        let path = PathBuf::from(arg);
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if matches!(
            ext,
            "o" | "obj" | "a" | "lib" | "so" | "dylib" | "dll" | "res" | "lo"
        ) {
            inputs.push(path);
        }
    }
    inputs
}

fn discover_cc_link_sidecars(output: &Path) -> Vec<Artifact> {
    let Some(stem) = output.file_stem().map(|name| name.to_os_string()) else {
        return Vec::new();
    };
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut sidecars = Vec::new();
    for (suffix, kind) in [
        (".map", ArtifactKind::Other("map")),
        (".pdb", ArtifactKind::DebugSidecar),
        (".lib", ArtifactKind::Library),
        (".exp", ArtifactKind::Other("exp")),
    ] {
        let mut name = stem.clone();
        name.push(suffix);
        let path = parent.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file()) {
            let store_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            sidecars.push(Artifact {
                path,
                kind,
                store_name,
                required: false,
            });
        }
    }
    let dsym = parent.join(format!(
        "{}.dSYM",
        output
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    if dsym.is_dir() {
        let file_name = output
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        sidecars.push(Artifact {
            path: dsym,
            kind: ArtifactKind::DebugBundle,
            store_name: format!("{file_name}.dsym.tar"),
            required: false,
        });
    }
    sidecars
}

/// What a sidecar path held before the compile, to tell a fresh write from a
/// file an earlier build left there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CcSidecarSnapshot {
    sidecar: CcSidecarOutput,
    path: Option<PathBuf>,
    before: Option<(std::time::SystemTime, u64)>,
}

fn cc_sidecar_stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

pub(crate) fn snapshot_cc_sidecars(parsed: &CcArgs) -> Vec<CcSidecarSnapshot> {
    parsed
        .sidecar_outputs()
        .into_iter()
        .map(|sidecar| {
            let path = parsed.invocation_env.resolve(&sidecar.path);
            let before = path.as_deref().and_then(cc_sidecar_stamp);
            CcSidecarSnapshot { sidecar, path, before }
        })
        .collect()
}

/// The sidecars this compile wrote, or why its outputs cannot be stored.
///
/// A sidecar counts only when this compile wrote it. A required one that is
/// missing means the compiler put it somewhere the model did not predict. A
/// file left unchanged from before the compile cannot be told apart from a
/// stale one. Either way the entry would restore the wrong set.
fn discover_cc_sidecars(snapshots: Vec<CcSidecarSnapshot>) -> Result<Vec<Artifact>, String> {
    let mut artifacts = Vec::new();
    for snapshot in snapshots {
        let name = snapshot.sidecar.role.store_name();
        let path = snapshot
            .path
            .ok_or_else(|| format!("{name} path could not be resolved"))?;
        let after = std::fs::symlink_metadata(&path).ok();
        let written = match &after {
            None => false,
            Some(meta) if !meta.file_type().is_file() || !regular_output_is_independent(&path, meta) => {
                return Err(format!("{} is not a plain file", path.display()));
            }
            Some(_) => match snapshot.before {
                None => true,
                Some(before) => cc_sidecar_stamp(&path) != Some(before),
            },
        };
        match (written, after.is_some(), snapshot.sidecar.presence) {
            (true, _, _) => artifacts.push(Artifact {
                path,
                store_name: name.to_string(),
                kind: ArtifactKind::Other("sidecar"),
                required: true,
            }),
            (false, false, CcSidecarPresence::IfWritten) => {}
            (false, false, CcSidecarPresence::Required) => {
                return Err(format!("the compiler did not write {}", path.display()));
            }
            (false, true, _) => {
                return Err(format!("{} is unchanged since before the compile", path.display()));
            }
        }
    }
    Ok(artifacts)
}

fn with_cc_sidecars(
    parsed: &CcArgs,
    artifacts: ArtifactSet,
    snapshots: Vec<CcSidecarSnapshot>,
) -> ArtifactSet {
    if artifacts.is_empty() || snapshots.is_empty() {
        return artifacts;
    }
    match discover_cc_sidecars(snapshots) {
        Ok(sidecars) => {
            let mut outputs = artifacts.outputs().to_vec();
            outputs.extend(sidecars);
            ArtifactSet::new(outputs)
        }
        Err(reason) => {
            log::warn!("cc: {}: {reason}; not caching it", cc_trace_name(parsed));
            ArtifactSet::empty()
        }
    }
}

/// Discover a successful C/C++ compile's cacheable outputs while preserving
/// the parsed role of `-MF` instead of trying to recover it from a filename.
fn discover_cc_output_artifacts(parsed: &CcArgs) -> ArtifactSet {
    // Store only lexical regular files. Following a symlink here would let an
    // output such as `/dev/null` or a FIFO enter the blob-ingest path, while a
    // symlink needs compiler-specific replacement semantics on later runs.
    fn is_plain_file(path: &std::path::Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|meta| {
            meta.file_type().is_file() && regular_output_is_independent(path, &meta)
        })
    }

    let Some(object) = parsed
        .object_output_path()
        .and_then(|path| parsed.invocation_env.resolve(&path))
        .filter(|path| is_plain_file(path))
    else {
        return ArtifactSet::empty();
    };
    let object_name = object
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (kind, store_name) = match parsed.mode {
        CompileMode::Compile => (ArtifactKind::Object, CC_OBJECT_STORE_NAME.to_string()),
        _ => (classify_by_filename(&object_name), object_name),
    };
    let mut outputs = vec![Artifact {
        path: object.clone(),
        kind,
        store_name,
        required: true,
    }];
    if parsed.mode == CompileMode::Link {
        outputs.extend(discover_cc_link_sidecars(&object));
    }

    if let Some(depinfo) = parsed
        .depinfo_output_path()
        .and_then(|path| parsed.invocation_env.resolve(&path))
        .filter(|path| is_plain_file(path))
    {
        outputs.push(Artifact {
            path: depinfo,
            store_name: CC_DEPINFO_STORE_NAME.to_string(),
            kind: ArtifactKind::DepInfo,
            required: true,
        });
    }

    ArtifactSet::new(outputs)
}

#[cfg(test)]
mod tests;
