use crate::errors::SymfoLinkerError;
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path` so a reader never sees a half-written file.
///
/// The content goes to a temporary file next to the target first, is flushed to disk,
/// and only then replaces the target with a rename. `rename` is atomic within one
/// filesystem, which is why the temporary file is a sibling rather than a file in the
/// system temp directory. An interrupted write therefore leaves the previous file
/// intact; at worst a stray temporary file remains.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), SymfoLinkerError> {
    let parent = path.parent().ok_or_else(|| SymfoLinkerError::IoError {
        path: path.to_string_lossy().into_owned(),
        detail: "the path has no parent directory".to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|error| SymfoLinkerError::io(parent, &error))?;

    let temporary = parent.join(temporary_name(path));
    write_all(&temporary, bytes)?;
    fs::rename(&temporary, path).map_err(|error| {
        // The target still holds the previous content, so only the temporary file is
        // left to clean up. Its removal failing changes nothing about the result.
        let _ = fs::remove_file(&temporary);
        SymfoLinkerError::io(path, &error)
    })
}

fn write_all(temporary: &Path, bytes: &[u8]) -> Result<(), SymfoLinkerError> {
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(temporary)
        .map_err(|error| SymfoLinkerError::io(temporary, &error))?;
    file.write_all(bytes)
        .map_err(|error| SymfoLinkerError::io(temporary, &error))?;
    // Without this the rename can be durable while the content is not.
    file.sync_all()
        .map_err(|error| SymfoLinkerError::io(temporary, &error))
}

/// Unique per process and per call, so two writers never share a temporary file.
fn temporary_name(path: &Path) -> String {
    let stem = path.file_name().map_or_else(
        || "file".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!(
        ".{}.{}.{}.tmp",
        stem,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
