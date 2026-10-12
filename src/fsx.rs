//! Helpers for private on-disk state: owner-only directories and files, and
//! the stable hex fingerprints used to name them.
//!
//! The workspace keeps two stores beside the config file — the content cache
//! and the navigation database each own a SQLite database — and the config
//! itself is replaced through a temporary file. The two stores are Cronk's own:
//! they are created private, tightened if an earlier run left them wider, and
//! refused outright if they are symlinks, so a shared machine cannot read them
//! and an attacker cannot redirect them.
//!
//! The config path is neither owned nor private by contract: it is whatever
//! `--config` names. Its directory is created owner-only if missing but an
//! existing one keeps the permissions its owner gave it, and a symlinked
//! directory or file is followed rather than refused, so that a dotfiles
//! layout keeps working.
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
///
/// The message names the rule and the path but not the kind of store; callers
/// add that as context so it appears once.
pub(crate) fn reject_symlink(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("must not be a symlink: {}", path.display()),
        ));
    }
    Ok(())
}

/// Create `directory` and any missing parents with mode 0700.
///
/// An existing directory keeps the permissions its owner gave it — only newly
/// created components get 0700. Use this for a caller-supplied path such as
/// the config parent, which Cronk does not own and must not re-permission.
/// Use `owned_dir` for directories Cronk created and is responsible for.
///
/// Does not reject a symlink.
pub(crate) fn private_dir(directory: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)
}

/// Create `directory` and any missing parents, owner-only (0700), and enforce
/// 0700 even if the directory already existed.
///
/// This is for directories Cronk owns, where an earlier run or a stray tool may
/// have left the mode wider than intended. `restrict` sets the mode outright
/// rather than narrowing it, so a directory the owner deliberately made
/// read-only (0500) becomes writable again.
///
/// Does not reject a symlink; callers that need that call `reject_symlink`
/// first.
pub(crate) fn owned_dir(directory: &Path) -> io::Result<()> {
    private_dir(directory)?;
    restrict(directory, 0o700)
}

/// Open `path` for writing without truncating it, owner-only (0600), creating
/// it if absent. The mode is applied at creation and set again afterwards so
/// an existing file with wider permissions is tightened.
///
/// Refuses a symlink, so it is safe for the SQLite databases that name
/// themselves from a fingerprint. Does not validate the directory.
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
/// Sets the mode outright rather than narrowing it. A no-op off unix, where
/// these modes do not exist.
fn restrict(path: &Path, mode: u32) -> io::Result<()> {
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
    fn owned_dir_tightens_wider_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("state")).unwrap();
        fs::set_permissions(dir.path().join("state"), fs::Permissions::from_mode(0o777)).unwrap();
        owned_dir(&dir.path().join("state")).unwrap();
        assert_eq!(mode(&dir.path().join("state")), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_leaves_an_existing_directory_permissions_alone() {
        // The config parent is user-supplied; re-permissioning a directory the
        // owner chose -- including the working directory for a bare --config
        // filename -- would be a change the user never asked for.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("shared");
        fs::create_dir(&existing).unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&existing).unwrap();
        assert_eq!(mode(&existing), 0o755);
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

    #[cfg(unix)]
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

    #[cfg(not(unix))]
    #[test]
    fn sync_dir_is_a_documented_no_op_off_unix() {
        // Opening a directory for sync is not portable, so elsewhere this must
        // succeed rather than fail — a config save cannot depend on it.
        let dir = tempfile::tempdir().unwrap();
        sync_dir(dir.path()).unwrap();
        sync_dir(&dir.path().join("absent")).unwrap();
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
}
