/// End this process the way a wrapped compiler ended: exit with its code, or,
/// for a negative code, die of the signal it negates.
pub fn exit_process(code: i32) -> ! {
    if code < 0 {
        raise_to_signal(-code);
    }
    std::process::exit(code)
}

/// Die of `signal` without dumping core, so the parent sees a signal death
/// rather than an exit code.
#[cfg(unix)]
pub fn raise_to_signal(signal: i32) -> ! {
    // SAFETY: plain libc calls on this thread's signal state; nothing here
    // touches Rust-managed memory except the local sigset.
    unsafe {
        let no_core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &no_core);
        libc::signal(signal, libc::SIG_DFL);
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signal);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        libc::raise(signal);
    }
    std::process::exit(128 + signal)
}

#[cfg(not(unix))]
pub fn raise_to_signal(signal: i32) -> ! {
    std::process::exit(128 + signal)
}
