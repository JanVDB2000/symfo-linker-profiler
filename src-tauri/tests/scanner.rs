use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::discovery::{scan, valid_package_name};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
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
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn maps_require_and_dev_without_writing_project_files() {
    let root = Workspace::new();
    let manifest =
        r#"{"require":{"acme/bundle":"^1","php":"^8"},"require-dev":{"acme/tools":"*"}}"#;
    let app = root.project("app", manifest);
    root.project("bundle", r#"{"name":"acme/bundle"}"#);
    root.project("tools", r#"{"name":"acme/tools"}"#);
    fs::create_dir_all(app.join("vendor/acme/bundle")).unwrap();
    fs::write(app.join("vendor/acme/bundle/original.txt"), "original").unwrap();
    let result = scan(&root.0).unwrap();
    let packages = &result.projects[0].packages;
    assert_eq!(result.projects.len(), 3);
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].mode, "vendor");
    assert_eq!(packages[1].dependency_type, "requireDev");
    assert_eq!(packages[1].link_status, "missing");
    assert_eq!(
        fs::read_to_string(app.join("composer.json")).unwrap(),
        manifest
    );
    assert_eq!(
        fs::read_to_string(app.join("vendor/acme/bundle/original.txt")).unwrap(),
        "original"
    );
    assert!(!root.0.join(".symfolinker").exists());
    assert!(!app.join("vendor/.symfolinker").exists());
}

#[test]
fn scans_only_direct_children_and_reports_invalid_manifests() {
    let root = Workspace::new();
    root.project("good", "{}");
    root.project("broken", "{invalid");
    root.project(".hidden", "{}");
    root.project("nested/deep", "{}");
    let result = scan(&root.0).unwrap();
    assert_eq!(result.projects.len(), 1);
    assert_eq!(result.projects[0].id, "good");
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.warnings[0].params["name"], "broken");
    assert_eq!(
        result.warnings[0].key,
        "{name}: composer.json is invalid or unreadable; project skipped."
    );
}

