//! Atomic file replacement with a backup copy.
//!
//! New contents are never written into the destination file directly: they are
//! first written to a temporary sibling file, flushed to disk and only then
//! renamed over the destination.  Renaming inside a single directory is atomic,
//! so any reader (and any crash) observes either the complete old file or the
//! complete new one -- never a half-written file.
//!
//! Before a file is overwritten its previous contents are moved aside to a
//! sibling `*.bak` file.  If the operation that follows fails, the backup is
//! moved back, restoring the original file; once the whole operation has
//! succeeded the backup is removed so that it does not litter the file system.
//!
//! [`write_atomic`] covers the common case of "back up, write atomically,
//! clean up", while [`replace`] performs only the atomic replacement (the
//! caller manages the backup) and [`BackupGuard`] can be used to wrap a longer
//! sequence of steps (for example writing the file and then updating the
//! Windows registry).

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};

/// Suffix appended to the target path to form the backup path.
const BACKUP_SUFFIX: &str = ".bak";
/// Suffix of the temporary files created next to the target.
const TEMP_SUFFIX: &str = ".tmp";
/// How many times to retry creating a temporary file with a fresh name.
const TEMP_ATTEMPTS: u32 = 32;

/// Disambiguates temporary files created by concurrently running writers.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns the sibling path used to store the backup of `path`.
fn backup_path(path: &Path) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(BACKUP_SUFFIX);
    PathBuf::from(name)
}

/// Returns the directory that holds `path` (or `.` for a bare file name).
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Writes `contents` to `path`, restoring the previous file on failure.
///
/// If `path` already exists it is first moved to a backup file.  The new
/// contents are then written to a temporary file and renamed over `path`;
/// should any step fail, the backup is put back in place.  On success the
/// backup is deleted and no temporary file is left behind.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let guard = BackupGuard::create(path)?;
    replace(path, contents)?;
    guard.commit();
    Ok(())
}

/// Atomically replaces the contents of `path` with `contents`.
///
/// The new contents are written to a temporary file in the same directory and
/// renamed over `path`.  Unlike [`write_atomic`] this neither creates nor
/// restores a backup, so the caller is expected to have arranged rollback
/// already (for example with a [`BackupGuard`]).  The destination is expected
/// not to exist while this runs; [`BackupGuard::create`] guarantees that by
/// moving the old file aside first.
pub fn replace(path: &Path, contents: &[u8]) -> Result<()> {
    let (temp, mut file) = create_temp_file(path)?;

    if let Err(error) = file
        .write_all(contents)
        .context("failed to write the temporary file")
    {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(error) = file
        .sync_all()
        .context("failed to flush the temporary file to disk")
    {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    // Close the file before renaming it (required on Windows).
    drop(file);

    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error).with_context(|| {
            format!("failed to move the temporary file over '{}'", path.display())
        });
    }

    // Make the rename itself durable where the platform supports it.
    sync_parent_dir(path);
    Ok(())
}

/// Creates a temporary file in the directory of `path`, returning its path and
/// an open handle ready to be written to.
fn create_temp_file(path: &Path) -> Result<(PathBuf, File)> {
    let dir = parent_dir(path);
    let file_name = path.file_name().unwrap_or_default();

    let mut last_error = None;
    for _ in 0..TEMP_ATTEMPTS {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(file_name);
        name.push(format!(".{}.{}{}", std::process::id(), counter, TEMP_SUFFIX));
        let candidate = dir.join(name);

        match OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create a temporary file in '{}'", dir.display())
                });
            }
        }
    }

    let error = last_error.expect("at least one attempt was made");
    Err(error).with_context(|| format!("failed to create a temporary file in '{}'", dir.display()))
}

/// Best-effort flush of the directory entry after a rename (Unix only).
#[cfg(unix)]
fn sync_parent_dir(path: &Path) {
    if let Ok(dir) = File::open(parent_dir(path)) {
        let _ = dir.sync_all();
    }
}

/// Best-effort flush of the directory entry after a rename (non-Unix).
#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) {}

/// Guards an existing file while it is being replaced.
///
/// Creating a guard moves the current file (if any) to a backup.  The caller
/// then performs the replacement and either [`commit`](BackupGuard::commit)s the
/// result or lets the guard go out of scope, in which case the original file is
/// restored from the backup.
#[must_use = "the backup is restored unless commit() is called"]
pub struct BackupGuard {
    target: PathBuf,
    backup: Option<PathBuf>,
}

