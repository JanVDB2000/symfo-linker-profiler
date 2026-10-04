use crate::compose::{parse_compose_config, suggest_php_service, to_container_path, VolumeMapping};
use crate::config;
use crate::container_mounts::{mount_plan, override_json};
use crate::discovery::compose_file;
use crate::models::{ContainerCheck, ContainerReport, Message, Project};
use crate::runtime::container_output;
use std::path::Path;

pub struct RunningContext {
    pub project_name: String,
    pub files: Vec<String>,
    pub environment_files: Vec<String>,
    pub volumes: Vec<VolumeMapping>,
}

pub fn parse_running_context(
    stdout: &str,
    project: &Path,
    service: &str,
) -> Option<RunningContext> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let container = value.as_array()?.first()?;
    let labels = &container["Config"]["Labels"];
    if labels["com.docker.compose.service"].as_str()? != service {
        return None;
    }
    if service.is_empty()
        || service == "."
        || service == ".."
        || !service
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
    {
        return None;
    }
    let working_dir = labels["com.docker.compose.project.working_dir"].as_str()?;
    if Path::new(working_dir).canonicalize().ok()? != project.canonicalize().ok()? {
        return None;
    }
    let project_name = labels["com.docker.compose.project"].as_str()?.to_owned();
    let files: Vec<String> = labels["com.docker.compose.project.config_files"]
        .as_str()?
        .split(',')
        .map(str::to_owned)
        .collect();
    if files.is_empty()
        || files
            .iter()
            .any(|file| !Path::new(file).is_absolute() || !Path::new(file).is_file())
    {
        return None;
    }
    let environment_files: Vec<String> = labels["com.docker.compose.project.environment_file"]
        .as_str()
        .unwrap_or("")
        .split(',')
        .filter(|file| !file.is_empty())
        .map(str::to_owned)
        .collect();
    if environment_files
        .iter()
        .any(|file| !Path::new(file).is_absolute() || !Path::new(file).is_file())
    {
        return None;
    }
    let volumes = container["Mounts"]
        .as_array()?
        .iter()
        .filter(|mount| mount["Type"] == "bind")
        .filter_map(|mount| {
            Some(VolumeMapping {
                host_path: mount["Source"].as_str()?.to_owned(),
                container_path: mount["Destination"].as_str()?.to_owned(),
            })
        })
        .collect();
    Some(RunningContext {
        project_name,
        files,
        environment_files,
        volumes,
    })
}

fn running_context(
    project: &Path,
    service: &str,
    run: &mut impl FnMut(&Path, &[&str]) -> Result<String, Message>,
) -> Option<RunningContext> {
    let ids = run(project, &["compose", "ps", "-q", service]).ok()?;
    let id = ids.lines().find(|id| !id.trim().is_empty())?.trim();
    let stdout = run(project, &["inspect", id]).ok()?;
    parse_running_context(&stdout, project, service)
}

/// Inspects mounts and validates linked packages inside the container (plan sections
/// 26 to 29).
///
/// Deliberately not part of the five-minute status poll: `docker compose config` plus
/// one `exec` per linked package is far too expensive to run on a timer. The Runtime
/// panel asks for this explicitly instead.
///
/// Docker configuration is only ever read.
pub fn inspect(development_root: &Path, project: &Project) -> ContainerReport {
    inspect_with_output(development_root, project, &mut container_output)
}

