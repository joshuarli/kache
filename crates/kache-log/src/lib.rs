use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, Once};

use log::Log;

/// Parses the `KACHE_LOG` / `KACHE_LOG_FILE` filter dialect: a comma-separated
/// list where each segment is a bare level (`warn`), a `kache=<level>` target
/// override (including module paths like `kache::cache_key=trace`), or `off`.
/// The last applicable segment wins. Segments naming other targets are
/// ignored; anything else makes the whole value invalid (`None`), and the
/// caller falls back to its default filter.
pub fn parse_level_filter(input: &str) -> Option<log::LevelFilter> {
    let mut found: Option<log::LevelFilter> = None;
    for segment in input.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((target, level)) = segment.split_once('=') {
            let target = target.trim();
            if target == "kache" || target.starts_with("kache::") {
                found = Some(parse_level(level.trim())?);
            }
        } else if segment == "kache" {
            found = Some(log::LevelFilter::Trace);
        } else {
            found = Some(parse_level(segment)?);
        }
    }
    found
}

fn parse_level(raw: &str) -> Option<log::LevelFilter> {
    let level = raw.to_ascii_lowercase();
    match level.as_str() {
        "off" => Some(log::LevelFilter::Off),
        "error" => Some(log::LevelFilter::Error),
        "warn" | "warning" => Some(log::LevelFilter::Warn),
        "info" => Some(log::LevelFilter::Info),
        "debug" => Some(log::LevelFilter::Debug),
        "trace" => Some(log::LevelFilter::Trace),
        _ => None,
    }
}

/// A file sink with its own level gate.
pub struct FileSink {
    file: Mutex<File>,
    level: log::LevelFilter,
}

impl FileSink {
    pub fn new(file: File, level: log::LevelFilter) -> Self {
        Self {
            file: Mutex::new(file),
            level,
        }
    }
}

struct InnerConfig {
    stderr_level: log::LevelFilter,
    file: Option<FileSink>,
}

impl InnerConfig {
    fn max_level(&self) -> log::LevelFilter {
        self.stderr_level.max(
            self.file
                .as_ref()
                .map(|sink| sink.level)
                .unwrap_or(log::LevelFilter::Off),
        )
    }
}

/// Per-invocation log routing, resolved from the invocation's own
/// environment: a stderr level plus an optional file sink.
///
/// A compiler invocation served by the daemon runs in a process whose
/// startup environment is the daemon's, not the client's. Entering the
/// client's config while the request runs routes that request's records to
/// the client's stderr routing and file, the way a per-invocation
/// subscriber used to. Cheap to clone: the file handle is shared.
#[derive(Clone)]
pub struct InvocationConfig {
    inner: std::sync::Arc<InnerConfig>,
}

impl InvocationConfig {
    pub fn new(stderr_level: log::LevelFilter, file: Option<FileSink>) -> Self {
        Self {
            inner: std::sync::Arc::new(InnerConfig { stderr_level, file }),
        }
    }

    fn need(&self) -> log::LevelFilter {
        self.inner.max_level()
    }
}

thread_local! {
    static SCOPE: RefCell<Vec<std::sync::Arc<InnerConfig>>> = const { RefCell::new(Vec::new()) };
}

static INSTALLED: std::sync::OnceLock<std::sync::Arc<InnerConfig>> =
    std::sync::OnceLock::new();

/// The config that would route a record logged on this thread right now:
/// the innermost entered invocation, else the installed logger's config,
/// else silent.
pub fn current() -> InvocationConfig {
    let inner = SCOPE.try_with(|scope| scope.borrow().last().cloned()).ok().flatten().or_else(|| INSTALLED.get().cloned()).unwrap_or_else(|| {
        std::sync::Arc::new(InnerConfig {
            stderr_level: log::LevelFilter::Off,
            file: None,
        })
    });
    InvocationConfig { inner }
}

