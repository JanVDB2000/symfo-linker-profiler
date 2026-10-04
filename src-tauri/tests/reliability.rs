use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::{
    backup, config, journal,
    links::{self, LinkState},
    lock::RootLock,
    profile::{self, ProfileEntry},
    recovery, swap,
    write_guard::WriteGuard,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const PACKAGE: &str = "acme/bundle";
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "symfolinker-reliability-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("bundle")).unwrap();
        fs::write(
            root.join("bundle/composer.json"),
            r#"{"name":"acme/bundle"}"#,
        )
        .unwrap();
        for app in ["app", "shop"] {
            let project = root.join(app);
            let vendor = backup::vendor_path(&project, PACKAGE);
            fs::create_dir_all(&vendor).unwrap();
            fs::write(
                project.join("composer.json"),
                r#"{"require":{"acme/bundle":"*"}}"#,
            )
            .unwrap();
            fs::write(vendor.join("marker"), "composer").unwrap();
        }
        Self(root.canonicalize().unwrap())
    }
    fn entry(project: &str, mode: &str) -> ProfileEntry {
        ProfileEntry {
            project_id: project.into(),
            package_name: PACKAGE.into(),
            mode: mode.into(),
        }
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn composer_reinstall_can_be_linked_again_and_both_vendor_versions_survive() {
    let workspace = Workspace::new();
    let app = workspace.0.join("app");
    let vendor = backup::vendor_path(&app, PACKAGE);
    profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).unwrap();
    // Simulate Composer replacing the link with a newly installed directory.
    {
        let guard = WriteGuard::new(&workspace.0);
        links::remove_link(&guard, &app, &vendor).unwrap();
    }
    fs::create_dir_all(&vendor).unwrap();
    fs::write(vendor.join("marker"), "composer-new").unwrap();
    profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).unwrap();
    assert_eq!(
        fs::read_to_string(backup::backup_path(&app, PACKAGE).join("marker")).unwrap(),
        "composer-new"
    );
    let history = app.join("vendor/.symfolinker/.history/acme/bundle");
    let archived = fs::read_dir(history)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().is_dir())
        .unwrap();
    assert_eq!(
        fs::read_to_string(archived.path().join("marker")).unwrap(),
        "composer"
    );
    profile::apply(&workspace.0, &[Workspace::entry("app", "vendor")]).unwrap();
    assert_eq!(
        fs::read_to_string(vendor.join("marker")).unwrap(),
        "composer-new"
    );
}

#[test]
fn failed_relink_after_composer_reinstall_rolls_back_both_directories() {
    let workspace = Workspace::new();
    let app = workspace.0.join("app");
    let guard = WriteGuard::new(&workspace.0);
    let lock = RootLock::acquire(&workspace.0).unwrap();
    backup::backup_vendor(&lock, &guard, &app, PACKAGE).unwrap();
    let vendor = backup::vendor_path(&app, PACKAGE);
    fs::create_dir_all(&vendor).unwrap();
    fs::write(vendor.join("marker"), "composer-new").unwrap();
    let outcome: Result<(), _> =
        backup::with_current_vendor_backup(&lock, &guard, &app, PACKAGE, |_| {
            Err(symfolinker_lib::errors::SymfoLinkerError::BrokenSymlink {
                path: "simulated link failure".into(),
            })
        });
    assert!(outcome.is_err());
    assert_eq!(
        fs::read_to_string(vendor.join("marker")).unwrap(),
        "composer-new"
    );
    assert_eq!(
        fs::read_to_string(backup::backup_path(&app, PACKAGE).join("marker")).unwrap(),
        "composer"
    );
}

#[test]
fn profile_round_trip_preserves_all_originals() {
    let workspace = Workspace::new();
    let entries = [
        Workspace::entry("app", "local"),
        Workspace::entry("shop", "local"),
    ];
    profile::apply(&workspace.0, &entries).unwrap();
    // Applying twice is idempotent.
    profile::apply(&workspace.0, &entries).unwrap();
    profile::apply(
        &workspace.0,
        &[
            Workspace::entry("app", "vendor"),
            Workspace::entry("shop", "vendor"),
        ],
    )
    .unwrap();
    for app in ["app", "shop"] {
        assert_eq!(
            fs::read_to_string(backup::vendor_path(&workspace.0.join(app), PACKAGE).join("marker"))
                .unwrap(),
            "composer"
        );
    }
}

#[test]
fn invalid_profile_is_refused_before_the_first_change() {
    let workspace = Workspace::new();
    assert!(profile::apply(
        &workspace.0,
        &[
            Workspace::entry("app", "local"),
            Workspace::entry("absent", "local")
        ]
    )
    .is_err());
    assert!(backup::vendor_path(&workspace.0.join("app"), PACKAGE)
        .join("marker")
        .is_file());
    assert!(!backup::backup_path(&workspace.0.join("app"), PACKAGE).exists());
}

#[test]
fn a_later_failure_rolls_back_completed_profile_changes() {
    let workspace = Workspace::new();
    profile::apply(
        &workspace.0,
        &[
            Workspace::entry("app", "local"),
            Workspace::entry("shop", "local"),
        ],
    )
    .unwrap();
    fs::create_dir_all(swap::staged_link_path(&workspace.0.join("shop"), PACKAGE)).unwrap();
    assert!(profile::apply(
        &workspace.0,
        &[
            Workspace::entry("app", "vendor"),
            Workspace::entry("shop", "vendor")
        ]
    )
    .is_err());
    for app in ["app", "shop"] {
        assert_eq!(
            links::inspect_link(
                &backup::vendor_path(&workspace.0.join(app), PACKAGE),
                &workspace.0.join("bundle")
            ),
            LinkState::Linked
        );
        assert!(backup::backup_path(&workspace.0.join(app), PACKAGE)
            .join("marker")
            .is_file());
    }
}

