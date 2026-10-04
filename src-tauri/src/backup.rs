use crate::discovery::is_link;
use crate::errors::SymfoLinkerError;
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
    let backup = backup_path(project, package);
    guard.assert_vendor_path(project, &vendor)?;
    guard.assert_vendor_path(project, &backup)?;

    match fs::symlink_metadata(&backup) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
        Ok(_) => {
            return Err(SymfoLinkerError::BackupInvalid {
                path: backup.to_string_lossy().into_owned(),
            })
        }
        Err(_) => {
            return Err(SymfoLinkerError::BackupMissing {
                path: backup.to_string_lossy().into_owned(),
            })
        }
    }

    // Anything sitting at the vendor path would be destroyed by the rename. A fresh
    // `composer install` can recreate it, so stop instead (hard rule 2.4).
    if fs::symlink_metadata(&vendor).is_ok() {
        return Err(SymfoLinkerError::VendorPathOccupied {
            path: vendor.to_string_lossy().into_owned(),
        });
    }

    create_parent(&vendor)?;
    fs::rename(&backup, &vendor).map_err(|error| SymfoLinkerError::io(&backup, &error))?;
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
        Ok(value) => Ok(value),
        Err(step_error) => {
            restore_vendor(lock, guard, project, package)?;
            Err(step_error)
        }
    }
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
