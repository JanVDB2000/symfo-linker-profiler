use crate::models::{GitInfo, Message, PackageStatus, Project, ScanResult};
use crate::{autoload, backup, constraint, journal};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// How deep below the development root a project may sit.
///
/// A flat root needs one level. Grouped roots (`<team>/<project>`), a group holding
/// repositories that each contain packages (`<team>/<repo>/packages/<package>`) and
/// everything in between need a few more. Four is deep enough for the layouts people
/// actually keep their work in, and shallow enough that a root chosen one directory
/// too high cannot turn a scan into a walk of the whole drive.
const MAX_DEPTH: usize = 4;

/// Directory names that are never descended into: they hold dependencies or build
/// output rather than projects, and they are the large ones.
const SKIPPED: [&str; 4] = ["vendor", "node_modules", "target", "dist"];

/// How many directories one scan may read. A tree deeper and wider than this is
/// scanned as far as the budget allows and then reported as incomplete, rather than
/// leaving the interface waiting on an unbounded walk.
const MAX_DIRECTORIES: usize = 4_000;

#[derive(Deserialize)]
struct Composer {
    name: Option<String>,
    #[serde(default)]
    require: BTreeMap<String, String>,
    #[serde(default, rename = "require-dev")]
    require_dev: BTreeMap<String, String>,
    /// A project is also somebody's package: these three describe what it provides
    /// when it is linked into another project, rather than what it consumes.
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    autoload: Option<serde_json::Value>,
    #[serde(default)]
    extra: Option<serde_json::Value>,
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
    // An unreadable root is an error rather than a warning: a walk that cannot start
    // would otherwise report an empty workspace as if nothing were there.
    if fs::read_dir(&root).is_err() {
        return Err(Message::new("The development root could not be read."));
    }
    let mut warnings = Vec::new();
    let mut found = Vec::new();
    for (id, canonical) in project_directories(&root, &mut warnings) {
        // The directory name is what a project is called; the group it sits in is
        // carried separately, so a grouped root reads as a grouped root rather than
        // as a list of paths.
        let (group, name) = match id.rsplit_once('/') {
            Some((group, name)) => (Some(group.to_owned()), name.to_owned()),
            None => (None, id.clone()),
        };
        let composer: Composer = match fs::read(canonical.join("composer.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            Some(value) => value,
            None => {
                warnings.push(Message::with(
                    "{name}: composer.json is invalid or unreadable; project skipped.",
                    "name",
                    id.clone(),
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
                id.clone(),
            ));
        }
        let git = inspect_git(&canonical);
        let compose_file = compose_file(&canonical).map(|name| name.to_owned());
        found.push((
            Project {
                id,
                name,
                group,
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
    // Sorting on the id keeps the projects of one group together, and is the same
    // order as sorting on the name while the root is flat.
    found.sort_by(|a, b| a.0.id.cmp(&b.0.id));
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
        // Read once per project rather than once per package: it is a single file
        // describing all of them, and the scan already walks every requirement.
        let journal = match journal::read_checked(&found[i].2) {
            Ok(records) => records,
            Err(_) => {
                warnings.push(Message::with("{name}: backup journal is invalid or unreadable. Recovery requires a valid journal.", "name", found[i].0.id.clone()));
                journal::Journal::new()
            }
        };
        // Read once per project as well: one file describes the autoload rules of
        // every installed package, and it is the record the generated autoloader was
        // built from. Parsing it per package would re-read a few hundred kilobytes.
        let installed = fs::read(found[i].2.join("vendor/composer/installed.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        for (package, record) in &journal {
            if record.state != journal::SwapState::Linked {
                let mut warning = Message::with(
                    "{name}: interrupted operation for {package}. Use Health to recover.",
                    "name",
                    found[i].0.id.clone(),
                );
                warning.params.insert("package".to_owned(), package.clone());
                warnings.push(warning);
            }
        }
        let mut packages = Vec::new();
        for (kind, requirements) in [
            ("require", &found[i].1.require),
            ("requireDev", &found[i].1.require_dev),
        ] {
            for (name, required) in requirements {
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
                    match journal.get(name) {
                        Some(record)
                            if record.package != *name
                                || Path::new(&record.project) != found[i].2 =>
                        {
                            "invalid"
                        }
                        record => backup::backup_status(&backup_path, record),
                    }
                } else {
                    "invalid"
                };
                // What the checkout provides, judged against what this project asks
                // for. Both answers are read from the local project's own manifest,
                // which the first pass already parsed.
                let local_version = constraint::local_version(
                    local.1.version.as_deref(),
                    local.1.extra.as_ref(),
                    local.0.git.as_ref().map(|git| git.branch.as_str()),
                );
                let constraint_status = local_version
                    .as_deref()
                    .map_or(constraint::UNKNOWN, |version| {
                        constraint::status(required, version)
                    });
                let installed_autoload = installed
                    .as_ref()
                    .and_then(|json| autoload::installed_autoload(json, name));
                let autoload_status = if safe {
                    autoload::status(installed_autoload.as_ref(), local.1.autoload.as_ref())
                } else {
                    autoload::UNKNOWN
                };
                // A finished link that is no longer on disk, with a Composer directory
                // in its place: nothing in this app moved it, so `composer install`
                // did. Switching back to LOCAL is the repair, and it is offered as one.
                let relinkable = safe
                    && mode == "vendor"
                    && journal
                        .get(name)
                        .is_some_and(|record| record.state == journal::SwapState::Linked);
                packages.push(PackageStatus {
                    package_name: name.clone(),
                    constraint: required.clone(),
                    dependency_type: kind.into(),
                    local_project_id: local.0.id.clone(),
                    local_path: local.0.path.clone(),
                    vendor_path: display(&vendor_path),
                    backup_path: display(&backup_path),
                    mode,
                    link_status,
                    backup_status,
                    git: local.0.git.clone(),
                    local_version,
                    constraint_status,
                    autoload_status,
                    relinkable,
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

/// Every project below the development root, as (id, canonical path) pairs in the
/// order they were found.
///
/// The walk is breadth-first and bounded three ways: a directory that holds a
/// composer.json is a project and is never descended into, nothing deeper than
/// `MAX_DEPTH` is read, and `MAX_DIRECTORIES` caps the total. That makes a flat root,
/// a root grouped by team or client, and anything in between the same thing to the
/// rest of the scan - a project is wherever its manifest is.
///
/// Links are skipped rather than followed, at every level: a linked directory is
/// either a second name for a project that is already in the list, or a way out of
/// the workspace entirely.
fn project_directories(root: &Path, warnings: &mut Vec<Message>) -> Vec<(String, PathBuf)> {
    let mut queue = VecDeque::from([(root.to_path_buf(), 0)]);
    let mut found = Vec::new();
    let mut budget = MAX_DIRECTORIES;
    while let Some((directory, depth)) = queue.pop_front() {
        if budget == 0 {
            warnings.push(Message::new(
                "The development root holds too many directories to scan completely. Projects deeper in the tree may be missing.",
            ));
            break;
        }
        budget -= 1;
        let Ok(entries) = fs::read_dir(&directory) else {
            warnings.push(Message::with(
                "{name}: this directory could not be read; projects inside it were skipped.",
                "name",
                relative_id(root, &directory),
            ));
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                warnings.push(Message::new(
                    "A directory in the development root could not be read.",
                ));
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIPPED.contains(&name.as_str()) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if !meta.is_dir() || is_link(&meta) {
                continue;
            }
            // Resolve once, and only keep what stays inside this root.
            let canonical = match entry.path().canonicalize() {
                Ok(path) if path.starts_with(root) => path,
                _ => continue,
            };
            if canonical.join("composer.json").exists() {
                found.push((relative_id(root, &canonical), canonical));
            } else if depth + 1 < MAX_DEPTH {
                queue.push_back((canonical, depth + 1));
            }
        }
    }
    found
}

/// A project's path below the root, with forward slashes on every platform: the id the
/// interface sends back, and the one thing that stays unique when two groups hold a
/// project of the same name.
fn relative_id(root: &Path, project: &Path) -> String {
    project
        .strip_prefix(root)
        .unwrap_or(project)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
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
