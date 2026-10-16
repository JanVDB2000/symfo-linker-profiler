use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    sync::mpsc,
    time::{Duration, Instant},
};
use symfolinker_lib::discovery::scan;
use symfolinker_lib::models::Project;
use symfolinker_lib::watch::{classify, strongest, targets, watch, Change};

/// Short enough to keep the test quick, long enough to still coalesce a burst.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Generous: a watcher's first events can take a moment to arrive on a loaded machine.
const DEADLINE: Duration = Duration::from_secs(10);

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-watch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// An application requiring one local bundle, with the directories a real project
    /// has: a repository, a generated autoloader and a vendor namespace.
    fn app(&self) -> Project {
        let app = self.0.join("apps/app");
        fs::create_dir_all(app.join(".git")).unwrap();
        fs::create_dir_all(app.join("vendor/composer")).unwrap();
        fs::create_dir_all(app.join("vendor/acme")).unwrap();
        fs::write(app.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            app.join("composer.json"),
            r#"{"name":"acme/app","require":{"acme/bundle":"*"}}"#,
        )
        .unwrap();
        let bundle = self.0.join("library/bundle");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("composer.json"), r#"{"name":"acme/bundle"}"#).unwrap();
        scan(&self.0)
            .unwrap()
            .projects
            .into_iter()
            .find(|project| project.id == "apps/app")
            .unwrap()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn watches_git_the_autoloader_and_each_mapped_package_but_nothing_else() {
    let workspace = Workspace::new();
    let project = workspace.app();
    let watched = targets(&project);
    let ends_with = |suffix: &str| {
        watched.iter().any(|target| {
            target
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with(suffix)
        })
    };
    assert!(ends_with("apps/app/.git"), "{watched:?}");
    assert!(ends_with("apps/app/vendor/composer"), "{watched:?}");
    assert!(ends_with("apps/app/vendor/acme"), "{watched:?}");
    assert_eq!(watched.len(), 3, "{watched:?}");
    // Only directories that exist are watched, so nothing has to be created first.
    assert!(watched.iter().all(|target| target.is_dir()));
}

#[test]
fn a_vendor_change_outranks_a_branch_change_in_the_same_burst() {
    assert_eq!(
        classify(Path::new("D:/dev/app/.git/HEAD")),
        Some(Change::Branch)
    );
    assert_eq!(
        classify(Path::new("D:/dev/app/vendor/composer/installed.json")),
        Some(Change::Vendor)
    );
    assert_eq!(classify(Path::new("D:/dev/app/src/Kernel.php")), None);
    // A burst holding both: the scan a vendor change needs refreshes Git as well.
    assert_eq!(
        strongest(&[
            PathBuf::from("D:/dev/app/.git/index"),
            PathBuf::from("D:/dev/app/vendor/acme/bundle"),
        ]),
        Some(Change::Vendor)
    );
    assert_eq!(strongest(&[PathBuf::from("D:/dev/app/README.md")]), None);
}

/// Writes to `path` until a change arrives or the deadline passes. A watcher may need
/// a moment before its first event, and one write is not worth failing a test over.
fn await_change(changes: &mpsc::Receiver<Change>, path: &Path, contents: &str) -> Option<Change> {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        fs::write(path, contents).unwrap();
        if let Ok(change) = changes.recv_timeout(Duration::from_millis(400)) {
            return Some(change);
        }
    }
    None
}

#[test]
fn reports_a_composer_install_and_a_branch_change_once_each() {
    let workspace = Workspace::new();
    let project = workspace.app();
    let app = PathBuf::from(&project.path);
    let (sender, changes) = mpsc::channel();
    let watch = watch(&project, DEBOUNCE, move |change| {
        let _ = sender.send(change);
    })
    .expect("the workspace directories exist, so a watch must be possible");

    let installed = app.join("vendor/composer/installed.json");
    assert_eq!(
        await_change(&changes, &installed, r#"{"packages":[]}"#),
        Some(Change::Vendor)
    );

    // Drain what the previous burst still produces, so the next assertion is about
    // the change it names rather than about timing.
    while changes.recv_timeout(Duration::from_millis(600)).is_ok() {}
    assert_eq!(
        await_change(
            &changes,
            &app.join(".git/HEAD"),
            "ref: refs/heads/feature\n"
        ),
        Some(Change::Branch)
    );

    // Dropping the watch ends it: nothing arrives afterwards.
    drop(watch);
    while changes.recv_timeout(Duration::from_millis(600)).is_ok() {}
    fs::write(&installed, r#"{"packages":[{"name":"acme/bundle"}]}"#).unwrap();
    assert!(changes.recv_timeout(Duration::from_secs(2)).is_err());
}
