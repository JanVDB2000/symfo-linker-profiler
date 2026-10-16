//! The preserved Composer versions under `vendor/.symfolinker/.history/`.
//!
//! Archiving is what keeps a switch from ever deleting a package: when `composer
//! install` has put a new directory where a link used to be, the older preserved
//! version moves into history instead of being overwritten. Nothing cleans that up,
//! on purpose - an automatic deletion is precisely the behaviour this app exists to
//! avoid - which leaves the opposite problem: a directory that only grows, with no way
//! to see what is in it or what it costs.
//!
//! So this module lists it, measures it, puts one version back, and removes one on
//! request. Removal is the single destructive operation in SymfoLinker: it is never
//! automatic, never a fallback for an error, always one named archive the user chose,
//! and always something SymfoLinker itself moved there.

use crate::{
    backup, discovery,
    errors::SymfoLinkerError,
    journal::{self, BackupRecord, SwapState},
    links,
    lock::RootLock,
    models::{ArchiveEntry, ScanResult},
    write_guard::WriteGuard,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Everything kept for one project, newest first.
pub fn list(root: &Path, project_id: &str) -> Result<Vec<ArchiveEntry>, SymfoLinkerError> {
    let (_, project) = resolve(root, project_id)?;
    let mut entries: Vec<ArchiveEntry> = archives(&project)
        .iter()
        .map(|(package, path)| describe(&project, package, path))
        .collect();
    entries.sort_by(|a, b| {
        b.archived_at
            .cmp(&a.archived_at)
            .then_with(|| a.package_name.cmp(&b.package_name))
            .then_with(|| b.archive_id.cmp(&a.archive_id))
    });
    Ok(entries)
}

/// Removes one archive and its sidecar, and reports what is left.
///
/// The path comes from the directory listing rather than from the caller's id, so a
/// name out of the webview can never become a path of its own. The WriteGuard still
/// has to allow it on top of that, which keeps the deletion inside `<project>/vendor/`
/// with no link anywhere in the chain.
pub fn delete(
    root: &Path,
    project_id: &str,
    package: &str,
    archive_id: &str,
) -> Result<Vec<ArchiveEntry>, SymfoLinkerError> {
    let (root, project) = resolve(root, project_id)?;
    let _lock = RootLock::acquire(&root)?;
    let guard = WriteGuard::new(&root);
    let target = locate(&project, package, archive_id)?;
    guard.assert_vendor_path(&project, &target)?;

    let meta =
        fs::symlink_metadata(&target).map_err(|error| SymfoLinkerError::io(&target, &error))?;
    if discovery::is_link(&meta) {
        // A link is unlinked, never followed: whatever it points at is not ours.
        links::remove_link(&guard, &project, &target)?;
    } else if meta.is_dir() {
        fs::remove_dir_all(&target).map_err(|error| SymfoLinkerError::io(&target, &error))?;
    } else {
        fs::remove_file(&target).map_err(|error| SymfoLinkerError::io(&target, &error))?;
    }
    let sidecar = target.with_extension("json");
    if guard.assert_vendor_path(&project, &sidecar).is_ok() && is_plain_file(&sidecar) {
        // The archive is gone; a description of nothing would only mislead.
        let _ = fs::remove_file(&sidecar);
    }
    list(&root, project_id)
}

/// Makes an archived version the active backup again, so switching to VENDOR restores
/// that version instead of the newest one.
///
/// Only while the package is LOCAL. On VENDOR the Composer directory is in place and
/// the backup slot is not what the project uses, so exchanging it would change nothing
/// visible while quietly making the recorded state describe a different version.
/// Whatever occupies the backup path is archived first, so no version is ever lost,
/// and a failed move puts that back before reporting.
pub fn restore(
    root: &Path,
    project_id: &str,
    package: &str,
    archive_id: &str,
) -> Result<ScanResult, SymfoLinkerError> {
    let (root, project) = resolve(root, project_id)?;
    let lock = RootLock::acquire(&root)?;
    let guard = WriteGuard::new(&root);
    let target = locate(&project, package, archive_id)?;
    guard.assert_vendor_path(&project, &target)?;
    match fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_dir() && !discovery::is_link(&meta) => {}
        // A file or a link was moved aside at some point. It is not a Composer version
        // and must not be moved back into a vendor path as though it were one.
        _ => {
            return Err(SymfoLinkerError::BackupInvalid {
                path: target.to_string_lossy().into_owned(),
            })
        }
    }

    let vendor = backup::vendor_path(&project, package);
    links::require_removable_link(&guard, &project, &vendor)?;
    let backup = backup::backup_path(&project, package);
    guard.assert_vendor_path(&project, &backup)?;
    let archived = match fs::symlink_metadata(&backup) {
        Ok(_) => Some(backup::archive_backup(&lock, &guard, &project, package)?),
        Err(_) => None,
    };

    if let Err(error) = fs::rename(&target, &backup) {
        if let Some((archive, previous)) = archived {
            fs::rename(&archive, &backup)
                .map_err(|failure| SymfoLinkerError::io(&archive, &failure))?;
            if let Some(record) = previous {
                journal::record(&guard, &project, record)?;
            }
        }
        return Err(SymfoLinkerError::io(&target, &error));
    }

    // The journal describes the backup, so it has to describe this version now: it is
    // what a switch to VENDOR will move back, and what the interface reports.
    let sidecar = target.with_extension("json");
    let version = read_record(&sidecar).and_then(|record| record.version);
    journal::record(
        &guard,
        &project,
        BackupRecord::new(package, &project, version, SwapState::Linked),
    )?;
    if is_plain_file(&sidecar) {
        let _ = fs::remove_file(&sidecar);
    }
    discovery::scan(&root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed)
}

