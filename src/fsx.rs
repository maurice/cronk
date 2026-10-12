//! Helpers for private on-disk state: owner-only directories and files, and
//! the stable hex fingerprints used to name them.
//!
//! The workspace keeps several private stores next to the config file — the
//! content cache and the navigation database each own a SQLite database, and
//! the config itself is replaced through a temporary file. All of them must be
//! unreachable by other users on a shared machine, must not be reachable
//! through an attacker-planted symlink, and must be created the same way every
//! time so a later version can rely on the modes it finds.
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

/// Lowercase hex SHA-256 of `value`, used to name a store by installation or
/// credential scope without recording either.
pub(crate) fn hex_sha256(value: impl AsRef<[u8]>) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Reject a path that is a symlink, so creating or opening it cannot write
/// through to a target someone else chose. Missing paths are fine: this guards
/// creation, not existence.
pub(crate) fn reject_symlink(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Path must not be a symlink: {}", path.display()),
        ));
    }
    Ok(())
}

/// Create `directory` and any missing parents, owner-only (0700).
///
/// The mode is applied at creation and set again afterwards, so a permissive
/// process umask cannot leave the directory group- or world-accessible even
/// briefly. Existing permissions are tightened, never widened.
pub(crate) fn private_dir(directory: &Path) -> io::Result<()> {
    reject_symlink(directory)?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    restrict(directory, 0o700)
}

/// Open `path` for writing without truncating it, owner-only (0600), creating
/// it if absent. The mode is applied at creation and set again afterwards so
/// an existing file with wider permissions is tightened.
pub(crate) fn private_file(path: &Path) -> io::Result<File> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    restrict(path, 0o600)?;
    Ok(file)
}

/// Owner-only permissions on `path`: 0700 for directories, 0600 for files.
/// A no-op off unix, where the modes do not exist.
pub(crate) fn restrict(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

/// Flush `directory` so a rename into it survives a crash.
pub(crate) fn sync_dir(directory: &Path) -> Result<()> {
    File::open(directory)
        .and_then(|handle| handle.sync_all())
        .with_context(|| format!("sync directory {}", directory.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_sha256_is_stable_and_matches_the_known_digest() {
        // Pins the fingerprint format: these stores are named by this value,
        // so changing it silently abandons every existing cache on disk.
        assert_eq!(
            hex_sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex_sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_ne!(hex_sha256("a"), hex_sha256("b"));
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_and_file_are_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        private_dir(&nested).unwrap();
        assert_eq!(mode(&nested), 0o700);

        let path = nested.join("state");
        private_file(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert!(path.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_tightens_wider_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("state")).unwrap();
        fs::set_permissions(dir.path().join("state"), fs::Permissions::from_mode(0o777)).unwrap();
        private_dir(&dir.path().join("state")).unwrap();
        assert_eq!(mode(&dir.path().join("state")), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn private_file_tightens_wider_existing_permissions_without_truncating() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        fs::write(&path, "keep me").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        private_file(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_paths_are_refused_and_their_targets_untouched() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("secret"), "private original").unwrap();

        let linked_dir = dir.path().join("linked-dir");
        symlink(&target, &linked_dir).unwrap();
        assert!(private_dir(&linked_dir).is_err());

        let linked_file = dir.path().join("linked-file");
        symlink(target.join("secret"), &linked_file).unwrap();
        assert!(private_file(&linked_file).is_err());

        assert_eq!(
            fs::read_to_string(target.join("secret")).unwrap(),
            "private original"
        );
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
}
