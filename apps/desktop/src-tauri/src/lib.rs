use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, mpsc},
    thread,
    time::Duration,
};

use easilygit_app::ProjectService;
use easilygit_domain::{AppError, CheckpointPage, ManagedFile, ProjectOverview, ProjectState};
use easilygit_git::SystemGitBackend;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, async_runtime};

#[derive(Default)]
struct ProjectWatcher(Mutex<Option<RecommendedWatcher>>);

#[derive(Clone)]
struct WatchTargets {
    root: PathBuf,
    git_directory: Option<PathBuf>,
    refs_directory: Option<PathBuf>,
}

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
async fn project_overview(
    path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectOverview, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.project_overview(&PathBuf::from(path))).await
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
async fn load_more_checkpoints(
    path: String,
    offset: usize,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<CheckpointPage, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.load_more_checkpoints(&PathBuf::from(path), offset)).await
}

#[tauri::command]
async fn checkpoint_details(
    path: String,
    checkpoint_id: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<easilygit_domain::CheckpointDetails, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.checkpoint_details(&PathBuf::from(path), &checkpoint_id)).await
}

#[tauri::command]
async fn list_managed_files(
    path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<Vec<ManagedFile>, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.managed_files(&PathBuf::from(path))).await
}

#[tauri::command]
async fn file_preview(
    path: String,
    checkpoint_id: String,
    file_path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<easilygit_domain::FilePreview, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.file_preview(&PathBuf::from(path), &checkpoint_id, &file_path))
        .await
}

#[tauri::command]
async fn delete_checkpoint(
    path: String,
    checkpoint_id: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.delete_checkpoint(&PathBuf::from(path), &checkpoint_id)).await
}

#[tauri::command]
async fn merge_checkpoints(
    path: String,
    source_id: String,
    target_id: String,
    label: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || {
        service.merge_checkpoints(&PathBuf::from(path), &source_id, &target_id, &label)
    })
    .await
}

#[tauri::command]
async fn set_file_managed(
    path: String,
    file_path: String,
    managed: bool,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.set_file_managed(&PathBuf::from(path), &file_path, managed)).await
}

#[tauri::command]
async fn disable_protection(
    path: String,
    service: State<'_, ProjectService<SystemGitBackend>>,
) -> Result<ProjectState, CommandError> {
    let service = service.inner().clone();
    run_blocking(move || service.disable_protection(&PathBuf::from(path))).await
}

#[tauri::command]
fn list_managed_projects(app: AppHandle) -> Result<Vec<String>, String> {
    let path = catalog_file(&app)?;
    read_project_catalog(&path)
}

#[tauri::command]
fn add_managed_project(app: AppHandle, path: String) -> Result<Vec<String>, String> {
    let root = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("Could not open folder: {error}"))?;
    if !root.is_dir() {
        return Err("The selected path is not a folder".to_owned());
    }
    let catalog = catalog_file(&app)?;
    let mut projects = read_project_catalog(&catalog)?;
    let root = root.to_string_lossy().into_owned();
    if !projects.iter().any(|project| project == &root) {
        projects.push(root);
        write_project_catalog(&catalog, &projects)?;
    }
    Ok(projects)
}

#[tauri::command]
fn remove_managed_project(app: AppHandle, path: String) -> Result<Vec<String>, String> {
    let catalog = catalog_file(&app)?;
    let mut projects = read_project_catalog(&catalog)?;
    projects.retain(|project| project != &path);
    write_project_catalog(&catalog, &projects)?;
    Ok(projects)
}

fn catalog_file(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate MosSaver data folder: {error}"))?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create MosSaver data folder: {error}"))?;
    Ok(directory.join("managed-folders.json"))
}

fn read_project_catalog(path: &Path) -> Result<Vec<String>, String> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read managed folder list: {error}")),
    };
    let mut projects: Vec<String> = serde_json::from_slice(&contents)
        .map_err(|error| format!("Managed folder list is invalid: {error}"))?;
    let mut seen = HashSet::new();
    projects.retain(|project| seen.insert(project.clone()));
    Ok(projects)
}

