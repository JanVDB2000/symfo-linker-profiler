use crate::discovery::scan;
use crate::errors::SymfoLinkerError;
use crate::lock::RootLock;
use crate::models::ScanResult;
use crate::swap::{activate_local, activate_vendor};
use crate::write_guard::WriteGuard;
use std::path::{Path, PathBuf};

/// A swap target, resolved from ids rather than from paths supplied by the caller.
struct Target {
    project: PathBuf,
    local_project: PathBuf,
}

/// Looks up the project and its local source by scanning the development root.
///
/// The frontend sends ids, never paths (plan section 33). Re-reading the filesystem is
/// what makes that safe: a mutation can only ever reach a project the scanner itself
/// discovered under this root, with the package mapping the scanner itself derived.
fn resolve(root: &Path, project_id: &str, package: &str) -> Result<Target, SymfoLinkerError> {
    let result = scan(root).map_err(|_| SymfoLinkerError::ProjectNotFound {
        id: project_id.to_owned(),
    })?;

    let project = result
        .projects
        .iter()
        .find(|candidate| candidate.id == project_id)
        .ok_or_else(|| SymfoLinkerError::ProjectNotFound {
            id: project_id.to_owned(),
        })?;

    let status = project
        .packages
        .iter()
        .find(|candidate| candidate.package_name == package)
        .ok_or_else(|| SymfoLinkerError::PackageNotFound {
            name: package.to_owned(),
        })?;

    Ok(Target {
        project: PathBuf::from(&project.path),
        local_project: PathBuf::from(&status.local_path),
    })
}

/// Switches to the local project, then reports the freshly scanned state.
///
/// The result is read back from disk rather than predicted, so the interface never
/// shows an outcome the filesystem did not actually produce (plan section 48).
pub fn activate_local_package(
    root: &Path,
    project_id: &str,
    package: &str,
) -> Result<ScanResult, SymfoLinkerError> {
    let root = root
        .canonicalize()
        .map_err(|e| SymfoLinkerError::io(root, &e))?;
    let lock = RootLock::acquire(&root)?;
    let target = resolve(&root, project_id, package)?;
    let guard = WriteGuard::new(root.clone());

    activate_local(
        &lock,
        &guard,
        &target.project,
        package,
        &target.local_project,
    )?;
    rescan(&root)
}

/// Switches back to the Composer version, then reports the freshly scanned state.
pub fn activate_vendor_package(
    root: &Path,
    project_id: &str,
    package: &str,
) -> Result<ScanResult, SymfoLinkerError> {
    let root = root
        .canonicalize()
        .map_err(|e| SymfoLinkerError::io(root, &e))?;
    let lock = RootLock::acquire(&root)?;
    let target = resolve(&root, project_id, package)?;
    let guard = WriteGuard::new(root.clone());

    activate_vendor(&lock, &guard, &target.project, package)?;
    rescan(&root)
}

/// The swap itself succeeded, so a failing rescan must not read as a failed swap.
fn rescan(root: &Path) -> Result<ScanResult, SymfoLinkerError> {
    scan(root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed)
}
