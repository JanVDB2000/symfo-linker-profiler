//! Restoring the links a `composer install` replaced.
//!
//! Installing dependencies writes a fresh directory over a linked package. The journal
//! still records a finished link, the vendor path now holds Composer's files, and
//! nothing in this app moved them: that combination is what `discovery` reports as
//! `relinkable`. Until now the only way back was to remember which packages had been
//! linked and toggle each one by hand, which is exactly the kind of bookkeeping the
//! journal already does.
//!
//! The switching itself is not reimplemented here. A profile already resolves entries
//! against a fresh scan under one lock, validates every one before moving anything,
//! and reverses completed changes when a later one fails - which is precisely what
//! restoring a set of links needs.

use crate::{
    discovery,
    errors::SymfoLinkerError,
    models::ScanResult,
    profile::{self, ProfileEntry},
};
use std::path::Path;

/// Every package whose link was replaced from outside, optionally limited to one
/// project. The caller gets entries it can show before anything is switched.
pub fn pending(
    root: &Path,
    project_id: Option<&str>,
) -> Result<Vec<ProfileEntry>, SymfoLinkerError> {
    let scan = discovery::scan(root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed)?;
    Ok(scan
        .projects
        .iter()
        .filter(|project| project_id.is_none_or(|id| project.id == id))
        .flat_map(|project| {
            project
                .packages
                .iter()
                .filter(|package| package.relinkable)
                .map(|package| ProfileEntry {
                    project_id: project.id.clone(),
                    package_name: package.package_name.clone(),
                    mode: "local".to_owned(),
                })
        })
        .collect())
}

/// Switches every replaced link back to LOCAL and reports the state read back.
///
/// Nothing to do is a success, not an error: the interface offers this where a count
/// is shown, and that count can be stale by the time it is clicked.
pub fn apply(root: &Path, project_id: Option<&str>) -> Result<ScanResult, SymfoLinkerError> {
    let root = root
        .canonicalize()
        .map_err(|error| SymfoLinkerError::io(root, &error))?;
    let entries = pending(&root, project_id)?;
    if entries.is_empty() {
        return discovery::scan(&root).map_err(|_| SymfoLinkerError::ScanAfterSwapFailed);
    }
    profile::apply(&root, &entries)
}
