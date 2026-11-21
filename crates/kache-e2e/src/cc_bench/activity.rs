//! Whether the host ran unrelated work while a replay was timed.
//!
//! A load average counts runnable tasks, including the build being timed,
//! so it cannot say whether anything else was busy. Two estimates are taken
//! before and after each build, and the larger counts:
//!
//! - the host's busy CPU time (`/proc/stat`, or the Mach host CPU ticks on
//!   macOS) less the build's own metered CPU and the benchmark's long-lived
//!   processes (the kache daemon). This sees short-lived processes, such as
//!   a parallel compile's, that no process listing catches.
//! - per-process CPU time (`/proc/<pid>/stat` on Linux, `ps` elsewhere),
//!   which names the busiest processes outside the benchmark.

use deser::Serialize;
use std::collections::BTreeMap;
#[cfg(not(target_os = "linux"))]
use std::process::Command;
use std::time::Instant;

/// Average cores unrelated processes may use across a replay before the
/// host is ineligible for acceptance. Idle desktop daemons stay well below.
pub const MAX_UNRELATED_CORES: f64 = 0.5;

/// How many of the busiest unrelated processes the evidence names.
const TOP: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub cpu_ms: u64,
    /// Linux start time in ticks since boot, independent of the command name.
    pub start_time: Option<u64>,
    /// The whole command line, used only to tell the benchmark's processes
    /// apart; it never reaches the report.
    pub command: String,
}

/// Every process's cumulative CPU time, keyed by PID.
#[derive(Debug, Clone)]
pub struct Snapshot {
    at: Instant,
    processes: BTreeMap<u32, Process>,
    /// Host-wide busy CPU time since boot.
    system_busy_ms: Option<u64>,
    /// Each process's executable file name.
    names: BTreeMap<u32, String>,
}

/// One unrelated process in the evidence. Only its executable's file name
/// is recorded: arguments and paths can name other people's work.
#[derive(Debug, Clone, Serialize)]
pub struct Busy {
    pub pid: u32,
    pub cpu_ms: u64,
    pub name: String,
}

/// What processes outside the benchmark used across one interval.
#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub wall_ms: f64,
    /// The larger of the two estimates below.
    pub unrelated_cpu_ms: u64,
    /// Host busy time less the build's and the benchmark's own CPU; `None`
    /// where the host counters cannot be read.
    pub unrelated_system_cpu_ms: Option<u64>,
    /// The sum over sampled processes outside the benchmark.
    pub unrelated_process_cpu_ms: u64,
    pub unrelated_cores: f64,
    pub busiest: Vec<Busy>,
    pub eligible: bool,
}

