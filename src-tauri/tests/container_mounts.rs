use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use symfolinker_lib::{
    compose::VolumeMapping,
    container::{apply_mounts_with_output, parse_running_context},
    container_mounts::{mount_plan, resolve_target},
    discovery::scan,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "symfolinker-mounts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("blue-master/vendor/blue-master/blue-master-bundle")).unwrap();
        fs::create_dir_all(root.join("blue-master-bundle")).unwrap();
        fs::write(
            root.join("blue-master/composer.json"),
            r#"{"require":{"blue-master/blue-master-bundle":"*"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("blue-master-bundle/composer.json"),
            r#"{"name":"blue-master/blue-master-bundle"}"#,
        )
        .unwrap();
        fs::write(root.join("blue-master/compose.yaml"), "services: {}").unwrap();
        fs::write(
            root.join("blue-master/compose.override.yaml"),
            "services: {}",
        )
        .unwrap();
        fs::write(root.join("blue-master/.env.custom"), "MODE=development").unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn context(&self, extra_mount: bool) -> String {
        let app = self.0.join("blue-master");
        let mut mounts =
            vec![serde_json::json!({"Type":"bind","Source":app,"Destination":"/application"})];
        if extra_mount {
            mounts.push(serde_json::json!({"Type":"bind","Source":self.0.join("blue-master-bundle"),"Destination":"/blue-master-bundle"}));
        }
        serde_json::json!([{"Config":{"Labels":{
            "com.docker.compose.service":"blue-master-php-fpm",
            "com.docker.compose.project":"custom-blue-master",
            "com.docker.compose.project.environment_file":app.join(".env.custom"),
            "com.docker.compose.project.working_dir":app,
            "com.docker.compose.project.config_files":format!("{},{}",app.join("compose.yaml").display(),app.join("compose.override.yaml").display())
        }}, "Mounts":mounts}]).to_string()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn application_only_mount_requires_the_sibling_at_the_actual_link_destination() {
    let workspace = Workspace::new();
    let scan = scan(&workspace.0).unwrap();
    let project = scan
        .projects
        .iter()
        .find(|p| p.id == "blue-master")
        .unwrap();
    let volumes = vec![VolumeMapping {
        host_path: project.path.clone(),
        container_path: "/application".into(),
    }];
    let plan = mount_plan(project, &volumes);
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].container_path, "/blue-master-bundle");
    assert_eq!(
        PathBuf::from(&plan[0].host_path),
        workspace.0.join("blue-master-bundle")
    );
}

#[test]
fn mounting_the_whole_workspace_preserves_the_existing_relative_link() {
    let workspace = Workspace::new();
    let scan = scan(&workspace.0).unwrap();
    let project = scan
        .projects
        .iter()
        .find(|p| p.id == "blue-master")
        .unwrap();
    assert!(mount_plan(
        project,
        &[VolumeMapping {
            host_path: workspace.0.to_string_lossy().into(),
            container_path: "/workspace".into()
        }]
    )
    .is_empty());
}

#[test]
fn target_resolution_handles_nested_mounts_and_refuses_the_container_root() {
    assert_eq!(
        resolve_target("/application/vendor/acme", "../../../bundle"),
        Some("/bundle".into())
    );
    assert_eq!(
        resolve_target("/workspace/app/vendor/acme", "../../../bundle"),
        Some("/workspace/bundle".into())
    );
    assert_eq!(resolve_target("/application", "../../etc"), None);
    assert_eq!(resolve_target("/application", "/"), None);
}

#[test]
fn labels_must_match_the_selected_project_and_service() {
    let workspace = Workspace::new();
    assert!(parse_running_context(
        &workspace.context(false),
        &workspace.0.join("blue-master"),
        "blue-master-php-fpm"
    )
    .is_some());
    assert!(parse_running_context(
        &workspace.context(false),
        &workspace.0.join("blue-master"),
        "database"
    )
    .is_none());
    assert!(parse_running_context(
        &workspace.context(false),
        &workspace.0.join("blue-master-bundle"),
        "blue-master-php-fpm"
    )
    .is_none());
}

#[test]
fn applying_mounts_retains_original_files_and_recreates_only_the_selected_service() {
    let workspace = Workspace::new();
    let app = workspace.0.join("blue-master");
    let original = fs::read(app.join("compose.yaml")).unwrap();
    let mut commands = Vec::new();
    let mut recreated = false;
    let report = apply_mounts_with_output(
        &workspace.0,
        "blue-master",
        "blue-master-php-fpm",
        &mut |_, args, timeout| {
            commands.push(args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>());
            if args.first() == Some(&"inspect") {
                return Ok(workspace.context(recreated));
            }
            if args.contains(&"-q") {
                return Ok("container-id".into());
            }
            if args.contains(&"up") {
                assert_eq!(timeout, Duration::from_secs(120));
                recreated = true;
                return Ok(String::new());
            }
            if args == ["compose", "config", "--format", "json"] {
                return Ok(
                    serde_json::json!({"services":{"blue-master-php-fpm":{"volumes":[]}}})
                        .to_string(),
                );
            }
            Ok(String::new())
        },
    )
    .unwrap();
    assert!(report.mount_plan.is_empty());
    let up = commands
        .iter()
        .find(|args| args.contains(&"up".to_owned()))
        .unwrap();
    assert_eq!(up.last().unwrap(), "blue-master-php-fpm");
    assert!(up.contains(&"--no-deps".to_owned()));
    assert!(up.contains(&"--env-file".to_owned()));
    assert!(up.contains(&app.join(".env.custom").to_string_lossy().into_owned()));
    assert!(up.contains(&"custom-blue-master".to_owned()));
    assert!(up.contains(&app.join("compose.yaml").to_string_lossy().into_owned()));
    assert!(up.contains(
        &app.join("compose.override.yaml")
            .to_string_lossy()
            .into_owned()
    ));
    assert_eq!(fs::read(app.join("compose.yaml")).unwrap(), original);
    let json: serde_json::Value = serde_json::from_slice(
        &fs::read(
            workspace
                .0
                .join(".symfolinker/compose/blue-master/blue-master-php-fpm.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        json["services"]["blue-master-php-fpm"]["volumes"][0]["target"],
        "/blue-master-bundle"
    );
    assert_eq!(
        json["services"]["blue-master-php-fpm"]["volumes"][0]["bind"]["create_host_path"],
        false
    );
}

#[test]
fn unverifiable_context_never_recreates_a_container() {
    let workspace = Workspace::new();
    let mut restarted = false;
    assert!(apply_mounts_with_output(
        &workspace.0,
        "blue-master",
        "blue-master-php-fpm",
        &mut |_, args, _| {
            restarted |= args.contains(&"up");
            Ok(String::new())
        }
    )
    .is_err());
    assert!(!restarted);
    assert!(!workspace.0.join(".symfolinker/compose").exists());
}

#[test]
fn an_unreadable_override_is_preserved_without_recreating_the_service() {
    let workspace = Workspace::new();
    let override_path = workspace
        .0
        .join(".symfolinker/compose/blue-master/blue-master-php-fpm.json");
    fs::create_dir_all(override_path.parent().unwrap()).unwrap();
    fs::write(&override_path, b"{broken").unwrap();
    let result = apply_mounts_with_output(
        &workspace.0,
        "blue-master",
        "blue-master-php-fpm",
        &mut |_, args, _| {
            assert!(!args.contains(&"up"));
            if args.first() == Some(&"inspect") {
                Ok(workspace.context(false))
            } else {
                Ok("container-id".into())
            }
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(override_path).unwrap(), b"{broken");
}
