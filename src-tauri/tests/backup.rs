use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::backup::{
    backup_path, backup_vendor, restore_vendor, vendor_path, with_vendor_backup,
};
use symfolinker_lib::errors::SymfoLinkerError;
use symfolinker_lib::lock::RootLock;
use symfolinker_lib::write_guard::WriteGuard;

const PACKAGE: &str = "acme/bundle";
const MARKER: &str = "composer.json";
const ORIGINAL: &str = r#"{"name":"acme/bundle"}"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A development root with one project, cleaned up on drop.
struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-backup-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        // Canonical, because the WriteGuard compares paths lexically and temp
        // directories reach us through a link or a short name on some platforms.
        Self(path.canonicalize().unwrap())
    }

    fn root(&self) -> &Path {
        &self.0
    }

    /// Project with the Composer package present at its vendor path.
    fn project_with_vendor(&self, name: &str) -> PathBuf {
        let project = self.0.join(name);
        let vendor = vendor_path(&project, PACKAGE);
        fs::create_dir_all(&vendor).unwrap();
        fs::write(vendor.join(MARKER), ORIGINAL).unwrap();
        project
    }

    fn guard(&self) -> WriteGuard {
        WriteGuard::new(self.0.clone())
    }

    fn lock(&self) -> RootLock {
        RootLock::acquire(&self.0).unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn marker_contents(directory: &Path) -> String {
    fs::read_to_string(directory.join(MARKER)).unwrap()
}

#[test]
fn moves_the_composer_directory_to_the_backup_location_with_its_contents() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());

    let backup = backup_vendor(&lock, &guard, &project, PACKAGE).unwrap();

    assert_eq!(backup, backup_path(&project, PACKAGE));
    assert!(backup.is_dir(), "backup directory should exist");
    assert!(
        !vendor_path(&project, PACKAGE).exists(),
        "vendor path should be free after the move"
    );
    // Preserved, not recreated: the package contents must survive untouched.
    assert_eq!(marker_contents(&backup), ORIGINAL);
}

#[test]
fn restores_the_backup_to_the_vendor_location() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());
    backup_vendor(&lock, &guard, &project, PACKAGE).unwrap();

    let vendor = restore_vendor(&lock, &guard, &project, PACKAGE).unwrap();

    assert_eq!(vendor, vendor_path(&project, PACKAGE));
    assert_eq!(marker_contents(&vendor), ORIGINAL);
    assert!(
        !backup_path(&project, PACKAGE).exists(),
        "backup should no longer occupy the backup location"
    );
}

#[test]
fn refuses_to_overwrite_an_existing_backup_and_changes_nothing() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());
    let backup = backup_path(&project, PACKAGE);
    fs::create_dir_all(&backup).unwrap();
    fs::write(backup.join(MARKER), "older backup").unwrap();

    let error = backup_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::BackupAlreadyExists {
            path: backup.to_string_lossy().into_owned()
        }
    );
    // Hard rule 2.4: no destructive fallback, so both sides stay exactly as they were.
    assert_eq!(marker_contents(&backup), "older backup");
    assert_eq!(marker_contents(&vendor_path(&project, PACKAGE)), ORIGINAL);
}

#[test]
fn reports_a_missing_vendor_package_instead_of_creating_one() {
    let workspace = Workspace::new();
    let project = workspace.0.join("app");
    fs::create_dir_all(project.join("vendor")).unwrap();
    let (lock, guard) = (workspace.lock(), workspace.guard());

    let error = backup_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::VendorPackageMissing {
            path: vendor_path(&project, PACKAGE)
                .to_string_lossy()
                .into_owned()
        }
    );
    assert!(!backup_path(&project, PACKAGE).exists());
}

#[test]
fn reports_a_missing_backup_when_restoring() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());

    let error = restore_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::BackupMissing {
            path: backup_path(&project, PACKAGE)
                .to_string_lossy()
                .into_owned()
        }
    );
}

#[test]
fn refuses_to_restore_over_a_recreated_vendor_directory() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());
    backup_vendor(&lock, &guard, &project, PACKAGE).unwrap();
    // Someone ran `composer install` while the package was backed up.
    let vendor = vendor_path(&project, PACKAGE);
    fs::create_dir_all(&vendor).unwrap();
    fs::write(vendor.join(MARKER), "reinstalled by composer").unwrap();

    let error = restore_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::VendorPathOccupied {
            path: vendor.to_string_lossy().into_owned()
        }
    );
    assert_eq!(marker_contents(&vendor), "reinstalled by composer");
    assert!(backup_path(&project, PACKAGE).is_dir());
}

