use std::{ffi::OsString, path::Path};

use easilygit_domain::{AppError, AppResult, Checkpoint, CheckpointKind};
use tempfile::tempdir;
use uuid::Uuid;

use crate::metadata::Metadata;

use super::{SystemGitBackend, commands, files};

pub(super) fn create_checkpoint(
    backend: &SystemGitBackend,
    root: &Path,
    label: &str,
    automatic: bool,
) -> AppResult<Checkpoint> {
    let (metadata_path, mut metadata) = backend.load_metadata(root)?;
    let checkpoint = write_checkpoint(root, &mut metadata, label, automatic)?;
    backend.save_metadata(&metadata_path, &metadata)?;
    Ok(checkpoint)
}

pub(super) fn write_checkpoint(
    root: &Path,
    metadata: &mut Metadata,
    label: &str,
    automatic: bool,
) -> AppResult<Checkpoint> {
    let temporary = tempdir().map_err(|error| AppError::Internal(error.to_string()))?;
    let index_path = temporary.path().join("index");
    let mut envs = commands::app_environment(root)?;
    envs.push(commands::index_env(&index_path));

    if let Some(head) = head_commit(root) {
        commands::run_git_with_env(root, &["read-tree", &head], &envs)?;
    } else {
        commands::run_git_with_env(root, &["read-tree", "--empty"], &envs)?;
    }
    commands::run_git_with_env(root, &["add", "--all", "--", "."], &envs)?;
    files::apply_exclusions(root, &envs, &metadata.excluded_paths)?;
    let tree = output_text(root, &["write-tree"], &envs)?;
    let id = Uuid::new_v4().simple().to_string();
    let mut commit_args = vec!["commit-tree".to_owned(), tree];
    if let Some(head) = head_commit(root) {
        commit_args.push("-p".to_owned());
        commit_args.push(head);
    }
    let effective_label = if label.trim().is_empty() {
        if automatic {
            "Automatic safety point"
        } else {
            "Saved version"
        }
    } else {
        label.trim()
    };
    commit_args.push("-m".to_owned());
    commit_args.push(effective_label.to_owned());
    let commit_args: Vec<&str> = commit_args.iter().map(String::as_str).collect();
    let identity = commands::identity_env();
    let mut commit_envs = envs.clone();
    commit_envs.extend(identity);
    let commit = output_text(root, &commit_args, &commit_envs)?;
    let reference = commands::checkpoint_ref(&id);
    commands::run_git_with_env(root, &["update-ref", &reference, &commit], &envs)?;
    let created_at = output_text(root, &["show", "-s", "--format=%cI", &commit], &envs)?;

    let checkpoint = Checkpoint {
        id,
        label: effective_label.to_owned(),
        created_at,
        kind: if automatic {
            CheckpointKind::Automatic
        } else {
            CheckpointKind::User
        },
    };
    metadata.checkpoints.push(checkpoint.clone());
    metadata.current_checkpoint = Some(checkpoint.id.clone());
    Ok(checkpoint)
}

pub(super) fn head_commit(root: &Path) -> Option<String> {
    let output = commands::output(root, &["rev-parse", "--verify", "HEAD"], &[]).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn output_text(root: &Path, args: &[&str], envs: &[(OsString, OsString)]) -> AppResult<String> {
    let output = commands::run_git_with_env(root, args, envs)?;
    String::from_utf8(output)
        .map(|value| value.trim().to_owned())
        .map_err(|_| AppError::Git("Git returned invalid text output".into()))
}
