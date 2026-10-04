use crate::discovery::is_link;
use crate::errors::SymfoLinkerError;
use crate::write_guard::WriteGuard;
use std::{fs, path::Path, path::PathBuf};

/// How the vendor path points at the local project.
///
/// Plan section 23 wants relative symlinks, because an absolute target breaks once the
/// tree is mounted at a different prefix inside a container (section 24). Windows only
/// grants symlink creation under Developer Mode or elevation, so a junction is the
/// fallback there. A junction stores an absolute path, so the distinction is reported
/// rather than hidden: container path mapping cannot be assumed to work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// Relative symlink, valid in both host and container trees.
    RelativeSymlink,
    /// Windows directory junction with an absolute target.
    Junction,
}

/// What currently occupies a vendor path.
#[derive(Debug, PartialEq)]
pub enum LinkState {
    /// A real directory: the Composer version is in place.
    NotLinked,
    /// A link resolving to the expected local project.
    Linked,
    /// A link whose target does not exist.
    Broken,
    /// A link resolving somewhere other than the expected local project.
    UnexpectedTarget { resolved: String },
    /// Nothing is there.
    Missing,
}

/// Relative path from the link's own directory to `local_project`.
///
/// The link lives *at* `link`, so the path is calculated from its parent: a link at
/// `dev/app/vendor/acme/bundle` pointing to `dev/bundle` becomes `../../../bundle`.
/// Both inputs must be canonical, or the result is meaningless rather than wrong.
pub fn relative_target(link: &Path, local_project: &Path) -> Option<PathBuf> {
    let parent = link.parent()?;
    pathdiff::diff_paths(local_project, parent)
}

/// Reads what sits at `vendor`, without following the link to judge it.
pub fn inspect_link(vendor: &Path, local_project: &Path) -> LinkState {
    let Ok(meta) = fs::symlink_metadata(vendor) else {
        return LinkState::Missing;
    };
    if !is_link(&meta) {
        return LinkState::NotLinked;
    }
    match vendor.canonicalize() {
        Err(_) => LinkState::Broken,
        Ok(resolved) if resolved == local_project => LinkState::Linked,
        Ok(resolved) => LinkState::UnexpectedTarget {
            resolved: resolved.to_string_lossy().into_owned(),
        },
    }
}

/// Points `vendor` at `local_project`. The vendor path must already be free, which is
/// what `backup::backup_vendor` guarantees by moving the Composer directory aside.
pub fn create_link(
    guard: &WriteGuard,
    project: &Path,
    vendor: &Path,
    local_project: &Path,
) -> Result<LinkKind, SymfoLinkerError> {
    guard.assert_vendor_path(project, vendor)?;

    if !local_project.is_dir() {
        return Err(SymfoLinkerError::LocalProjectNotFound {
            path: local_project.to_string_lossy().into_owned(),
        });
    }
    if fs::symlink_metadata(vendor).is_ok() {
        return Err(SymfoLinkerError::UnexpectedSymlink {
            path: vendor.to_string_lossy().into_owned(),
        });
    }

    let relative = relative_target(vendor, local_project).ok_or_else(|| {
        SymfoLinkerError::LocalProjectNotFound {
            path: local_project.to_string_lossy().into_owned(),
        }
    })?;
    platform_link(vendor, &relative, local_project)
}

/// Checks that `vendor` holds a link this app may remove, without removing it.
///
/// Separate from `remove_link` so a caller with more work ahead of it can refuse while
/// everything is still in place, and refuse with exactly the same wording.
pub fn require_removable_link(
    guard: &WriteGuard,
    project: &Path,
    vendor: &Path,
) -> Result<(), SymfoLinkerError> {
    guard.assert_vendor_path(project, vendor)?;

    let meta = fs::symlink_metadata(vendor).map_err(|_| SymfoLinkerError::BrokenSymlink {
        path: vendor.to_string_lossy().into_owned(),
    })?;
    // Refusing a real directory is the point: that would delete Composer's files.
    if !is_link(&meta) {
        return Err(SymfoLinkerError::UnexpectedSymlink {
            path: vendor.to_string_lossy().into_owned(),
        });
    }
    Ok(())
}

/// Removes the link itself. Never touches what it points at.
pub fn remove_link(
    guard: &WriteGuard,
    project: &Path,
    vendor: &Path,
) -> Result<(), SymfoLinkerError> {
    require_removable_link(guard, project, vendor)?;
    remove_platform_link(vendor)
}

#[cfg(windows)]
fn platform_link(
    vendor: &Path,
    relative: &Path,
    local_project: &Path,
) -> Result<LinkKind, SymfoLinkerError> {
    // ERROR_PRIVILEGE_NOT_HELD: no Developer Mode and not elevated.
    const PRIVILEGE_NOT_HELD: i32 = 1314;

    match std::os::windows::fs::symlink_dir(relative, vendor) {
        Ok(()) => Ok(LinkKind::RelativeSymlink),
        Err(error) if error.raw_os_error() == Some(PRIVILEGE_NOT_HELD) => {
            // A junction needs an absolute target, so the relative form is lost here.
            junction::create(local_project, vendor)
                .map(|_| LinkKind::Junction)
                .map_err(|error| SymfoLinkerError::io(vendor, &error))
        }
        Err(error) => Err(SymfoLinkerError::io(vendor, &error)),
    }
}

#[cfg(not(windows))]
fn platform_link(
    vendor: &Path,
    relative: &Path,
    _local_project: &Path,
) -> Result<LinkKind, SymfoLinkerError> {
    std::os::unix::fs::symlink(relative, vendor)
        .map(|_| LinkKind::RelativeSymlink)
        .map_err(|error| SymfoLinkerError::io(vendor, &error))
}

#[cfg(windows)]
fn remove_platform_link(vendor: &Path) -> Result<(), SymfoLinkerError> {
    // Both directory symlinks and junctions are removed as directories on Windows;
    // this unlinks the entry and leaves the target untouched.
    fs::remove_dir(vendor).map_err(|error| SymfoLinkerError::io(vendor, &error))
}

#[cfg(not(windows))]
fn remove_platform_link(vendor: &Path) -> Result<(), SymfoLinkerError> {
    fs::remove_file(vendor).map_err(|error| SymfoLinkerError::io(vendor, &error))
}
