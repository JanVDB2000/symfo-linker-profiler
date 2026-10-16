use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::backup::vendor_path;
use symfolinker_lib::errors::SymfoLinkerError;
use symfolinker_lib::lock::RootLock;
use symfolinker_lib::write_guard::WriteGuard;
use symfolinker_lib::{preflight, swap};

const PACKAGE: &str = "acme/bundle";
const MARKER: &str = "composer.json";

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-preflight-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn project(&self, name: &str, manifest: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(MARKER), manifest).unwrap();
        path
    }

    /// An `app` with the package installed, plus its sibling checkout.
    fn installed(&self) -> PathBuf {
        let app = self.project(
            "app",
            r#"{"name":"acme/app","require":{"acme/bundle":"^1.0"}}"#,
        );
        self.project("bundle", r#"{"name":"acme/bundle","version":"1.0.0"}"#);
        let vendor = vendor_path(&app, PACKAGE);
        fs::create_dir_all(&vendor).unwrap();
        fs::write(vendor.join(MARKER), r#"{"name":"acme/bundle"}"#).unwrap();
        app
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn leftovers(project: &Path) -> Vec<String> {
    let preserved = project.join("vendor").join(".symfolinker");
    fs::read_dir(&preserved)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(".preflight"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn reports_a_writable_project_and_the_link_kind_it_would_get() {
    let workspace = Workspace::new();
    let app = workspace.installed();

    let report = preflight::check(&workspace.0, "app").unwrap();

    assert!(report.root_writable);
    assert!(report.vendor_writable);
    // Whichever this platform grants is correct; that it is known is the point.
    assert!(matches!(
        report.link_kind,
        Some("symlink") | Some("junction")
    ));
    assert!(report.message.is_none());
    assert!(report.free_bytes.is_some_and(|bytes| bytes > 0));
    assert_eq!(report.preserved_bytes, 0, "nothing is preserved yet");
    assert!(
        leftovers(&app).is_empty(),
        "a check that leaves files behind is litter, not a check"
    );
}

/// The probes may answer a question, but they must not quietly install a vendor
/// directory in a project that has never had one.
#[test]
fn a_project_without_a_vendor_directory_is_reported_as_not_switchable() {
    let workspace = Workspace::new();
    let app = workspace.project(
        "app",
        r#"{"name":"acme/app","require":{"acme/bundle":"^1.0"}}"#,
    );
    workspace.project("bundle", r#"{"name":"acme/bundle"}"#);

    let report = preflight::check(&workspace.0, "app").unwrap();

    assert!(report.root_writable);
    assert!(!report.vendor_writable);
    assert_eq!(report.link_kind, None);
    assert!(report.message.is_some(), "a red cross needs a reason");
    assert_eq!(report.preserved_bytes, 0);
    assert!(!app.join("vendor").exists(), "nothing was created");
}

#[test]
fn counts_what_is_preserved_so_the_free_space_has_a_scale() {
    let workspace = Workspace::new();
    let app = workspace.installed();
    let lock = RootLock::acquire(&workspace.0).unwrap();
    let guard = WriteGuard::new(workspace.0.clone());
    swap::activate_local(&lock, &guard, &app, PACKAGE, &workspace.0.join("bundle")).unwrap();
    drop(lock);

    let report = preflight::check(&workspace.0, "app").unwrap();

    assert!(report.vendor_writable);
    assert!(
        report.preserved_bytes > 0,
        "the preserved Composer version occupies space"
    );
    assert!(leftovers(&app).is_empty());
}

#[test]
fn an_unknown_project_is_refused() {
    let workspace = Workspace::new();
    workspace.installed();
    let error = preflight::check(&workspace.0, "nope").unwrap_err();
    assert_eq!(
        error,
        SymfoLinkerError::ProjectNotFound {
            id: "nope".to_owned()
        }
    );
}

/// The probes write, so they take the development root's lock like any other write.
#[test]
fn checking_waits_for_a_switch_rather_than_writing_beside_it() {
    let workspace = Workspace::new();
    workspace.installed();
    let held = RootLock::acquire(&workspace.0).unwrap();
    let error = preflight::check(&workspace.0, "app").unwrap_err();
    assert!(matches!(error, SymfoLinkerError::LockUnavailable { .. }));
    drop(held);
    assert!(preflight::check(&workspace.0, "app").is_ok());
}