fn write_project_catalog(path: &Path, projects: &[String]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Managed folder list has no parent folder".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create managed folder list: {error}"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Could not prepare managed folder list: {error}"))?;
    serde_json::to_writer_pretty(&mut temporary, projects)
        .map_err(|error| format!("Could not encode managed folder list: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("Could not save managed folder list: {error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("Could not replace managed folder list: {}", error.error))?;
    Ok(())
}

#[tauri::command]
fn watch_projects(
    paths: Vec<String>,
    app: AppHandle,
    watcher_state: State<'_, ProjectWatcher>,
) -> Result<(), String> {
    let targets: Vec<WatchTargets> = paths
        .into_iter()
        .filter_map(|path| PathBuf::from(path).canonicalize().ok())
        .filter(|root| root.is_dir())
        .map(|root| {
            let git_directory = git_path(&root, &["--absolute-git-dir"]);
            let common_directory = git_path(&root, &["--git-common-dir"]);
            let refs_directory = common_directory.map(|directory| directory.join("refs"));
            WatchTargets {
                root,
                git_directory,
                refs_directory,
            }
        })
        .collect();
    let (sender, receiver) = mpsc::channel::<String>();

    thread::spawn(move || {
        while let Ok(first_root) = receiver.recv() {
            let mut changed_roots = vec![first_root];
            loop {
                match receiver.recv_timeout(Duration::from_millis(350)) {
                    Ok(root) => {
                        if !changed_roots.contains(&root) {
                            changed_roots.push(root);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        for root in changed_roots {
                            let _ = app.emit("project-files-changed", &root);
                        }
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    });

    let targets_for_events = targets.clone();
    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<Event>| {
            if let Ok(event) = event {
                for target in &targets_for_events {
                    if should_refresh_targets(target, &event) {
                        let _ = sender.send(target.root.to_string_lossy().into_owned());
                    }
                }
            }
        },
        Config::default(),
    )
    .map_err(|error| error.to_string())?;
    for target in &targets {
        watcher
            .watch(&target.root, RecursiveMode::Recursive)
            .map_err(|error| error.to_string())?;
        if let Some(directory) = target
            .git_directory
            .as_ref()
            .filter(|directory| !directory.starts_with(&target.root))
        {
            watcher
                .watch(directory, RecursiveMode::NonRecursive)
                .map_err(|error| error.to_string())?;
        }
        if let Some(directory) = target
            .refs_directory
            .as_ref()
            .filter(|directory| directory.is_dir() && !directory.starts_with(&target.root))
        {
            watcher
                .watch(directory, RecursiveMode::Recursive)
                .map_err(|error| error.to_string())?;
        }
    }

    let mut current = watcher_state
        .0
        .lock()
        .map_err(|_| "Project watcher state is unavailable".to_owned())?;
    *current = Some(watcher);
    Ok(())
}

fn should_refresh_targets(target: &WatchTargets, event: &Event) -> bool {
    should_refresh_project(&target.root, event)
        || target
            .git_directory
            .as_ref()
            .is_some_and(|directory| should_refresh_git_directory(directory, event))
        || target
            .refs_directory
            .as_ref()
            .is_some_and(|directory| should_refresh_reference_directory(directory, event))
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

    event.paths.iter().any(|path| {
        path.strip_prefix(root).is_ok_and(|relative| {
            !relative.as_os_str().is_empty() && project_path_is_relevant(relative)
        })
    })
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
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(ProjectService::new(SystemGitBackend))
        .manage(ProjectWatcher::default())
        .invoke_handler(tauri::generate_handler![
            open_project,
            project_overview,
            initialize_project,
            refresh_project,
            save_version,
            restore_checkpoint,
            undo_restore,
            load_more_checkpoints,
            checkpoint_details,
            file_preview,
            delete_checkpoint,
            merge_checkpoints,
            list_managed_files,
            set_file_managed,
            disable_protection,
            list_managed_projects,
            add_managed_project,
            remove_managed_project,
            watch_projects,
            stop_project_watch
        ])
        .run(tauri::generate_context!())
        .expect("error while running MosSaver");
}

#[cfg(test)]
mod tests {
    use super::{
        WatchTargets, read_project_catalog, should_refresh_git_directory, should_refresh_project,
        should_refresh_reference_directory, should_refresh_targets, write_project_catalog,
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
        let easilygit_object_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(root.join(".git/easilygit/store.git/objects/ab/cdef"));
        let branch_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(root.join(".git/refs/heads/main"));
        let easilygit_ref_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(root.join(".git/refs/easilygit/checkpoints/id"));
        let outside_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(Path::new("other-project/file.txt").to_path_buf());

        assert!(should_refresh_project(root, &source_change));
        assert!(!should_refresh_project(root, &git_object_change));
        assert!(!should_refresh_project(root, &easilygit_object_change));
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

    #[test]
    fn watcher_matches_events_for_multiple_managed_projects() {
        let first = WatchTargets {
            root: Path::new("first").to_path_buf(),
            git_directory: None,
            refs_directory: None,
        };
        let second = WatchTargets {
            root: Path::new("second").to_path_buf(),
            git_directory: None,
            refs_directory: None,
        };
        let first_change = Event::new(EventKind::Modify(ModifyKind::Any))
            .add_path(Path::new("first/src/lib.rs").to_path_buf());
        assert!(should_refresh_targets(&first, &first_change));
        assert!(!should_refresh_targets(&second, &first_change));
    }

    #[test]
    fn project_catalog_is_deduplicated_and_replaced_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("managed-folders.json");
        let projects = vec![
            "/first".to_owned(),
            "/second".to_owned(),
            "/first".to_owned(),
        ];
        write_project_catalog(&path, &projects).unwrap();
        assert_eq!(
            read_project_catalog(&path).unwrap(),
            vec!["/first", "/second"]
        );
        write_project_catalog(&path, &["/second".to_owned()]).unwrap();
        assert_eq!(read_project_catalog(&path).unwrap(), vec!["/second"]);
    }
}
