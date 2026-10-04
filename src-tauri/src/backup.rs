use crate::discovery::is_link;
use crate::errors::SymfoLinkerError;
use crate::journal::{self, BackupRecord, SwapState};
use crate::lock::RootLock;
use crate::write_guard::WriteGuard;
use std::{fs, path::Path, path::PathBuf};

/// Where Composer put the package.
pub fn vendor_path(project: &Path, package: &str) -> PathBuf {
    project.join("vendor").join(package)
}

/// Where the original Composer directory is preserved (plan section 15):
/// `<project>/vendor/.symfolinker/<vendor>/<package>`.
pub fn backup_path(project: &Path, package: &str) -> PathBuf {
    project.join("vendor").join(".symfolinker").join(package)
}

/// How a preserved directory relates to what SymfoLinker recorded about it.
///
/// A directory on its own cannot say whether it is a finished backup, a leftover from
/// an older version, or an operation that never completed. The journal supplies that,
/// so the interface can tell the three apart instead of calling them all available.
pub fn backup_status(path: &Path, record: Option<&BackupRecord>) -> &'static str {
    let present = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => true,
        Ok(_) => return "invalid",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return "invalid",
    };
    match (present, record.map(|record| record.state)) {
        // A transient state that outlived its operation: the process died mid-swap.
        (_, Some(SwapState::BackingUp | SwapState::Restoring)) => "interrupted",
        (true, Some(SwapState::Linked)) => "available",
        // A directory nobody recorded: an older SymfoLinker, or placed there by hand.
        (true, None) => "unrecognized",
        // Recorded as preserved, yet the directory is gone. Switching back would fail.
        (false, Some(SwapState::Linked)) => "lost",
        (false, None) => "missing",
    }
}

/// Checks that the preserved directory is there and safe to move back, without
/// changing anything.
///
/// Callers use this to refuse before they touch the vendor path: `activate_vendor`
/// removes a working link, and doing that first on a missing backup would leave the
/// package in neither place.
pub fn require_restorable_backup(
    guard: &WriteGuard,
    project: &Path,
    package: &str,
) -> Result<PathBuf, SymfoLinkerError> {
    let backup = backup_path(project, package);
    guard.assert_vendor_path(project, &backup)?;
    if let Some(record) = journal::read_checked(project)?.get(package) {
        if record.package != package || Path::new(&record.project) != project {
            return Err(SymfoLinkerError::BackupInvalid {
                path: backup.to_string_lossy().into_owned(),
            });
        }
    }
    match fs::symlink_metadata(&backup) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => Ok(backup),
        Ok(_) => Err(SymfoLinkerError::BackupInvalid {
            path: backup.to_string_lossy().into_owned(),
        }),
        Err(_) => Err(SymfoLinkerError::BackupMissing {
            path: backup.to_string_lossy().into_owned(),
        }),
    }
}