impl Snapshot {
    /// Unreadable host activity is unknown and ineligible for acceptance.
    pub fn take() -> Option<Self> {
        #[cfg(target_os = "linux")]
        let (processes, names) = {
            let mut processes = BTreeMap::new();
            let mut names = BTreeMap::new();
            for entry in std::fs::read_dir("/proc").ok()?.flatten() {
                let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse().ok()) else {
                    continue;
                };
                if let Some((process, name)) = linux_process(pid) {
                    processes.insert(pid, process);
                    names.insert(pid, name);
                }
            }
            if processes.is_empty() {
                return None;
            }
            (processes, names)
        };
        #[cfg(not(target_os = "linux"))]
        let (processes, names) = {
            let output = Command::new("ps").args(["-A", "-o", "pid=,time=,command="]).env_clear().env("LC_ALL", "C").output().ok()?;
            let names = Command::new("ps").args(["-A", "-o", "pid=,comm="]).env_clear().env("LC_ALL", "C").output().ok()?;
            if !output.status.success() || !names.status.success() {
                return None;
            }
            (parse_ps(&String::from_utf8_lossy(&output.stdout)), parse_names(&String::from_utf8_lossy(&names.stdout)))
        };
        Some(Snapshot { at: Instant::now(), processes, system_busy_ms: system_busy_ms(), names })
    }

    /// Activity between `self` and `later`. This process, every process for
    /// which `ours` holds, and `build_cpu_ms` (the build's own metered CPU)
    /// are the benchmark's.
    pub fn until(&self, later: &Snapshot, ours: impl Fn(&str) -> bool, build_cpu_ms: u64) -> Activity {
        let wall_ms = later.at.duration_since(self.at).as_secs_f64() * 1000.0;
        let me = std::process::id();
        let mut own_ms = 0;
        let mut busy = Vec::new();
        for (pid, process) in &later.processes {
            let before = self
                .processes
                .get(pid)
                .filter(|p| match (p.start_time, process.start_time) {
                    (Some(before), Some(after)) => before == after,
                    (None, None) => p.command == process.command,
                    _ => false,
                })
                .map_or(0, |p| p.cpu_ms);
            let cpu_ms = process.cpu_ms.saturating_sub(before);
            if *pid == me || ours(&process.command) {
                own_ms += cpu_ms;
            } else if cpu_ms > 0 {
                busy.push(Busy { pid: *pid, cpu_ms, name: later.names.get(pid).cloned().unwrap_or_else(|| "?".into()) });
            }
        }
        busy.sort_by_key(|b| std::cmp::Reverse(b.cpu_ms));
        let unrelated_process_cpu_ms: u64 = busy.iter().map(|b| b.cpu_ms).sum();
        let unrelated_system_cpu_ms = self
            .system_busy_ms
            .zip(later.system_busy_ms)
            .map(|(start, end)| end.saturating_sub(start).saturating_sub(build_cpu_ms + own_ms));
        let unrelated_cpu_ms = unrelated_process_cpu_ms.max(unrelated_system_cpu_ms.unwrap_or(0));
        let unrelated_cores = if wall_ms > 0.0 { unrelated_cpu_ms as f64 / wall_ms } else { 0.0 };
        busy.truncate(TOP);
        Activity {
            wall_ms,
            unrelated_cpu_ms,
            unrelated_system_cpu_ms,
            unrelated_process_cpu_ms,
            unrelated_cores,
            busiest: busy,
            eligible: unrelated_cores <= MAX_UNRELATED_CORES,
        }
    }
}

/// Clock ticks per second in both host counters: Linux `USER_HZ` and the
/// Mach CPU load ticks are both 100 Hz.
const TICK_MS: u64 = 10;

#[cfg(target_os = "linux")]
fn linux_process(pid: u32) -> Option<(Process, String)> {
    let root = std::path::PathBuf::from(format!("/proc/{pid}"));
    let before = std::fs::read_to_string(root.join("stat")).ok()?;
    let (_, _, start_time) = parse_linux_stat(&before)?;
    let command = std::fs::read(root.join("cmdline")).ok()?;
    let after = std::fs::read_to_string(root.join("stat")).ok()?;
    let (name, cpu_ms, after_start_time) = parse_linux_stat(&after)?;
    // Keep the identity, CPU counter and command from one process lifetime.
    if start_time != after_start_time {
        return None;
    }
    let command = String::from_utf8_lossy(&command).split_terminator('\0').collect::<Vec<_>>().join(" ");
    let command = if command.is_empty() { format!("[{name}]") } else { command };
    Some((Process { cpu_ms, start_time: Some(start_time), command }, name.rsplit('/').next()?.to_string()))
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_stat(text: &str) -> Option<(&str, u64, u64)> {
    let (head, rest) = text.rsplit_once(") ")?;
    let (_, name) = head.split_once('(')?;
    let mut fields = rest.split_whitespace();
    // Fields 14/15 (CPU) and 22 (start), after the parenthesized comm.
    let user: u64 = fields.nth(11)?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    let start_time = fields.nth(6)?.parse().ok()?;
    Some((name, user.checked_add(system)?.checked_mul(TICK_MS)?, start_time))
}

#[cfg(target_os = "linux")]
fn system_busy_ms() -> Option<u64> {
    parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)
}

