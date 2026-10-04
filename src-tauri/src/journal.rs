use crate::atomic::write_atomically;
use crate::errors::SymfoLinkerError;
use crate::write_guard::WriteGuard;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Bumped when the stored shape changes. An unknown version reads as an empty journal,
/// so a newer file never renders as if it described the packages on disk.
const JOURNAL_VERSION: u32 = 1;

/// What SymfoLinker was doing with a package when the record was last written.
///
/// The two transient states are the point of the journal: a record still in one of them
/// means the process died mid-swap, which is otherwise indistinguishable from a finished
/// one by looking at the directories alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SwapState {
    /// The Composer directory is being moved aside; no link exists yet.
    BackingUp,
    /// The swap finished: the backup holds Composer's version, the vendor path a link.
    Linked,
    /// The backup is being moved back; the link is already gone.
    Restoring,
}

/// What is preserved in a backup, and how it got there (plan section 15).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRecord {
    pub package: String,
    /// The project the backup belongs to, as it was when the swap started.
    pub project: String,
    /// The version Composer installed, when it could be determined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub state: SwapState,
    /// Seconds since the Unix epoch; 0 when the clock could not be read.
    #[serde(default)]
    pub updated_at: u64,
}

impl BackupRecord {
    pub fn new(package: &str, project: &Path, version: Option<String>, state: SwapState) -> Self {
        Self {
            package: package.to_owned(),
            project: project.to_string_lossy().into_owned(),
            version,
            state,
            updated_at: now(),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    version: u32,
    #[serde(default)]
    packages: BTreeMap<String, BackupRecord>,
}

/// Every backup this project knows about, keyed by package name.
pub type Journal = BTreeMap<String, BackupRecord>;

/// One file per project, next to the backups it describes.
///
/// A package name always contains a slash, so a backup directory is always at least two
/// segments deep and can never collide with this file.
pub fn journal_path(project: &Path) -> PathBuf {
    project
        .join("vendor")
        .join(".symfolinker")
        .join("state.json")
}

/// Reads what was recorded. A missing, unreadable or unknown file is an empty journal:
/// provenance that cannot be read must never stop a workspace from being scanned.
pub fn read(project: &Path) -> Journal {
    read_checked(project).unwrap_or_default()
}

pub fn read_checked(project: &Path) -> Result<Journal, SymfoLinkerError> {
    let path = journal_path(project);
    let invalid = || SymfoLinkerError::BackupInvalid {
        path: path.to_string_lossy().into_owned(),
    };
    if !crate::discovery::safe_ancestors(project, &path) {
        return Err(invalid());
    }
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Journal::new()),
        Err(e) => return Err(SymfoLinkerError::io(&path, &e)),
        Ok(meta) if crate::discovery::is_link(&meta) || !meta.is_file() => return Err(invalid()),
        Ok(_) => {}
    }
    let bytes = fs::read(&path).map_err(|e| SymfoLinkerError::io(&path, &e))?;
    let stored: Stored = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if stored.version != JOURNAL_VERSION {
        return Err(invalid());
    }
    Ok(stored.packages)
}

/// Stores one record, leaving every other package untouched.
pub fn record(
    guard: &WriteGuard,
    project: &Path,
    record: BackupRecord,
) -> Result<(), SymfoLinkerError> {
    let mut journal = read(project);
    journal.insert(record.package.clone(), record);
    write(guard, project, journal)
}

/// Drops the record for one package, which is what finishing a restore means.
pub fn forget(guard: &WriteGuard, project: &Path, package: &str) -> Result<(), SymfoLinkerError> {
    let mut journal = read(project);
    if journal.remove(package).is_none() {
        return Ok(());
    }
    write(guard, project, journal)
}

fn write(guard: &WriteGuard, project: &Path, packages: Journal) -> Result<(), SymfoLinkerError> {
    let path = journal_path(project);
    guard.assert_vendor_path(project, &path)?;
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            if crate::discovery::is_link(&meta)
                || !meta.is_file()
                || fs::read(&path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
                    .is_none_or(|stored| stored.version != JOURNAL_VERSION)
            {
                return Err(SymfoLinkerError::BackupInvalid {
                    path: path.to_string_lossy().into_owned(),
                });
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(SymfoLinkerError::io(&path, &error)),
    }
    let stored = Stored {
        version: JOURNAL_VERSION,
        packages,
    };
    let json = serde_json::to_vec_pretty(&stored).map_err(|error| SymfoLinkerError::IoError {
        path: path.to_string_lossy().into_owned(),
        detail: error.to_string(),
    })?;
    write_atomically(&path, &json)
}

/// The version Composer installed for this package.
///
/// `vendor/composer/installed.json` is the authoritative answer, because a package's own
/// `composer.json` usually omits `version` and lets the VCS tag decide. The package
/// manifest is the fallback for the rare package that does declare one.
pub fn installed_version(project: &Path, package: &str, vendor_package: &Path) -> Option<String> {
    installed_json_version(project, package).or_else(|| manifest_version(vendor_package))
}

fn installed_json_version(project: &Path, package: &str) -> Option<String> {
    let bytes = fs::read(
        project
            .join("vendor")
            .join("composer")
            .join("installed.json"),
    )
    .ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    // Composer 2 wraps the list in an object; Composer 1 wrote a bare array.
    let packages = parsed.get("packages").unwrap_or(&parsed).as_array()?.iter();
    packages
        .filter(|entry| entry.get("name").and_then(|name| name.as_str()) == Some(package))
        .find_map(|entry| Some(entry.get("version")?.as_str()?.to_owned()))
}

fn manifest_version(vendor_package: &Path) -> Option<String> {
    let bytes = fs::read(vendor_package.join("composer.json")).ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(parsed.get("version")?.as_str()?.to_owned())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
