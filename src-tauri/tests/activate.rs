use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::activate::{activate_local_package, activate_vendor_package};
use symfolinker_lib::errors::SymfoLinkerError;
use symfolinker_lib::models::ScanResult;

const PACKAGE: &str = "acme/bundle";

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    /// A root the scanner can map: `app` requires `acme/bundle`, `bundle` provides it.
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "symfolinker-activate-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();

        let app = root.join("app");
        fs::create_dir_all(&app).unwrap();
        fs::write(
            app.join("composer.json"),
            r#"{"name":"acme/app","require":{"acme/bundle":"^1.0"}}"#,
        )
        .unwrap();
        let vendor = app.join("vendor").join("acme").join("bundle");
        fs::create_dir_all(&vendor).unwrap();
        fs::write(vendor.join("marker.txt"), "composer").unwrap();

        let bundle = root.join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("composer.json"), r#"{"name":"acme/bundle"}"#).unwrap();
        fs::write(bundle.join("marker.txt"), "local").unwrap();

        Self(root)
    }

    fn marker(&self) -> String {
        fs::read_to_string(
            self.0
                .join("app")
                .join("vendor")
                .join("acme")
                .join("bundle")
                .join("marker.txt"),
        )
        .unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mode(result: &ScanResult) -> &'static str {
    result
        .projects
        .iter()
        .find(|project| project.id == "app")
        .unwrap()
        .packages
        .iter()
        .find(|package| package.package_name == PACKAGE)
        .unwrap()
        .mode
}

#[test]
fn activating_local_returns_the_state_read_back_from_disk() {
    let workspace = Workspace::new();

    let result = activate_local_package(&workspace.0, "app", PACKAGE).unwrap();

    // Plan section 48: the interface is fed the scanned result, never a predicted one.
    assert_eq!(mode(&result), "local");
    assert_eq!(workspace.marker(), "local");
}

#[test]
fn activating_vendor_returns_the_state_read_back_from_disk() {
    let workspace = Workspace::new();
    activate_local_package(&workspace.0, "app", PACKAGE).unwrap();

    let result = activate_vendor_package(&workspace.0, "app", PACKAGE).unwrap();

    assert_eq!(mode(&result), "vendor");
    assert_eq!(workspace.marker(), "composer");
}

#[test]
fn an_unknown_project_id_is_refused() {
    let workspace = Workspace::new();

    let error = activate_local_package(&workspace.0, "ghost", PACKAGE).unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::ProjectNotFound {
            id: "ghost".to_owned()
        }
    );
    assert_eq!(workspace.marker(), "composer");
}

#[test]
fn a_package_the_project_does_not_require_is_refused() {
    let workspace = Workspace::new();

    let error = activate_local_package(&workspace.0, "app", "acme/other").unwrap_err();

    assert_eq!(
        error,
        SymfoLinkerError::PackageNotFound {
            name: "acme/other".to_owned()
        }
    );
    assert_eq!(workspace.marker(), "composer");
}

#[test]
fn the_lock_is_released_so_a_second_switch_can_follow() {
    let workspace = Workspace::new();

    // Each call acquires and releases the development-root lock on its own.
    activate_local_package(&workspace.0, "app", PACKAGE).unwrap();
    activate_vendor_package(&workspace.0, "app", PACKAGE).unwrap();
    activate_local_package(&workspace.0, "app", PACKAGE).unwrap();

    assert_eq!(workspace.marker(), "local");
}