/// Enters `config` for the calling thread until the guard drops. Scopes
/// nest; the innermost wins. Entering also ratchets the process-wide max
/// level up when the scope needs more than is currently enabled, so a
/// request asking for trace records is not dropped by the install-time
/// level before its records reach this thread's routing. The max is never
/// lowered: concurrent threads without a scope keep their own routing.
pub fn enter(config: &InvocationConfig) -> InvocationGuard {
    let need = config.need();
    if need > log::max_level() {
        log::set_max_level(need);
    }
    SCOPE.with(|scope| scope.borrow_mut().push(config.inner.clone()));
    InvocationGuard { _private: () }
}

/// Ends the innermost invocation scope on the calling thread.
pub struct InvocationGuard {
    _private: (),
}

impl Drop for InvocationGuard {
    fn drop(&mut self) {
        SCOPE.with(|scope| {
            scope.borrow_mut().pop();
        });
    }
}

/// Minimal process-wide logger: one line per record on stderr, optionally
/// mirrored to a file with an independent level gate. An entered
/// [`InvocationConfig`] reroutes the calling thread's records without
/// touching this logger or any other thread.
pub struct KacheLogger {
    stderr_write: Box<dyn Fn() -> Box<dyn Write + Send> + Send + Sync>,
    config: InvocationConfig,
}

impl KacheLogger {
    /// `stderr_write` is a factory (not a single handle) so each record goes
    /// through the calling thread's current stderr routing.
    pub fn new(
        stderr_level: log::LevelFilter,
        stderr_write: impl Fn() -> Box<dyn Write + Send> + Send + Sync + 'static,
        file: Option<FileSink>,
    ) -> Self {
        Self {
            stderr_write: Box::new(stderr_write),
            config: InvocationConfig::new(stderr_level, file),
        }
    }

    pub fn max_level(&self) -> log::LevelFilter {
        self.config.need()
    }

    fn active(&self) -> std::sync::Arc<InnerConfig> {
        SCOPE
            .try_with(|scope| scope.borrow().last().cloned())
            .ok()
            .flatten()
            .unwrap_or_else(|| self.config.inner.clone())
    }

    /// Test seam: route one record through this logger without installing
    /// anything process-wide.
    pub fn emit(&self, level: log::Level, target: &str, message: &str) {
        let args = format_args!("{message}");
        self.log(
            &log::Record::builder()
                .level(level)
                .target(target)
                .args(args)
                .build(),
        );
    }
}

impl log::Log for KacheLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        let active = self.active();
        if record.level() > active.max_level() {
            return;
        }
        if Capture::push(record) {
            return;
        }
        let line = format!("{} {}\n", record.level(), record.args());
        if record.level() <= active.stderr_level {
            let _ = (self.stderr_write)().write_all(line.as_bytes());
        }
        if let Some(sink) = &active.file
            && record.level() <= sink.level
        {
            let _ = sink
                .file
                .lock()
                .map(|mut file| file.write_all(line.as_bytes()));
        }
    }

    fn flush(&self) {}
}

/// Installs `logger` as the process-wide logger. Only the first call wins;
/// later calls drop their logger, so tests and repeated invocations in one
/// process stay safe.
pub fn install(logger: KacheLogger) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let max = logger.max_level();
        let _ = INSTALLED.set(logger.config.inner.clone());
        let logger: &'static KacheLogger = Box::leak(Box::new(logger));
        if log::set_logger(logger).is_ok() {
            log::set_max_level(max);
        }
    });
}

