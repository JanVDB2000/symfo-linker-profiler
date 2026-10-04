use crate::backup::{require_restorable_backup, restore_vendor, vendor_path, with_vendor_backup};
use crate::errors::SymfoLinkerError;
use crate::links::{create_link, remove_link, require_removable_link, LinkKind};
use crate::lock::RootLock;
use crate::write_guard::WriteGuard;
use std::path::Path;

/// Switches a package from the Composer version to the local project.
///
/// The Composer directory is moved to the backup location first, and the link is
/// created in its place. If linking fails the move is undone, so the package is never
/// left missing (plan sections 16 and 19).
///
/// There is deliberately no `toggle`: plan section 21 wants the caller to state the
/// target state, which keeps error handling unambiguous.
pub fn activate_local(
    lock: &RootLock,
    guard: &WriteGuard,
    project: &Path,
    package: &str,
    local_project: &Path,
) -> Result<LinkKind, SymfoLinkerError> {
    // Checked before the move, so a missing local project changes nothing at all.
    if !local_project.is_dir() {
        return Err(SymfoLinkerError::LocalProjectNotFound {
            path: local_project.to_string_lossy().into_owned(),
        });
    }

    let vendor = vendor_path(project, package);
    with_vendor_backup(lock, guard, project, package, |_backup| {
        create_link(guard, project, &vendor, local_project)
    })
}

/// Switches a package back to the Composer version.
///
/// Removing the link and moving the preserved directory back are two steps, and the
/// vendor path is empty in between. Both ends of that window are covered: the backup is
/// checked before the link is touched, and a restore that fails anyway puts the link
/// back. A caller that sees an error can therefore keep working with the local source,
/// instead of finding the package in neither place (plan sections 19 and 20).
pub fn staged_link_path(project: &Path, package: &str) -> std::path::PathBuf {
    project
        .join("vendor/.symfolinker/.restore-links")
        .join(package)
}

pub fn activate_vendor(
    lock: &RootLock,
    guard: &WriteGuard,
    project: &Path,
    package: &str,
) -> Result<(), SymfoLinkerError> {
    let vendor = vendor_path(project, package);
    require_removable_link(guard, project, &vendor)?;
    require_restorable_backup(guard, project, package)?;
    let staged = staged_link_path(project, package);
    guard.assert_vendor_path(project, &staged)?;
    if std::fs::symlink_metadata(&staged).is_ok() {
        return Err(SymfoLinkerError::VendorPathOccupied {
            path: staged.to_string_lossy().into_owned(),
        });
    }
    std::fs::create_dir_all(staged.parent().expect("staged path has parent"))
        .map_err(|e| SymfoLinkerError::io(&staged, &e))?;
    // Persist intent before moving the link, including broken links and junctions.
    let mut record = crate::journal::read(project)
        .remove(package)
        .unwrap_or_else(|| {
            crate::journal::BackupRecord::new(
                package,
                project,
                None,
                crate::journal::SwapState::Restoring,
            )
        });
    record.state = crate::journal::SwapState::Restoring;
    crate::journal::record(guard, project, record.clone())?;
    std::fs::rename(&vendor, &staged).map_err(|e| SymfoLinkerError::io(&vendor, &e))?;
    if let Err(error) = restore_vendor(lock, guard, project, package) {
        std::fs::rename(&staged, &vendor).map_err(|e| SymfoLinkerError::io(&vendor, &e))?;
        return Err(error);
    }
    // The Composer package is already restored. A leftover staged link is safe and
    // can be cleaned by recovery; cleanup failure must not report a failed restore.
    if remove_link(guard, project, &staged).is_err() {
        let _ = crate::journal::record(guard, project, record);
    }
    Ok(())
}