/// Moves the Composer directory aside, preserving it (hard rule 2.3).
///
/// `rename` rather than copy + delete: it is atomic on one filesystem, so there is
/// no window where the package exists in neither place. Nothing is ever deleted.
/// Returns the backup path.
pub fn backup_vendor(
    _lock: &RootLock,
    guard: &WriteGuard,
    project: &Path,
    package: &str,
) -> Result<PathBuf, SymfoLinkerError> {
    let vendor = vendor_path(project, package);
    let backup = backup_path(project, package);
    guard.assert_vendor_path(project, &vendor)?;
    guard.assert_vendor_path(project, &backup)?;

    require_real_directory(&vendor).map_err(|_| SymfoLinkerError::VendorPackageMissing {
        path: vendor.to_string_lossy().into_owned(),
    })?;

    // Hard rule 2.4: never overwrite an existing backup to "fix" the situation.
    match fs::symlink_metadata(&backup) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => {
            return Err(SymfoLinkerError::BackupAlreadyExists {
                path: backup.to_string_lossy().into_owned(),
            })
        }
        Ok(_) => {
            return Err(SymfoLinkerError::BackupInvalid {
                path: backup.to_string_lossy().into_owned(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SymfoLinkerError::io(&backup, &error)),
    }

    // Read while the Composer directory is still in place, and record before the move,
    // so nothing is ever moved that SymfoLinker cannot account for afterwards. This is
    // the one journal write allowed to fail the operation: nothing has moved yet.
    let version = journal::installed_version(project, package, &vendor);
    journal::record(
        guard,
        project,
        BackupRecord::new(package, project, version, SwapState::BackingUp),
    )?;

    create_parent(&backup)?;
    fs::rename(&vendor, &backup).map_err(|error| SymfoLinkerError::io(&vendor, &error))?;
    Ok(backup)
}

/// Moves the preserved Composer directory back to its vendor location (plan section 20).
/// Returns the restored vendor path.
pub fn restore_vendor(
    _lock: &RootLock,
    guard: &WriteGuard,
    project: &Path,
    package: &str,
) -> Result<PathBuf, SymfoLinkerError> {
    let vendor = vendor_path(project, package);
    guard.assert_vendor_path(project, &vendor)?;
    let backup = require_restorable_backup(guard, project, package)?;

    // Anything sitting at the vendor path would be destroyed by the rename. A fresh
    // `composer install` can recreate it, so stop instead (hard rule 2.4).
    if fs::symlink_metadata(&vendor).is_ok() {
        return Err(SymfoLinkerError::VendorPathOccupied {
            path: vendor.to_string_lossy().into_owned(),
        });
    }

    // From here the journal is written best effort. A record that cannot be updated
    // leaves a transient state behind, which the interface reports as an interrupted
    // operation: a conservative outcome, and a better one than failing a move the
    // filesystem is perfectly able to perform.
    let _ = journal::record(
        guard,
        project,
        BackupRecord::new(
            package,
            project,
            stored_version(project, package),
            SwapState::Restoring,
        ),
    );

    create_parent(&vendor)?;
    fs::rename(&backup, &vendor).map_err(|error| SymfoLinkerError::io(&backup, &error))?;
    let _ = journal::forget(guard, project, package);
    Ok(vendor)
}

/// Runs `step` with the Composer directory backed up, rolling back if it fails
/// (plan section 19). Milestone 3 passes symlink creation as the step.
///
/// If the rollback itself fails the rollback error is returned, not the step error:
/// a half-finished swap is the condition the user must act on (hard rule 2.5).
pub fn with_vendor_backup<T>(
    lock: &RootLock,
    guard: &WriteGuard,
    project: &Path,
    package: &str,
    step: impl FnOnce(&Path) -> Result<T, SymfoLinkerError>,
) -> Result<T, SymfoLinkerError> {
    let backup = backup_vendor(lock, guard, project, package)?;
    match step(&backup) {
        Ok(value) => {
            // Best effort for the same reason as in restore_vendor: the swap is already
            // on disk, and a record stuck on backing up reports an interrupted swap
            // rather than undoing one that succeeded.
            let _ = journal::record(
                guard,
                project,
                BackupRecord::new(
                    package,
                    project,
                    stored_version(project, package),
                    SwapState::Linked,
                ),
            );
            Ok(value)
        }
        Err(step_error) => {
            restore_vendor(lock, guard, project, package)?;
            Err(step_error)
        }
    }
}

/// The version recorded when the backup was made, so later writes keep it.
fn stored_version(project: &Path, package: &str) -> Option<String> {
    journal::read(project)
        .get(package)
        .and_then(|record| record.version.clone())
}

/// A link is not a Composer directory, so links are rejected rather than followed.
fn require_real_directory(path: &Path) -> Result<(), ()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => Ok(()),
        _ => Err(()),
    }
}

fn create_parent(path: &Path) -> Result<(), SymfoLinkerError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| SymfoLinkerError::io(parent, &error))
}
