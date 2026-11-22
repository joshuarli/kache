use anyhow::Result;
use std::path::{Path, PathBuf};

/// Resolve the daemon runtime, relocating only an implicit overlong socket path.
pub fn resolve_runtime(
    cache: &Path,
    runtime: PathBuf,
    explicit: bool,
    socket: Option<&Path>,
) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use anyhow::Context;
        let endpoint = socket
            .map(Path::to_path_buf)
            .unwrap_or_else(|| runtime.join("daemon.sock"));
        if kache_fs::validate_socket_path(&endpoint).is_ok() {
            return Ok(runtime);
        }
        if explicit || socket.is_some() {
            kache_fs::validate_socket_path(&endpoint)?;
        }
        let identity = cache_identity(cache)?;
        let digest = blake3::hash(identity.as_os_str().as_encoded_bytes());
        let runtime = kache_fs::private_runtime_dir(&digest.to_hex()[..32])
            .context("provisioning private daemon runtime for deep cache")?;
        kache_fs::validate_socket_path(&runtime.join("daemon.sock"))?;
        Ok(runtime)
    }
    #[cfg(not(unix))]
    {
        let _ = (cache, explicit, socket);
        Ok(runtime)
    }
}

#[cfg(unix)]
fn cache_identity(cache: &Path) -> Result<PathBuf> {
    let mut ancestor = cache;
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut identity) => {
                for name in missing.into_iter().rev() {
                    if name == ".." {
                        identity.pop();
                    } else {
                        identity.push(name);
                        match identity.canonicalize() {
                            Ok(existing) => identity = existing,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
                return Ok(identity);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let (Some(name), Some(parent)) = (ancestor.components().next_back(), ancestor.parent()) else {
                    return Err(error.into());
                };
                missing.push(name.as_os_str());
                ancestor = parent;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    #[test]
    fn default_deep_caches_have_distinct_stable_short_runtimes() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("deep".repeat(30)).join("cache");
        let runtime = resolve_runtime(&cache, cache.clone(), false, None).unwrap();
        assert_ne!(runtime, cache);
        assert!(runtime.starts_with("/tmp"));
        kache_fs::validate_socket_path(&runtime.join("daemon.sock")).unwrap();
        let other = resolve_runtime(
            &cache.with_extension("other"),
            cache.with_extension("other"),
            false,
            None,
        )
        .unwrap();
        assert_ne!(runtime, other);
        std::fs::create_dir_all(&cache).unwrap();
        assert_eq!(
            runtime,
            resolve_runtime(&cache, cache.clone(), false, None).unwrap()
        );
        let alias = root.path().join("alias".repeat(24));
        symlink(cache.parent().unwrap(), &alias).unwrap();
        let alias = alias.join("cache");
        assert_eq!(
            runtime,
            resolve_runtime(&alias, alias.clone(), false, None).unwrap()
        );
        assert!(resolve_runtime(&cache, cache.clone(), true, None).is_err());
        assert_eq!(
            resolve_runtime(
                &cache,
                cache.clone(),
                false,
                Some(Path::new("/tmp/short.sock"))
            )
            .unwrap(),
            cache
        );
        assert_eq!(
            resolve_runtime(Path::new("/short"), PathBuf::from("/short"), false, None).unwrap(),
            PathBuf::from("/short")
        );
        std::fs::remove_dir(runtime).unwrap();
        std::fs::remove_dir(other).unwrap();
    }

    #[test]
    fn identity_supports_missing_components_before_parent_traversal() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("deep".repeat(30));
        let through_missing = cache.join("missing/../store");
        let direct = cache.join("store");
        assert_eq!(
            cache_identity(&through_missing).unwrap(),
            cache_identity(&direct).unwrap()
        );
        let runtime = resolve_runtime(&direct, direct.clone(), false, None).unwrap();
        std::fs::create_dir_all(&through_missing).unwrap();
        assert_eq!(
            runtime,
            resolve_runtime(&direct, direct.clone(), false, None).unwrap()
        );
        std::fs::remove_dir(runtime).unwrap();
    }

    #[test]
    fn identity_preserves_non_utf8_path_bytes() {
        let root = tempfile::tempdir().unwrap();
        let cache = root
            .path()
            .join(std::ffi::OsString::from_vec(vec![0xff; 120]));
        let through_missing = cache.join("missing/../store");
        let direct = cache.join("store");
        assert_eq!(
            cache_identity(&direct).unwrap(),
            root.path()
                .canonicalize()
                .unwrap()
                .join(cache.file_name().unwrap())
                .join("store")
        );
        assert_eq!(
            cache_identity(&through_missing).unwrap(),
            cache_identity(&direct).unwrap()
        );
        let runtime = resolve_runtime(&direct, direct.clone(), false, None).unwrap();
        // APFS rejects non-UTF-8 directory names.
        #[cfg(not(target_os = "macos"))]
        {
            std::fs::create_dir_all(&through_missing).unwrap();
            assert_eq!(
                runtime,
                resolve_runtime(&direct, direct.clone(), false, None).unwrap()
            );
        }
        std::fs::remove_dir(runtime).unwrap();
    }

    #[test]
    fn a_missing_prefix_before_parent_traversal_does_not_hide_an_existing_alias() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = root.path().join("alias".repeat(24));
        symlink(&real, &alias).unwrap();
        let cache = root
            .path()
            .join("missing/..")
            .join(alias.file_name().unwrap())
            .join("store");
        let runtime = resolve_runtime(&cache, cache.clone(), false, None).unwrap();
        assert_eq!(
            cache_identity(&cache).unwrap(),
            cache_identity(&real.join("store")).unwrap()
        );
        std::fs::create_dir_all(&cache).unwrap();
        assert_eq!(
            runtime,
            resolve_runtime(&cache, cache.clone(), false, None).unwrap()
        );
        std::fs::remove_dir(runtime).unwrap();
    }
}
