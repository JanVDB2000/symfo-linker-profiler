//! Can this project be switched at all, asked before a switch rather than during one.
//!
//! A swap moves directories and creates a link. Each of those can fail on a read-only
//! vendor directory, on a Windows installation without Developer Mode, or on a full
//! disk - and each failure then arrives halfway through, as an operating system error
//! on a path the user never typed. The scanner cannot answer any of it, because it
//! only reads: the only honest way to know whether a write will work is to write.
//!
//! So these probes really do write, in the one place writing is allowed, and remove
//! what they made. The link probe goes through exactly the same `links::create_link`
//! a real switch uses, which is why its answer is worth having: it reports the link
//! kind this project would actually get, not the one the platform promises.

use crate::{
    discovery, errors::SymfoLinkerError, history, links, links::LinkKind, lock::RootLock,
    models::PreflightReport, write_guard::WriteGuard,
};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

/// Runs every check for one project. Only an unresolvable project or an unavailable
/// lock fails outright; a failing probe is a reported result, not an error.
pub fn check(root: &Path, project_id: &str) -> Result<PreflightReport, SymfoLinkerError> {
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
    let project = PathBuf::from(&project.path);
    // The probes write, so they take the lock like any other write. Checking while a
    // swap is running would also report on a vendor directory in motion.
    let lock = RootLock::acquire(&root)?;
    let guard = WriteGuard::new(&root);

    let root_result = probe_root(&root);
    let vendor_result = probe_vendor(&guard, &project);
    let link_result = probe_link(&guard, &project);

    Ok(PreflightReport {
        root_writable: root_result.is_ok(),
        // Answered by the lock this check already holds, so it costs nothing extra.
        locking_supported: lock.locking_supported(),
        vendor_writable: vendor_result.is_ok(),
        link_kind: link_result.as_ref().ok().map(|kind| match kind {
            LinkKind::RelativeSymlink => "symlink",
            LinkKind::Junction => "junction",
        }),
        free_bytes: fs4::available_space(&project).ok(),
        preserved_bytes: history::measure(&preserved(&project)).0,
        // The first failure's own wording: a red cross without a reason is not a check.
        message: [root_result, vendor_result, link_result.map(|_| ())]
            .into_iter()
            .find_map(|result| result.err())
            .map(|error| error.message()),
    })
}

/// `<root>/.symfolinker/` holds the lock and the configuration, above the projects.
fn probe_root(root: &Path) -> Result<(), SymfoLinkerError> {
    let directory = root.join(".symfolinker");
    fs::create_dir_all(&directory).map_err(|error| SymfoLinkerError::io(&directory, &error))?;
    write_and_remove(&directory.join(probe_name("preflight")))
}

/// `<project>/vendor/` is the only writable zone inside a project, so a swap can only
/// work if a new entry can appear there.
fn probe_vendor(guard: &WriteGuard, project: &Path) -> Result<(), SymfoLinkerError> {
    let directory = preserved_directory(guard, project)?;
    let probe = directory.join(probe_name("preflight"));
    guard.assert_vendor_path(project, &probe)?;
    write_and_remove(&probe)
}

/// `<project>/vendor/.symfolinker/`, created if needed, which is where both probes go.
///
/// The vendor directory itself is never created: a project without one has not been
/// installed yet, and creating it here would be a write nobody asked for - including
/// as a side effect of only asking a question.
fn preserved_directory(guard: &WriteGuard, project: &Path) -> Result<PathBuf, SymfoLinkerError> {
    let vendor = project.join("vendor");
    if !vendor.is_dir() {
        return Err(SymfoLinkerError::VendorPackageMissing {
            path: vendor.to_string_lossy().into_owned(),
        });
    }
    let directory = preserved(project);
    guard.assert_vendor_path(project, &directory)?;
    fs::create_dir_all(&directory).map_err(|error| SymfoLinkerError::io(&directory, &error))?;
    Ok(directory)
}

/// What a link in this project would be, by making one and reading back which kind it
/// became. The target is the project directory itself: a directory that certainly
/// exists, and one a relative target can reach from inside `vendor/`.
fn probe_link(guard: &WriteGuard, project: &Path) -> Result<LinkKind, SymfoLinkerError> {
    let directory = preserved_directory(guard, project)?;
    let probe = directory.join(probe_name("preflight-link"));
    guard.assert_vendor_path(project, &probe)?;
    // A probe left behind by an interrupted run would make `create_link` refuse.
    if fs::symlink_metadata(&probe).is_ok() {
        links::remove_link(guard, project, &probe)?;
    }
    let kind = links::create_link(guard, project, &probe, project)?;
    links::remove_link(guard, project, &probe)?;
    Ok(kind)
}

/// Writing is the check; leaving the file behind would turn a check into litter.
fn write_and_remove(probe: &Path) -> Result<(), SymfoLinkerError> {
    File::options()
        .write(true)
        .create_new(true)
        .open(probe)
        .map_err(|error| SymfoLinkerError::io(probe, &error))?;
    fs::remove_file(probe).map_err(|error| SymfoLinkerError::io(probe, &error))
}

/// Where SymfoLinker keeps what it preserves, inside the project's vendor directory.
fn preserved(project: &Path) -> PathBuf {
    project.join("vendor").join(".symfolinker")
}

/// Per process, so two instances checking at once cannot remove each other's probe.
fn probe_name(kind: &str) -> String {
    format!(".{}-{}", kind, std::process::id())
}
