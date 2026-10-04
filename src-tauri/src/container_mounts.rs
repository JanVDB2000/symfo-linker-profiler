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

pub fn mount_plan(project: &Project, volumes: &[VolumeMapping]) -> Vec<ContainerMount> {
    let mut by_target = BTreeMap::new();
    for package in &project.packages {
        let Some(vendor) = to_container_path(volumes, Path::new(&package.vendor_path)) else {
            continue;
        };
        let Some((parent, _)) = vendor.rsplit_once('/') else {
            continue;
        };
        let target = if package.mode == "local" {
            std::fs::read_link(&package.vendor_path)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
        } else {
            None
        };
        let target = target.or_else(|| {
            relative_target(
                Path::new(&package.vendor_path),
                Path::new(&package.local_path),
            )
            .map(|path| path.to_string_lossy().replace('\\', "/"))
        });
        let Some(target) = target.and_then(|target| resolve_target(parent, &target)) else {
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
