use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::backup::{backup_path, history_root, vendor_path};
use symfolinker_lib::errors::SymfoLinkerError;
use symfolinker_lib::journal;
use symfolinker_lib::links::{create_link, inspect_link, remove_link, LinkState};
use symfolinker_lib::lock::RootLock;
use symfolinker_lib::write_guard::WriteGuard;
use symfolinker_lib::{history, swap};

const PACKAGE: &str = "acme/bundle";
const MARKER: &str = "composer.json";

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-history-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    /// An `app` with `acme/bundle` installed at version 1.0.0, plus its checkout.
    fn app(&self) -> PathBuf {
        let app = self.project(
            "app",
            r#"{"name":"acme/app","require":{"acme/bundle":"^1.0"}}"#,
        );
        self.project("bundle", r#"{"name":"acme/bundle","version":"1.0.0"}"#);
        install(&app, "1.0.0");
        app
    }

    fn project(&self, name: &str, manifest: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(MARKER), manifest).unwrap();
        path
    }

    fn lock(&self) -> RootLock {
        RootLock::acquire(&self.0).unwrap()
    }

    fn guard(&self) -> WriteGuard {
        WriteGuard::new(self.0.clone())
    }

    /// Links the package, which preserves whatever Composer installed.
    fn link(&self, app: &Path) {
        let lock = self.lock();
        swap::activate_local(&lock, &self.guard(), app, PACKAGE, &self.0.join("bundle")).unwrap();
    }

    /// A linked package replaced by a freshly installed one, then linked again: the
    /// only way a second preserved version comes into being.
    fn reinstall_and_link(&self, app: &Path, version: &str) {
        remove_link(&self.guard(), app, &vendor_path(app, PACKAGE)).unwrap();
        install(app, version);
        self.link(app);
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn install(app: &Path, version: &str) {
    let vendor = vendor_path(app, PACKAGE);
    fs::create_dir_all(&vendor).unwrap();
    fs::write(
        vendor.join(MARKER),
        format!("{{\"name\":\"acme/bundle\",\"version\":\"{version}\"}}"),
    )
    .unwrap();
}

fn installed_version(path: &Path) -> String {
    let manifest = fs::read_to_string(path.join(MARKER)).unwrap();
    serde_json::from_str::<serde_json::Value>(&manifest).unwrap()["version"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn lists_what_is_preserved_with_what_it_occupies() {
    let workspace = Workspace::new();
    let app = workspace.app();
    workspace.link(&app);
    workspace.reinstall_and_link(&app, "2.0.0");

    let entries = history::list(&workspace.0, "app").unwrap();
    assert_eq!(entries.len(), 1, "the sidecar is not an entry of its own");
    let entry = &entries[0];
    assert_eq!(entry.package_name, PACKAGE);
    assert!(entry.directory);
    assert!(
        entry.recognized,
        "its sidecar names this package and project"
    );
    assert_eq!(entry.files, 1);
    assert!(entry.size_bytes > 0);
    assert!(entry.archived_at > 0);
    assert_eq!(entry.version.as_deref(), Some("1.0.0"));
    assert_eq!(installed_version(Path::new(&entry.path)), "1.0.0");
    // The newest version took the backup's place rather than overwriting the older one.
    assert_eq!(installed_version(&backup_path(&app, PACKAGE)), "2.0.0");
}

#[test]
fn restoring_exchanges_the_preserved_version_without_losing_either() {
    let workspace = Workspace::new();
    let app = workspace.app();
    workspace.link(&app);
    workspace.reinstall_and_link(&app, "2.0.0");
    let archived = history::list(&workspace.0, "app").unwrap().remove(0);

    let result = history::restore(&workspace.0, "app", PACKAGE, &archived.archive_id).unwrap();

    assert_eq!(
        installed_version(&backup_path(&app, PACKAGE)),
        "1.0.0",
        "the chosen version is the one a switch to VENDOR will restore"
    );
    let left = history::list(&workspace.0, "app").unwrap();
    assert_eq!(
        left.len(),
        1,
        "the version it replaced is preserved in turn"
    );
    assert_eq!(installed_version(Path::new(&left[0].path)), "2.0.0");
    assert!(left[0].recognized);
    // The link is untouched: restoring changes what is preserved, not what is in use.
    assert_eq!(
        inspect_link(&vendor_path(&app, PACKAGE), &workspace.0.join("bundle")),
        LinkState::Linked
    );
    assert_eq!(result.projects[0].packages[0].mode, "local");
    assert_eq!(result.projects[0].packages[0].backup_status, "available");
    // What the journal says is restorable has to be this version.
    assert_eq!(
        journal::read(&app).get(PACKAGE).unwrap().version.as_deref(),
        Some("1.0.0")
    );
    assert!(
        !Path::new(&archived.path).with_extension("json").exists(),
        "the restored archive's sidecar does not outlive it"
    );
}

/// On VENDOR the Composer directory is in use and the backup slot is not, so
/// exchanging it would change nothing visible while the record started to lie.
#[test]
fn restoring_is_refused_while_the_package_is_on_vendor() {
    let workspace = Workspace::new();
    let app = workspace.app();
    workspace.link(&app);
    workspace.reinstall_and_link(&app, "2.0.0");
    let archived = history::list(&workspace.0, "app").unwrap().remove(0);
    let lock = workspace.lock();
    swap::activate_vendor(&lock, &workspace.guard(), &app, PACKAGE).unwrap();
    drop(lock);

    let error = history::restore(&workspace.0, "app", PACKAGE, &archived.archive_id).unwrap_err();
    assert_eq!(
        error,
        SymfoLinkerError::UnexpectedSymlink {
            path: vendor_path(&app, PACKAGE).to_string_lossy().into_owned()
        }
    );
    assert!(Path::new(&archived.path).exists(), "nothing was moved");
}

#[test]
fn removing_one_archive_leaves_the_package_and_its_backup_alone() {
    let workspace = Workspace::new();
    let app = workspace.app();
    workspace.link(&app);
    workspace.reinstall_and_link(&app, "2.0.0");
    let archived = history::list(&workspace.0, "app").unwrap().remove(0);

    let left = history::delete(&workspace.0, "app", PACKAGE, &archived.archive_id).unwrap();

    assert!(left.is_empty());
    assert!(!Path::new(&archived.path).exists());
    assert!(
        !Path::new(&archived.path).with_extension("json").exists(),
        "the sidecar goes with the archive it described"
    );
    assert_eq!(installed_version(&backup_path(&app, PACKAGE)), "2.0.0");
    assert_eq!(
        inspect_link(&vendor_path(&app, PACKAGE), &workspace.0.join("bundle")),
        LinkState::Linked
    );
}

#[test]
fn an_unknown_archive_is_refused_rather_than_turned_into_a_path() {
    let workspace = Workspace::new();
    let app = workspace.app();
    workspace.link(&app);
    workspace.reinstall_and_link(&app, "2.0.0");
    let archived = history::list(&workspace.0, "app").unwrap().remove(0);

    for (package, id) in [
        (PACKAGE, "../../../escape"),
        (PACKAGE, "does-not-exist"),
        ("acme/other", archived.archive_id.as_str()),
    ] {
        let error = history::delete(&workspace.0, "app", package, id).unwrap_err();
        assert!(matches!(error, SymfoLinkerError::BackupMissing { .. }));
    }
    assert!(Path::new(&archived.path).exists());
}

/// Anything may have been moved out of the way, including a stray file. It is listed
/// so it can be removed, and refused for restoring, because it is not a package.
#[test]
fn a_stray_entry_is_listed_and_removable_but_never_restorable() {
    let workspace = Workspace::new();
    let app = workspace.app();
    workspace.link(&app);
    let history = history_root(&app, PACKAGE);
    fs::create_dir_all(&history).unwrap();
    fs::write(history.join("stray"), "not a package").unwrap();

    let entries = history::list(&workspace.0, "app").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].archive_id, "stray");
    assert!(!entries[0].directory);
    assert!(!entries[0].recognized);
    assert_eq!(entries[0].size_bytes, 0);

    let error = history::restore(&workspace.0, "app", PACKAGE, "stray").unwrap_err();
    assert!(matches!(error, SymfoLinkerError::BackupInvalid { .. }));
    assert!(history::delete(&workspace.0, "app", PACKAGE, "stray")
        .unwrap()
        .is_empty());
    assert!(!history.join("stray").exists());
}

/// Reading must not be more generous than writing. A link where a package directory
/// belongs resolves outside the project, and walking it would report the sizes and
/// paths of someone else's tree back to the interface.
#[test]
fn a_link_where_a_package_directory_belongs_is_not_walked() {
    let workspace = Workspace::new();
    let app = workspace.app();
    workspace.link(&app);
    workspace.reinstall_and_link(&app, "2.0.0");
    // A tree outside the project, shaped exactly like the history layout.
    let outside = workspace.0.join("elsewhere");
    fs::create_dir_all(outside.join("bundle")).unwrap();
    fs::write(outside.join("bundle/secret.txt"), "not ours").unwrap();
    let planted = history_root(&app, PACKAGE)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("acme-linked");
    create_link(&workspace.guard(), &app, &planted, &outside).unwrap();

    let entries = history::list(&workspace.0, "app").unwrap();

    assert_eq!(entries.len(), 1, "only the real archive is listed");
    assert_eq!(entries[0].package_name, PACKAGE);
    assert!(
        !entries.iter().any(|entry| entry.path.contains("elsewhere")),
        "nothing outside the project is reported"
    );
    assert!(
        outside.join("bundle/secret.txt").exists(),
        "nothing was touched"
    );
}

#[test]
fn a_project_without_history_reports_nothing() {
    let workspace = Workspace::new();
    workspace.app();
    assert!(history::list(&workspace.0, "app").unwrap().is_empty());
    let error = history::list(&workspace.0, "nope").unwrap_err();
    assert_eq!(
        error,
        SymfoLinkerError::ProjectNotFound {
            id: "nope".to_owned()
        }
    );
}
