pub mod activate;
pub mod atomic;
pub mod autoload;
pub mod backup;
pub mod compose;
pub mod config;
pub mod constraint;
pub mod container;
pub mod container_mounts;
pub mod discovery;
pub mod dump_autoload;
pub mod engine;
pub mod errors;
pub mod external;
pub mod history;
pub mod journal;
pub mod links;
pub mod lock;
pub mod models;
pub mod preflight;
pub mod process;
pub mod profile;
pub mod recovery;
pub mod relink;
pub mod runtime;
pub mod swap;
pub mod watch;
pub mod write_guard;

#[cfg(feature = "desktop")]
#[tauri::command]
async fn scan_projects(development_root: String) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        // Scanning is the user asking to look again, which includes looking for a
        // container engine that was not running when the app started.
        engine::forget();
        discovery::scan(std::path::Path::new(&development_root))
    })
    .await
    .map_err(|_| models::Message::new("The scan could not be completed."))?
}

/// Keep repeated status polls inexpensive while a project is selected:
/// inspect only Git and docker compose ps for this project.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn project_status(project_path: String) -> Result<models::ProjectStatus, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        runtime::project_status(std::path::Path::new(&project_path))
    })
    .await
    .map_err(|_| models::Message::new("The project status could not be read."))?
}

/// Switches a package to the local project. Ids are resolved against a fresh scan, so
/// the webview never supplies a filesystem path for a mutation.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn activate_local(
    development_root: String,
    project_id: String,
    package_name: String,
) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        activate::activate_local_package(
            std::path::Path::new(&development_root),
            &project_id,
            &package_name,
        )
        .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The switch could not be completed."))?
}

/// Switches a package back to the preserved Composer version.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn activate_vendor(
    development_root: String,
    project_id: String,
    package_name: String,
) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        activate::activate_vendor_package(
            std::path::Path::new(&development_root),
            &project_id,
            &package_name,
        )
        .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The switch could not be completed."))?
}

/// Inspects mounts and validates linked packages inside the container. Explicit rather
/// than polled: it runs `docker compose config` plus one exec per linked package.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn inspect_container(
    development_root: String,
    project_id: String,
) -> Result<models::ContainerReport, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = std::path::Path::new(&development_root);
        let scan = discovery::scan(root)?;
        let project = scan
            .projects
            .into_iter()
            .find(|project| project.id == project_id)
            .ok_or_else(|| {
                models::Message::with(
                    "Project {id} is no longer in this workspace.",
                    "id",
                    project_id.clone(),
                )
            })?;
        Ok(container::inspect(root, &project))
    })
    .await
    .map_err(|_| models::Message::new("The container could not be inspected."))?
}

/// Stores which Compose service runs PHP for a project, above the projects.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn set_php_service(
    development_root: String,
    project_path: String,
    service: Option<String>,
) -> Result<(), models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        config::set_php_service(
            std::path::Path::new(&development_root),
            &project_path,
            service,
        )
        .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The PHP service could not be saved."))?
}

/// Regenerates one project's autoloader with `composer dump-autoload`, in its PHP
/// container when one is running and on the host otherwise. The only Composer command
/// this app runs, and the only repair for an autoloader that no longer matches the
/// checkout it was built from.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn dump_autoload(
    development_root: String,
    project_id: String,
) -> Result<models::AutoloadDump, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        dump_autoload::dump(std::path::Path::new(&development_root), &project_id)
    })
    .await
    .map_err(|_| models::Message::new("The autoloader could not be regenerated."))?
}

/// Which external tools this computer actually has, so the interface only offers
/// buttons that can work.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn external_tools() -> Vec<&'static str> {
    external::available(&std::env::var_os("PATH").unwrap_or_default())
}

/// Hands one project's directory to an editor, a terminal or the file manager. Takes
/// an id and never a path: the directory comes from a scan, like every other action.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn open_external(
    development_root: String,
    project_id: String,
    tool: String,
) -> Result<(), models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        external::open(std::path::Path::new(&development_root), &project_id, &tool)
    })
    .await
    .map_err(|_| models::Message::new("This tool could not be started."))?
}

/// The watch belonging to the project on screen, if any.
///
/// One at a time: the interface shows one project, and a workspace of fifty projects
/// has no use for fifty watchers. Storing it here is what keeps it alive, and
/// replacing it is what stops the previous one.
#[cfg(feature = "desktop")]
#[derive(Default)]
struct Watches(std::sync::Mutex<Option<watch::Watch>>);