/// Busy ticks from the aggregate `cpu` line: everything but idle and iowait.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_proc_stat(text: &str) -> Option<u64> {
    let fields: Vec<u64> = text.lines().next()?.strip_prefix("cpu ")?.split_whitespace().map(|f| f.parse().ok()).collect::<Option<_>>()?;
    let busy: u64 = fields.iter().enumerate().filter(|(i, _)| *i != 3 && *i != 4).map(|(_, v)| v).sum();
    Some(busy * TICK_MS)
}

#[cfg(target_os = "macos")]
// libc deprecates its mach_* bindings in favor of the `mach2` crate; one host
// counter does not justify a new dependency (as in src/heartbeat.rs).
#[allow(deprecated)]
fn system_busy_ms() -> Option<u64> {
    let mut info = libc::host_cpu_load_info { cpu_ticks: [0; 4] };
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    // SAFETY: `info` is a host_cpu_load_info and `count` its size in
    // integers, which is what HOST_CPU_LOAD_INFO writes.
    let status = unsafe {
        libc::host_statistics(
            libc::mach_host_self(),
            libc::HOST_CPU_LOAD_INFO,
            (&mut info as *mut libc::host_cpu_load_info).cast(),
            &mut count,
        )
    };
    (status == libc::KERN_SUCCESS).then(|| {
        let [user, system, _idle, nice] = info.cpu_ticks;
        (u64::from(user) + u64::from(system) + u64::from(nice)) * TICK_MS
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn system_busy_ms() -> Option<u64> {
    None
}

/// `ps -o pid=,time=,command=` output. `time` is `[[dd-]hh:]mm:ss[.cc]`:
/// procps prints whole seconds, macOS prints hundredths.
#[cfg(any(not(target_os = "linux"), test))]
pub fn parse_ps(text: &str) -> BTreeMap<u32, Process> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let cpu_ms = parse_cpu_time(fields.next()?)?;
            let command = fields.collect::<Vec<_>>().join(" ");
            Some((pid, Process { cpu_ms, start_time: None, command }))
        })
        .collect()
}

/// `ps -o pid=,comm=` output, reduced to each executable's file name.
#[cfg(any(not(target_os = "linux"), test))]
pub fn parse_names(text: &str) -> BTreeMap<u32, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, comm) = line.split_once(char::is_whitespace)?;
            let comm = comm.trim();
            Some((pid.parse().ok()?, comm.rsplit('/').next().unwrap_or(comm).to_string()))
        })
        .collect()
}

