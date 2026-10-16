use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::backup::{backup_path, history_root, vendor_path};
use symfolinker_lib::discovery::scan;
use symfolinker_lib::journal::{self, BackupRecord, SwapState};
use symfolinker_lib::links::{inspect_link, remove_link, LinkState};
use symfolinker_lib::lock::RootLock;
use symfolinker_lib::models::Project;
use symfolinker_lib::write_guard::WriteGuard;
use symfolinker_lib::{relink, swap};

const MARKER: &str = "composer.json";

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-relink-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    /// An `app` requiring every named package, with each one installed in `vendor/`
    /// and present as a sibling checkout - the shape every switch starts from.
    fn app_with(&self, packages: &[&str]) -> PathBuf {
        let requires: Vec<String> = packages
            .iter()
            .map(|name| format!("\"acme/{name}\":\"^1.0\""))
            .collect();
        let app = self.project(
            "app",
            &format!(
                "{{\"name\":\"acme/app\",\"require\":{{{}}}}}",
                requires.join(",")
            ),
        );
        for name in packages {
            self.project(
                name,
                &format!("{{\"name\":\"acme/{name}\",\"version\":\"1.0.0\"}}"),
            );
            install(&app, name, "composer");
        }
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

    fn app(&self) -> Project {
        scan(&self.0)
            .unwrap()
            .projects
            .into_iter()
            .find(|project| project.id == "app")
            .unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes a Composer directory at the vendor path, as an install would.
fn install(app: &Path, name: &str, content: &str) {
    let vendor = vendor_path(app, &format!("acme/{name}"));
    fs::create_dir_all(&vendor).unwrap();
    fs::write(vendor.join(MARKER), content).unwrap();
}

#[test]
fn a_composer_reinstall_over_a_link_is_reported_as_relinkable() {
    let workspace = Workspace::new();
    let app = workspace.app_with(&["bundle"]);
    let guard = workspace.guard();
    // Recorded as linked, while a Composer directory occupies the vendor path: the
    // state a reinstall leaves behind.
    journal::record(
        &guard,
        &app,
        BackupRecord::new("acme/bundle", &app, Some("1.0.0".into()), SwapState::Linked),
    )
    .unwrap();

    let package = &workspace.app().packages[0];
    assert_eq!(package.mode, "vendor");
    assert_eq!(package.backup_status, "lost");
    assert!(package.relinkable);
}

#[test]
fn a_package_nobody_linked_is_not_relinkable() {
    let workspace = Workspace::new();
    workspace.app_with(&["bundle"]);
    let package = &workspace.app().packages[0];
    assert_eq!(package.mode, "vendor");
    assert_eq!(package.backup_status, "missing");
    assert!(!package.relinkable);
}

#[test]
fn an_active_link_is_not_relinkable() {
    let workspace = Workspace::new();
    let app = workspace.app_with(&["bundle"]);
    let lock = workspace.lock();
    let guard = workspace.guard();
    swap::activate_local(
        &lock,
        &guard,
        &app,
        "acme/bundle",
        &workspace.0.join("bundle"),
    )
    .unwrap();
    drop(lock);

    let package = &workspace.app().packages[0];
    assert_eq!(package.mode, "local");
    assert!(!package.relinkable);
}

/// The repair this feature exists for, for every package at once: two links replaced
/// by one `composer install`, restored in a single pass.
#[test]
fn restores_every_replaced_link_in_one_pass() {
    let workspace = Workspace::new();
    let app = workspace.app_with(&["bundle", "tools"]);
    let lock = workspace.lock();
    let guard = workspace.guard();
    for name in ["bundle", "tools"] {
        swap::activate_local(
            &lock,
            &guard,
            &app,
            &format!("acme/{name}"),
            &workspace.0.join(name),
        )
        .unwrap();
    }
    drop(lock);
    for name in ["bundle", "tools"] {
        remove_link(&guard, &app, &vendor_path(&app, &format!("acme/{name}"))).unwrap();
        install(&app, name, "reinstalled");
    }

    let before = workspace.app();
    assert!(before.packages.iter().all(|package| package.relinkable));
    assert_eq!(
        relink::pending(&workspace.0, None).unwrap().len(),
        2,
        "both packages are waiting to be relinked"
    );

    relink::apply(&workspace.0, None).unwrap();

    let after = workspace.app();
    for (package, name) in after.packages.iter().zip(["bundle", "tools"]) {
        assert_eq!(package.mode, "local", "{name}");
        assert!(!package.relinkable, "{name}");
        assert_eq!(
            inspect_link(
                &vendor_path(&app, &package.package_name),
                &workspace.0.join(name)
            ),
            LinkState::Linked
        );
        // The reinstalled version is now the preserved one, and the version it
        // replaced is kept in history rather than overwritten.
        assert_eq!(
            fs::read_to_string(backup_path(&app, &package.package_name).join(MARKER)).unwrap(),
            "reinstalled"
        );
        let archives: Vec<_> = fs::read_dir(history_root(&app, &package.package_name))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .collect();
        assert_eq!(archives.len(), 1, "{name} keeps its earlier version");
        assert_eq!(
            fs::read_to_string(archives[0].path().join(MARKER)).unwrap(),
            "composer"
        );
    }
}

/// The interface offers this next to a count, and a count can be stale by the time it
/// is clicked. Nothing to do has to read as success, not as a failed switch.
#[test]
fn relinking_nothing_reports_the_current_state() {
    let workspace = Workspace::new();
    workspace.app_with(&["bundle"]);
    let result = relink::apply(&workspace.0, None).unwrap();
    assert_eq!(result.projects.len(), 2);
    assert_eq!(result.projects[0].packages[0].mode, "vendor");
}

#[test]
fn one_project_can_be_relinked_without_touching_the_others() {
    let workspace = Workspace::new();
    let app = workspace.app_with(&["bundle"]);
    let other = workspace.project(
        "other",
        r#"{"name":"acme/other","require":{"acme/bundle":"^1.0"}}"#,
    );
    install(&other, "bundle", "composer");
    let lock = workspace.lock();
    let guard = workspace.guard();
    for project in [&app, &other] {
        swap::activate_local(
            &lock,
            &guard,
            project,
            "acme/bundle",
            &workspace.0.join("bundle"),
        )
        .unwrap();
    }
    drop(lock);
    for project in [&app, &other] {
        remove_link(&guard, project, &vendor_path(project, "acme/bundle")).unwrap();
        install(project, "bundle", "reinstalled");
    }

    assert_eq!(relink::pending(&workspace.0, Some("app")).unwrap().len(), 1);
    relink::apply(&workspace.0, Some("app")).unwrap();

    let result = scan(&workspace.0).unwrap();
    let mode = |id: &str| {
        result
            .projects
            .iter()
            .find(|project| project.id == id)
            .unwrap()
            .packages[0]
            .mode
    };
    assert_eq!(mode("app"), "local");
    assert_eq!(mode("other"), "vendor", "the other project was left alone");
}
