use crate::atomic::write_atomically;
use crate::errors::SymfoLinkerError;
use crate::lock::RootLock;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, path::PathBuf};

/// Persisted settings for a development root (plan section 27).
///
/// Hard rule 2.2: this lives in `<root>/.symfolinker/config.json`, above the projects,
/// so selecting a PHP service never writes anything into a project directory.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct AppConfig {
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectConfig>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub php_service: Option<String>,
}

fn config_path(development_root: &Path) -> PathBuf {
    development_root.join(".symfolinker").join("config.json")
}

/// Reads the stored configuration.
///
/// A missing or unreadable file is an empty configuration, not an error: a corrupt
/// config must never stop the workspace from being scanned.
pub fn load(development_root: &Path) -> AppConfig {
    fs::read(config_path(development_root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The PHP service chosen for this project, if the user picked one.
pub fn php_service(development_root: &Path, project_path: &str) -> Option<String> {
    load(development_root)
        .projects
        .get(project_path)
        .and_then(|project| project.php_service.clone())
}

/// Stores the PHP service for one project, leaving every other entry untouched.
/// Passing `None` clears the choice so automatic suggestion takes over again.
///
/// Reading, changing and writing the file is one operation, so it runs under the same
/// development-root lock the swaps take: two writers that each loaded the file first
/// would otherwise have the last one silently drop the other's choice. The write itself
/// replaces the file rather than truncating it, so an interrupted save leaves the
/// previous configuration readable instead of a half-written one.
pub fn set_php_service(
    development_root: &Path,
    project_path: &str,
    service: Option<String>,
) -> Result<(), SymfoLinkerError> {
    let _lock = RootLock::acquire(development_root)?;

    let mut config = load(development_root);
    config
        .projects
        .entry(project_path.to_owned())
        .or_default()
        .php_service = service;

    let path = config_path(development_root);
    let json = serde_json::to_vec_pretty(&config).map_err(|error| SymfoLinkerError::IoError {
        path: path.to_string_lossy().into_owned(),
        detail: error.to_string(),
    })?;
    write_atomically(&path, &json)
}
