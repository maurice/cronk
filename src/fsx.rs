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
/// briefly. `restrict` sets the mode outright rather than narrowing it, so a
/// directory the owner deliberately made read-only (0500) becomes writable
/// again; that is the intent here, since Cronk owns these directories.
///
/// Does not reject a symlink. Callers that must not follow one — the stores
/// that live in a fixed location beside the workspace — call `reject_symlink`
/// first. The config writer deliberately does not, so that a dotfiles layout
/// symlinking the config directory keeps working.
pub(crate) fn private_dir(directory: &Path) -> io::Result<()> {
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

/// Owner-only permissions on an open handle. Preferable to `restrict` when the
/// handle is already open, because it cannot be redirected by a symlink swapped
/// in between the two calls.
pub(crate) fn restrict_handle(file: &File, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (file, mode);
    Ok(())
}

/// Flush `directory` so a rename into it survives a crash.
///
/// Unix-only: opening a directory for sync is not portable, so elsewhere this
/// is a no-op rather than a failure.
pub(crate) fn sync_dir(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(directory)
        .and_then(|handle| handle.sync_all())
        .with_context(|| format!("sync directory {}", directory.display()))?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
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
    fn private_file_refuses_a_symlinked_path() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "private original").unwrap();

        let linked_file = dir.path().join("linked-file");
        symlink(&target, &linked_file).unwrap();
        assert!(private_file(&linked_file).is_err());

        assert_eq!(fs::read_to_string(&target).unwrap(), "private original");
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_is_idempotent_and_restricts_missing_intermediates() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        private_dir(&nested).unwrap();
        // Recreating an existing directory is not an error, and the mode holds.
        private_dir(&nested).unwrap();
        assert_eq!(mode(&nested), 0o700);
        assert_eq!(mode(&dir.path().join("a")), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn restrict_handle_matches_restrict_on_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        let file = private_file(&path).unwrap();
        restrict_handle(&file, 0o600).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn sync_dir_accepts_a_directory_and_reports_the_path_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        sync_dir(dir.path()).unwrap();

        let missing = dir.path().join("absent");
        let error = sync_dir(&missing).unwrap_err();
        assert!(
            format!("{error:#}").contains("absent"),
            "error should name the directory: {error:#}"
        );
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
}