pub fn inspect_with_output(
    development_root: &Path,
    project: &Project,
    run: &mut impl FnMut(&Path, &[&str]) -> Result<String, Message>,
) -> ContainerReport {
    let project_path = Path::new(&project.path);

    // Plan section 30: without a compose file the runtime is native and host
    // validation is all that is required.
    if compose_file(project_path).is_none() {
        return ContainerReport::unavailable(Message::new(
            "This project has no Compose file; it runs natively.",
        ));
    }

    let stdout = match run(project_path, &["compose", "config", "--format", "json"]) {
        Ok(stdout) => stdout,
        // Already a translatable message, so it is passed through unchanged.
        Err(message) => return ContainerReport::unavailable(message),
    };

    let services = parse_compose_config(&stdout);
    let names: Vec<String> = services
        .iter()
        .map(|service| service.name.clone())
        .collect();

    let chosen = config::php_service(development_root, &project.path);
    let selected = chosen.clone().or_else(|| suggest_php_service(&services));
    let Some(selected) = selected else {
        return ContainerReport {
            services: names,
            php_service: None,
            suggested: false,
            volumes: Vec::new(),
            project_container_path: None,
            checks: Vec::new(),
            message: Some(Message::new(
                "No PHP service could be identified. Choose one to validate containers.",
            )),
            mount_plan: Vec::new(),
            can_apply_mounts: false,
        };
    };

    let configured_volumes = services
        .iter()
        .find(|service| service.name == selected)
        .map(|service| service.volumes.clone())
        .unwrap_or_default();
    // Inspect the running container's mounts, which can differ from edited Compose
    // files or from an override used when this service was created.
    let context = running_context(project_path, &selected, run);
    let can_apply_mounts = context.is_some();
    let volumes = context
        .map(|context| context.volumes)
        .unwrap_or(configured_volumes);
    let mount_plan = mount_plan(project, &volumes);

    let checks = validate_links(project_path, &selected, &volumes, project, run);

    ContainerReport {
        services: names,
        project_container_path: to_container_path(&volumes, project_path),
        suggested: chosen.is_none(),
        php_service: Some(selected),
        message: volumes.is_empty().then(|| {
            Message::new("This service has no bind mounts, so host paths cannot be mapped.")
        }),
        volumes: volumes
            .iter()
            .map(|volume| (volume.host_path.clone(), volume.container_path.clone()))
            .collect(),
        checks,
        mount_plan,
        can_apply_mounts,
    }
}

/// New mounts require recreating the service. Keep the original Compose files and
/// project name from its labels, add a generated override above the projects, and
/// recreate only the selected PHP service after the UI's explicit confirmation.
pub fn apply_mounts(
    root: &Path,
    project_id: &str,
    service: &str,
) -> Result<ContainerReport, Message> {
    apply_mounts_with_output(
        root,
        project_id,
        service,
        &mut crate::runtime::container_output_with_timeout,
    )
}

