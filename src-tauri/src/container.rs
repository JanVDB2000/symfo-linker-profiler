use crate::compose::{
    parse_compose_config, same_host_path, suggest_php_service, to_container_path, VolumeMapping,
};
use crate::config;
use crate::container_mounts::{mount_plan, override_json};
use crate::discovery::compose_file;
use crate::models::{ContainerCheck, ContainerMount, ContainerReport, Message, MountSync, Project};
use crate::runtime::container_output;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Reading Compose configuration or a container's labels is a local call; the twenty
/// seconds match the runtime module's own Compose timeout.
const COMPOSE_TIMEOUT: Duration = Duration::from_secs(20);

/// Recreating a service stops and starts a container, which is far slower than a read.
const RECREATE_TIMEOUT: Duration = Duration::from_secs(120);

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
            synced: None,
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
        synced: None,
    }
}

/// Why mounts are being synchronised.
///
/// A request from the Runtime panel must say plainly when nothing could be changed.
/// The automatic pass after a switch must not: a container that is simply not running
/// is an ordinary state, and reporting it as an error would read as a failed switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTrigger {
    Automatic,
    Requested,
}

/// Brings the container's bind mounts in line with the packages that are LOCAL now.
///
/// A package switched to LOCAL gains the mount its source needs; one switched back to
/// VENDOR loses that mount again. The generated override is rewritten to the complete
/// desired set rather than appended to, which is what makes removal possible at all.
///
/// Docker cannot change the mounts of a running container, so the PHP service is
/// recreated -- but only when the set actually changed, so switching packages the
/// container already sees restarts nothing. The original Compose files, project name
/// and environment files come from the running container's own labels and are reused
/// unchanged; project files are never written.
pub fn sync_mounts(
    root: &Path,
    project_id: &str,
    service: Option<&str>,
    trigger: SyncTrigger,
) -> Result<ContainerReport, Message> {
    sync_mounts_with_output(
        root,
        project_id,
        service,
        trigger,
        &mut crate::runtime::container_output_with_timeout,
    )
}

/// The Runtime panel's explicit request, for one service the user chose.
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
    run: &mut impl FnMut(&Path, &[&str], Duration) -> Result<String, Message>,
) -> Result<ContainerReport, Message> {
    sync_mounts_with_output(root, project_id, Some(service), SyncTrigger::Requested, run)
}

pub fn sync_mounts_with_output(
    root: &Path,
    project_id: &str,
    service: Option<&str>,
    trigger: SyncTrigger,
    run: &mut impl FnMut(&Path, &[&str], Duration) -> Result<String, Message>,
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

    // A native project has nothing to mount into, and answering before touching the
    // engine keeps the automatic pass free of processes for projects without Compose.
    if compose_file(path).is_none() {
        return Ok(ContainerReport::unavailable(Message::new(
            "This project has no Compose file; it runs natively.",
        )));
    }

    let service = resolve_service(&root, project, service, &mut |dir, args| {
        run(dir, args, COMPOSE_TIMEOUT)
    });
    // Without a PHP service there is nothing to synchronise; the report explains why.
    let Some(service) = service else {
        return Ok(inspect_with_output(&root, project, &mut |dir, args| {
            run(dir, args, COMPOSE_TIMEOUT)
        }));
    };

    let context = running_context(path, &service, &mut |dir, args| {
        run(dir, args, COMPOSE_TIMEOUT)
    });
    let Some(context) = context else {
        if trigger == SyncTrigger::Requested {
            return Err(Message::new("The running service's Compose files could not be verified. No containers were changed."));
        }
        return Ok(inspect_with_output(&root, project, &mut |dir, args| {
            run(dir, args, COMPOSE_TIMEOUT)
        }));
    };

    let override_path = override_path(&root, project_id, &service)?;
    let generated = read_generated(&override_path, &service)?;

    // The service's own mounts, with SymfoLinker's generated ones taken out, so the
    // desired set reads the same whether or not an earlier synchronisation applied it.
    let base: Vec<VolumeMapping> = context
        .volumes
        .iter()
        .filter(|volume| {
            !generated
                .iter()
                .any(|mount| mount.container_path == volume.container_path)
        })
        .cloned()
        .collect();
    let desired = mount_plan(project, &base);

    let same = |left: &ContainerMount, right: &ContainerMount| {
        left.container_path == right.container_path
            && same_host_path(&left.host_path, &right.host_path)
    };
    let added: Vec<ContainerMount> = desired
        .iter()
        .filter(|mount| !generated.iter().any(|old| same(old, mount)))
        .cloned()
        .collect();
    let removed: Vec<ContainerMount> = generated
        .iter()
        .filter(|old| !desired.iter().any(|mount| same(old, mount)))
        .cloned()
        .collect();
    // The override can already be right while the container still runs without it,
    // for instance after an external `compose up` that omitted the generated file.
    let drifted = desired.iter().any(|mount| {
        !context.volumes.iter().any(|volume| {
            volume.container_path == mount.container_path
                && same_host_path(&volume.host_path, &mount.host_path)
        })
    });

    if added.is_empty() && removed.is_empty() && !drifted {
        let mut report = inspect_with_output(&root, project, &mut |dir, args| {
            run(dir, args, COMPOSE_TIMEOUT)
        });
        report.synced = Some(MountSync::default());
        return Ok(report);
    }

    for mount in &desired {
        if !Path::new(&mount.host_path).is_dir() {
            return Err(Message::with(
                "The local project {path} no longer exists.",
                "path",
                &mount.host_path,
            ));
        }
        // Someone else's mount at the same destination is never replaced.
        if base.iter().any(|volume| {
            volume.container_path == mount.container_path
                && !same_host_path(&volume.host_path, &mount.host_path)
        }) {
            return Err(Message::with(
                "A different source is already mounted at {path}. No containers were changed.",
                "path",
                &mount.container_path,
            ));
        }
        let source = Path::new(&mount.host_path).canonicalize().ok();
        if source
            .as_ref()
            .is_none_or(|source| !source.starts_with(&root) || !source.is_dir())
            || !mount.container_path.starts_with('/')
            || mount.container_path == "/"
            || mount.container_path.split('/').any(|part| part == "..")
        {
            return Err(unsafe_override());
        }
    }

    let json = serde_json::to_vec_pretty(&override_json(&service, &desired))
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
        COMPOSE_TIMEOUT,
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
            service.as_str(),
        ]
        .iter()
        .map(|value| (*value).to_owned()),
    );
    run(
        path,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        RECREATE_TIMEOUT,
    )?;

    let mut report = inspect_with_output(&root, project, &mut |dir, args| {
        run(dir, args, COMPOSE_TIMEOUT)
    });
    report.synced = Some(MountSync {
        added,
        removed,
        recreated: true,
    });
    Ok(report)
}