/// Watches one project for the changes that make the interface stale: a
/// `composer install` under `vendor`, and a branch that moves under `.git`. Each one
/// arrives as a `project-changed` event rather than being waited for by a poll.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn watch_project(
    app: tauri::AppHandle,
    state: tauri::State<'_, Watches>,
    development_root: String,
    project_id: String,
) -> Result<(), models::Message> {
    let root = std::path::PathBuf::from(&development_root);
    let wanted = project_id.clone();
    let project = tauri::async_runtime::spawn_blocking(move || {
        discovery::scan(&root)?
            .projects
            .into_iter()
            .find(|project| project.id == wanted)
            .ok_or_else(|| {
                models::Message::with(
                    "Project {id} is no longer in this workspace.",
                    "id",
                    wanted.clone(),
                )
            })
    })
    .await
    .map_err(|_| models::Message::new("The scan could not be completed."))??;
    let handle = app.clone();
    let watched = project_id.clone();
    let watcher = watch::watch(&project, watch::DEBOUNCE, move |change| {
        use tauri::Emitter;
        // A closed window ends the interest in these events; it is not an error.
        let _ = handle.emit(
            "project-changed",
            serde_json::json!({ "projectId": watched, "change": change }),
        );
    })?;
    // Assigning replaces the previous watch, and dropping that one is what stops it.
    // A lock poisoned by a panic elsewhere is taken anyway: losing a watch is the
    // worst case, and the interface polls again after that.
    *state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(watcher);
    Ok(())
}

/// Stops watching, which is what leaves the five-minute poll in charge again.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn stop_watching(state: tauri::State<'_, Watches>) -> Result<(), models::Message> {
    *state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    Ok(())
}

#[cfg(feature = "desktop")]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Watches::default())
        .invoke_handler(tauri::generate_handler![
            scan_projects,
            project_status,
            activate_local,
            activate_vendor,
            inspect_container,
            set_php_service,
            apply_profile,
            recover_project,
            apply_container_mounts,
            sync_container_mounts,
            relink_packages,
            project_history,
            delete_archive,
            restore_archive,
            preflight_checks,
            dump_autoload,
            external_tools,
            open_external,
            watch_project,
            stop_watching
        ])
        .run(tauri::generate_context!())
        .expect("SymfoLinker could not be started");
}

#[cfg(feature = "desktop")]
#[tauri::command]
async fn apply_container_mounts(
    development_root: String,
    project_id: String,
    service: String,
) -> Result<models::ContainerReport, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        container::apply_mounts(
            std::path::Path::new(&development_root),
            &project_id,
            &service,
        )
    })
    .await
    .map_err(|_| models::Message::new("The container could not be inspected."))?
}

/// Brings the container's bind mounts in line with the packages that are LOCAL now.
///
/// Runs automatically after every switch: a package that became LOCAL gets its source
/// mounted, one that went back to VENDOR loses that mount again, and the PHP service
/// is recreated only when the set actually changed. A project without Compose, without
/// a PHP service or without a running container is answered without changing anything.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn sync_container_mounts(
    development_root: String,
    project_id: String,
) -> Result<models::ContainerReport, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        container::sync_mounts(
            std::path::Path::new(&development_root),
            &project_id,
            None,
            container::SyncTrigger::Automatic,
        )
    })
    .await
    .map_err(|_| models::Message::new("The container mounts could not be synchronised."))?
}

#[cfg(feature = "desktop")]
#[tauri::command]
async fn apply_profile(
    development_root: String,
    entries: Vec<profile::ProfileEntry>,
) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        profile::apply(std::path::Path::new(&development_root), &entries)
            .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The switch could not be completed."))?
}

/// Switches every package back to LOCAL whose link an external `composer install`
/// replaced. One lock, one validation pass, and completed changes reversed when a
/// later one fails, because it is a profile underneath.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn relink_packages(
    development_root: String,
    project_id: Option<String>,
) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        relink::apply(
            std::path::Path::new(&development_root),
            project_id.as_deref(),
        )
        .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The switch could not be completed."))?
}

/// The preserved Composer versions of one project, with what each one occupies.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn project_history(
    development_root: String,
    project_id: String,
) -> Result<Vec<models::ArchiveEntry>, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        history::list(std::path::Path::new(&development_root), &project_id)
            .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The preserved versions could not be read."))?
}

/// Removes one preserved version for good. The only destructive operation in this app,
/// so it acts on a single archive the user named and nothing else.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn delete_archive(
    development_root: String,
    project_id: String,
    package_name: String,
    archive_id: String,
) -> Result<Vec<models::ArchiveEntry>, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        history::delete(
            std::path::Path::new(&development_root),
            &project_id,
            &package_name,
            &archive_id,
        )
        .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The preserved version could not be removed."))?
}

/// Makes a preserved version the active backup again, so a switch to VENDOR restores
/// that one. Reports the state read back from disk, like every other mutation.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn restore_archive(
    development_root: String,
    project_id: String,
    package_name: String,
    archive_id: String,
) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        history::restore(
            std::path::Path::new(&development_root),
            &project_id,
            &package_name,
            &archive_id,
        )
        .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The preserved version could not be restored."))?
}

/// Writes a probe in each place a switch needs one, then removes it again, so a
/// missing permission is reported before a swap rather than during it.
#[cfg(feature = "desktop")]
#[tauri::command]
async fn preflight_checks(
    development_root: String,
    project_id: String,
) -> Result<models::PreflightReport, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        preflight::check(std::path::Path::new(&development_root), &project_id)
            .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The write checks could not be completed."))?
}

#[cfg(feature = "desktop")]
#[tauri::command]
async fn recover_project(
    development_root: String,
    project_id: String,
) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
        recovery::recover(std::path::Path::new(&development_root), &project_id)
            .map_err(|error| error.message())
    })
    .await
    .map_err(|_| models::Message::new("The switch could not be completed."))?
}
