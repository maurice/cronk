//! Helpers for private on-disk state: owner-only directories and files, and
//! the stable hex fingerprints used to name them.
//!
//! The workspace keeps two stores beside the config file — the content cache
//! and the navigation database each own a SQLite database — and the config
//! itself is replaced through a temporary file. The two stores are Cronk's own:
//! on Unix they are created owner-only and tightened if an earlier run left
//! them wider. Their directory and database paths are refused if they are
//! symlinks when checked. These checks are not atomic with opening, and SQLite
//! later reopens each database by path, so they do not protect against concurrent
//! path replacement. The containing directory tree must be trusted.
//!
//! The config path is neither owned nor private by contract: it is whatever
//! `--config` names. On Unix its directory is created owner-only if missing, but
//! an existing one keeps the permissions its owner gave it, and a symlinked
//! directory is followed so that a dotfiles layout keeps working. A symlinked
//! destination *file* is different again: the atomic rename replaces the link
//! rather than writing through it, leaving whatever it pointed at untouched.
//!
//! `hex_sha256` lives here rather than with the other models because it is what
//! names these stores on disk: the value must stay stable or every existing
//! cache is silently abandoned.
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

/// Whether `path` is itself a symlink. Missing paths are not.
pub(crate) fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Reject a path that is a symlink at the time of the check.
///
/// Does not validate ancestors or prevent path replacement before later I/O.
///
/// The message names the rule and the path but not the kind of store; callers
/// add that as context so it appears once.
pub(crate) fn reject_symlink(path: &Path) -> io::Result<()> {
    if is_symlink(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("must not be a symlink: {}", path.display()),
        ));
    }
    Ok(())
}

/// Create `directory` and any missing parents with mode 0700 on Unix, subject
/// to the process umask.
///
/// The mode applies to every missing level, not just the leaf.
///
/// An existing directory keeps the permissions its owner gave it. Use this for
/// a caller-supplied path such as the config parent, which Cronk does not own
/// and must not re-permission. Use `owned_dir` for directories Cronk owns.
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

/// Create `directory` and any missing parents as `private_dir` does, and enforce
/// 0700 on Unix even if the directory already existed.
///
/// This is for directories Cronk owns, where an earlier run or a stray tool may
/// have left the mode wider than intended.
pub(crate) fn owned_dir(directory: &Path) -> io::Result<()> {
    private_dir(directory)?;
    set_mode(directory, 0o700)
}

/// Create `path` if absent, without truncating what is already there, and make
/// it owner-only (0600) on Unix.
///
/// Checks for a symlink before opening, but the check and open are not atomic.
/// Setting the mode on the open handle prevents the permission update from
/// following a path replaced after opening; it does not protect the open itself
/// or a caller's subsequent path-based I/O. Does not validate the containing
/// directory.
pub(crate) fn private_file(path: &Path) -> io::Result<()> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    set_mode_handle(&file, 0o600)
}

/// Set `path`'s mode outright — this does not narrow it, so a file the owner
/// deliberately made read-only (0400) becomes writable again. That is the
/// intent for files Cronk owns. A no-op off unix, where modes do not exist.
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

/// Owner-only permissions on an open handle: 0700 for directories, 0600 for
/// files. Sets the mode outright, as `set_mode` does.
pub(crate) fn set_mode_handle(file: &File, mode: u32) -> io::Result<()> {
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
/// is a no-op rather than a failure, and callers must not depend on it.
pub(crate) fn sync_dir(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

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
    fn private_dir_creates_every_missing_level_at_0700() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        private_dir(&nested).unwrap();
        assert_eq!(mode(&dir.path().join("a")), 0o700);
        assert_eq!(mode(&dir.path().join("a/b")), 0o700);
        assert_eq!(mode(&nested), 0o700);
        // Recreating an existing directory is not an error.
        private_dir(&nested).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn owned_dir_tightens_wider_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o777)).unwrap();
        owned_dir(&state).unwrap();
        assert_eq!(mode(&state), 0o700);
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
    fn private_file_refuses_a_symlinked_path_and_leaves_the_link() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "private original").unwrap();

        let linked = dir.path().join("linked-file");
        symlink(&target, &linked).unwrap();

        let error = private_file(&linked).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(error.to_string().contains("linked-file"), "{error}");
        // The link itself survives, so the refusal is recoverable.
        assert!(is_symlink(&linked));
        assert_eq!(fs::read_to_string(&target).unwrap(), "private original");
    }

    #[cfg(unix)]
    #[test]
    fn set_mode_handle_matches_set_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        set_mode_handle(&file, 0o600).unwrap();
        assert_eq!(mode(&path), 0o600);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        set_mode(&path, 0o600).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn sync_dir_accepts_a_directory_and_reports_the_path_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        sync_dir(dir.path()).unwrap();

        let missing = dir.path().join("absent");
        assert!(sync_dir(&missing).is_err());
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
