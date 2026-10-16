use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::autoload::{installed_autoload, status, DIFFERS, MATCHES, UNKNOWN};
use symfolinker_lib::discovery::scan;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-autoload-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn project(&self, name: &str, manifest: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("composer.json"), manifest).unwrap();
        path
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Composer treats `src/` and `src` as one directory, so a trailing slash must never
/// produce a warning the user cannot act on.
#[test]
fn ignores_differences_that_mean_the_same_thing() {
    let installed = json!({"psr-4": {"Acme\\Bundle\\": "src/"}, "files": ["bootstrap.php"]});
    let local = json!({"files": ["./bootstrap.php"], "psr-4": {"Acme\\Bundle\\": "src"}});
    assert_eq!(status(Some(&installed), Some(&local)), MATCHES);

    let without_files = json!({"psr-4": {"Acme\\Bundle\\": "src"}, "classmap": []});
    assert_eq!(status(Some(&installed), Some(&without_files)), DIFFERS);
    let same_with_empty =
        json!({"psr-4": {"Acme\\Bundle\\": "src"}, "files": ["bootstrap.php"], "classmap": []});
    assert_eq!(status(Some(&installed), Some(&same_with_empty)), MATCHES);

    // One path or a list of one path are the same rule.
    let single = json!({"psr-4": {"Acme\\Bundle\\": "src"}});
    let listed = json!({"psr-4": {"Acme\\Bundle\\": ["src/"]}});
    assert_eq!(status(Some(&single), Some(&listed)), MATCHES);
}

/// The failure this check is for: a checkout that moved or renamed its sources while
/// the generated autoloader still points at the old rules.
#[test]
fn reports_rules_the_generated_autoloader_does_not_have() {
    let installed = json!({"psr-4": {"Acme\\Bundle\\": "src"}});
    assert_eq!(
        status(
            Some(&installed),
            Some(&json!({"psr-4": {"Acme\\Bundle\\": "lib"}}))
        ),
        DIFFERS
    );
    assert_eq!(
        status(
            Some(&installed),
            Some(&json!({"psr-4": {"Acme\\Renamed\\": "src"}}))
        ),
        DIFFERS
    );
    assert_eq!(
        status(
            Some(&installed),
            Some(&json!({"psr-4": {"Acme\\Bundle\\": "src"}, "files": ["boot.php"]}))
        ),
        DIFFERS
    );
}

#[test]
fn a_missing_side_is_unknown_rather_than_a_difference() {
    let autoload = json!({"psr-4": {"Acme\\Bundle\\": "src"}});
    assert_eq!(status(None, Some(&autoload)), UNKNOWN);
    assert_eq!(status(Some(&autoload), None), UNKNOWN);
    assert_eq!(status(None, None), UNKNOWN);
}

#[test]
fn reads_the_autoload_of_one_package_from_both_composer_layouts() {
    let autoload = json!({"psr-4": {"Acme\\Bundle\\": "src"}});
    let composer_two = json!({"packages": [
        {"name": "other/thing", "autoload": {"psr-4": {"Other\\": "src"}}},
        {"name": "acme/bundle", "autoload": autoload},
    ]});
    assert_eq!(
        installed_autoload(&composer_two, "acme/bundle").as_ref(),
        Some(&autoload)
    );
    let composer_one = json!([{"name": "acme/bundle", "autoload": autoload}]);
    assert_eq!(
        installed_autoload(&composer_one, "acme/bundle").as_ref(),
        Some(&autoload)
    );
    assert_eq!(installed_autoload(&composer_two, "acme/missing"), None);
}

/// End to end: the scanner has to pair the right two files, the consumer's generated
/// autoloader and the checkout's own manifest.
#[test]
fn the_scan_compares_the_checkout_with_the_generated_autoloader() {
    let root = Workspace::new();
    let app = root.project(
        "app",
        r#"{"name":"acme/app","require":{"acme/bundle":"^1.0","acme/tools":"^1.0"}}"#,
    );
    root.project(
        "bundle",
        r#"{"name":"acme/bundle","version":"1.2.0","autoload":{"psr-4":{"Acme\\Bundle\\":"lib/"}}}"#,
    );
    root.project(
        "tools",
        r#"{"name":"acme/tools","version":"1.0.0","autoload":{"psr-4":{"Acme\\Tools\\":"src/"}}}"#,
    );
    for package in ["bundle", "tools"] {
        fs::create_dir_all(app.join("vendor/acme").join(package)).unwrap();
    }
    fs::create_dir_all(app.join("vendor/composer")).unwrap();
    fs::write(
        app.join("vendor/composer/installed.json"),
        r#"{"packages":[
            {"name":"acme/bundle","version":"1.0.0","autoload":{"psr-4":{"Acme\\Bundle\\":"src/"}}},
            {"name":"acme/tools","version":"1.0.0","autoload":{"psr-4":{"Acme\\Tools\\":"src"}}}
        ]}"#,
    )
    .unwrap();

    let result = scan(&root.0).unwrap();
    let app = result
        .projects
        .iter()
        .find(|project| project.id == "app")
        .unwrap();
    let bundle = &app.packages[0];
    let tools = &app.packages[1];
    assert_eq!(bundle.package_name, "acme/bundle");
    // The checkout autoloads from lib/, the generated autoloader looks in src/.
    assert_eq!(bundle.autoload_status, DIFFERS);
    assert_eq!(bundle.local_version.as_deref(), Some("1.2.0"));
    assert_eq!(bundle.constraint_status, "satisfied");
    assert_eq!(tools.autoload_status, MATCHES);
}

#[test]
fn a_project_without_an_installed_autoloader_is_not_judged() {
    let root = Workspace::new();
    let app = root.project("app", r#"{"require":{"acme/bundle":"^1.0"}}"#);
    root.project(
        "bundle",
        r#"{"name":"acme/bundle","autoload":{"psr-4":{"Acme\\Bundle\\":"src"}}}"#,
    );
    fs::create_dir_all(app.join("vendor/acme/bundle")).unwrap();

    let result = scan(&root.0).unwrap();
    let app = result
        .projects
        .iter()
        .find(|project| project.id == "app")
        .unwrap();
    assert_eq!(app.packages[0].autoload_status, UNKNOWN);
    assert_eq!(app.packages[0].local_version, None);
    assert_eq!(app.packages[0].constraint_status, UNKNOWN);
}
