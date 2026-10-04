use crate::discovery::{compose_file, display, inspect_git};
use crate::engine;
use crate::models::{ComposeService, DockerStatus, Message, ProjectStatus};
use serde::Deserialize;
use std::{path::Path, process::Command};

/// Rij zoals `docker compose ps --format json` die schrijft (PascalCase velden).
#[derive(Debug, Deserialize)]
struct ComposePsRow {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Service", default)]
    service: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "Health", default)]
    health: String,
    #[serde(rename = "Publishers", default)]
    publishers: Vec<Publisher>,
}

#[derive(Debug, Deserialize)]
struct Publisher {
    #[serde(rename = "PublishedPort", default)]
    published_port: u16,
    #[serde(rename = "TargetPort", default)]
    target_port: u16,
    #[serde(rename = "Protocol", default)]
    protocol: String,
}

/// Compose v2 writes JSON lines; older versions write a JSON array.
/// Support both formats without losing all status information for one invalid line.
pub fn parse_compose_ps(stdout: &str) -> Vec<ComposeService> {
    let trimmed = stdout.trim();
    let rows: Vec<ComposePsRow> = match serde_json::from_str::<Vec<ComposePsRow>>(trimmed) {
        Ok(rows) => rows,
        Err(_) => trimmed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<ComposePsRow>(line).ok())
            .collect(),
    };
    rows.into_iter()
        // Every field has a default, so arbitrary JSON could otherwise look like a service.
        // Require a container name to recognize a service row.
        .filter(|row| !row.name.is_empty())
        .map(|row| ComposeService {
            running: row.state.eq_ignore_ascii_case("running"),
            ports: format_ports(&row.publishers),
            health: (!row.health.is_empty()).then(|| row.health.clone()),
            service: if row.service.is_empty() {
                row.name.clone()
            } else {
                row.service
            },
            name: row.name,
            state: row.state,
            status: row.status,
        })
        .collect()
}

fn format_ports(publishers: &[Publisher]) -> String {
    let mut ports: Vec<String> = publishers
        .iter()
        .filter(|publisher| publisher.published_port != 0)
        .map(|publisher| {
            format!(
                "{}->{}/{}",
                publisher.published_port, publisher.target_port, publisher.protocol
            )
        })
        .collect();
    ports.sort();
    ports.dedup();
    ports.join(", ")
}

/// Builds a subprocess that never flashes a console window in a GUI application.
fn engine_command(binary: &str, dir: Option<&Path>, args: &[&str]) -> Command {
    let mut command = Command::new(binary);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

/// Whether this engine binary exists and its daemon answers. Used for detection.
pub(crate) fn engine_responds(binary: &str) -> bool {
    engine_command(
        binary,
        None,
        &["version", "--format", "{{.Server.Version}}"],
    )
    .output()
    .is_ok_and(|output| output.status.success())
}

/// Runs a compose subcommand on whichever engine is installed; errors stay translatable.
///
/// Docker and Podman take identical arguments for everything used here, so only the
/// binary differs.
pub(crate) fn container_output(dir: &Path, args: &[&str]) -> Result<String, Message> {
    let Some(engine) = engine::current() else {
        return Err(Message::new(
            "No container engine found. Is docker or podman in PATH?",
        ));
    };
    let output = engine_command(engine.binary(), Some(dir), args)
        .output()
        .map_err(|_| {
            Message::with(
                "{engine} CLI not found. Is it in PATH?",
                "engine",
                engine.label(),
            )
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.lines().find(|line| !line.trim().is_empty());
    Err(match detail {
        Some(detail) => {
            let mut message = Message::with("{engine} error: {detail}", "engine", engine.label());
            message
                .params
                .insert("detail".to_owned(), detail.trim().to_owned());
            message
        }
        None => Message::with(
            "{engine} returned an unknown error.",
            "engine",
            engine.label(),
        ),
    })
}

fn daemon_available(dir: &Path) -> bool {
    container_output(dir, &["version", "--format", "{{.Server.Version}}"]).is_ok()
}

pub fn inspect_docker(project: &Path, compose: Option<&str>) -> DockerStatus {
    let Some(compose) = compose else {
        let available = daemon_available(project);
        return DockerStatus {
            engine: engine::current().map(|engine| engine.label().to_owned()),
            available,
            compose_file: None,
            services: Vec::new(),
            message: Some(if available {
                Message::new("This project has no Compose file; it runs natively.")
            } else {
                Message::new(
                    "This project has no Compose file and no container engine is available.",
                )
            }),
        };
    };
    match container_output(project, &["compose", "ps", "--all", "--format", "json"]) {
        Ok(stdout) => {
            let services = parse_compose_ps(&stdout);
            let message = services
                .is_empty()
                .then(|| Message::new("No containers have been created for this Compose project."));
            DockerStatus {
                engine: engine::current().map(|engine| engine.label().to_owned()),
                available: true,
                compose_file: Some(compose.to_owned()),
                services,
                message,
            }
        }
        // Probe the daemon separately to distinguish Docker availability from Compose errors.
        Err(error) => DockerStatus {
            engine: engine::current().map(|engine| engine.label().to_owned()),
            available: false,
            compose_file: Some(compose.to_owned()),
            services: Vec::new(),
            message: Some(if daemon_available(project) {
                error
            } else {
                Message::with(
                    "{engine} is unavailable. Is it running?",
                    "engine",
                    engine::current().map_or("Docker", |engine| engine.label()),
                )
            }),
        },
    }
}

/// Refresh a scanned project's status and revalidate its path.
/// Polls must not run commands in directories that are no longer projects.
pub fn project_status(project: &Path) -> Result<ProjectStatus, Message> {
    let project = project
        .canonicalize()
        .map_err(|_| Message::new("The project no longer exists or is not readable."))?;
    if !project.join("composer.json").is_file() {
        return Err(Message::with(
            "{path} does not contain composer.json.",
            "path",
            display(&project),
        ));
    }
    Ok(ProjectStatus {
        git: inspect_git(&project),
        docker: inspect_docker(&project, compose_file(&project)),
    })
}
