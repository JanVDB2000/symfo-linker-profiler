use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub development_root: String,
    pub projects: Vec<Project>,
    pub warnings: Vec<Message>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub composer_name: Option<String>,
    pub path: String,
    pub git: Option<GitInfo>,
    pub compose_file: Option<String>,
    pub packages: Vec<PackageStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub branch: String,
    pub commit: String,
    pub dirty: bool,
    pub changed_files: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageStatus {
    pub package_name: String,
    pub constraint: String,
    pub dependency_type: String,
    pub local_project_id: String,
    pub local_path: String,
    pub vendor_path: String,
    pub backup_path: String,
    pub mode: &'static str,
    pub link_status: &'static str,
    pub backup_status: &'static str,
    pub git: Option<GitInfo>,
}

/// Live project status: refreshed Git state and container status.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatus {
    pub git: Option<GitInfo>,
    pub docker: DockerStatus,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerStatus {
    /// Which engine answered: Docker or Podman.
    pub engine: Option<String>,
    /// Whether the Docker daemon is reachable, independently of project services.
    pub available: bool,
    pub compose_file: Option<String>,
    pub services: Vec<ComposeService>,
    /// A translatable explanation of the runtime state.
    pub message: Option<Message>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeService {
    pub name: String,
    pub service: String,
    pub state: String,
    pub status: String,
    pub health: Option<String>,
    pub ports: String,
    pub running: bool,
}

/// A translation key with data parameters, resolved by the frontend at render time.
#[derive(Debug, PartialEq, Serialize)]
pub struct Message {
    pub key: &'static str,
    pub params: std::collections::BTreeMap<String, String>,
}

impl Message {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            params: std::collections::BTreeMap::new(),
        }
    }

    pub fn with(key: &'static str, name: &str, value: impl Into<String>) -> Self {
        let mut message = Self::new(key);
        message.params.insert(name.to_owned(), value.into());
        message
    }
}

/// Mounts and per-package validation for one project's container (plan sections 26-29).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerReport {
    pub services: Vec<String>,
    pub php_service: Option<String>,
    /// True when the service was guessed rather than chosen by the user.
    pub suggested: bool,
    /// Bind mounts as (host path, container path) pairs.
    pub volumes: Vec<(String, String)>,
    pub project_container_path: Option<String>,
    pub checks: Vec<ContainerCheck>,
    pub message: Option<Message>,
    pub mount_plan: Vec<ContainerMount>,
    pub can_apply_mounts: bool,
}

impl ContainerReport {
    /// Nothing could be inspected; the message explains why.
    pub fn unavailable(message: Message) -> Self {
        Self {
            services: Vec::new(),
            php_service: None,
            suggested: false,
            volumes: Vec::new(),
            project_container_path: None,
            checks: Vec::new(),
            message: Some(message),
            mount_plan: Vec::new(),
            can_apply_mounts: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerMount {
    pub package_name: String,
    pub host_path: String,
    pub container_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerCheck {
    pub package_name: String,
    pub container_path: String,
    pub exists: bool,
    pub link_target: Option<String>,
    /// False when no bind mount covers this path, so nothing could be checked.
    pub mapped: bool,
}
