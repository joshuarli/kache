use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub fn validate_socket_path(path: &Path) -> io::Result<()> {
    // SAFETY: sockaddr_un contains only integer fields and a char array.
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let limit = address.sun_path.len() - 1;
    let bytes = path.as_os_str().as_encoded_bytes().len();
    if bytes > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "socket path is {bytes} bytes, but a Unix socket path can be at most {limit} on this platform: {}. Point KACHE_SOCKET_PATH at a shorter absolute path, or set a shorter KACHE_RUNTIME_DIR / [cache] runtime_dir.",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// Stable, private node-local runtime. The shared directory outlives its callers;
/// daemon lifecycle locks own socket cleanup, not individual compiler clients.
pub fn private_runtime_dir(identity: &str) -> io::Result<PathBuf> {
    // Retain the system-owned alias: its resolved target can be arbitrarily deep.
    let directory = private_runtime_dir_in(Path::new("/tmp"), identity)?;
    Ok(Path::new("/tmp").join(directory.file_name().unwrap()))
}

fn private_runtime_dir_in(parent: &Path, identity: &str) -> io::Result<PathBuf> {
    if identity.len() != 32 || !identity.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid runtime identity",
        ));
    }
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let parent = parent.canonicalize()?;
    let parent_handle = trusted_parent(&parent, uid)?;
    let name = CString::new(format!("kache-{uid}-{identity}")).unwrap();
    // SAFETY: parent_handle owns a directory fd; name is one NUL-terminated component.
    if unsafe { libc::mkdirat(parent_handle.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }
    let directory = open_directory_at(&parent_handle, &name)?;
    let metadata = directory.metadata()?;
    verify_private(&metadata, uid)?;
    let path = parent.join(name.to_str().unwrap());
    let named = std::fs::symlink_metadata(&path)?;
    if !named.is_dir() || named.dev() != metadata.dev() || named.ino() != metadata.ino() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory was replaced",
        ));
    }
    Ok(path)
}

fn trusted_parent(parent: &Path, uid: u32) -> io::Result<File> {
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")?;
    verify_parent(&directory.metadata()?, uid)?;
    for component in parent.components().skip(1) {
        let name = CString::new(component.as_os_str().as_encoded_bytes()).unwrap();
        directory = open_directory_at(&directory, &name)?;
        verify_parent(&directory.metadata()?, uid)?;
    }
    Ok(directory)
}

fn verify_parent(metadata: &std::fs::Metadata, uid: u32) -> io::Result<()> {
    if (metadata.uid() != 0 && metadata.uid() != uid)
        || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe runtime ancestor",
        ));
    }
    Ok(())
}

fn open_directory_at(parent: &File, name: &CStr) -> io::Result<File> {
    // SAFETY: parent owns its fd; name is a live NUL-terminated component.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new owned fd, transferred exactly once to File.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn verify_private(metadata: &std::fs::Metadata, uid: u32) -> io::Result<()> {
    if metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory must be owned by the current user with permissions 0700",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::{PermissionsExt, symlink};
    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn private_runtime_is_shared_by_concurrent_callers_and_rejects_hostile_entries() {
        let root = crate::testutil::TempDir::new("runtime");
        let barrier = std::sync::Barrier::new(8);
        let paths = std::thread::scope(|scope| {
            (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        private_runtime_dir_in(root.path(), ID).unwrap()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(paths.iter().all(|p| p == &paths[0]));
        let path = &paths[0];
        let metadata = std::fs::metadata(path).unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o700);
        assert!(verify_private(&metadata, metadata.uid().wrapping_add(1)).is_err());
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_runtime_dir_in(root.path(), ID).is_err());
        assert_eq!(std::fs::metadata(path).unwrap().mode() & 0o7777, 0o755);
        std::fs::remove_dir(path).unwrap();
        let victim = root.path().join("victim");
        std::fs::create_dir(&victim).unwrap();
        symlink(&victim, path).unwrap();
        assert!(private_runtime_dir_in(root.path(), ID).is_err());
        assert!(
            std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn socket_limits_count_native_bytes_and_accept_the_boundary() {
        let limit = if cfg!(target_os = "linux") { 107 } else { 103 };
        assert!(validate_socket_path(Path::new(&"x".repeat(limit))).is_ok());
        assert!(validate_socket_path(Path::new(&"x".repeat(limit + 1))).is_err());
        assert!(
            validate_socket_path(Path::new(&std::ffi::OsString::from_vec(vec![0xff; limit])))
                .is_ok()
        );
        assert!(
            validate_socket_path(Path::new(&std::ffi::OsString::from_vec(vec![
                0xff;
                limit + 1
            ])))
            .is_err()
        );
        assert!(private_runtime_dir_in(Path::new("/tmp"), "../bad").is_err());
    }
}
