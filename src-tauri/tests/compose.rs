use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::compose::{
    parse_compose_config, suggest_php_service, to_container_path, ComposeServiceConfig,
    VolumeMapping,
};
use symfolinker_lib::config;

const CONFIG_JSON: &str = r#"{
  "services": {
    "php-fpm": {
      "volumes": [
        {"type": "bind", "source": "/home/user/dev", "target": "/var/www"},
        {"type": "volume", "source": "cache", "target": "/cache"}
      ]
    },
    "database": {
      "volumes": [{"type": "volume", "source": "pgdata", "target": "/var/lib/postgresql/data"}]
    }
  }
}"#;

fn mapping(host: &str, container: &str) -> VolumeMapping {
    VolumeMapping {
        host_path: host.to_owned(),
        container_path: container.to_owned(),
    }
}

fn service(name: &str, volumes: Vec<VolumeMapping>) -> ComposeServiceConfig {
    ComposeServiceConfig {
        name: name.to_owned(),
        volumes,
    }
}

#[test]
fn reads_services_and_keeps_only_bind_mounts() {
    let services = parse_compose_config(CONFIG_JSON);

    assert_eq!(services.len(), 2);
    let php = services.iter().find(|s| s.name == "php-fpm").unwrap();
    // A named volume has no host side, so it cannot map a host path.
    assert_eq!(php.volumes, vec![mapping("/home/user/dev", "/var/www")]);
    let database = services.iter().find(|s| s.name == "database").unwrap();
    assert!(database.volumes.is_empty());
}

#[test]
fn unreadable_configuration_yields_no_services_instead_of_failing() {
    assert!(parse_compose_config("not json").is_empty());
    assert!(parse_compose_config("").is_empty());
}

#[test]
fn suggests_the_conventional_php_service_names_in_order() {
    let services = vec![service("web", vec![]), service("php-fpm", vec![])];

    // php-fpm outranks web even though web comes first in the file.
    assert_eq!(suggest_php_service(&services).as_deref(), Some("php-fpm"));
}

#[test]
fn suggests_a_service_whose_name_contains_a_known_one() {
    let services = vec![service("database", vec![]), service("api-php-fpm", vec![])];

    assert_eq!(
        suggest_php_service(&services).as_deref(),
        Some("api-php-fpm")
    );
}

#[test]
fn suggests_nothing_rather_than_guessing_wrong() {
    let services = vec![service("database", vec![]), service("redis", vec![])];

    // Validating against the wrong container would look confident but mean nothing.
    assert_eq!(suggest_php_service(&services), None);
}

#[test]
fn maps_a_host_path_into_the_container() {
    let volumes = vec![mapping("/home/user/dev", "/var/www")];

    let mapped = to_container_path(&volumes, Path::new("/home/user/dev/project-b"));

    // Plan section 28.
    assert_eq!(mapped.as_deref(), Some("/var/www/project-b"));
}

#[test]
fn maps_the_mount_root_itself() {
    let volumes = vec![mapping("/home/user/dev", "/var/www")];

    assert_eq!(
        to_container_path(&volumes, Path::new("/home/user/dev")).as_deref(),
        Some("/var/www")
    );
}

#[test]
fn the_most_specific_mount_wins() {
    let volumes = vec![
        mapping("/home/user/dev", "/var/www"),
        mapping("/home/user/dev/app", "/srv/app"),
    ];

    assert_eq!(
        to_container_path(&volumes, Path::new("/home/user/dev/app/vendor")).as_deref(),
        Some("/srv/app/vendor")
    );
}

#[test]
fn a_path_outside_every_mount_is_not_mapped() {
    let volumes = vec![mapping("/home/user/dev", "/var/www")];

    assert_eq!(to_container_path(&volumes, Path::new("/etc/passwd")), None);
    // A sibling whose name merely starts the same must not match either.
    assert_eq!(
        to_container_path(&volumes, Path::new("/home/user/development")),
        None
    );
}

#[cfg(windows)]
#[test]
fn windows_host_paths_map_despite_separators_and_casing() {
    let volumes = vec![mapping(r"D:\dev", "/var/www")];

    assert_eq!(
        to_container_path(&volumes, Path::new(r"\\?\D:\Dev\project-b\vendor")).as_deref(),
        Some("/var/www/project-b/vendor")
    );
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-config-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn stores_the_php_service_above_the_projects() {
    let root = Root::new();
    let project = root.0.join("app").to_string_lossy().into_owned();

    config::set_php_service(&root.0, &project, Some("php-fpm".to_owned())).unwrap();

    // Hard rule 2.2: app state lives in <root>/.symfolinker, never in a project.
    assert!(root.0.join(".symfolinker").join("config.json").is_file());
    assert!(!Path::new(&project).join(".symfolinker").exists());
    assert_eq!(
        config::php_service(&root.0, &project).as_deref(),
        Some("php-fpm")
    );
}

#[test]
fn storing_one_project_leaves_the_others_untouched() {
    let root = Root::new();
    let first = root.0.join("app").to_string_lossy().into_owned();
    let second = root.0.join("api").to_string_lossy().into_owned();
    config::set_php_service(&root.0, &first, Some("php".to_owned())).unwrap();

    config::set_php_service(&root.0, &second, Some("backend".to_owned())).unwrap();

    assert_eq!(config::php_service(&root.0, &first).as_deref(), Some("php"));
    assert_eq!(
        config::php_service(&root.0, &second).as_deref(),
        Some("backend")
    );
}

#[test]
fn clearing_the_choice_restores_automatic_suggestion() {
    let root = Root::new();
    let project = root.0.join("app").to_string_lossy().into_owned();
    config::set_php_service(&root.0, &project, Some("php".to_owned())).unwrap();

    config::set_php_service(&root.0, &project, None).unwrap();

    assert_eq!(config::php_service(&root.0, &project), None);
}

#[test]
fn a_corrupt_config_reads_as_empty_rather_than_blocking_the_scan() {
    let root = Root::new();
    let directory = root.0.join(".symfolinker");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("config.json"), "{ not json").unwrap();

    assert_eq!(config::load(&root.0), config::AppConfig::default());
}
