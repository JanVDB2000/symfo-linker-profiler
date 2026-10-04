use crate::models::{GitInfo, Message, PackageStatus, Project, ScanResult};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path, process::Command};

#[derive(Deserialize)]
struct Composer {
    name: Option<String>,
    #[serde(default)]
    require: BTreeMap<String, String>,
    #[serde(default, rename = "require-dev")]
    require_dev: BTreeMap<String, String>,
}

// Composer package identifiers must never become arbitrary filesystem paths.
pub fn valid_package_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_alphanumeric()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c))
                && !part.ends_with('.')
        })
}

pub fn scan(root: &Path) -> Result<ScanResult, Message> {
    let root = root
        .canonicalize()
        .map_err(|_| Message::new("The development root does not exist or is not readable."))?;
    if !root.is_dir() {
        return Err(Message::new("Choose a directory as the development root."));
    }
    let entries =
        fs::read_dir(&root).map_err(|_| Message::new("The development root could not be read."))?;
    let mut warnings = Vec::new();
    let mut found = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(value) => value,
            Err(_) => {
                warnings.push(Message::new(
                    "A directory in the development root could not be read.",
                ));
                continue;
            }
        };
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        // Do not discover linked directories or follow projects outside this root.
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !meta.is_dir() || is_link(&meta) {
            continue;
        }
        let canonical = match path.canonicalize() {
            Ok(path) if path.starts_with(&root) => path,
            _ => continue,
        };
        let manifest = canonical.join("composer.json");
        if !manifest.exists() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let composer: Composer = match fs::read(&manifest)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(value) => value,
            None => {
                warnings.push(Message::with(
                    "{name}: composer.json is invalid or unreadable; project skipped.",
                    "name",
                    name.clone(),
                ));
                continue;
            }
        };
        if composer
            .name
            .as_ref()
            .is_some_and(|name| !valid_package_name(name))
        {
            warnings.push(Message::with(
                "{name}: invalid Composer package name; no local mapping available.",
                "name",
                name.clone(),
            ));
        }
        let git = inspect_git(&canonical);
        let compose_file = compose_file(&canonical).map(|name| name.to_owned());
        found.push((
            Project {
                id: name.clone(),
                name,
                composer_name: composer.name.clone(),
                path: display(&canonical),
                git,
                compose_file,
                packages: Vec::new(),
            },
            composer,
            canonical,
        ));
    }
    found.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    let mut index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, (_, composer, _)) in found.iter().enumerate() {
        if let Some(name) = &composer.name {
            if valid_package_name(name) {
                index.entry(name.clone()).or_default().push(i);
            }
        }
    }
    for (name, matches) in &index {
        if matches.len() > 1 {
            warnings.push(Message::with(
                "{name}: multiple local projects found; automatic mapping skipped.",
                "name",
                name.clone(),
            ));
        }
    }
    for i in 0..found.len() {
        let mut packages = Vec::new();
        for (kind, requirements) in [
            ("require", &found[i].1.require),
            ("requireDev", &found[i].1.require_dev),
        ] {
            for (name, constraint) in requirements {
                let Some(matches) = index.get(name) else {
                    continue;
                };
                if matches.len() != 1 || matches[0] == i {
                    continue;
                }
                let local = &found[matches[0]];
                let vendor_root = found[i].2.join("vendor");
                let vendor_path = vendor_root.join(name);
                let backup_path = vendor_root.join(".symfolinker").join(name);
                // Reject escaped or linked ancestor directories before inspecting package paths.
                let safe = safe_ancestors(&found[i].2, &vendor_path)
                    && safe_ancestors(&found[i].2, &backup_path);
                let (mode, link_status) = if safe {
                    inspect_package(&vendor_path, &local.2)
                } else {
                    ("unknown", "invalid")
                };
                let backup_status = if safe {
                    inspect_backup(&backup_path)
                } else {
                    "invalid"
                };
                packages.push(PackageStatus {
                    package_name: name.clone(),
                    constraint: constraint.clone(),
                    dependency_type: kind.into(),
                    local_project_id: local.0.id.clone(),
                    local_path: local.0.path.clone(),
                    vendor_path: display(&vendor_path),
                    backup_path: display(&backup_path),
                    mode,
                    link_status,
                    backup_status,
                    git: local.0.git.clone(),
                });
            }
        }
        packages.sort_by(|a, b| a.package_name.cmp(&b.package_name));
        found[i].0.packages = packages;
    }
    Ok(ScanResult {
        development_root: display(&root),
        projects: found.into_iter().map(|item| item.0).collect(),
        warnings,
    })
}

/// Compose filenames are defined once; runtime.rs uses the same order.
pub fn compose_file(dir: &Path) -> Option<&'static str> {
    [
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
    ]
    .iter()
    .find(|name| dir.join(name).is_file())
    .copied()
}

pub fn display(path: &Path) -> String {
    // Windows canonical paths use extended-length notation; keep them intact for IPC.
    path.to_string_lossy().into_owned()
}

/// Windows junctions are reparse points, not symlinks, so both are treated as links.
pub fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// True when no directory between `project` and `target`'s parent is a link or a
/// non-directory. Shared with the WriteGuard: a linked `vendor/` would otherwise let
/// a write escape the project.
pub fn safe_ancestors(project: &Path, target: &Path) -> bool {
    let Some(parent) = target.parent() else {
        return false;
    };
    let Ok(relative) = parent.strip_prefix(project) else {
        return false;
    };
    let mut current = project.to_path_buf();
    for part in relative.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if !meta.is_dir() || is_link(&meta) => return false,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        }
    }
    true
}

fn inspect_package(vendor: &Path, local: &Path) -> (&'static str, &'static str) {
    match fs::symlink_metadata(vendor) {
        Ok(meta) if is_link(&meta) => match vendor.canonicalize() {
            Ok(target) if target == local => ("local", "linked"),
            Ok(_) => ("unknown", "unexpectedTarget"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("unknown", "broken"),
            Err(_) => ("unknown", "invalid"),
        },
        Ok(meta) if meta.is_dir() => ("vendor", "notLinked"),
        Ok(_) => ("unknown", "invalid"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("unknown", "missing"),
        Err(_) => ("unknown", "invalid"),
    }
}

fn inspect_backup(path: &Path) -> &'static str {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => "available",
        Ok(_) => "invalid",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing",
        Err(_) => "invalid",
    }
}

fn git_output(path: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(path)
        .args(args);
    command.env("GIT_TERMINAL_PROMPT", "0");
    // A GUI app must not flash console windows for each repository.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().ok()?;
    output.status.success().then_some(output.stdout)
}

pub fn inspect_git(path: &Path) -> Option<GitInfo> {
    // Do not accidentally report a development-root repository as the project repo.
    if !path.join(".git").exists() {
        return None;
    }
    let branch = String::from_utf8_lossy(&git_output(path, &["branch", "--show-current"])?)
        .trim()
        .to_owned();
    let commit = git_output(path, &["rev-parse", "--short", "HEAD"])
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .unwrap_or_default();
    let status = git_output(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut changed_files = 0;
    let mut records = status
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        changed_files += 1;
        if record
            .iter()
            .take(2)
            .any(|byte| *byte == b'R' || *byte == b'C')
        {
            records.next();
        }
    }
    Some(GitInfo {
        branch: if branch.is_empty() {
            "detached HEAD".into()
        } else {
            branch
        },
        commit,
        dirty: changed_files > 0,
        changed_files,
    })
}
