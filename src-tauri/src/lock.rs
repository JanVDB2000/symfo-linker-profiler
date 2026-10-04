use crate::errors::SymfoLinkerError;
use fs4::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

/// Exclusive lock on a development root, held for the duration of a mutation.
///
/// Plan section 32: one global lock per development root is enough to stop two
/// swaps from touching the same vendor directory at once. The lock lives in
/// `<root>/.symfolinker/lock` (hard rule 2.2: app state sits above the projects,
/// never inside one).
///
/// Mutating functions take `&RootLock` as a parameter, so holding the lock is a
/// compile-time requirement rather than a convention someone can forget. The
/// operating system releases it when the handle closes, including on a crash.
#[derive(Debug)]
pub struct RootLock {
    file: File,
    path: PathBuf,
}

impl RootLock {
    pub fn acquire(development_root: &Path) -> Result<Self, SymfoLinkerError> {
        let directory = development_root.join(".symfolinker");
        fs::create_dir_all(&directory).map_err(|error| SymfoLinkerError::io(&directory, &error))?;

        let path = directory.join("lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| SymfoLinkerError::io(&path, &error))?;

        // try_lock, never a blocking lock: a frozen GUI is worse than a clear message.
        // Called through the trait explicitly, because newer Rust versions add an
        // inherent File::try_lock that would otherwise shadow it with other semantics.
        FileExt::try_lock(&file).map_err(|_| SymfoLinkerError::LockUnavailable {
            path: development_root.to_string_lossy().into_owned(),
        })?;

        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RootLock {
    fn drop(&mut self) {
        // Best effort: the handle closing releases the lock regardless.
        let _ = FileExt::unlock(&self.file);
    }
}
