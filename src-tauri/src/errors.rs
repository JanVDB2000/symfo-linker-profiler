use crate::models::Message;
use std::path::Path;

/// Explicit failure reasons for mutating operations (plan section 41).
///
/// Raw operating system errors never reach the GUI: every variant maps to a
/// translation key, so the frontend renders a sentence the user can act on.
/// Variants arrive with the milestone that can produce them.
#[derive(Debug, PartialEq)]
pub enum SymfoLinkerError {
    /// Nothing occupies the vendor path, so there is nothing to back up.
    VendorPackageMissing { path: String },
    /// A backup is already present and its contents are not known to be ours.
    BackupAlreadyExists { path: String },
    /// Restoring was requested while no backup exists.
    BackupMissing { path: String },
    /// The backup location is a file, a link, or otherwise unsafe to touch.
    BackupInvalid { path: String },
    /// Something occupies the vendor path, so restoring would destroy it. Not in the
    /// plan's section 41 list; added because the restore rename would be destructive.
    VendorPathOccupied { path: String },
    /// No project with this id exists under the development root.
    ProjectNotFound { id: String },
    /// The project does not require this package as a local dependency.
    PackageNotFound { name: String },
    /// The swap succeeded but the state afterwards could not be read back.
    ScanAfterSwapFailed,
    /// The local project that should be linked to does not exist.
    LocalProjectNotFound { path: String },
    /// A link occupies a path where one was not expected, or points elsewhere.
    UnexpectedSymlink { path: String },
    /// The link exists but resolves to nothing.
    BrokenSymlink { path: String },
    /// A write was attempted outside the single permitted mutation zone.
    WriteDenied { path: String },
    /// Another SymfoLinker instance holds the development-root lock.
    LockUnavailable { path: String },
    /// A filesystem call failed; `detail` carries the operating system wording.
    IoError { path: String, detail: String },
}

impl SymfoLinkerError {
    /// Wraps an I/O failure together with the path it happened on.
    pub fn io(path: &Path, error: &std::io::Error) -> Self {
        Self::IoError {
            path: path.to_string_lossy().into_owned(),
            detail: error.to_string(),
        }
    }

    /// The translation key and parameters the frontend renders.
    pub fn message(&self) -> Message {
        match self {
            Self::VendorPackageMissing { path } => {
                Message::with("No vendor package at {path}.", "path", path.clone())
            }
            Self::BackupAlreadyExists { path } => Message::with(
                "A backup already exists at {path}. SymfoLinker will not overwrite it.",
                "path",
                path.clone(),
            ),
            Self::BackupMissing { path } => {
                Message::with("No backup found at {path}.", "path", path.clone())
            }
            Self::BackupInvalid { path } => Message::with(
                "The backup location {path} is not a usable directory.",
                "path",
                path.clone(),
            ),
            Self::VendorPathOccupied { path } => Message::with(
                "{path} is occupied. Remove or move it before restoring the Composer version.",
                "path",
                path.clone(),
            ),
            Self::ProjectNotFound { id } => {
                Message::with("Project {id} is no longer in this workspace.", "id", id.clone())
            }
            Self::PackageNotFound { name } => Message::with(
                "{name} is not a local dependency of this project.",
                "name",
                name.clone(),
            ),
            Self::ScanAfterSwapFailed => {
                Message::new("The switch succeeded, but the workspace could not be read back. Rescan to continue.")
            }
            Self::LocalProjectNotFound { path } => Message::with(
                "The local project {path} no longer exists.",
                "path",
                path.clone(),
            ),
            Self::UnexpectedSymlink { path } => Message::with(
                "{path} is not the link SymfoLinker expected. Nothing was changed.",
                "path",
                path.clone(),
            ),
            Self::BrokenSymlink { path } => Message::with(
                "The link at {path} points to a target that no longer exists.",
                "path",
                path.clone(),
            ),
            Self::WriteDenied { path } => Message::with(
                "SymfoLinker may only modify files inside vendor/. Refused: {path}.",
                "path",
                path.clone(),
            ),
            Self::LockUnavailable { path } => Message::with(
                "Another SymfoLinker instance is working in this development root ({path}).",
                "path",
                path.clone(),
            ),
            Self::IoError { path, detail } => {
                let mut message = Message::with(
                    "SymfoLinker cannot modify {path}. Reason: {detail}",
                    "path",
                    path.clone(),
                );
                message.params.insert("detail".to_owned(), detail.clone());
                message
            }
        }
    }
}

impl From<SymfoLinkerError> for Message {
    fn from(error: SymfoLinkerError) -> Self {
        error.message()
    }
}