pub fn apply_mounts_with_output(
    root: &Path,
    project_id: &str,
    service: &str,
    run: &mut impl FnMut(&Path, &[&str], std::time::Duration) -> Result<String, Message>,
) -> Result<ContainerReport, Message> {
    let root = root
        .canonicalize()
        .map_err(|_| Message::new("The development root does not exist or is not readable."))?;
    let _lock = crate::lock::RootLock::acquire(&root).map_err(|error| error.message())?;
    let scan = crate::discovery::scan(&root)?;
    let project = scan
        .projects
        .iter()
        .find(|project| project.id == project_id)
        .ok_or_else(|| {
            Message::with(
                "Project {id} is no longer in this workspace.",
                "id",
                project_id,
            )
        })?;
    let path = Path::new(&project.path);
    let context = running_context(path, service, &mut |dir, args| run(dir, args, std::time::Duration::from_secs(20))).ok_or_else(|| Message::new("The running service's Compose files could not be verified. No containers were changed."))?;
    let plan = mount_plan(project, &context.volumes);
    if plan.is_empty() {
        return Ok(inspect_with_output(&root, project, &mut |dir, args| {
            run(dir, args, std::time::Duration::from_secs(20))
        }));
    }
    for mount in &plan {
        if context.volumes.iter().any(|volume| {
            volume.container_path == mount.container_path && volume.host_path != mount.host_path
        }) {
            return Err(Message::with(
                "A different source is already mounted at {path}. No containers were changed.",
                "path",
                &mount.container_path,
            ));
        }
        if !Path::new(&mount.host_path).is_dir() {
            return Err(Message::with(
                "The local project {path} no longer exists.",
                "path",
                &mount.host_path,
            ));
        }
    }
    let directory = root.join(".symfolinker/compose");
    if std::fs::symlink_metadata(&directory)
        .is_ok_and(|meta| !meta.is_dir() || crate::discovery::is_link(&meta))
    {
        return Err(Message::new(
            "The mount override directory is unsafe. No containers were changed.",
        ));
    }
    let directory = directory.join(project_id);
    if std::fs::symlink_metadata(&directory)
        .is_ok_and(|meta| !meta.is_dir() || crate::discovery::is_link(&meta))
    {
        return Err(Message::new(
            "The mount override directory is unsafe. No containers were changed.",
        ));
    }
    // One persistent override per project/service. Existing generated mounts are
    // retained when applying additional packages after an earlier recreation.
    let override_path = directory.join(format!("{service}.json"));
    if std::fs::symlink_metadata(&override_path)
        .is_ok_and(|meta| !meta.is_file() || crate::discovery::is_link(&meta))
    {
        return Err(Message::new(
            "The mount override directory is unsafe. No containers were changed.",
        ));
    }
    let mut mounts = plan;
    let unsafe_override =
        || Message::new("The mount override directory is unsafe. No containers were changed.");
    match std::fs::read(&override_path) {
        Ok(bytes) => {
            let previous: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| unsafe_override())?;
            let volumes = previous["services"][service]["volumes"]
                .as_array()
                .ok_or_else(unsafe_override)?;
            for volume in volumes {
                if volume["type"] != "bind" {
                    return Err(unsafe_override());
                }
                let source = volume["source"].as_str().ok_or_else(unsafe_override)?;
                let target = volume["target"].as_str().ok_or_else(unsafe_override)?;
                if !mounts.iter().any(|mount| mount.container_path == target) {
                    mounts.push(crate::models::ContainerMount {
                        package_name: String::new(),
                        host_path: source.to_owned(),
                        container_path: target.to_owned(),
                    });
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(unsafe_override()),
    }
    for mount in &mounts {
        let source = Path::new(&mount.host_path).canonicalize().ok();
        if source
            .as_ref()
            .is_none_or(|source| !source.starts_with(&root) || !source.is_dir())
            || !mount.container_path.starts_with('/')
            || mount.container_path == "/"
            || mount.container_path.split('/').any(|part| part == "..")
        {
            return Err(Message::new(
                "The mount override directory is unsafe. No containers were changed.",
            ));
        }
    }
    let json = serde_json::to_vec_pretty(&override_json(service, &mounts))
        .expect("mount JSON is serializable");
    crate::atomic::write_atomically(&override_path, &json).map_err(|error| error.message())?;
    let mut args = vec![
        "compose".to_owned(),
        "--project-name".to_owned(),
        context.project_name,
        "--project-directory".to_owned(),
        project.path.clone(),
    ];
    for file in context.environment_files {
        args.extend(["--env-file".to_owned(), file]);
    }
    let override_string = override_path.to_string_lossy().into_owned();
    for file in context
        .files
        .iter()
        .filter(|file| *file != &override_string)
    {
        args.extend(["-f".to_owned(), file.clone()]);
    }
    args.extend(["-f".to_owned(), override_string]);
    let mut validation = args.clone();
    validation.extend(["config".to_owned(), "--quiet".to_owned()]);
    run(
        path,
        &validation.iter().map(String::as_str).collect::<Vec<_>>(),
        std::time::Duration::from_secs(20),
    )?;
    args.extend(
        [
            "up",
            "-d",
            "--no-deps",
            "--no-build",
            "--pull",
            "never",
            "--force-recreate",
            service,
        ]
        .iter()
        .map(|value| (*value).to_owned()),
    );
    run(
        path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        std::time::Duration::from_secs(120),
    )?;
    Ok(inspect_with_output(&root, project, &mut |dir, args| {
        run(dir, args, std::time::Duration::from_secs(20))
    }))
}

/// Checks each locally linked package at the path the container actually sees.
fn validate_links(
    project_path: &Path,
    service: &str,
    volumes: &[VolumeMapping],
    project: &Project,
    run: &mut impl FnMut(&Path, &[&str]) -> Result<String, Message>,
) -> Vec<ContainerCheck> {
    project
        .packages
        .iter()
        .filter(|package| package.mode == "local")
        .map(|package| {
            let Some(container_path) = to_container_path(volumes, Path::new(&package.vendor_path))
            else {
                return ContainerCheck {
                    package_name: package.package_name.clone(),
                    container_path: String::new(),
                    exists: false,
                    link_target: None,
                    mapped: false,
                };
            };

            // Arguments are passed directly; no shell, so no quoting to get wrong.
            let exists = run(
                project_path,
                &[
                    "compose",
                    "exec",
                    "-T",
                    service,
                    "test",
                    "-e",
                    &container_path,
                ],
            )
            .is_ok();

            let link_target = run(
                project_path,
                &[
                    "compose",
                    "exec",
                    "-T",
                    service,
                    "readlink",
                    &container_path,
                ],
            )
            .ok()
            .map(|output| output.trim().to_owned())
            .filter(|target| !target.is_empty());

            ContainerCheck {
                package_name: package.package_name.clone(),
                container_path,
                exists,
                link_target,
                mapped: true,
            }
        })
        .collect()
}
