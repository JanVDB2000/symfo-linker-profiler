use crate::{
    compose::{to_container_path, VolumeMapping},
    links::relative_target,
    models::{ContainerMount, Project},
};
use std::{collections::BTreeMap, path::Path};

/// Resolve a host relative link against its container parent, using POSIX rules
/// even when SymfoLinker runs on Windows. Mount sources at the exact destination
/// of the existing symlink, rather than replacing a link through a bind mount.
pub fn resolve_target(parent: &str, target: &str) -> Option<String> {
    let joined = if target.starts_with('/') {
        target.to_owned()
    } else {
        format!("{parent}/{target}")
    };
    let mut parts = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| format!("/{}", parts.join("/")))
}

/// Every bind mount this project's container needs for the packages that are LOCAL
/// *right now*. It is a complete desired set, not a list of additions: a package that
/// switched back to VENDOR simply stops appearing, which is what lets a synchronisation
/// remove its mount again.
///
/// What `volumes` contains decides what the result means. Pass the service's own mounts
/// without SymfoLinker's generated ones to get the full set the override must hold; pass
/// the running container's mounts to get only what is still missing from it.
pub fn mount_plan(project: &Project, volumes: &[VolumeMapping]) -> Vec<ContainerMount> {
    let mut by_target = BTreeMap::new();
    for package in &project.packages {
        if package.mode != "local" {
            continue;
        }
        let Some(vendor) = to_container_path(volumes, Path::new(&package.vendor_path)) else {
            continue;
        };
        let Some((parent, _)) = vendor.rsplit_once('/') else {
            continue;
        };
        // A Windows junction records an absolute host target, which means nothing inside
        // a container. Only a relative link can be followed there, so anything else falls
        // back to the relative form the scanner would have created.
        let target = std::fs::read_link(&package.vendor_path)
            .ok()
            .filter(|target| target.is_relative())
            .or_else(|| {
                relative_target(
                    Path::new(&package.vendor_path),
                    Path::new(&package.local_path),
                )
            })
            .map(|target| target.to_string_lossy().replace('\\', "/"))
            .and_then(|target| resolve_target(parent, &target));
        let Some(target) = target else {
            continue;
        };
        if to_container_path(volumes, Path::new(&package.local_path)).as_deref()
            == Some(target.as_str())
        {
            continue;
        }
        by_target.entry(target.clone()).or_insert(ContainerMount {
            package_name: package.package_name.clone(),
            host_path: package.local_path.clone(),
            container_path: target,
        });
    }
    by_target.into_values().collect()
}

/// The generated override for one service. An empty `mounts` writes an empty volume
/// list rather than deleting the file: the path stays valid for an external
/// `docker compose up -f` that already references it, and an empty sequence
/// contributes nothing to the merged configuration.
pub fn override_json(service: &str, mounts: &[ContainerMount]) -> serde_json::Value {
    let volumes: Vec<_> = mounts
        .iter()
        .map(|mount| {
            serde_json::json!({
                "type": "bind", "source": mount.host_path, "target": mount.container_path,
                "bind": { "create_host_path": false }
            })
        })
        .collect();
    serde_json::json!({"services": {service: {"volumes": volumes}}})
}
