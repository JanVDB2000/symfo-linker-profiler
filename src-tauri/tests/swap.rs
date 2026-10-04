use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::backup::{backup_path, vendor_path};
use symfolinker_lib::errors::SymfoLinkerError;
use symfolinker_lib::links::{inspect_link, relative_target, LinkKind, LinkState};
use symfolinker_lib::lock::RootLock;
use symfolinker_lib::swap::{activate_local, activate_vendor};
use symfolinker_lib::write_guard::WriteGuard;

const PACKAGE: &str = "acme/bundle";
const MARKER: &str = "composer.json";
const COMPOSER_VERSION: &str = r#"{"name":"acme/bundle","source":"composer"}"#;
const LOCAL_VERSION: &str = r#"{"name":"acme/bundle","source":"local"}"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-swap-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    /// `app` with the Composer package installed, plus a sibling local `bundle`.
    fn app_and_local(&self) -> (PathBuf, PathBuf) {
        let project = self.0.join("app");
        let vendor = vendor_path(&project, PACKAGE);
        fs::create_dir_all(&vendor).unwrap();
        fs::write(vendor.join(MARKER), COMPOSER_VERSION).unwrap();

        let local = self.0.join("bundle");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join(MARKER), LOCAL_VERSION).unwrap();
        (project, local)
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

fn marker(directory: &Path) -> String {
    fs::read_to_string(directory.join(MARKER)).unwrap()
}

#[test]
fn calculates_a_relative_target_that_climbs_out_of_vendor() {
    let root = Path::new("/dev");
    let link = root.join("app").join("vendor").join("acme").join("bundle");

    let relative = relative_target(&link, &root.join("bundle")).unwrap();

    // Plan section 23: from vendor/acme/ up three levels, then into the project.
    assert_eq!(
        relative,
        Path::new("..").join("..").join("..").join("bundle")
    );
}

#[test]
fn a_deeper_local_path_climbs_the_same_number_of_levels() {
    let link = Path::new("/dev/app/vendor/acme/bundle");

    let relative = relative_target(link, Path::new("/dev/other/nested")).unwrap();

    assert_eq!(
        relative,
        Path::new("..")
            .join("..")
            .join("..")
            .join("other")
            .join("nested")
    );
}

#[test]
fn vendor_to_local_serves_the_local_files_and_preserves_the_composer_version() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());

    activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();

    let vendor = vendor_path(&project, PACKAGE);
    // Reading through the link must now yield the local project's file.
    assert_eq!(marker(&vendor), LOCAL_VERSION);
    assert_eq!(inspect_link(&vendor, &local), LinkState::Linked);
    // Hard rule 2.3: the Composer directory is preserved, not deleted.
    assert_eq!(marker(&backup_path(&project, PACKAGE)), COMPOSER_VERSION);
}

#[test]
fn local_to_vendor_restores_the_composer_version_and_leaves_the_local_project_alone() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());
    activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();

    activate_vendor(&lock, &guard, &project, PACKAGE).unwrap();

    let vendor = vendor_path(&project, PACKAGE);
    assert_eq!(marker(&vendor), COMPOSER_VERSION);
    assert_eq!(inspect_link(&vendor, &local), LinkState::NotLinked);
    assert!(!backup_path(&project, PACKAGE).exists());
    // Removing the link must never reach through it.
    assert_eq!(marker(&local), LOCAL_VERSION);
}

#[test]
fn a_full_round_trip_can_be_repeated() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());

    for _ in 0..3 {
        activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();
        activate_vendor(&lock, &guard, &project, PACKAGE).unwrap();
    }

    assert_eq!(marker(&vendor_path(&project, PACKAGE)), COMPOSER_VERSION);
    assert_eq!(marker(&local), LOCAL_VERSION);
}

#[test]
fn a_missing_local_project_changes_nothing() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());
    let absent = workspace.0.join("never-created");

    let error = activate_local(&lock, &guard, &project, PACKAGE, &absent).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::LocalProjectNotFound {
            path: absent.to_string_lossy().into_owned()
        }
    );
    // Critical test case: no changes at all, so the backup was never even made.
    assert_eq!(marker(&vendor_path(&project, PACKAGE)), COMPOSER_VERSION);
    assert!(!backup_path(&project, PACKAGE).exists());
}

#[test]
fn activating_vendor_on_a_real_directory_is_refused_rather_than_deleting_it() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());
    let vendor = vendor_path(&project, PACKAGE);

    let error = activate_vendor(&lock, &guard, &project, PACKAGE).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::UnexpectedSymlink {
            path: vendor.to_string_lossy().into_owned()
        }
    );
    assert_eq!(marker(&vendor), COMPOSER_VERSION);
}

#[test]
fn a_link_to_the_wrong_project_is_reported_as_an_unexpected_target() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());
    let decoy = workspace.0.join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    activate_local(&lock, &guard, &project, PACKAGE, &decoy).unwrap();

    let state = inspect_link(&vendor_path(&project, PACKAGE), &local);

    assert_eq!(
        state,
        LinkState::UnexpectedTarget {
            resolved: decoy.to_string_lossy().into_owned()
        }
    );
}

#[test]
fn a_link_whose_target_disappeared_is_reported_as_broken() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());
    activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();

    fs::remove_dir_all(&local).unwrap();

    assert_eq!(
        inspect_link(&vendor_path(&project, PACKAGE), &local),
        LinkState::Broken
    );
}

#[test]
fn a_broken_link_can_still_be_switched_back_to_the_composer_version() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());
    activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();
    fs::remove_dir_all(&local).unwrap();

    activate_vendor(&lock, &guard, &project, PACKAGE).unwrap();

    // Recovery must not depend on the local project still being there.
    assert_eq!(marker(&vendor_path(&project, PACKAGE)), COMPOSER_VERSION);
}

#[test]
fn an_empty_vendor_path_reports_missing() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    fs::remove_dir_all(vendor_path(&project, PACKAGE)).unwrap();

    assert_eq!(
        inspect_link(&vendor_path(&project, PACKAGE), &local),
        LinkState::Missing
    );
}

#[test]
fn reports_which_kind_of_link_was_created() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());

    let kind = activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();

    // Unix always gets a relative symlink. Windows gets one too under Developer Mode,
    // and otherwise falls back to a junction - which must be reported, not hidden,
    // because a junction stores an absolute target (plan section 24).
    #[cfg(not(windows))]
    assert_eq!(kind, LinkKind::RelativeSymlink);
    #[cfg(windows)]
    assert!(matches!(
        kind,
        LinkKind::RelativeSymlink | LinkKind::Junction
    ));

    // Whichever kind it is, the link must resolve to the local project.
    assert_eq!(
        inspect_link(&vendor_path(&project, PACKAGE), &local),
        LinkState::Linked
    );
}

/// Proves the fallback itself works, so a machine without symlink privileges is not
/// silently left unable to link. Skipped where symlinks are available.
#[cfg(windows)]
#[test]
fn falls_back_to_a_junction_when_symlinks_are_not_permitted() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());

    let probe = workspace.0.join("probe-link");
    let symlinks_allowed = std::os::windows::fs::symlink_dir(&local, &probe).is_ok();
    if symlinks_allowed {
        let _ = fs::remove_dir(&probe);
        return;
    }

    let kind = activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();

    assert_eq!(kind, LinkKind::Junction);
    assert_eq!(marker(&vendor_path(&project, PACKAGE)), LOCAL_VERSION);
}