#[cfg(any(not(target_os = "linux"), test))]
fn parse_cpu_time(text: &str) -> Option<u64> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, text),
    };
    let mut seconds = 0.0;
    for part in clock.split(':') {
        seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(days * 86_400_000 + (seconds * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_times_parse_in_both_ps_formats() {
        assert_eq!(parse_cpu_time("0:01.25"), Some(1250));
        assert_eq!(parse_cpu_time("12:34.50"), Some(754_500));
        assert_eq!(parse_cpu_time("01:02:03"), Some(3_723_000));
        assert_eq!(parse_cpu_time("2-00:00:01"), Some(172_801_000));
        assert_eq!(parse_cpu_time("x:01"), None);
    }

    fn snapshot(at: Instant, text: &str) -> Snapshot {
        let mut processes = parse_ps(text);
        for (pid, process) in &mut processes {
            process.start_time = Some(u64::from(*pid));
        }
        let names = processes.iter().map(|(pid, p)| (*pid, p.command.split(' ').next().unwrap().rsplit('/').next().unwrap().to_string())).collect();
        Snapshot { at, processes, system_busy_ms: None, names }
    }

    #[test]
    fn names_keep_only_the_executable_file_name() {
        let names = parse_names("  10 /Applications/Orb.app/Contents/MacOS/Orb Helper\n  20 python3\n");
        assert_eq!(names[&10], "Orb Helper");
        assert_eq!(names[&20], "python3");
    }

    #[test]
    fn evidence_names_processes_without_their_arguments() {
        let start = Instant::now();
        let before = snapshot(start, "  10 0:01.00 /usr/bin/tool --secret=/home/someone/project\n");
        let after = snapshot(start + std::time::Duration::from_secs(10), "  10 0:09.00 /usr/bin/tool --secret=/home/someone/project\n");
        let json = deser_json::to_string(&before.until(&after, |_| false, 0)).unwrap();
        assert!(json.contains("\"name\":\"tool\""), "{json}");
        assert!(!json.contains("someone"), "{json}");
    }

    #[test]
    fn proc_stat_busy_time_leaves_out_idle_and_iowait() {
        assert_eq!(parse_proc_stat("cpu  100 5 50 1000 40 1 2 3 0 0\ncpu0 1 1 1 1\n"), Some((100 + 5 + 50 + 1 + 2 + 3) * 10));
        assert_eq!(parse_proc_stat("intr 1 2\n"), None);
    }

    #[test]
    fn host_busy_time_less_the_benchmarks_own_sees_short_lived_work() {
        let start = Instant::now();
        let mut before = snapshot(start, "  20 0:01.00 /bench/kache-engine daemon run\n");
        let mut after = snapshot(start + std::time::Duration::from_secs(10), "  20 0:02.00 /bench/kache-engine daemon run\n");
        before.system_busy_ms = Some(50_000);
        after.system_busy_ms = Some(50_000 + 4000 + 1000 + 9000);
        let activity = before.until(&after, |c| c.starts_with("/bench/"), 4000);
        assert_eq!(activity.unrelated_process_cpu_ms, 0);
        assert_eq!(activity.unrelated_system_cpu_ms, Some(9000));
        assert_eq!(activity.unrelated_cpu_ms, 9000);
        assert!(!activity.eligible);
        after.system_busy_ms = Some(50_000 + 4000 + 1000 + 2000);
        assert!(before.until(&after, |c| c.starts_with("/bench/"), 4000).eligible);
    }

    #[test]
    fn activity_counts_only_unrelated_processes_and_new_ones_in_full() {
        let start = Instant::now();
        let before = snapshot(start, "  10 0:10.00 /usr/bin/indexer --all\n  20 0:01.00 /bench/kache-engine daemon run\n  30 0:05.00 old-name\n");
        let mut after = snapshot(
            start + std::time::Duration::from_secs(10),
            "  10 0:14.00 /usr/bin/indexer --all\n  20 0:09.00 /bench/kache-engine daemon run\n  30 0:01.00 reused-pid\n  40 0:02.00 /usr/bin/newcomer\n",
        );
        after.processes.get_mut(&30).unwrap().start_time = Some(31);
        let activity = before.until(&after, |command| command.starts_with("/bench/"), 0);
        assert_eq!(activity.unrelated_cpu_ms, 4000 + 1000 + 2000);
        assert!((activity.unrelated_cores - 0.7).abs() < 0.01, "{activity:?}");
        assert!(!activity.eligible);
        assert_eq!(activity.busiest[0].pid, 10);
        assert!(activity.busiest.iter().all(|b| b.pid != 20));
    }

    #[test]
    fn a_quiet_interval_is_eligible() {
        let start = Instant::now();
        let before = snapshot(start, "  10 0:10.00 /usr/bin/indexer\n");
        let after = snapshot(start + std::time::Duration::from_secs(10), "  10 0:11.00 /usr/bin/indexer\n");
        let activity = before.until(&after, |_| false, 0);
        assert!(activity.eligible, "{activity:?}");
        assert_eq!(activity.busiest.len(), 1);
    }

    #[test]
    fn renamed_process_keeps_its_cpu_baseline() {
        let start = Instant::now();
        let before = snapshot(start, "1009580 00:00:01 [kworker/u16:4-writeback]");
        let after = snapshot(start + std::time::Duration::from_millis(100), "1009580 00:00:01 [kworker/u16:4-ext4-rsv-conversion]");
        let activity = before.until(&after, |_| false, 0);
        assert_eq!(activity.unrelated_cpu_ms, 0);
        assert!(activity.eligible, "{activity:?}");
        assert!(activity.busiest.is_empty());
    }

    #[test]
    fn reused_pid_with_the_same_command_counts_its_new_counter_in_full() {
        let start = Instant::now();
        let before = snapshot(start, "30 00:00:05 /usr/bin/worker");
        for time in ["00:00:01", "00:00:08"] {
            let mut after = snapshot(start + std::time::Duration::from_secs(1), &format!("30 {time} /usr/bin/worker"));
            after.processes.get_mut(&30).unwrap().start_time = Some(31);
            let activity = before.until(&after, |_| false, 0);
            let expected = parse_cpu_time(time).unwrap();
            assert_eq!(activity.unrelated_cpu_ms, expected);
            assert_eq!(activity.busiest[0].cpu_ms, expected);
            assert!(!activity.eligible);
        }
    }

    #[test]
    fn renamed_own_process_only_subtracts_its_interval_cpu() {
        let start = Instant::now();
        let mut before = snapshot(start, "20 00:00:10 /bench/worker old");
        let mut after = snapshot(start + std::time::Duration::from_secs(1), "20 00:00:11 /bench/worker new");
        before.system_busy_ms = Some(100_000);
        after.system_busy_ms = Some(102_000);
        let activity = before.until(&after, |c| c.starts_with("/bench/"), 0);
        assert_eq!(activity.unrelated_system_cpu_ms, Some(1000));
        assert_eq!(activity.unrelated_cpu_ms, 1000);
        assert!(!activity.eligible);
    }

    #[test]
    fn empty_process_snapshots_have_no_process_cpu() {
        let start = Instant::now();
        let before = snapshot(start, "");
        let after = snapshot(start + std::time::Duration::from_secs(1), "");
        let activity = before.until(&after, |_| false, 0);
        assert_eq!(activity.unrelated_cpu_ms, 0);
        assert!(activity.busiest.is_empty());
        assert!(activity.eligible);
    }

    #[test]
    fn unknown_start_times_keep_the_conservative_command_fallback() {
        let start = Instant::now();
        for (before_time, after_time, command, expected) in
            [(None, None, "/usr/bin/worker", 1000), (None, None, "/usr/bin/replacement", 2000), (Some(30), None, "/usr/bin/worker", 2000), (None, Some(30), "/usr/bin/worker", 2000)]
        {
            let mut before = snapshot(start, "30 00:00:01 /usr/bin/worker");
            let mut after = snapshot(start + std::time::Duration::from_secs(1), &format!("30 00:00:02 {command}"));
            before.processes.get_mut(&30).unwrap().start_time = before_time;
            after.processes.get_mut(&30).unwrap().start_time = after_time;
            assert_eq!(before.until(&after, |_| false, 0).unrelated_cpu_ms, expected);
        }
    }

    #[test]
    fn linux_stat_uses_cpu_and_start_time_after_a_parenthesized_command() {
        let stat = "42 (worker ) tricky (name)) R 1 2 3 4 5 6 7 8 9 10 123 456 0 0 0 0 0 0 789 0";
        assert_eq!(parse_linux_stat(stat), Some(("worker ) tricky (name)", 5790, 789)));
        for invalid in [String::new(), "42 (worker) R".to_string(), stat.replace("123", "bad"), stat.replace("789", "bad"), stat.replace("123", &u64::MAX.to_string())] {
            assert_eq!(parse_linux_stat(&invalid), None, "{invalid}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_snapshot_records_current_process_start_time() {
        let snapshot = Snapshot::take().unwrap();
        let pid = std::process::id();
        let process = &snapshot.processes[&pid];
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        assert_eq!(process.start_time, Some(parse_linux_stat(&stat).unwrap().2));
        assert!(!process.command.is_empty());
        assert!(!snapshot.names[&pid].is_empty());
    }
}
