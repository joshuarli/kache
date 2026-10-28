use crate::InodeId;
use std::{fs, io, path::Path};

/// Identity of the object at `path`, following symlinks. Valid for comparing
/// live objects on this host; an inode can be reused after deletion.
pub fn file_identity(path: &Path) -> io::Result<InodeId> {
    let metadata = fs::metadata(path)?;
    metadata_identity(path, &metadata, true).map(|(id, _)| id)
}

/// Identity of the file an open handle refers to, whatever its path names
/// now. Compared with [`file_identity`] it tells whether the path was unlinked
/// or replaced after the handle was opened.
pub fn handle_identity(file: &fs::File) -> io::Result<InodeId> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok(InodeId {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

/// Identity of a directory. A symlink at the final component is rejected.
pub fn directory_identity(path: &Path) -> io::Result<InodeId> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a directory",
        ));
    }
    metadata_identity(path, &metadata, false).map(|(id, _)| id)
}

pub(crate) fn metadata_identity(
    path: &Path,
    metadata: &fs::Metadata,
    follow: bool,
) -> io::Result<(InodeId, u64)> {
    use std::os::unix::fs::MetadataExt;
    let _ = (path, follow);
    Ok((
        InodeId {
            dev: metadata.dev(),
            ino: metadata.ino(),
        },
        metadata.nlink(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn hardlinks_share_identity_but_copies_do_not() {
        let dir = TempDir::new("identity");
        let source = dir.write("source", 4096);
        let link = dir.path().join("link");
        let copy = dir.path().join("copy");
        fs::hard_link(&source, &link).unwrap();
        fs::copy(&source, &copy).unwrap();
        assert_eq!(
            file_identity(&source).unwrap(),
            file_identity(&link).unwrap()
        );
        assert_ne!(
            file_identity(&source).unwrap(),
            file_identity(&copy).unwrap()
        );
        assert!(directory_identity(&source).is_err());
        assert!(directory_identity(dir.path()).is_ok());
        assert!(file_identity(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn handle_identity_follows_the_file_not_the_path() {
        let dir = TempDir::new("handle-identity");
        let path = dir.write("lock", 16);
        let other = dir.write("other", 16);
        let handle = fs::File::open(&path).unwrap();
        assert_eq!(
            handle_identity(&handle).unwrap(),
            file_identity(&path).unwrap()
        );
        assert_ne!(
            handle_identity(&handle).unwrap(),
            file_identity(&other).unwrap()
        );

        // Replace what the path names while the handle stays open: the
        // handle keeps the old file's identity.
        let moved = dir.path().join("moved");
        fs::rename(&path, &moved).unwrap();
        fs::write(&path, b"new").unwrap();
        assert_eq!(
            handle_identity(&handle).unwrap(),
            file_identity(&moved).unwrap()
        );
        assert_ne!(
            handle_identity(&handle).unwrap(),
            file_identity(&path).unwrap()
        );
    }

    #[test]
    fn directory_identity_rejects_a_symlink() {
        let dir = TempDir::new("directory-symlink");
        let link = dir.path().join("alias");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        assert!(directory_identity(&link).is_err());
        assert_eq!(
            file_identity(&link).unwrap(),
            file_identity(dir.path()).unwrap()
        );
    }
}
