use serde::Deserialize;
use std::path::Path;

/// Service names that commonly run PHP, best guess first (plan section 27).
const PHP_SERVICE_NAMES: [&str; 5] = ["php-fpm", "php", "app", "backend", "web"];

/// One bind mount: a host directory made visible at a path inside the container.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeMapping {
    pub host_path: String,
    pub container_path: String,
}

/// A service as `docker compose config` reports it, with its bind mounts.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposeServiceConfig {
    pub name: String,
    pub volumes: Vec<VolumeMapping>,
}

#[derive(Debug, Deserialize)]
struct ConfigDocument {
    #[serde(default)]
    services: std::collections::BTreeMap<String, ConfigService>,
}

#[derive(Debug, Deserialize)]
struct ConfigService {
    #[serde(default)]
    volumes: Vec<ConfigVolume>,
}

#[derive(Debug, Deserialize)]
struct ConfigVolume {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    target: String,
}

/// Reads services and their bind mounts from `docker compose config --format json`.
///
/// Only bind mounts can map a host directory to a container path; named volumes and
/// tmpfs carry no host side, so they are dropped rather than reported as unusable.
/// Unparsable output yields no services instead of failing the whole runtime view.
pub fn parse_compose_config(stdout: &str) -> Vec<ComposeServiceConfig> {
    let Ok(document) = serde_json::from_str::<ConfigDocument>(stdout.trim()) else {
        return Vec::new();
    };
    document
        .services
        .into_iter()
        .map(|(name, service)| ComposeServiceConfig {
            name,
            volumes: service
                .volumes
                .into_iter()
                .filter(|volume| volume.kind == "bind")
                .filter(|volume| !volume.source.is_empty() && !volume.target.is_empty())
                .map(|volume| VolumeMapping {
                    host_path: volume.source,
                    container_path: volume.target,
                })
                .collect(),
        })
        .collect()
}

/// Picks the service most likely to run PHP, so the user rarely has to choose.
///
/// Returns `None` rather than guessing when nothing matches: validating against the
/// wrong container would report a confident but meaningless result.
pub fn suggest_php_service(services: &[ComposeServiceConfig]) -> Option<String> {
    for candidate in PHP_SERVICE_NAMES {
        if let Some(service) = services.iter().find(|service| service.name == candidate) {
            return Some(service.name.clone());
        }
    }
    // A compose project naming its service `api-php-fpm` should still be recognised.
    services
        .iter()
        .find(|service| {
            PHP_SERVICE_NAMES
                .iter()
                .any(|name| service.name.contains(name))
        })
        .map(|service| service.name.clone())
}

/// Translates a host path into the path the container sees (plan section 28).
///
/// The longest matching mount wins, so a specific mount nested inside a broader one
/// is honoured. Comparison is separator-insensitive, and case-insensitive on Windows,
/// because compose and the filesystem disagree about both.
pub fn to_container_path(volumes: &[VolumeMapping], host_path: &Path) -> Option<String> {
    let target = normalise(&host_path.to_string_lossy());

    let mut best: Option<(usize, String)> = None;
    for volume in volumes {
        let source = normalise(&volume.host_path);
        let Some(remainder) = strip_mount(&target, &source) else {
            continue;
        };
        if best
            .as_ref()
            .is_some_and(|(length, _)| *length >= source.len())
        {
            continue;
        }
        let container = volume.container_path.trim_end_matches('/');
        let mapped = if remainder.is_empty() {
            container.to_owned()
        } else {
            format!("{container}/{remainder}")
        };
        best = Some((source.len(), mapped));
    }
    best.map(|(_, mapped)| mapped)
}

/// Returns the part of `path` below `mount`, or `None` when it is not inside it.
fn strip_mount(path: &str, mount: &str) -> Option<String> {
    let mount = mount.trim_end_matches('/');
    if path == mount {
        return Some(String::new());
    }
    let prefix = format!("{mount}/");
    path.strip_prefix(&prefix).map(|rest| rest.to_owned())
}

/// Windows extended-length prefixes and backslashes never appear inside a container.
fn normalise(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = path.replace('\\', "/");
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path
    }
}
