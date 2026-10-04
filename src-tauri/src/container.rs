use crate::compose::{parse_compose_config, suggest_php_service, to_container_path, VolumeMapping};
use crate::config;
use crate::discovery::compose_file;
use crate::models::{ContainerCheck, ContainerReport, Message, Project};
use crate::runtime::container_output;
use std::path::Path;

/// Inspects mounts and validates linked packages inside the container (plan sections
/// 26 to 29).
///
/// Deliberately not part of the five-minute status poll: `docker compose config` plus
/// one `exec` per linked package is far too expensive to run on a timer. The Runtime
/// panel asks for this explicitly instead.
///
/// Docker configuration is only ever read.
pub fn inspect(development_root: &Path, project: &Project) -> ContainerReport {
    let project_path = Path::new(&project.path);

    // Plan section 30: without a compose file the runtime is native and host
    // validation is all that is required.
    if compose_file(project_path).is_none() {
        return ContainerReport::unavailable(Message::new(
            "This project has no Compose file; it runs natively.",
        ));
    }

    let stdout = match container_output(project_path, &["compose", "config", "--format", "json"]) {
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
        };
    };

    let volumes = services
        .iter()
        .find(|service| service.name == selected)
        .map(|service| service.volumes.clone())
        .unwrap_or_default();

    let checks = validate_links(project_path, &selected, &volumes, project);

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
    }
}

/// Checks each locally linked package at the path the container actually sees.
fn validate_links(
    project_path: &Path,
    service: &str,
    volumes: &[VolumeMapping],
    project: &Project,
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
            let exists = container_output(
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

            let link_target = container_output(
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
