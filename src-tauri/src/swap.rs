use crate::backup::{restore_vendor, vendor_path, with_vendor_backup};
use crate::errors::SymfoLinkerError;
use crate::links::{create_link, remove_link, LinkKind};
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
/// Removes the link, then moves the preserved directory back (plan section 20). If the
/// restore fails after the link was removed, the backup is still intact and the error
/// names it, so no work is lost.
pub fn activate_vendor(
    lock: &RootLock,
    guard: &WriteGuard,
    project: &Path,
    package: &str,
) -> Result<(), SymfoLinkerError> {
    let vendor = vendor_path(project, package);
    // Refuses a real directory, so an already-restored package is never deleted.
    remove_link(guard, project, &vendor)?;
    restore_vendor(lock, guard, project, package)?;
    Ok(())
}