/// Canonical root and project path, resolved from a fresh scan.
///
/// Ids come from the webview and paths never do (plan section 33): re-reading the
/// filesystem is what makes a project id safe to act on.
fn resolve(root: &Path, project_id: &str) -> Result<(PathBuf, PathBuf), SymfoLinkerError> {
    let root = root
        .canonicalize()
        .map_err(|error| SymfoLinkerError::io(root, &error))?;
    let scan = discovery::scan(&root).map_err(|_| SymfoLinkerError::ProjectNotFound {
        id: project_id.to_owned(),
    })?;
    let project = scan
        .projects
        .iter()
        .find(|project| project.id == project_id)
        .ok_or_else(|| SymfoLinkerError::ProjectNotFound {
            id: project_id.to_owned(),
        })?;
    Ok((root, PathBuf::from(&project.path)))
}

/// Every archive in a project, as (package, path) pairs read from the filesystem.
///
/// The layout is `<history>/<vendor>/<name>/<archive>`, mirroring a package name, so
/// only entries two levels deep whose directories form a valid package name are
/// considered. Sidecars live beside the archives and are skipped by their extension.
///
/// Only real directories are walked on the way down. A link at the vendor or package
/// level would otherwise make this report the sizes and paths of whatever it resolves
/// to, outside the project; the mutating paths already refuse that through the
/// WriteGuard, and reading has no reason to be more generous than writing. The
/// archives themselves are listed whatever they are, links and files included,
/// because being listed is how something strange gets removed.
fn archives(project: &Path) -> Vec<(String, PathBuf)> {
    let history = project.join("vendor").join(".symfolinker").join(".history");
    if !discovery::safe_ancestors(project, &history) || !is_real_directory(&history) {
        return Vec::new();
    }
    let mut found = Vec::new();
    for vendor in real_directories(&history) {
        let Some(vendor_name) = file_name(&vendor) else {
            continue;
        };
        for package_dir in real_directories(&vendor) {
            let Some(package_name) = file_name(&package_dir) else {
                continue;
            };
            let package = format!("{vendor_name}/{package_name}");
            if !discovery::valid_package_name(&package) {
                continue;
            }
            for archive in children(&package_dir) {
                if archive
                    .extension()
                    .is_some_and(|extension| extension == "json")
                {
                    continue;
                }
                found.push((package.clone(), archive));
            }
        }
    }
    found
}

/// The archive with this id, or an error naming where it was expected.
fn locate(project: &Path, package: &str, archive_id: &str) -> Result<PathBuf, SymfoLinkerError> {
    archives(project)
        .into_iter()
        .find(|(found, path)| {
            found == package && file_name(path).is_some_and(|name| name == archive_id)
        })
        .map(|(_, path)| path)
        .ok_or_else(|| SymfoLinkerError::BackupMissing {
            path: backup::history_root(project, package)
                .join(archive_id)
                .to_string_lossy()
                .into_owned(),
        })
}

fn describe(project: &Path, package: &str, path: &Path) -> ArchiveEntry {
    let record = read_record(&path.with_extension("json"));
    let recognized = record
        .as_ref()
        .is_some_and(|record| record.package == package && Path::new(&record.project) == project);
    let directory =
        fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !discovery::is_link(&meta));
    let (size_bytes, files) = if directory { measure(path) } else { (0, 0) };
    ArchiveEntry {
        package_name: package.to_owned(),
        archive_id: file_name(path).unwrap_or_default(),
        path: discovery::display(path),
        version: record.as_ref().and_then(|record| record.version.clone()),
        archived_at: record
            .as_ref()
            .map(|record| record.updated_at)
            .filter(|moment| *moment > 0)
            .or_else(|| stamped_at(path))
            .unwrap_or_default(),
        size_bytes,
        files,
        recognized,
        directory,
    }
}

/// Size and file count of a preserved directory, never following links.
///
/// A link inside an archive points at something that belongs to someone else, and
/// counting its target would attribute a whole checkout to a few kilobytes of history.
/// Shared with the pre-flight report, which measures what is preserved in total.
pub fn measure(path: &Path) -> (u64, usize) {
    let mut bytes = 0;
    let mut files = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        for child in children(&current) {
            match fs::symlink_metadata(&child) {
                Ok(meta) if discovery::is_link(&meta) => files += 1,
                Ok(meta) if meta.is_dir() => pending.push(child),
                Ok(meta) => {
                    bytes += meta.len();
                    files += 1;
                }
                Err(_) => {}
            }
        }
    }
    (bytes, files)
}

/// Archive names start with the millisecond they were made, which is the only
/// timestamp left once a sidecar is missing.
fn stamped_at(path: &Path) -> Option<u64> {
    file_name(path)?
        .split('-')
        .next()?
        .parse::<u64>()
        .ok()
        .map(|millis| millis / 1000)
}

fn read_record(sidecar: &Path) -> Option<BackupRecord> {
    if !is_plain_file(sidecar) {
        return None;
    }
    serde_json::from_slice(&fs::read(sidecar).ok()?).ok()
}

/// A link or a directory where a sidecar is expected is not a sidecar.
fn is_plain_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && !discovery::is_link(&meta))
}

/// The children of `path` that are directories in their own right, not links to one.
fn real_directories(path: &Path) -> Vec<PathBuf> {
    children(path)
        .into_iter()
        .filter(|child| is_real_directory(child))
        .collect()
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !discovery::is_link(&meta))
}

fn children(path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    entries.flatten().map(|entry| entry.path()).collect()
}

fn file_name(path: &Path) -> Option<String> {
    Some(path.file_name()?.to_string_lossy().into_owned())
}