thread_local! {
    static CAPTURED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

struct Capture;

impl Capture {
    /// Pushes a rendered record into the calling thread's capture buffer when
    /// one is active. Returns whether a buffer was active.
    fn push(record: &log::Record<'_>) -> bool {
        CAPTURED.with(|slot| {
            if let Some(buffer) = slot.borrow_mut().as_mut() {
                buffer.push(format!("{} {}", record.level(), record.args()));
                true
            } else {
                false
            }
        })
    }
}

/// Thread-local log capture for tests. Records logged on the calling thread
/// while the guard lives are collected as `LEVEL message` lines instead of
/// reaching any sink, so concurrent tests stay isolated from each other.
pub struct CaptureGuard {
    _private: (),
}

impl CaptureGuard {
    fn new() -> Self {
        ensure_test_logger();
        CAPTURED.with(|slot| {
            *slot.borrow_mut() = Some(Vec::new());
        });
        Self { _private: () }
    }

    /// Drains the records captured so far without ending the capture.
    pub fn records(&self) -> Vec<String> {
        CAPTURED.with(|slot| slot.borrow().clone().unwrap_or_default())
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURED.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

/// Starts capturing the calling thread's log records. The returned guard ends
/// the capture on drop; [`CaptureGuard::records`] reads what was captured.
pub fn capture() -> CaptureGuard {
    CaptureGuard::new()
}

struct TestLogger;

impl log::Log for TestLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        Capture::push(record);
    }

    fn flush(&self) {}
}

fn ensure_test_logger() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        static LOGGER: TestLogger = TestLogger;
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_parses_the_values_callers_actually_set() {
        use log::LevelFilter::{Debug, Error, Info, Off, Trace, Warn};
        assert_eq!(parse_level_filter("off"), Some(Off));
        assert_eq!(parse_level_filter("warn"), Some(Warn));
        assert_eq!(parse_level_filter("kache=debug"), Some(Debug));
        assert_eq!(parse_level_filter("kache=warn"), Some(Warn));
        assert_eq!(parse_level_filter("trace"), Some(Trace));
        assert_eq!(parse_level_filter("kache=info"), Some(Info));
        assert_eq!(parse_level_filter("error"), Some(Error));
        assert_eq!(parse_level_filter("WARNING"), Some(Warn));
        assert_eq!(parse_level_filter("kache"), Some(Trace));
        assert_eq!(parse_level_filter("invalid["), None);
        assert_eq!(parse_level_filter("kache=bogus"), None);
        assert_eq!(parse_level_filter(""), None);
    }

    #[test]
    fn module_targets_count_as_kache() {
        assert_eq!(
            parse_level_filter("kache::cache_key=trace"),
            Some(log::LevelFilter::Trace)
        );
        assert_eq!(
            parse_level_filter("kache::cache_key=trace,kache=warn"),
            Some(log::LevelFilter::Warn)
        );
    }

    #[test]
    fn last_applicable_segment_wins() {
        assert_eq!(
            parse_level_filter("warn,kache=error"),
            Some(log::LevelFilter::Error)
        );
        assert_eq!(
            parse_level_filter("kache=error,warn"),
            Some(log::LevelFilter::Warn)
        );
    }

    #[test]
    fn other_targets_do_not_move_our_level() {
        assert_eq!(
            parse_level_filter("foo=debug,kache=warn"),
            Some(log::LevelFilter::Warn)
        );
        assert_eq!(
            parse_level_filter("kache=error,other=off"),
            Some(log::LevelFilter::Error)
        );
    }

    #[test]
    fn logger_gates_stderr_and_file_independently() {
        let stderr = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer_stderr = std::sync::Arc::clone(&stderr);
        let dir = tempfile_dir();
        let path = dir.join("kache.log");
        let file = File::create(&path).unwrap();
        let logger = KacheLogger::new(
            log::LevelFilter::Warn,
            move || {
                let writer_stderr = std::sync::Arc::clone(&writer_stderr);
                Box::new(SharedVec(writer_stderr)) as Box<dyn Write + Send>
            },
            Some(FileSink::new(file, log::LevelFilter::Info)),
        );
        logger.emit(log::Level::Info, "kache::test", "info-only");
        logger.emit(log::Level::Warn, "kache::test", "warn-both");
        logger.emit(log::Level::Debug, "kache::test", "debug-neither");
        let err = String::from_utf8(stderr.lock().unwrap().clone()).unwrap();
        assert_eq!(err, "WARN warn-both\n");
        let logged = std::fs::read_to_string(&path).unwrap();
        assert_eq!(logged, "INFO info-only\nWARN warn-both\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entered_scope_reroutes_thread_records_to_its_own_file() {
        let dir = tempfile_dir();
        let global_file = File::create(dir.join("global.log")).unwrap();
        let logger = KacheLogger::new(
            log::LevelFilter::Off,
            || Box::new(std::io::sink()) as Box<dyn Write + Send>,
            Some(FileSink::new(global_file, log::LevelFilter::Trace)),
        );
        let scope_file = File::create(dir.join("scope.log")).unwrap();
        let scope = InvocationConfig::new(
            log::LevelFilter::Off,
            Some(FileSink::new(scope_file, log::LevelFilter::Trace)),
        );
        logger.emit(log::Level::Info, "kache::test", "before-scope");
        {
            let _guard = enter(&scope);
            logger.emit(log::Level::Info, "kache::test", "in-scope");
            assert_eq!(current().need(), log::LevelFilter::Trace);
        }
        logger.emit(log::Level::Info, "kache::test", "after-scope");
        assert_eq!(current().need(), log::LevelFilter::Off);
        let global = std::fs::read_to_string(dir.join("global.log")).unwrap();
        let scoped = std::fs::read_to_string(dir.join("scope.log")).unwrap();
        assert!(global.contains("before-scope"), "{global}");
        assert!(!global.contains("in-scope"), "{global}");
        assert!(global.contains("after-scope"), "{global}");
        assert_eq!(scoped, "INFO in-scope\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nested_scopes_restore_the_outer_config() {
        let dir = tempfile_dir();
        let outer_file = File::create(dir.join("outer.log")).unwrap();
        let outer = InvocationConfig::new(
            log::LevelFilter::Off,
            Some(FileSink::new(outer_file, log::LevelFilter::Info)),
        );
        let inner_file = File::create(dir.join("inner.log")).unwrap();
        let inner = InvocationConfig::new(
            log::LevelFilter::Off,
            Some(FileSink::new(inner_file, log::LevelFilter::Warn)),
        );
        let logger = KacheLogger::new(
            log::LevelFilter::Off,
            || Box::new(std::io::sink()) as Box<dyn Write + Send>,
            None,
        );
        let _outer = enter(&outer);
        logger.emit(log::Level::Info, "kache::test", "outer-info");
        logger.emit(log::Level::Warn, "kache::test", "outer-warn");
        {
            let _inner = enter(&inner);
            logger.emit(log::Level::Info, "kache::test", "inner-info-dropped");
            logger.emit(log::Level::Warn, "kache::test", "inner-warn");
        }
        logger.emit(log::Level::Info, "kache::test", "outer-again");
        let outer_log = std::fs::read_to_string(dir.join("outer.log")).unwrap();
        let inner_log = std::fs::read_to_string(dir.join("inner.log")).unwrap();
        assert!(outer_log.contains("outer-info"), "{outer_log}");
        assert!(outer_log.contains("outer-warn"), "{outer_log}");
        assert!(!outer_log.contains("inner-"), "{outer_log}");
        assert!(outer_log.contains("outer-again"), "{outer_log}");
        assert!(!inner_log.contains("inner-info-dropped"), "{inner_log}");
        assert!(inner_log.contains("inner-warn"), "{inner_log}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entering_a_scope_ratchets_the_process_max_level_up() {
        // Never lowers the global max: other threads' routing must survive.
        // Entering only raises, so assert the postcondition, not the delta.
        let scope = InvocationConfig::new(log::LevelFilter::Trace, None);
        let _guard = enter(&scope);
        assert!(log::max_level() >= log::LevelFilter::Trace);
    }

    #[test]
    fn below_level_records_are_dropped_even_while_capturing() {
        let logger = KacheLogger::new(log::LevelFilter::Off, || Box::new(std::io::sink()) as Box<dyn Write + Send>, None);
        let guard = capture();
        logger.emit(log::Level::Debug, "kache::test", "too-quiet");
        assert!(guard.records().is_empty());
    }

    #[test]
    fn capture_collects_only_the_capturing_thread() {
        let guard = capture();
        log::info!(target: "kache::test", "captured-message");
        let child = std::thread::spawn(|| {
            log::info!(target: "kache::test", "other-thread-message");
        });
        child.join().unwrap();
        let records = guard.records();
        assert_eq!(records, vec!["INFO captured-message"]);
    }

    struct SharedVec(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for SharedVec {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kache-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
