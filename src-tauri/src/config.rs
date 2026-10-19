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
    /// Which local checkout backs a package that has more than one, as
    /// `vendor/package` to project id. Root-wide rather than per project: the same
    /// clone is wanted everywhere in a root, and asking once per consumer would mean
    /// answering the same question for every application that requires the bundle.
    ///
    /// `serde(default)` on both members is what keeps a configuration written by an
    /// older version readable.
    #[serde(default)]
    pub packages: BTreeMap<String, String>,
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

/// Which checkout backs each package that has more than one. Empty when none is set.
pub fn package_sources(development_root: &Path) -> BTreeMap<String, String> {
    load(development_root).packages
}

/// Stores which local checkout backs one package, for every project in this root.
/// Passing `None` clears the choice, so the scanner asks again.
///
/// Read, change and write is one operation under the development-root lock, for the
/// same reason as [`set_php_service`]: two writers that each loaded the file first
/// would have the last one silently drop the other's choice.
pub fn set_package_source(
    development_root: &Path,
    package_name: &str,
    project_id: Option<String>,
) -> Result<(), SymfoLinkerError> {
    let _lock = RootLock::acquire_for(development_root, "a checkout choice")?;

    let mut config = load(development_root);
    match project_id {
        Some(id) => config.packages.insert(package_name.to_owned(), id),
        None => config.packages.remove(package_name),
    };

    let path = config_path(development_root);
    let json = serde_json::to_vec_pretty(&config).map_err(|error| SymfoLinkerError::IoError {
        path: path.to_string_lossy().into_owned(),
        detail: error.to_string(),
    })?;
    write_atomically(&path, &json)
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
