use crate::{
    backup, discovery,
    errors::SymfoLinkerError,
    journal::{self, SwapState},
    links,
    lock::RootLock,
    models::ScanResult,
    swap,
    write_guard::WriteGuard,
};
use std::{fs, path::Path};

/// Explicit recovery, never performed by the read-only scanner. Paths come from a
/// fresh scan and validated journal keys, including packages with no local checkout.
pub fn recover(root: &Path, project_id: &str) -> Result<ScanResult, SymfoLinkerError> {
    let root = root
        .canonicalize()
        .map_err(|e| SymfoLinkerError::io(root, &e))?;
    let lock = RootLock::acquire(&root)?;
    let guard = WriteGuard::new(&root);
    let scan = discovery::scan(&root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed)?;
    let project = scan
        .projects
        .iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| SymfoLinkerError::ProjectNotFound {
            id: project_id.to_owned(),
        })?;
    let project = Path::new(&project.path);
    for (package, mut record) in journal::read_checked(project)? {
        if !discovery::valid_package_name(&package)
            || record.package != package
            || Path::new(&record.project) != project
        {
            return Err(SymfoLinkerError::BackupInvalid {
                path: journal::journal_path(project)
                    .to_string_lossy()
                    .into_owned(),
            });
        }
        let vendor = backup::vendor_path(project, &package);
        let backup = backup::backup_path(project, &package);
        let staged = swap::staged_link_path(project, &package);
        guard.assert_vendor_path(project, &vendor)?;
        guard.assert_vendor_path(project, &backup)?;
        guard.assert_vendor_path(project, &staged)?;
        let vendor_meta = fs::symlink_metadata(&vendor).ok();
        let backup_present = fs::symlink_metadata(&backup).is_ok();
        if vendor_meta.is_none() && backup_present {
            backup::restore_vendor(&lock, &guard, project, &package)?;
        } else if let Some(meta) = vendor_meta {
            if discovery::is_link(&meta) && backup_present {
                backup::require_restorable_backup(&guard, project, &package)?;
                // A disappeared checkout must not make its original Composer
                // package unreachable through the UI's local-project mapping.
                if vendor.canonicalize().is_err() || record.state == SwapState::Restoring {
                    swap::activate_vendor(&lock, &guard, project, &package)?;
                    continue;
                }
                record.state = SwapState::Linked;
                journal::record(&guard, project, record)?;
                continue;
            }
            if meta.is_dir() && !discovery::is_link(&meta) && !backup_present {
                journal::forget(&guard, project, &package)?;
            } else {
                return Err(SymfoLinkerError::VendorPathOccupied {
                    path: vendor.to_string_lossy().into_owned(),
                });
            }
        } else {
            return Err(SymfoLinkerError::BackupMissing {
                path: backup.to_string_lossy().into_owned(),
            });
        }
        if fs::symlink_metadata(&staged).is_ok() {
            links::remove_link(&guard, project, &staged)?;
        }
    }
    discovery::scan(&root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed)
}