#[test]
fn rolls_back_to_the_vendor_location_when_the_step_fails() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());

    let result = with_vendor_backup(&lock, &guard, &project, PACKAGE, |backup| {
        // Milestone 3 creates the symlink here; a failure must undo the move.
        assert!(backup.is_dir(), "step runs with the backup in place");
        Err::<(), _>(SymfoLinkerError::IoError {
            path: backup.to_string_lossy().into_owned(),
            detail: "simulated link failure".to_owned(),
        })
    });

    assert!(result.is_err(), "the step error must propagate");
    // Hard rule 2.5: never leave the user with a half-finished swap.
    assert_eq!(marker_contents(&vendor_path(&project, PACKAGE)), ORIGINAL);
    assert!(!backup_path(&project, PACKAGE).exists());
}

#[test]
fn keeps_the_backup_when_the_step_succeeds() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());

    let value = with_vendor_backup(&lock, &guard, &project, PACKAGE, |_| Ok(7)).unwrap();

    assert_eq!(value, 7);
    assert!(backup_path(&project, PACKAGE).is_dir());
    assert!(!vendor_path(&project, PACKAGE).exists());
}

#[test]
fn a_file_blocking_the_backup_directory_is_refused_before_any_disk_change() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());
    // A file where vendor/.symfolinker must go makes every backup path unreachable.
    fs::write(
        project.join("vendor").join(".symfolinker"),
        "not a directory",
    )
    .unwrap();

    let error = backup_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    // The guard rejects the path first, so this never reaches a filesystem call.
    assert_eq!(
        error,
        SymfoLinkerError::WriteDenied {
            path: backup_path(&project, PACKAGE)
                .to_string_lossy()
                .into_owned()
        }
    );
    assert_eq!(marker_contents(&vendor_path(&project, PACKAGE)), ORIGINAL);
}

/// Windows refuses to rename a directory while a file inside it is open, which is the
/// realistic "permission denied" for this operation: an editor or PHP process holds it.
#[cfg(windows)]
#[test]
fn surfaces_an_io_failure_rather_than_a_raw_os_error() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());
    let held = fs::File::open(vendor_path(&project, PACKAGE).join(MARKER)).unwrap();

    let error = backup_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    assert!(
        matches!(error, SymfoLinkerError::IoError { .. }),
        "expected IoError, got {error:?}"
    );
    // No raw OS struct reaches the GUI: the message carries a path and a reason.
    let message = error.message();
    assert!(message.params.contains_key("path"));
    assert!(message.params.contains_key("detail"));
    // The package is still where it was; nothing was half-moved.
    assert_eq!(marker_contents(&vendor_path(&project, PACKAGE)), ORIGINAL);
    assert!(!backup_path(&project, PACKAGE).exists());
    drop(held);
}

/// On Unix a read-only parent directory is what blocks the rename.
#[cfg(unix)]
#[test]
fn surfaces_an_io_failure_rather_than_a_raw_os_error() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");
    let (lock, guard) = (workspace.lock(), workspace.guard());
    let namespace = vendor_path(&project, PACKAGE)
        .parent()
        .unwrap()
        .to_path_buf();
    fs::set_permissions(&namespace, fs::Permissions::from_mode(0o555)).unwrap();

    let error = backup_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    fs::set_permissions(&namespace, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(error, SymfoLinkerError::IoError { .. }),
        "expected IoError, got {error:?}"
    );
    let message = error.message();
    assert!(message.params.contains_key("path"));
    assert!(message.params.contains_key("detail"));
    assert_eq!(marker_contents(&vendor_path(&project, PACKAGE)), ORIGINAL);
}

#[test]
fn a_second_lock_on_the_same_root_is_refused() {
    let workspace = Workspace::new();
    let _held = workspace.lock();

    let error = RootLock::acquire(workspace.root()).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::LockUnavailable {
            path: workspace.root().to_string_lossy().into_owned()
        }
    );
}

#[test]
fn the_lock_is_released_when_dropped() {
    let workspace = Workspace::new();
    drop(workspace.lock());

    assert!(
        RootLock::acquire(workspace.root()).is_ok(),
        "a released lock must be reacquirable"
    );
}

#[test]
fn the_lock_lives_above_the_projects_not_inside_one() {
    let workspace = Workspace::new();
    let project = workspace.project_with_vendor("app");

    let lock = workspace.lock();

    // Hard rule 2.2: no .symfolinker directory at an individual project root.
    assert_eq!(
        lock.path(),
        workspace.root().join(".symfolinker").join("lock")
    );
    assert!(!project.join(".symfolinker").exists());
}
