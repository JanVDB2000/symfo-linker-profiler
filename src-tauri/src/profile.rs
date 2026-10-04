use crate::{
    backup, discovery, errors::SymfoLinkerError, lock::RootLock, models::ScanResult, swap,
    write_guard::WriteGuard,
};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileEntry {
    pub project_id: String,
    pub package_name: String,
    pub mode: String,
}

/// Resolve and validate the complete profile under one lock before moving anything.
/// Completed changes are reversed if a later package cannot be switched.
pub fn apply(root: &Path, entries: &[ProfileEntry]) -> Result<ScanResult, SymfoLinkerError> {
    let root = root
        .canonicalize()
        .map_err(|e| SymfoLinkerError::io(root, &e))?;
    let lock = RootLock::acquire(&root)?;
    let guard = WriteGuard::new(&root);
    let scan = discovery::scan(&root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed)?;
    let mut seen = BTreeSet::new();
    let mut changes: Vec<(PathBuf, String, PathBuf, bool)> = Vec::new();
    for entry in entries {
        if !matches!(entry.mode.as_str(), "local" | "vendor")
            || !seen.insert((&entry.project_id, &entry.package_name))
        {
            return Err(SymfoLinkerError::PackageNotFound {
                name: entry.package_name.clone(),
            });
        }
        let project = scan
            .projects
            .iter()
            .find(|p| p.id == entry.project_id)
            .ok_or_else(|| SymfoLinkerError::ProjectNotFound {
                id: entry.project_id.clone(),
            })?;
        let pkg = project
            .packages
            .iter()
            .find(|p| p.package_name == entry.package_name)
            .ok_or_else(|| SymfoLinkerError::PackageNotFound {
                name: entry.package_name.clone(),
            })?;
        let project_path = PathBuf::from(&project.path);
        let vendor = backup::vendor_path(&project_path, &entry.package_name);
        guard.assert_vendor_path(&project_path, &vendor)?;
        if pkg.mode == "unknown" {
            return Err(SymfoLinkerError::UnexpectedSymlink {
                path: pkg.vendor_path.clone(),
            });
        }
        if pkg.mode == entry.mode {
            continue;
        }
        let to_local = entry.mode == "local";
        if to_local {
            guard.assert_vendor_path(
                &project_path,
                &backup::backup_path(&project_path, &entry.package_name),
            )?;
            if fs::symlink_metadata(&pkg.backup_path).is_ok() {
                return Err(SymfoLinkerError::BackupAlreadyExists {
                    path: pkg.backup_path.clone(),
                });
            }
            if !Path::new(&pkg.local_path).is_dir() {
                return Err(SymfoLinkerError::LocalProjectNotFound {
                    path: pkg.local_path.clone(),
                });
            }
        } else {
            backup::require_restorable_backup(&guard, &project_path, &entry.package_name)?;
        }
        changes.push((
            project_path,
            entry.package_name.clone(),
            PathBuf::from(&pkg.local_path),
            to_local,
        ));
    }
    let mut completed = Vec::new();
    for (index, (project, package, local, to_local)) in changes.iter().enumerate() {
        let outcome = if *to_local {
            swap::activate_local(&lock, &guard, project, package, local).map(|_| ())
        } else {
            swap::activate_vendor(&lock, &guard, project, package)
        };
        if let Err(error) = outcome {
            for previous in completed.into_iter().rev() {
                let (project, package, local, to_local): &(PathBuf, String, PathBuf, bool) =
                    &changes[previous];
                let rollback = if *to_local {
                    swap::activate_vendor(&lock, &guard, project, package)
                } else {
                    swap::activate_local(&lock, &guard, project, package, local).map(|_| ())
                };
                rollback?;
            }
            return Err(error);
        }
        completed.push(index);
    }
    discovery::scan(&root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed)
}
