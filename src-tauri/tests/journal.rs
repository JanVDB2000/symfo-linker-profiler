use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::backup::{backup_path, backup_status, vendor_path};
use symfolinker_lib::discovery::scan;
use symfolinker_lib::journal::{self, BackupRecord, SwapState};
use symfolinker_lib::lock::RootLock;
use symfolinker_lib::swap::{activate_local, activate_vendor};
use symfolinker_lib::write_guard::WriteGuard;

const PACKAGE: &str = "acme/bundle";
const MARKER: &str = "composer.json";
const INSTALLED: &str = r#"{"packages":[{"name":"acme/bundle","version":"1.4.2"}]}"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-journal-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    /// A consumer with the Composer package installed, and the local source next to it.
    fn app_and_local(&self) -> (PathBuf, PathBuf) {
        let project = self.0.join("app");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join(MARKER),
            r#"{"require":{"acme/bundle":"^1.4"}}"#,
        )
        .unwrap();

        let vendor = vendor_path(&project, PACKAGE);
        fs::create_dir_all(&vendor).unwrap();
        fs::write(vendor.join(MARKER), r#"{"name":"acme/bundle"}"#).unwrap();

        let composer = project.join("vendor").join("composer");
        fs::create_dir_all(&composer).unwrap();
        fs::write(composer.join("installed.json"), INSTALLED).unwrap();

        let local = self.0.join("bundle");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join(MARKER), r#"{"name":"acme/bundle"}"#).unwrap();
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

#[test]
fn a_swap_records_what_was_preserved_and_where_it_came_from() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());

    activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();

    let record = journal::read(&project).remove(PACKAGE).unwrap();
    assert_eq!(record.package, PACKAGE);
    assert_eq!(record.project, project.to_string_lossy());
    // From vendor/composer/installed.json, read before the directory was moved.
    assert_eq!(record.version.as_deref(), Some("1.4.2"));
    assert_eq!(record.state, SwapState::Linked);
    assert!(record.updated_at > 0);
}

#[test]
fn switching_back_clears_the_record_together_with_the_backup_it_described() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());

    activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();
    activate_vendor(&lock, &guard, &project, PACKAGE).unwrap();

    assert!(journal::read(&project).is_empty());
    assert!(!backup_path(&project, PACKAGE).exists());
}

#[test]
fn a_recorded_swap_reads_as_available_and_an_unrecorded_directory_does_not() {
    let workspace = Workspace::new();
    let (project, local) = workspace.app_and_local();
    let (lock, guard) = (workspace.lock(), workspace.guard());
    let backup = backup_path(&project, PACKAGE);

    activate_local(&lock, &guard, &project, PACKAGE, &local).unwrap();
    let recorded = journal::read(&project);

    assert_eq!(backup_status(&backup, recorded.get(PACKAGE)), "available");
    // The same directory without a record: an older version, or placed there by hand.
    assert_eq!(backup_status(&backup, None), "unrecognized");
}

#[test]
fn a_record_left_in_a_transient_state_reports_an_interrupted_swap() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let backup = backup_path(&project, PACKAGE);
    fs::create_dir_all(&backup).unwrap();
    let absent = project.join("vendor").join(".symfolinker").join("absent");

    for state in [SwapState::BackingUp, SwapState::Restoring] {
        let record = BackupRecord::new(PACKAGE, &project, None, state);
        // Preserved directory present or not: a transient state means the operation
        // never finished, and that is what the user has to be told either way.
        assert_eq!(backup_status(&backup, Some(&record)), "interrupted");
        assert_eq!(backup_status(&absent, Some(&record)), "interrupted");
    }
}

#[test]
fn a_backup_that_was_recorded_and_then_vanished_is_reported_as_lost() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let record = BackupRecord::new(PACKAGE, &project, None, SwapState::Linked);

    let status = backup_status(&backup_path(&project, PACKAGE), Some(&record));

    // Not "missing": SymfoLinker preserved something here and it is gone, which is a
    // different problem from a package that was never swapped at all.
    assert_eq!(status, "lost");
}

#[test]
fn one_record_per_package_leaves_the_others_untouched() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let guard = workspace.guard();
    let other = "acme/tools";

    journal::record(
        &guard,
        &project,
        BackupRecord::new(
            PACKAGE,
            &project,
            Some("1.0.0".to_owned()),
            SwapState::Linked,
        ),
    )
    .unwrap();
    journal::record(
        &guard,
        &project,
        BackupRecord::new(other, &project, None, SwapState::BackingUp),
    )
    .unwrap();
    journal::forget(&guard, &project, other).unwrap();

    let records = journal::read(&project);
    assert_eq!(records.len(), 1);
    assert_eq!(records[PACKAGE].version.as_deref(), Some("1.0.0"));
    // Forgetting a package that was never recorded is not an error.
    journal::forget(&guard, &project, "acme/absent").unwrap();
}

#[test]
fn an_unreadable_or_newer_journal_reads_as_no_records_and_still_scans() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let path = journal::journal_path(&project);
    fs::create_dir_all(path.parent().unwrap()).unwrap();

    for content in [
        "not json at all",
        r#"{"version":99,"packages":{"acme/bundle":{"package":"acme/bundle","project":"x","state":"linked","updatedAt":1}}}"#,
    ] {
        fs::write(&path, content).unwrap();
        assert!(journal::read(&project).is_empty(), "{content}");
        // Provenance that cannot be read must never stop a workspace from scanning.
        assert!(scan(&workspace.0).is_ok(), "{content}");
    }
}

#[test]
fn the_package_manifest_supplies_the_version_when_composer_recorded_none() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let vendor = vendor_path(&project, PACKAGE);
    fs::remove_file(
        project
            .join("vendor")
            .join("composer")
            .join("installed.json"),
    )
    .unwrap();
    fs::write(
        vendor.join(MARKER),
        r#"{"name":"acme/bundle","version":"2.0.0"}"#,
    )
    .unwrap();

    let version = journal::installed_version(&project, PACKAGE, &vendor);

    assert_eq!(version.as_deref(), Some("2.0.0"));
}

#[test]
fn a_package_nobody_installed_has_no_version_rather_than_a_wrong_one() {
    let workspace = Workspace::new();
    let (project, _local) = workspace.app_and_local();
    let vendor = vendor_path(&project, "acme/other");

    assert_eq!(
        journal::installed_version(&project, "acme/other", &vendor),
        None
    );
}
