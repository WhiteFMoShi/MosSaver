use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

use easilygit_domain::{AppError, AppResult};
use tempfile::tempdir;

use crate::{SystemGitBackend, metadata};

use super::{commands, snapshot};

pub(super) fn restore_checkpoint(
    backend: &SystemGitBackend,
    root: &Path,
    id: &str,
) -> AppResult<()> {
    let (metadata_path, mut metadata) = backend.load_metadata(root)?;
    if !metadata
        .checkpoints
        .iter()
        .any(|checkpoint| checkpoint.id == id)
    {
        return Err(AppError::CheckpointNotFound);
    }
    let target_commit = commit_for(root, id)?;
    let previous_restore_backup = metadata.last_restore_backup.clone();
    let backup = snapshot::write_checkpoint(
        root,
        &mut metadata,
        "Automatic safety point before restore",
        true,
    )?;
    metadata.last_restore_backup = Some(backup.id.clone());
    metadata.current_checkpoint = Some(id.to_owned());
    metadata::save(&metadata_path, &metadata)?;

    match apply_commit(root, &target_commit) {
        Ok(()) => Ok(()),
        Err(error) => {
            let backup_commit = commit_for(root, &backup.id)?;
            if let Err(rollback_error) = apply_commit(root, &backup_commit) {
                metadata.current_checkpoint = None;
                let metadata_error = metadata::save(&metadata_path, &metadata).err();
                return Err(AppError::RestoreFailed(format!(
                    "{error}. The safety restore also failed: {rollback_error}{}",
                    metadata_error
                        .map(|error| format!(". Recovery information could not be saved: {error}"))
                        .unwrap_or_default()
                )));
            }
            metadata.current_checkpoint = Some(backup.id);
            metadata.last_restore_backup = previous_restore_backup;
            metadata::save(&metadata_path, &metadata)?;
            Err(error)
        }
    }
}

pub(super) fn undo_restore(backend: &SystemGitBackend, root: &Path) -> AppResult<()> {
    let (metadata_path, mut metadata) = backend.load_metadata(root)?;
    let backup_id = metadata
        .last_restore_backup
        .clone()
        .ok_or(AppError::NothingToUndo)?;
    let target_commit = commit_for(root, &backup_id)?;

    let current = snapshot::write_checkpoint(
        root,
        &mut metadata,
        "Automatic safety point before undo",
        true,
    )?;
    metadata.last_restore_backup = None;
    metadata.current_checkpoint = Some(backup_id.clone());
    metadata::save(&metadata_path, &metadata)?;

    match apply_commit(root, &target_commit) {
        Ok(()) => Ok(()),
        Err(error) => {
            let current_commit = commit_for(root, &current.id)?;
            if let Err(rollback_error) = apply_commit(root, &current_commit) {
                metadata.current_checkpoint = None;
                metadata.last_restore_backup = Some(current.id.clone());
                let metadata_error = metadata::save(&metadata_path, &metadata).err();
                return Err(AppError::RestoreFailed(format!(
                    "{error}. The safety restore also failed: {rollback_error}{}",
                    metadata_error
                        .map(|error| format!(". Recovery information could not be saved: {error}"))
                        .unwrap_or_default()
                )));
            }
            metadata.current_checkpoint = Some(current.id);
            metadata.last_restore_backup = Some(backup_id);
            metadata::save(&metadata_path, &metadata)?;
            Err(error)
        }
    }
}

fn commit_for(root: &Path, id: &str) -> AppResult<String> {
    let reference = commands::checkpoint_ref(id);
    let output = commands::output(root, &["rev-parse", "--verify", &reference], &[])?;
    if !output.status.success() {
        return Err(AppError::CheckpointNotFound);
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| AppError::Git("Git returned an invalid checkpoint id".into()))
}

