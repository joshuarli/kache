//! Volume observations. Callers decide how to react to storage pressure.

use crate::VolumeUsage;
use std::path::Path;

/// OS-reported filesystem identity, scoped to the current host and observation.
/// IDs can be reused after an unmount. Distinct IDs do not prove independent
/// capacity: APFS volumes can share a container's free space.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "deser", derive(deser::Serialize, deser::Deserialize))]
#[cfg_attr(feature = "deser", deser(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum VolumeId {
    /// Nonzero `statvfs.f_fsid`; zero means the OS did not identify the volume.
    Unix(u64),
}

impl std::fmt::Display for VolumeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix(id) => write!(f, "unix:{id:x}"),
        }
    }
}

/// Inode counts reported by the filesystem, for the calling user.
/// Some filesystems allocate inodes dynamically; these are observations,
/// not a promise of a fixed inode limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "deser", derive(deser::Serialize, deser::Deserialize))]
pub struct InodeUsage {
    pub total: u64,
    pub available: u64,
}

/// Capacity of the filesystem hosting a path. Missing identity or inode
/// counters remain unknown; they do not mean zero capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "deser", derive(deser::Serialize, deser::Deserialize))]
pub struct VolumeStats {
    pub id: Option<VolumeId>,
    pub usage: VolumeUsage,
    pub inodes: Option<InodeUsage>,
}

/// Read volume statistics without walking directories. The path must exist;
/// symlinks are followed. Unix reports identity, bytes and inodes together via
/// `statvfs`. Failed capacity probes return `None`.
pub fn volume_stats(path: &Path) -> Option<VolumeStats> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(usage) = crate::testutil::mock_volume_usage(path) {
        return usage.map(|usage| VolumeStats {
            id: None,
            usage,
            inodes: None,
        });
    }
    unix_stats(path)
}

fn unix_stats(path: &Path) -> Option<VolumeStats> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: path is NUL-terminated and stat is a valid output buffer.
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    unix_counts(
        count(stat.f_fsid),
        count(stat.f_blocks),
        count(stat.f_bavail),
        count(stat.f_frsize),
        count(stat.f_bsize),
        count(stat.f_files),
        count(stat.f_favail),
    )
}

#[cfg(any(unix, test))]
fn count(value: impl Into<u64>) -> u64 {
    value.into()
}

#[cfg(any(unix, test))]
fn unix_counts(
    id: u64,
    blocks: u64,
    available: u64,
    fragment_size: u64,
    block_size: u64,
    inodes: u64,
    available_inodes: u64,
) -> Option<VolumeStats> {
    let unit = if fragment_size > 0 {
        fragment_size
    } else {
        block_size
    };
    // Invalid or unrepresentable counters must not wrap into apparent headroom.
    if unit == 0 {
        return None;
    }
    blocks.checked_mul(unit)?;
    available.checked_mul(unit)?;
    Some(VolumeStats {
        id: (id != 0).then_some(VolumeId::Unix(id)),
        usage: crate::volume_bytes(blocks, available, fragment_size, block_size),
        inodes: (inodes != 0).then_some(InodeUsage {
            total: inodes,
            available: available_inodes,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn counters_preserve_units_identity_and_available_inodes() {
        let stats = unix_counts(27, 100, 3, 4096, 16384, 200, 7).unwrap();
        assert_eq!(stats.id, Some(VolumeId::Unix(27)));
        assert_eq!(stats.id.as_ref().unwrap().to_string(), "unix:1b");
        assert_eq!(
            stats.usage,
            VolumeUsage {
                total: 409_600,
                free: 12_288
            }
        );
        assert_eq!(
            stats.inodes,
            Some(InodeUsage {
                total: 200,
                available: 7
            })
        );
        let blocks = unix_counts(28, 100, 3, 0, 16384, 200, 0).unwrap();
        assert_eq!(
            blocks.usage,
            VolumeUsage {
                total: 1_638_400,
                free: 49_152
            }
        );
        assert_eq!(blocks.inodes.unwrap().available, 0);
        assert_ne!(stats.id, blocks.id);
        assert_eq!(count(u32::MAX), 4_294_967_295);
        assert_eq!(count(u64::MAX), 18_446_744_073_709_551_615);
    }

    #[test]
    fn unavailable_counters_stay_unknown_and_overflow_is_rejected() {
        let unknown = unix_counts(0, 100, 3, 4096, 16384, 0, 42).unwrap();
        assert_eq!(unknown.id, None);
        assert_eq!(unknown.inodes, None);
        assert!(unix_counts(1, 1, 1, 0, 0, 1, 1).is_none());
        assert!(unix_counts(1, u64::MAX, 1, 4096, 0, 1, 1).is_none());
        assert!(unix_counts(1, 1, u64::MAX, 4096, 0, 1, 1).is_none());
        let empty = unix_counts(1, 0, 0, 4096, 0, 1, 0).unwrap();
        assert_eq!(empty.usage, VolumeUsage { total: 0, free: 0 });
    }

    #[test]
    fn paths_on_one_volume_share_identity_and_missing_paths_are_unknown() {
        let dir = TempDir::new("volume-stats");
        let child = dir.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let root = volume_stats(dir.path()).unwrap();
        let nested = volume_stats(&child).unwrap();
        assert!(root.id.is_some());
        assert_eq!(root.id, nested.id);
        assert!(root.usage.total > 0);
        assert!(volume_stats(&dir.path().join("missing")).is_none());
        assert!(volume_stats(Path::new("\0")).is_none());
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();
        assert_eq!(volume_stats(&alias).unwrap().id, nested.id);
        assert_eq!(
            crate::volume_usage(dir.path()).unwrap().total,
            root.usage.total
        );
    }

    #[cfg(feature = "deser")]
    #[test]
    fn observations_round_trip_with_explicit_unknown_fields() {
        for stats in [
            unix_counts(7, 100, 3, 4096, 0, 200, 0).unwrap(),
            VolumeStats {
                id: None,
                usage: VolumeUsage {
                    total: 100,
                    free: 7,
                },
                inodes: None,
            },
        ] {
            let json = deser_json::to_string(&stats).unwrap();
            assert_eq!(deser_json::from_str::<VolumeStats>(&json).unwrap(), stats);
        }
    }
}
