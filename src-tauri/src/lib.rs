pub mod activate;
pub mod backup;
pub mod compose;
pub mod config;
pub mod container;
pub mod discovery;
pub mod engine;
pub mod errors;
pub mod links;
pub mod lock;
pub mod models;
pub mod runtime;
pub mod swap;
pub mod write_guard;

#[cfg(feature = "desktop")]
#[tauri::command]
async fn scan_projects(development_root: String) -> Result<models::ScanResult, models::Message> {
    tauri::async_runtime::spawn_blocking(move || {
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

#[cfg(feature = "desktop")]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            scan_projects,
            project_status,
            activate_local,
            activate_vendor,
            inspect_container,
            set_php_service
        ])
        .run(tauri::generate_context!())
        .expect("SymfoLinker could not be started");
}
