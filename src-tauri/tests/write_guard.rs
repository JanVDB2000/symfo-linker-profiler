use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::errors::SymfoLinkerError;
use symfolinker_lib::write_guard::WriteGuard;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-guard-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn project(&self, name: &str) -> PathBuf {
        let project = self.0.join(name);
        fs::create_dir_all(project.join("vendor")).unwrap();
        project
    }

    fn guard(&self) -> WriteGuard {
        WriteGuard::new(self.0.clone())
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_denied(result: Result<(), SymfoLinkerError>, target: &Path) {
    assert_eq!(
        result,
        Err(SymfoLinkerError::WriteDenied {
            path: target.to_string_lossy().into_owned()
        })
    );
}

#[test]
fn allows_the_vendor_package_path() {
    let root = Root::new();
    let project = root.project("app");
    let target = project.join("vendor").join("acme").join("bundle");

    assert_eq!(root.guard().assert_vendor_path(&project, &target), Ok(()));
}

#[test]
fn allows_the_backup_path_inside_vendor() {
    let root = Root::new();
    let project = root.project("app");
    let target = project
        .join("vendor")
        .join(".symfolinker")
        .join("acme")
        .join("bundle");

    assert_eq!(root.guard().assert_vendor_path(&project, &target), Ok(()));
}

#[test]
fn denies_project_files_outside_vendor() {
    let root = Root::new();
    let project = root.project("app");
    let guard = root.guard();

    // Plan section 22 lists exactly these as forbidden.
    for name in ["composer.json", "src/Service.php", ".git/config"] {
        let target = project.join(name);
        assert_denied(guard.assert_vendor_path(&project, &target), &target);
    }
}

#[test]
fn denies_the_vendor_directory_itself() {
    let root = Root::new();
    let project = root.project("app");
    let target = project.join("vendor");

    // Renaming or deleting vendor/ wholesale would take every package with it.
    assert_denied(root.guard().assert_vendor_path(&project, &target), &target);
}

#[test]
fn denies_a_path_that_climbs_out_with_parent_components() {
    let root = Root::new();
    let project = root.project("app");
    let target = project
        .join("vendor")
        .join("acme")
        .join("..")
        .join("..")
        .join("composer.json");

    assert_denied(root.guard().assert_vendor_path(&project, &target), &target);
}

#[test]
fn denies_a_target_in_another_project() {
    let root = Root::new();
    let project = root.project("app");
    let other = root.project("bundle");
    let target = other.join("vendor").join("acme").join("bundle");

    assert_denied(root.guard().assert_vendor_path(&project, &target), &target);
}

#[test]
fn denies_a_project_outside_the_development_root() {
    let root = Root::new();
    let outside = std::env::temp_dir().join("symfolinker-guard-outside");
    let target = outside.join("vendor").join("acme").join("bundle");

    assert_denied(root.guard().assert_vendor_path(&outside, &target), &target);
}

#[test]
fn denies_the_development_root_itself_as_a_project() {
    let root = Root::new();
    let target = root.0.join("vendor").join("acme").join("bundle");

    assert_denied(root.guard().assert_vendor_path(&root.0, &target), &target);
}

#[test]
fn allows_a_backup_path_whose_directories_do_not_exist_yet() {
    let root = Root::new();
    let project = root.project("app");
    // The first backup is written before vendor/.symfolinker exists.
    let target = project
        .join("vendor")
        .join(".symfolinker")
        .join("never")
        .join("created");

    assert_eq!(root.guard().assert_vendor_path(&project, &target), Ok(()));
}

#[test]
fn denies_a_target_behind_a_file_where_a_directory_belongs() {
    let root = Root::new();
    let project = root.project("app");
    // vendor/acme is a file, so vendor/acme/bundle can never be a safe write target.
    fs::write(project.join("vendor").join("acme"), "not a directory").unwrap();
    let target = project.join("vendor").join("acme").join("bundle");

    assert_denied(root.guard().assert_vendor_path(&project, &target), &target);
}
