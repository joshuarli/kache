//! Filesystem identity and physical reclamation.

use crate::sharing::Sharing;
use std::path::Path;

/// Best-effort bytes the filesystem would reclaim by unlinking this blob.
pub fn blob_reclaimable_bytes(path: &Path) -> Option<u64> {
    retainer_from_meta(path).map(|r| r.private_bytes)
}

/// Would unlinking this store name leave the blob's blocks held by another
/// live file? Blocks held only by snapshots do not count: they are freed
/// once the snapshots are deleted, and keeping the entry would pin the store
/// over its size limit for as long as any snapshot exists.
pub fn blob_has_external_retainer(path: &Path) -> bool {
    retainer_from_meta(path).is_some_and(|r| r.held_by_live_file())
}

pub struct BlobRetainer {
    pub size: u64,
    /// Another live file (hardlink or clone) holds every block.
    pub cloned: bool,
    pub private_bytes: u64,
    /// Bytes only filesystem snapshots hold.
    pub snapshot_bytes: u64,
}

impl BlobRetainer {
    /// True when unlinking frees nothing now and nothing later either.
    pub fn held_by_live_file(&self) -> bool {
        self.private_bytes == 0 && self.snapshot_bytes == 0
    }
}

pub fn retainer_from_meta(path: &Path) -> Option<BlobRetainer> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let (sharing, links) = crate::sharing::probe_with_links(path, size);
    // A blob a build's target directory still hardlinks frees nothing when the
    // store drops its own name (kunobi-ninja/kache#725).
    // `cache.shared_hardlink_restores` makes them, and a link made by
    // anything else holds the blocks just the same.
    if links > 1 {
        return Some(BlobRetainer {
            size,
            cloned: true,
            private_bytes: 0,
            snapshot_bytes: 0,
        });
    }
    Some(retainer_from_sharing(size, sharing))
}

pub fn retainer_from_sharing(size: u64, sharing: Sharing) -> BlobRetainer {
    let cloned = sharing.shared && sharing.private_bytes == 0;
    BlobRetainer {
        size,
        cloned,
        private_bytes: if cloned {
            0
        } else {
            sharing.private_bytes.min(size)
        },
        snapshot_bytes: sharing.snapshot_bytes,
    }
}

/// Whether any execute bit is set.
pub fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_executable_reads_any_execute_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        for (mode, expected) in [
            (0o644, false),
            (0o600, false),
            (0o755, true),
            (0o700, true),
            (0o010, true),
        ] {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                is_executable(&std::fs::metadata(&file).unwrap()),
                expected,
                "mode {mode:o}"
            );
        }
    }

    #[test]
    fn retainer_from_sharing_treats_fully_cloned_as_unreclaimable() {
        let r = retainer_from_sharing(
            4096,
            Sharing {
                shared: true,
                private_bytes: 0,
                snapshot_bytes: 0,
            },
        );
        assert!(r.cloned);
        assert_eq!(r.private_bytes, 0);
    }

    #[test]
    fn retainer_from_sharing_treats_private_file_as_reclaimable() {
        let r = retainer_from_sharing(
            4096,
            Sharing {
                shared: false,
                private_bytes: 4096,
                snapshot_bytes: 0,
            },
        );
        assert!(!r.cloned);
        assert_eq!(r.private_bytes, 4096);
    }

    #[test]
    fn snapshot_only_blobs_do_not_block_eviction() {
        let snapshot = retainer_from_sharing(
            4096,
            Sharing {
                shared: false,
                private_bytes: 0,
                snapshot_bytes: 4096,
            },
        );
        assert!(!snapshot.cloned, "no live file holds the blocks");
        assert_eq!(snapshot.private_bytes, 0, "nothing frees right away");
        assert_eq!(snapshot.snapshot_bytes, 4096);
        assert!(!snapshot.held_by_live_file());

        let cloned = retainer_from_sharing(
            4096,
            Sharing {
                shared: true,
                private_bytes: 0,
                snapshot_bytes: 0,
            },
        );
        assert!(cloned.held_by_live_file());

        let private = retainer_from_sharing(
            4096,
            Sharing {
                shared: false,
                private_bytes: 4096,
                snapshot_bytes: 0,
            },
        );
        assert!(!private.held_by_live_file());
    }

    #[test]
    fn retainer_preserves_partial_reclaim_measurement() {
        let r = retainer_from_sharing(
            4096,
            Sharing {
                shared: true,
                private_bytes: 1024,
                snapshot_bytes: 0,
            },
        );
        assert!(!r.cloned, "partly private blobs can reclaim some disk");
        assert_eq!(r.private_bytes, 1024);
    }

    /// The #725 guard. Hardlinks exist, and the count must not come from
    /// a gated guard that evicts entries whose blobs
    /// a target directory still holds, freeing nothing.
    #[test]
    fn a_hardlinked_blob_reclaims_nothing_and_is_reported_as_retained() {
        let dir = tempfile::tempdir().unwrap();
        let blob = dir.path().join("blob.bin");
        std::fs::write(&blob, vec![7u8; 4096]).unwrap();

        let alone = retainer_from_meta(&blob).expect("a plain file measures");
        assert!(!alone.cloned, "an unlinked blob is reclaimable");
        assert_eq!(alone.private_bytes, 4096);
        assert!(!blob_has_external_retainer(&blob));
        assert_eq!(blob_reclaimable_bytes(&blob), Some(4096));

        std::fs::hard_link(&blob, dir.path().join("target-copy.bin")).unwrap();
        let linked = retainer_from_meta(&blob).expect("a hardlinked file measures");
        assert!(linked.cloned, "a second name still holds every block");
        assert_eq!(linked.size, 4096, "the logical size is still reported");
        assert_eq!(linked.private_bytes, 0);
        assert!(blob_has_external_retainer(&blob));
        assert_eq!(blob_reclaimable_bytes(&blob), Some(0));

        assert_eq!(
            retainer_from_meta(dir.path()).map(|r| r.size),
            None,
            "a directory is not a blob"
        );
        assert_eq!(blob_reclaimable_bytes(&dir.path().join("missing")), None);
    }

}
