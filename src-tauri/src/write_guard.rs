use crate::discovery::safe_ancestors;
use crate::errors::SymfoLinkerError;
use std::path::{Component, Path, PathBuf};

/// The single gate every mutating filesystem operation passes through (plan section 22).
///
/// Hard rule 2.1: the only writable zone inside a project is `<project>/vendor/`.
/// Everything else - `composer.json`, `src/`, `.git/`, the project root itself - is
/// read-only, so this guard refuses rather than trusting callers to behave.
pub struct WriteGuard {
    development_root: PathBuf,
}

impl WriteGuard {
    pub fn new(development_root: impl Into<PathBuf>) -> Self {
        Self {
            development_root: development_root.into(),
        }
    }

    pub fn development_root(&self) -> &Path {
        &self.development_root
    }

    /// Allows `target` only when it sits strictly inside `<project>/vendor/`.
    ///
    /// `project` is expected to be canonical, as discovery produces it. Traversal
    /// components are rejected outright, so containment can be judged lexically, and
    /// no intermediate directory may be a link - otherwise a junctioned `vendor/`
    /// would let a write land outside the project after all.
    pub fn assert_vendor_path(
        &self,
        project: &Path,
        target: &Path,
    ) -> Result<(), SymfoLinkerError> {
        let denied = || SymfoLinkerError::WriteDenied {
            path: target.to_string_lossy().into_owned(),
        };

        if has_traversal(target) || has_traversal(project) {
            return Err(denied());
        }
        if !project.starts_with(&self.development_root) || project == self.development_root {
            return Err(denied());
        }

        let vendor = project.join("vendor");
        // Strictly inside: the vendor directory itself must never be renamed or removed.
        if !target.starts_with(&vendor) || target == vendor {
            return Err(denied());
        }
        if !safe_ancestors(project, target) {
            return Err(denied());
        }
        Ok(())
    }
}

/// `..` would break lexical containment; `.` is harmless but signals an uncleaned path.
fn has_traversal(path: &Path) -> bool {
    path.components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
}