impl BackupGuard {
    /// Moves an existing `target` out of the way to a backup file.
    ///
    /// If `target` does not exist the guard is created without a backup and the
    /// caller may simply create the file.
    pub fn create(target: &Path) -> Result<Self> {
        if std::fs::symlink_metadata(target).is_err() {
            return Ok(Self {
                target: target.to_path_buf(),
                backup: None,
            });
        }

        let backup = backup_path(target);
        // A leftover backup from an earlier run would make the rename fail.
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(target, &backup).with_context(|| {
            format!(
                "failed to back up '{}' to '{}'",
                target.display(),
                backup.display()
            )
        })?;

        Ok(Self {
            target: target.to_path_buf(),
            backup: Some(backup),
        })
    }

    /// Removes the backup, keeping the newly written file.
    pub fn commit(mut self) {
        if let Some(backup) = self.backup.take() {
            let _ = std::fs::remove_file(backup);
        }
    }

    /// Restores the original file from the backup (best effort).
    fn restore(&mut self) {
        if let Some(backup) = self.backup.take() {
            let _ = std::fs::remove_file(&self.target);
            let _ = std::fs::rename(&backup, &self.target);
        }
    }
}

impl Drop for BackupGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = format!(
                "bdexmlconf-atomic-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            );
            let path = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// Names of all entries in the directory, sorted for stable output.
        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn creates_missing_file_without_backup() {
        let dir = TempDir::new("create");
        let target = dir.0.join("config.cfg");

        write_atomic(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!backup_path(&target).exists());
        assert_eq!(dir.entries(), ["config.cfg"]);
    }

    #[test]
    fn replaces_file_and_removes_backup_on_success() {
        let dir = TempDir::new("replace");
        let target = dir.0.join("config.cfg");
        std::fs::write(&target, b"old").unwrap();

        write_atomic(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(
            !backup_path(&target).exists(),
            "the backup must be cleaned up on success"
        );
    }

    #[test]
    fn leaves_no_temporary_file_after_success() {
        let dir = TempDir::new("no-temp");
        let target = dir.0.join("config.cfg");
        std::fs::write(&target, b"old").unwrap();

        write_atomic(&target, b"new").unwrap();

        assert_eq!(
            dir.entries(),
            ["config.cfg"],
            "only the destination file may remain"
        );
    }

    #[test]
    fn replace_does_not_leave_a_partial_file() {
        let dir = TempDir::new("replace-partial");

        // Writing into a directory that does not exist must fail without
        // creating the destination (or a stray temporary file).
        let missing = dir.0.join("missing").join("config.cfg");
        assert!(replace(&missing, b"data").is_err());
        assert!(!missing.exists());
        assert!(!dir.0.join("missing").exists());
    }

    #[test]
    fn restores_file_when_not_committed() {
        let dir = TempDir::new("restore");
        let target = dir.0.join("config.cfg");
        std::fs::write(&target, b"old").unwrap();

        {
            let _guard = BackupGuard::create(&target).unwrap();
            // Emulate a partially completed replacement.
            std::fs::write(&target, b"partially written").unwrap();
            // Dropped without commit: the original contents come back.
        }

        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!backup_path(&target).exists());
    }

    #[test]
    fn removes_leftover_backup_before_replacing() {
        let dir = TempDir::new("stale");
        let target = dir.0.join("config.cfg");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(backup_path(&target), b"stale").unwrap();

        write_atomic(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!backup_path(&target).exists());
    }

    #[test]
    fn concurrent_temp_names_do_not_collide() {
        // Two temporary files created back to back must use different names.
        let dir = TempDir::new("unique");
        let target = dir.0.join("config.cfg");
        let (first, _file) = create_temp_file(&target).unwrap();
        let (second, _file) = create_temp_file(&target).unwrap();
        assert_ne!(first, second);
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_write_restores_original_and_leaves_no_debris() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("readonly");
        let target = dir.0.join("config.cfg");
        std::fs::write(&target, b"old").unwrap();

        // Make the directory read-only so that creating the temporary file
        // fails.  `set_permissions` does not affect the already-open target.
        let readonly = std::fs::Permissions::from_mode(0o555);
        std::fs::set_permissions(&dir.0, readonly).unwrap();

        // Root bypasses directory permissions; in that case the write would
        // succeed, so only assert when the failure actually happens.
        let result = write_atomic(&target, b"new");

        // Restore write permission before inspecting/cleaning up.
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o755)).unwrap();

        if result.is_err() {
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"old",
                "the original file must be restored"
            );
            assert_eq!(dir.entries(), ["config.cfg"]);
        }
    }
}
