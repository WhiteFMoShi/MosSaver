use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, mpsc},
    thread,
    time::Duration,
};

use easilygit_app::ProjectService;
use easilygit_domain::{AppError, ProjectState};
use easilygit_git::SystemGitBackend;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State, async_runtime};

#[derive(Default)]
struct ProjectWatcher(Mutex<Option<RecommendedWatcher>>);

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

#[tauri::command]
fn watch_project(
    path: String,
    app: AppHandle,
    watcher_state: State<'_, ProjectWatcher>,
) -> Result<(), String> {
    let root = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let root_for_events = root.clone();
    let git_directory = git_path(&root, &["--absolute-git-dir"]);
    let common_directory = git_path(&root, &["--git-common-dir"]);
    let git_and_common_directory_match = git_directory == common_directory;
    let git_directory_for_events = git_directory.clone();
    let refs_directory = common_directory
        .as_ref()
        .map(|directory| directory.join("refs"));
    let refs_directory_for_events = refs_directory.clone();
    let root_for_payload = root.to_string_lossy().into_owned();
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        while receiver.recv().is_ok() {
            loop {
                match receiver.recv_timeout(Duration::from_millis(350)) {
                    Ok(()) => continue,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let _ = app.emit("project-files-changed", &root_for_payload);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    });

    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<Event>| {
            if event.as_ref().is_ok_and(|event| {
                should_refresh_project(&root_for_events, event)
                    || git_directory_for_events
                        .as_ref()
                        .is_some_and(|directory| should_refresh_git_directory(directory, event))
                    || refs_directory_for_events.as_ref().is_some_and(|directory| {
                        should_refresh_reference_directory(directory, event)
                    })
            }) {
                let _ = sender.send(());
            }
        },
        Config::default(),
    )
    .map_err(|error| error.to_string())?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| error.to_string())?;
    if let Some(directory) = git_directory.filter(|directory| !directory.starts_with(&root)) {
        watcher
            .watch(&directory, RecursiveMode::NonRecursive)
            .map_err(|error| error.to_string())?;
    }
    if let Some(directory) = common_directory
        .filter(|directory| !directory.starts_with(&root) && !git_and_common_directory_match)
    {
        watcher
            .watch(&directory, RecursiveMode::NonRecursive)
            .map_err(|error| error.to_string())?;
    }
    if let Some(directory) =
        refs_directory.filter(|directory| directory.is_dir() && !directory.starts_with(&root))
    {
        watcher
            .watch(&directory, RecursiveMode::Recursive)
            .map_err(|error| error.to_string())?;
    }

    let mut current = watcher_state
        .0
        .lock()
        .map_err(|_| "Project watcher state is unavailable".to_owned())?;
    *current = Some(watcher);
    Ok(())
}

#[tauri::command]
fn stop_project_watch(watcher_state: State<'_, ProjectWatcher>) -> Result<(), String> {
    let mut current = watcher_state
        .0
        .lock()
        .map_err(|_| "Project watcher state is unavailable".to_owned())?;
    *current = None;
    Ok(())
}

fn should_refresh_project(root: &Path, event: &Event) -> bool {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return false;
    }

    event
        .paths
        .iter()
        .any(|path| path.strip_prefix(root).is_ok_and(project_path_is_relevant))
}

fn project_path_is_relevant(relative: &Path) -> bool {
    let mut components = relative.components();
    if components
        .next()
        .is_none_or(|component| component.as_os_str() != ".git")
    {
        return true;
    }
    git_metadata_path_is_relevant(components.map(|component| component.as_os_str()))
}

fn git_metadata_path_is_relevant<'a>(components: impl Iterator<Item = &'a OsStr>) -> bool {
    let parts: Vec<&OsStr> = components.collect();
    let Some(first) = parts.first().copied() else {
        return false;
    };
    if first == "HEAD" || first == "index" || first == "packed-refs" || first == "shallow" {
        return true;
    }
    if first != "refs" || parts.len() < 2 {
        return false;
    }
    parts[1] == "heads" || parts[1] == "remotes" || parts[1] == "tags"
}

fn should_refresh_git_directory(root: &Path, event: &Event) -> bool {
    is_file_change(event)
        && event.paths.iter().any(|path| {
            path.strip_prefix(root).is_ok_and(|relative| {
                git_metadata_path_is_relevant(
                    relative.components().map(|component| component.as_os_str()),
                )
            })
        })
}

fn should_refresh_reference_directory(root: &Path, event: &Event) -> bool {
    is_file_change(event)
        && event.paths.iter().any(|path| {
            path.strip_prefix(root).is_ok_and(|relative| {
                relative.components().next().is_some_and(|component| {
                    let name = component.as_os_str();
                    name == "heads" || name == "remotes" || name == "tags"
                })
            })
        })
}

fn is_file_change(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

fn git_path(root: &Path, args: &[&str]) -> Option<PathBuf> {
    let mut command = Command::new("git");
    command
        .args(["rev-parse"])
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX");
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    path.canonicalize().ok()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ProjectService::new(SystemGitBackend))
        .manage(ProjectWatcher::default())
        .invoke_handler(tauri::generate_handler![
            open_project,
            initialize_project,
            refresh_project,
            save_version,
            restore_checkpoint,
            undo_restore,
            watch_project,
            stop_project_watch
        ])
        .run(tauri::generate_context!())
        .expect("error while running EasilyGit");
}

#[cfg(test)]
mod tests {
    use super::{
        should_refresh_git_directory, should_refresh_project, should_refresh_reference_directory,
    };
    use notify::{Event, EventKind, event::ModifyKind};
    use std::path::Path;

    #[test]
    fn watcher_refreshes_project_files_and_git_history_but_ignores_easilygit_objects() {
        let root = Path::new("project");
        let source_change =
            Event::new(EventKind::Modify(ModifyKind::Any)).add_path(root.join("src/main.rs"));
        let git_object_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(root.join(".git/objects/ab/cdef"));
        let branch_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(root.join(".git/refs/heads/main"));
        let easilygit_ref_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(root.join(".git/refs/easilygit/checkpoints/id"));
        let outside_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(Path::new("other-project/file.txt").to_path_buf());

        assert!(should_refresh_project(root, &source_change));
        assert!(!should_refresh_project(root, &git_object_change));
        assert!(should_refresh_project(root, &branch_change));
        assert!(!should_refresh_project(root, &easilygit_ref_change));
        assert!(!should_refresh_project(root, &outside_change));

        let git_directory = Path::new("linked-worktree-gitdir");
        let head_change =
            Event::new(EventKind::Modify(ModifyKind::Any)).add_path(git_directory.join("HEAD"));
        assert!(should_refresh_git_directory(git_directory, &head_change));

        let refs_directory = Path::new("shared-gitdir/refs");
        let tag_change = Event::new(EventKind::Create(notify::event::CreateKind::File))
            .add_path(refs_directory.join("tags/release"));
        assert!(should_refresh_reference_directory(
            refs_directory,
            &tag_change
        ));
    }
}
