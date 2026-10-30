use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn run(cargo_args: Vec<OsString>) -> Result<()> {
    let cargo = real_cargo_program(false)?;
    let mut command = Command::new(cargo);
    command.args(cargo_args).env(
        "RUSTC_WRAPPER",
        crate::platform::frontend_executable().context("locating the kache frontend")?,
    );
    run_cargo(command)
}

pub(crate) fn run_shim(cargo_args: Vec<OsString>) -> Result<()> {
    let cargo = real_cargo_program(true)?;
    let mut command = Command::new(cargo);
    command.args(cargo_args);
    run_cargo(command)
}

fn run_cargo(mut command: Command) -> Result<()> {
    use std::os::unix::process::ExitStatusExt as _;
    let mut child = command.spawn().with_context(|| {
        format!(
            "running Cargo program {:?}",
            command.get_program().to_string_lossy()
        )
    })?;
    let status = child.wait().context("waiting for Cargo")?;
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    if let Some(signal) = status.signal() {
        raise_to_signal(signal);
    }
    std::process::exit(101);
}

/// Die of `signal` without dumping core, so the parent sees a signal death
/// rather than an exit code.
#[cfg(unix)]
fn raise_to_signal(signal: i32) -> ! {
    unsafe {
        let no_core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &no_core);
        libc::signal(signal, libc::SIG_DFL);
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, signal);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        libc::raise(signal);
        std::process::exit(128 + signal);
    }
}

#[cfg(not(unix))]
fn raise_to_signal(signal: i32) -> ! {
    std::process::exit(128 + signal);
}

fn real_cargo_program(shimmed: bool) -> Result<PathBuf> {
    let program = std::env::var_os("KACHE_REAL_CARGO")
        .or_else(|| (!shimmed).then(|| std::env::var_os("CARGO")).flatten())
        .or_else(|| {
            crate::compiler::shim::resolve_real_compiler_from_env("cargo")
                .map(PathBuf::into_os_string)
        })
        .context("finding Cargo behind the kache shim")?;
    let cwd = std::env::current_dir().context("resolving the Cargo working directory")?;
    resolve_cargo_program(&program, &cwd)
}

fn resolve_cargo_program(program: &OsStr, cwd: &Path) -> Result<PathBuf> {
    let path = Path::new(program);
    let executable = match program.to_str() {
        Some(program) => crate::compiler::resolve_program_on_path(program),
        None => Some(path.to_path_buf()),
    }
    .with_context(|| format!("resolving Cargo program {program:?}"))?;
    let executable = if executable.is_absolute() {
        executable
    } else {
        cwd.join(executable)
    };
    let resolved_identity = executable
        .canonicalize()
        .with_context(|| format!("resolving Cargo program {program:?}"))?;
    let current = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    let frontend = crate::platform::frontend_executable()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    if Some(&resolved_identity) == current.as_ref() || Some(&resolved_identity) == frontend.as_ref()
    {
        bail!(
            "resolved Cargo program {:?} points back to kache; set KACHE_REAL_CARGO to the real Cargo binary",
            program
        );
    }
    // Preserve the launcher path. In a rustup installation `cargo` is a symlink
    // to the shared rustup binary, whose behavior depends on argv[0]. Executing
    // its canonical target would launch `rustup`, not the Cargo proxy.
    Ok(executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn cargo_resolution_preserves_a_launcher_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("rustup");
        let launcher = dir.path().join("cargo");
        std::fs::write(&target, "#!/bin/sh\nexit 0\n").unwrap();
        std::os::unix::fs::symlink(&target, &launcher).unwrap();

        assert_eq!(
            resolve_cargo_program(launcher.as_os_str(), dir.path()).unwrap(),
            launcher
        );
    }

    #[test]
    fn bare_cargo_program_resolves_through_path() {
        let resolved = resolve_cargo_program(OsStr::new("cargo"), Path::new(".")).unwrap();
        assert!(resolved.is_absolute(), "resolved Cargo path: {resolved:?}");
        assert!(resolved.is_file(), "resolved Cargo path: {resolved:?}");
    }

}
