use std::path::PathBuf;

use easilygit_app::ProjectService;
use easilygit_domain::{AppError, ProjectState};
use easilygit_git::SystemGitBackend;
use serde::Serialize;
use tauri::{State, async_runtime};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandError {
    code: &'static str,
    message: String,
}

impl From<AppError> for CommandError {
    fn from(error: AppError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, CommandError> {
    async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| CommandError {
            code: "INTERNAL_ERROR",
            message: format!("Background task failed: {error}"),
        })?
        .map_err(CommandError::from)
}

#[tauri::command]
async fn open_project(
    path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.open_project(&PathBuf::from(path))).await
}

#[tauri::command]
async fn initialize_project(
    path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.enable_protection(&PathBuf::from(path))).await
}

#[tauri::command]
async fn refresh_project(
    path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.refresh(&PathBuf::from(path))).await
}

#[tauri::command]
async fn save_version(
    path: String,
    label: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.save_version(&PathBuf::from(path), &label)).await
}

#[tauri::command]
async fn restore_checkpoint(
    path: String,
    checkpoint_id: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.restore(&PathBuf::from(path), &checkpoint_id)).await
}

#[tauri::command]
async fn undo_restore(
    path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.undo_restore(&PathBuf::from(path))).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ProjectService::new(SystemGitBackend))
        .invoke_handler(tauri::generate_handler![
            open_project,
            initialize_project,
            refresh_project,
            save_version,
            restore_checkpoint,
            undo_restore
        ])
        .run(tauri::generate_context!())
        .expect("error while running EasilyGit");
}