#[test]
fn rejects_ambiguous_package_mapping() {
    let root = Workspace::new();
    root.project("app", r#"{"require":{"acme/bundle":"*"}}"#);
    root.project("bundle-a", r#"{"name":"acme/bundle"}"#);
    root.project("bundle-b", r#"{"name":"acme/bundle"}"#);
    let result = scan(&root.0).unwrap();
    assert!(result.projects[0].packages.is_empty());
    assert_eq!(
        result.warnings[0].key,
        "{name}: multiple local projects found; automatic mapping skipped."
    );
    assert_eq!(result.warnings[0].params["name"], "acme/bundle");
}

#[test]
fn rejects_traversal_and_platform_requirements_as_package_names() {
    for name in [
        "../bundle",
        "acme/../bundle",
        "acme\\bundle",
        "/bundle",
        "acme/",
        "acme/..",
        "acme/bundle.",
        "C:/bundle",
        "php",
        "ext-json",
        "Acme/bundle",
    ] {
        assert!(!valid_package_name(name), "accepted {name}");
    }
    assert!(valid_package_name("acme/my_bundle-2"));
}

#[test]
fn reports_an_unrecorded_backup_directory_and_an_invalid_vendor_file() {
    let root = Workspace::new();
    let app = root.project("app", r#"{"require":{"acme/bundle":"*"}}"#);
    root.project("bundle", r#"{"name":"acme/bundle"}"#);
    fs::create_dir_all(app.join("vendor/.symfolinker/acme/bundle")).unwrap();
    fs::create_dir_all(app.join("vendor/acme")).unwrap();
    fs::write(app.join("vendor/acme/bundle"), "not a directory").unwrap();
    let result = scan(&root.0).unwrap();
    let package = &result.projects[0].packages[0];
    assert_eq!(package.mode, "unknown");
    assert_eq!(package.link_status, "invalid");
    // A directory SymfoLinker never recorded: placed by hand, or left behind by a
    // version that predates the journal. Restorable, but not something to vouch for.
    assert_eq!(package.backup_status, "unrecognized");
}

#[test]
fn root_must_be_an_existing_directory() {
    let root = Workspace::new();
    assert!(scan(&root.0.join("missing")).is_err());
    fs::write(root.0.join("file"), "").unwrap();
    assert!(scan(&root.0.join("file")).is_err());
}

#[cfg(unix)]
fn symlink_directory(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}
#[cfg(windows)]
fn symlink_directory(target: &Path, link: &Path) {
    // Fail clearly if Windows Developer Mode / symlink privileges are unavailable.
    std::os::windows::fs::symlink_dir(target, link)
        .expect("Enable Windows Developer Mode to run symlink tests");
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Requires Windows Developer Mode or symlink privilege; run with --include-ignored"
)]
fn detects_expected_unexpected_and_broken_links() {
    let root = Workspace::new();
    let app = root.project("app", r#"{"require":{"acme/bundle":"*"}}"#);
    let local = root.project("bundle", r#"{"name":"acme/bundle"}"#);
    let other = root.project("other", "{}");
    fs::create_dir_all(app.join("vendor/acme")).unwrap();
    let vendor = app.join("vendor/acme/bundle");
    symlink_directory(Path::new("../../../bundle"), &vendor);
    assert_eq!(scan(&root.0).unwrap().projects[0].packages[0].mode, "local");
    remove_link(&vendor);
    symlink_directory(&other, &vendor);
    assert_eq!(
        scan(&root.0).unwrap().projects[0].packages[0].link_status,
        "unexpectedTarget"
    );
    remove_link(&vendor);
    symlink_directory(&local.join("missing"), &vendor);
    assert_eq!(
        scan(&root.0).unwrap().projects[0].packages[0].link_status,
        "broken"
    );
}

#[cfg(windows)]
fn remove_link(path: &Path) {
    fs::remove_dir(path).unwrap();
}
#[cfg(unix)]
fn remove_link(path: &Path) {
    fs::remove_file(path).unwrap();
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Requires Windows Developer Mode or symlink privilege; run with --include-ignored"
)]
fn refuses_linked_vendor_ancestors_and_ignores_linked_projects() {
    let root = Workspace::new();
    let outside = Workspace::new();
    let app = root.project("app", r#"{"require":{"acme/bundle":"*"}}"#);
    let bundle = root.project("bundle", r#"{"name":"acme/bundle"}"#);
    fs::create_dir_all(outside.0.join("acme/bundle")).unwrap();
    symlink_directory(&outside.0, &app.join("vendor"));
    symlink_directory(&bundle, &root.0.join("alias"));
    let result = scan(&root.0).unwrap();
    assert_eq!(result.projects.len(), 2);
    assert_eq!(result.projects[0].packages[0].link_status, "invalid");
}

#[cfg(windows)]
fn junction(target: &Path, link: &Path) {
    let output = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link.to_string_lossy().replace('/', "\\"))
        .arg(target.to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
#[test]
fn detects_junctions_and_rejects_linked_ancestors_without_symlink_privileges() {
    let root = Workspace::new();
    let app = root.project("app", r#"{"require":{"acme/bundle":"*"}}"#);
    let bundle = root.project("bundle", r#"{"name":"acme/bundle"}"#);
    let other = root.project("other", "{}");
    fs::create_dir_all(app.join("vendor/acme")).unwrap();
    let vendor = app.join("vendor/acme/bundle");
    junction(&bundle, &vendor);
    assert_eq!(scan(&root.0).unwrap().projects[0].packages[0].mode, "local");
    remove_link(&vendor);
    junction(&other, &vendor);
    assert_eq!(
        scan(&root.0).unwrap().projects[0].packages[0].link_status,
        "unexpectedTarget"
    );
    remove_link(&vendor);
    junction(&bundle, &root.0.join("alias"));
    assert_eq!(scan(&root.0).unwrap().projects.len(), 3);
    // Replace the empty namespace with a junction to test ancestor escapes.
    fs::remove_dir(app.join("vendor/acme")).unwrap();
    junction(&other, &app.join("vendor/acme"));
    assert_eq!(
        scan(&root.0).unwrap().projects[0].packages[0].link_status,
        "invalid"
    );
}

// Project a links acme/b and acme/d, while d links acme/c in turn. Every project is
// inspected on its own path, so multiple links in one project and links inside a project
// that is itself a link target must all resolve independently.
#[test]
#[cfg_attr(
    windows,
    ignore = "Requires Windows Developer Mode or symlink privilege; run with --include-ignored"
)]
fn supports_multiple_links_per_project_and_chained_projects() {
    let root = Workspace::new();
    let a = root.project(
        "a",
        r#"{"name":"acme/a","require":{"acme/b":"*","acme/d":"*"}}"#,
    );
    root.project("b", r#"{"name":"acme/b"}"#);
    root.project("c", r#"{"name":"acme/c"}"#);
    let d = root.project("d", r#"{"name":"acme/d","require":{"acme/c":"*"}}"#);
    fs::create_dir_all(a.join("vendor/acme")).unwrap();
    fs::create_dir_all(d.join("vendor/acme")).unwrap();
    symlink_directory(Path::new("../../../b"), &a.join("vendor/acme/b"));
    symlink_directory(Path::new("../../../d"), &a.join("vendor/acme/d"));
    symlink_directory(Path::new("../../../c"), &d.join("vendor/acme/c"));
    let result = scan(&root.0).unwrap();
    let links: Vec<_> = result
        .projects
        .iter()
        .flat_map(|project| {
            project.packages.iter().map(move |package| {
                (
                    project.id.as_str(),
                    package.package_name.as_str(),
                    package.local_project_id.as_str(),
                    package.mode,
                    package.link_status,
                )
            })
        })
        .collect();
    assert_eq!(
        links,
        [
            ("a", "acme/b", "b", "local", "linked"),
            ("a", "acme/d", "d", "local", "linked"),
            ("d", "acme/c", "c", "local", "linked"),
        ]
    );
    assert!(result.warnings.is_empty());
}

#[cfg(windows)]
#[test]
fn supports_chained_junction_links_without_symlink_privileges() {
    let root = Workspace::new();
    let a = root.project(
        "a",
        r#"{"name":"acme/a","require":{"acme/b":"*","acme/d":"*"}}"#,
    );
    let b = root.project("b", r#"{"name":"acme/b"}"#);
    let c = root.project("c", r#"{"name":"acme/c"}"#);
    let d = root.project("d", r#"{"name":"acme/d","require":{"acme/c":"*"}}"#);
    fs::create_dir_all(a.join("vendor/acme")).unwrap();
    fs::create_dir_all(d.join("vendor/acme")).unwrap();
    junction(&b, &a.join("vendor/acme/b"));
    junction(&d, &a.join("vendor/acme/d"));
    junction(&c, &d.join("vendor/acme/c"));
    let result = scan(&root.0).unwrap();
    let a_packages = &result.projects[0].packages;
    assert_eq!(a_packages.len(), 2);
    assert!(a_packages
        .iter()
        .all(|package| package.mode == "local" && package.link_status == "linked"));
    assert_eq!(a_packages[1].local_project_id, "d");
    let d_packages = &result.projects[3].packages;
    assert_eq!(d_packages.len(), 1);
    assert_eq!(d_packages[0].local_project_id, "c");
    assert_eq!(d_packages[0].mode, "local");
    assert!(result.warnings.is_empty());
}

#[test]
fn git_status_counts_files_and_reads_branch() {
    let root = Workspace::new();
    let app = root.project("app", "{}");
    let output = Command::new("git")
        .args(["init", "-b", "main"])
        .arg(&app)
        .output()
        .expect("Git required for scanner tests");
    assert!(output.status.success());
    fs::write(app.join("untracked.txt"), "local edit").unwrap();
    let result = scan(&root.0).unwrap();
    let git = result.projects[0].git.as_ref().unwrap();
    assert_eq!(git.branch, "main");
    assert!(git.dirty);
    assert_eq!(git.changed_files, 2);
}