fn unsafe_override() -> Message {
    Message::new("The mount override directory is unsafe. No containers were changed.")
}

/// `<root>/.symfolinker/compose/<project id>/<service>.json`, above the projects. The
/// id is a path in a grouped root, so that directory can be nested.
///
/// Every component must be a plain directory or file: a link there could redirect the
/// write into a project, or out of the workspace entirely. The service name is already
/// restricted to Compose's own character set by `parse_running_context`, so it cannot
/// traverse out of the directory.
fn override_path(root: &Path, project_id: &str, service: &str) -> Result<PathBuf, Message> {
    let base = root.join(".symfolinker");
    // In a grouped root the project id is a path, so the override directory is nested
    // as well. The id comes from a scan, but it is still spelled out one component at
    // a time: a path that walks upwards must never be assembled here, whatever is
    // asked for.
    let mut directory = base.join("compose");
    for part in project_id.split('/') {
        if part.is_empty() || part.starts_with('.') || part.contains('\\') {
            return Err(unsafe_override());
        }
        directory.push(part);
    }
    let file = directory.join(format!("{service}.json"));
    // Every directory from `.symfolinker` down, not just the first and the last: a
    // link anywhere along the way could redirect this write into a project, or out of
    // the workspace entirely.
    if std::fs::symlink_metadata(&base)
        .is_ok_and(|meta| !meta.is_dir() || crate::discovery::is_link(&meta))
        || !crate::discovery::safe_ancestors(&base, &file)
    {
        return Err(unsafe_override());
    }
    if std::fs::symlink_metadata(&file)
        .is_ok_and(|meta| !meta.is_file() || crate::discovery::is_link(&meta))
    {
        return Err(unsafe_override());
    }
    Ok(file)
}

/// The mounts an earlier synchronisation generated, so this one knows what to keep,
/// what to drop, and which of the container's mounts are its own doing.
///
/// A file that is not the shape this app writes is reported rather than overwritten:
/// replacing it would silently discard whatever it actually meant.
fn read_generated(path: &Path, service: &str) -> Result<Vec<ContainerMount>, Message> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(unsafe_override()),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| unsafe_override())?;
    let volumes = value["services"][service]["volumes"]
        .as_array()
        .ok_or_else(unsafe_override)?;
    volumes
        .iter()
        .map(|volume| {
            if volume["type"] != "bind" {
                return Err(unsafe_override());
            }
            Ok(ContainerMount {
                // Which package owned a generated mount is deliberately not stored:
                // the scan is the only authority on that, and it is read again here.
                package_name: String::new(),
                host_path: volume["source"]
                    .as_str()
                    .ok_or_else(unsafe_override)?
                    .to_owned(),
                container_path: volume["target"]
                    .as_str()
                    .ok_or_else(unsafe_override)?
                    .to_owned(),
            })
        })
        .collect()
}

/// The PHP service to synchronise: the caller's choice, else the stored one, else the
/// suggestion. Only the last case has to read the Compose configuration, so the
/// automatic pass over a configured project costs one process less.
fn resolve_service(
    root: &Path,
    project: &Project,
    requested: Option<&str>,
    run: &mut impl FnMut(&Path, &[&str]) -> Result<String, Message>,
) -> Option<String> {
    if let Some(service) = requested {
        return Some(service.to_owned());
    }
    if let Some(service) = config::php_service(root, &project.path) {
        return Some(service);
    }
    let stdout = run(
        Path::new(&project.path),
        &["compose", "config", "--format", "json"],
    )
    .ok()?;
    suggest_php_service(&parse_compose_config(&stdout))
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
