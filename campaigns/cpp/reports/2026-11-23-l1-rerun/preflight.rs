#[path = "/home/ubuntu/repos/kache/.work/wt/l1-acceptance-rerun/crates/kache-e2e/src/cc_bench/activity.rs"]
mod activity;

fn main() {
    assert!(std::env::var_os("TMPDIR").is_none());
    assert!(std::env::var_os("KACHE_CACHE_DIR").is_none());
    let compiler = std::env::args().nth(1).expect("compiler");
    let root = "/home/ubuntu/repos/kache/.work/wt/l1-acceptance-rerun";
    let prefixes = [
        format!("{root}/.work/tmp/cc-bench/{compiler}"),
        "/home/ubuntu/repos/kache/.work/wt/cc-bench-l1-rerun/.work/tmp/ccache/ccache".into(),
        format!("{root}/target/release"),
    ];
    let Some(start) = activity::Snapshot::take() else { std::process::exit(3) };
    std::thread::sleep(std::time::Duration::from_secs(5));
    let Some(later) = activity::Snapshot::take() else { std::process::exit(3) };
    let result = start.until(&later, |command| prefixes.iter().any(|p| command.contains(p)), 0);
    println!("Preflight only; unmodified ee2402bf activity.rs; compiler={compiler}; TMPDIR/KACHE_CACHE_DIR unset");
    println!("Unrelated CPU limit: {} cores", activity::MAX_UNRELATED_CORES);
    println!("{result:#?}");
    println!("PREFLIGHT_VERDICT={}", if result.eligible { "ELIGIBLE" } else { "INELIGIBLE" });
    std::process::exit(if result.eligible { 0 } else { 3 });
}