fn apply_commit(root: &Path, commit: &str) -> AppResult<()> {
    let target_paths = tree_paths(root, commit)?;
    let current_paths = worktree_paths(root)?;
    let target_set: HashSet<&str> = target_paths.iter().map(String::as_str).collect();
    let current_set: HashSet<&str> = current_paths.iter().map(String::as_str).collect();

    for path in current_paths
        .iter()
        .filter(|path| !target_set.contains(path.as_str()))
    {
        let full_path = safe_worktree_path(root, path)?;
        match fs::symlink_metadata(&full_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
                fs::remove_file(&full_path).map_err(map_restore_io)?;
                remove_empty_parents(root, &full_path);
            }
            Ok(metadata) if metadata.is_dir() => {
                return Err(AppError::RestoreFailed(format!(
                    "The folder '{}' contains files that cannot be replaced safely.",
                    path
                )));
            }
            Ok(_) => {
                return Err(AppError::RestoreFailed(format!(
                    "The file '{}' cannot be replaced safely.",
                    path
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_restore_io(error)),
        }
    }

    for path in &target_paths {
        let full_path = safe_worktree_path(root, path)?;
        match fs::symlink_metadata(&full_path) {
            Ok(_) if !current_set.contains(path.as_str()) && is_ignored(root, path)? => {
                return Err(AppError::RestoreFailed(format!(
                    "The ignored file '{}' already exists and will not be overwritten. Move it first, then retry.",
                    path
                )));
            }
            Ok(metadata) if metadata.is_dir() => {
                return Err(AppError::RestoreFailed(format!(
                    "The folder '{}' conflicts with a file in the selected version.",
                    path
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_restore_io(error)),
        }
    }

    let temporary = tempdir().map_err(|error| AppError::Internal(error.to_string()))?;
    let index_path = temporary.path().join("restore-index");
    let envs = [commands::index_env(&index_path)];
    commands::run_git_with_env(root, &["read-tree", commit], &envs).map_err(as_restore_error)?;
    commands::run_git_with_env(root, &["checkout-index", "--all", "--force"], &envs)
        .map_err(as_restore_error)?;
    Ok(())
}

fn tree_paths(root: &Path, commit: &str) -> AppResult<Vec<String>> {
    let bytes = commands::run_git(root, &["ls-tree", "-r", "-z", "--name-only", commit])?;
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = std::str::from_utf8(path).map_err(|_| AppError::InvalidPath)?;
            validate_relative_path(path)?;
            Ok(path.to_owned())
        })
        .collect()
}

fn worktree_paths(root: &Path) -> AppResult<Vec<String>> {
    let bytes = commands::run_git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = std::str::from_utf8(path).map_err(|_| AppError::InvalidPath)?;
            validate_relative_path(path)?;
            Ok(path.to_owned())
        })
        .collect()
}

fn validate_relative_path(path: &str) -> AppResult<()> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::InvalidPath);
    }
    Ok(())
}

fn safe_worktree_path(root: &Path, relative: &str) -> AppResult<PathBuf> {
    validate_relative_path(relative)?;
    let relative = Path::new(relative);
    let mut cursor = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(part) = component else {
            return Err(AppError::InvalidPath);
        };
        cursor.push(part);
        if components.peek().is_some() {
            match fs::symlink_metadata(&cursor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(AppError::RestoreFailed(format!(
                        "The folder '{}' is a symbolic link, so the restore was stopped for safety.",
                        cursor.strip_prefix(root).unwrap_or(&cursor).display()
                    )));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(AppError::RestoreFailed(format!(
                        "A file blocks the folder '{}' needed by this version.",
                        cursor.strip_prefix(root).unwrap_or(&cursor).display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(map_restore_io(error)),
            }
        }
    }
    Ok(cursor)
}

fn is_ignored(root: &Path, path: &str) -> AppResult<bool> {
    let output = commands::output(root, &["check-ignore", "--quiet", "--", path], &[])?;
    Ok(output.status.success())
}

fn remove_empty_parents(root: &Path, file: &Path) {
    let Ok(relative) = file.strip_prefix(root) else {
        return;
    };
    let mut parent = relative.parent();
    while let Some(relative_parent) = parent {
        if relative_parent.as_os_str().is_empty() {
            break;
        }
        let full = root.join(relative_parent);
        let Ok(metadata) = fs::symlink_metadata(&full) else {
            break;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            break;
        }
        match fs::remove_dir(&full) {
            Ok(()) => parent = relative_parent.parent(),
            Err(_) => break,
        }
    }
}

fn map_restore_io(error: std::io::Error) -> AppError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        _ => AppError::RestoreFailed(error.to_string()),
    }
}

fn as_restore_error(error: AppError) -> AppError {
    match error {
        AppError::PermissionDenied => AppError::PermissionDenied,
        other => AppError::RestoreFailed(other.to_string()),
    }
}