#[test]
fn missing_backup_does_not_remove_a_working_link() {
    let workspace = Workspace::new();
    profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).unwrap();
    let app = workspace.0.join("app");
    fs::rename(
        backup::backup_path(&app, PACKAGE),
        app.join("preserved-elsewhere"),
    )
    .unwrap();
    let lock = RootLock::acquire(&workspace.0).unwrap();
    assert!(swap::activate_vendor(&lock, &WriteGuard::new(&workspace.0), &app, PACKAGE).is_err());
    assert_eq!(
        links::inspect_link(
            &backup::vendor_path(&app, PACKAGE),
            &workspace.0.join("bundle")
        ),
        LinkState::Linked
    );
}

#[test]
fn recovery_restores_an_interrupted_backup_without_the_local_checkout() {
    let workspace = Workspace::new();
    let app = workspace.0.join("app");
    {
        let lock = RootLock::acquire(&workspace.0).unwrap();
        backup::backup_vendor(&lock, &WriteGuard::new(&workspace.0), &app, PACKAGE).unwrap();
    }
    fs::remove_dir_all(workspace.0.join("bundle")).unwrap();
    recovery::recover(&workspace.0, "app").unwrap();
    assert_eq!(
        fs::read_to_string(backup::vendor_path(&app, PACKAGE).join("marker")).unwrap(),
        "composer"
    );
    assert!(journal::read(&app).is_empty());
}

#[test]
fn recovery_completes_a_restore_interrupted_after_staging_the_link() {
    let workspace = Workspace::new();
    profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).unwrap();
    let app = workspace.0.join("app");
    let staged = swap::staged_link_path(&app, PACKAGE);
    fs::create_dir_all(staged.parent().unwrap()).unwrap();
    fs::rename(backup::vendor_path(&app, PACKAGE), &staged).unwrap();
    recovery::recover(&workspace.0, "app").unwrap();
    assert!(backup::vendor_path(&app, PACKAGE).join("marker").is_file());
    assert!(fs::symlink_metadata(staged).is_err());
}

#[test]
fn recovery_restores_a_broken_link_after_its_checkout_disappears() {
    let workspace = Workspace::new();
    profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).unwrap();
    fs::remove_dir_all(workspace.0.join("bundle")).unwrap();
    recovery::recover(&workspace.0, "app").unwrap();
    assert!(backup::vendor_path(&workspace.0.join("app"), PACKAGE)
        .join("marker")
        .is_file());
}

#[test]
fn a_backup_owned_by_another_project_is_refused_before_unlinking() {
    let workspace = Workspace::new();
    profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).unwrap();
    let app = workspace.0.join("app");
    let mut records = journal::read(&app);
    let mut record = records.remove(PACKAGE).unwrap();
    record.project = workspace.0.join("shop").to_string_lossy().into_owned();
    journal::record(&WriteGuard::new(&workspace.0), &app, record).unwrap();
    let lock = RootLock::acquire(&workspace.0).unwrap();
    assert!(swap::activate_vendor(&lock, &WriteGuard::new(&workspace.0), &app, PACKAGE).is_err());
    assert_eq!(
        links::inspect_link(
            &backup::vendor_path(&app, PACKAGE),
            &workspace.0.join("bundle")
        ),
        LinkState::Linked
    );
}

#[test]
fn a_corrupt_journal_is_not_overwritten_by_a_new_swap() {
    let workspace = Workspace::new();
    let app = workspace.0.join("app");
    fs::create_dir_all(app.join("vendor/.symfolinker")).unwrap();
    fs::write(journal::journal_path(&app), "invalid json").unwrap();
    assert!(profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).is_err());
    assert!(backup::vendor_path(&app, PACKAGE).join("marker").is_file());
    assert_eq!(
        fs::read_to_string(journal::journal_path(&app)).unwrap(),
        "invalid json"
    );
}

#[test]
fn configuration_writers_obey_the_root_lock() {
    let workspace = Workspace::new();
    let lock = RootLock::acquire(&workspace.0).unwrap();
    assert!(config::set_php_service(&workspace.0, "app", Some("php".into())).is_err());
    drop(lock);
    config::set_php_service(&workspace.0, "app", Some("php".into())).unwrap();
    config::set_php_service(&workspace.0, "shop", Some("fpm".into())).unwrap();
    assert_eq!(config::php_service(&workspace.0, "app"), Some("php".into()));
}

#[cfg(windows)]
#[test]
fn failed_restore_puts_the_exact_link_back_even_if_broken() {
    use std::os::windows::fs::OpenOptionsExt;
    let workspace = Workspace::new();
    profile::apply(&workspace.0, &[Workspace::entry("app", "local")]).unwrap();
    let app = workspace.0.join("app");
    fs::remove_dir_all(workspace.0.join("bundle")).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(backup::backup_path(&app, PACKAGE).join("marker"))
        .unwrap();
    let lock = RootLock::acquire(&workspace.0).unwrap();
    assert!(swap::activate_vendor(&lock, &WriteGuard::new(&workspace.0), &app, PACKAGE).is_err());
    assert_eq!(
        links::inspect_link(
            &backup::vendor_path(&app, PACKAGE),
            &workspace.0.join("bundle")
        ),
        LinkState::Broken
    );
    drop(held);
    swap::activate_vendor(&lock, &WriteGuard::new(&workspace.0), &app, PACKAGE).unwrap();
}
