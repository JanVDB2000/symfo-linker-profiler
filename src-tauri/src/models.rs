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
    /// The project's path below the development root, with forward slashes: the
    /// directory name itself in a flat root, `<group>/<name>` in a grouped one. Unique
    /// either way, which is what makes it safe for the interface to act on.
    pub id: String,
    /// The directory name on its own.
    pub name: String,
    /// The directories between the root and the project, or `None` when the project
    /// sits directly in the root.
    pub group: Option<String>,
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
    /// The version line the local checkout declares: a branch alias for its current
    /// branch, or an explicit `version`. `None` when it declares neither.
    pub local_version: Option<String>,
    /// Whether that version line fits `constraint`: `satisfied`, `mismatch`, or
    /// `unknown` when it cannot be judged with certainty.
    pub constraint_status: &'static str,
    /// Whether the checkout's autoload rules match the ones the generated autoloader
    /// was built from: `matches`, `differs` or `unknown`.
    pub autoload_status: &'static str,
    /// True when SymfoLinker recorded a finished link that is no longer on disk and a
    /// Composer directory took its place - what `composer install` leaves behind.
    pub relinkable: bool,
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

/// What `composer dump-autoload` did, and where it did it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoloadDump {
    /// `container` or `host`: which PHP regenerated the autoloader.
    pub location: &'static str,
    /// The Compose service it ran in, when it ran in a container.
    pub service: Option<String>,
    /// Composer's own output, verbatim and bounded to its last lines.
    pub output: String,
    /// Why the host was used although the project has containers, when that happened.
    pub message: Option<Message>,
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
    /// What a synchronisation changed, so the interface can explain why the PHP
    /// service restarted. `None` when the report comes from a plain inspection.
    pub synced: Option<MountSync>,
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
            synced: None,
        }
    }
}

/// The difference one synchronisation made to the generated bind mounts.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountSync {
    /// Mounts added because their package is now LOCAL.
    pub added: Vec<ContainerMount>,
    /// Mounts dropped because their package went back to VENDOR.
    pub removed: Vec<ContainerMount>,
    /// Whether the PHP service was actually recreated.
    pub recreated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerMount {
    pub package_name: String,
    pub host_path: String,
    pub container_path: String,
}

/// One preserved Composer version under `vendor/.symfolinker/.history/`.
///
/// History is never cleaned up automatically, because deleting a package directory to
/// make room is exactly what this app exists to avoid. That makes an explicit view of
/// what is kept, and what it costs, part of the feature rather than an extra.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    pub package_name: String,
    /// The archive's directory name, which is also how a caller addresses it.
    pub archive_id: String,
    pub path: String,
    /// The version recorded in the sidecar, when it was known at archiving time.
    pub version: Option<String>,
    /// Seconds since the Unix epoch; 0 when neither sidecar nor name carried a time.
    pub archived_at: u64,
    pub size_bytes: u64,
    pub files: usize,
    /// True when a sidecar describes this archive and names this package and project.
    pub recognized: bool,
    /// False for anything that is not a plain directory: a file or link that was moved
    /// out of the way. Listed so it can be removed, not restored.
    pub directory: bool,
}

/// What a switch needs from the filesystem, checked before one is attempted.
///
/// Every field is the answer to a question that otherwise surfaces halfway through a
/// swap, as an operating system error on a path the user never chose.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    /// `<root>/.symfolinker/` can be created and written: the lock lives there.
    pub root_writable: bool,
    /// `<project>/vendor/` accepts a new entry, which every backup and link needs.
    pub vendor_writable: bool,
    /// What a link in this project would become: `symlink` or `junction`. `None` when
    /// neither could be created, which blocks switching to LOCAL entirely.
    pub link_kind: Option<&'static str>,
    /// Space available to this user on the project's filesystem, when it could be read.
    pub free_bytes: Option<u64>,
    /// What the preserved versions already occupy, so the free space has a scale.
    pub preserved_bytes: u64,
    /// The first failure's own wording, so a refusal is never just a red cross.
    pub message: Option<Message>,
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
