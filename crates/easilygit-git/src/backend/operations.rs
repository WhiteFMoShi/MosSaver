use std::{collections::HashMap, path::Path};

use easilygit_domain::{AppError, AppResult, Checkpoint, CheckpointDeleteBlock, CheckpointKind};
use uuid::Uuid;

use super::{SystemGitBackend, commands, history};
use crate::metadata::Metadata;

fn editable_checkpoint<'a>(metadata: &'a Metadata, id: &str) -> AppResult<&'a Checkpoint> {
    let checkpoint = metadata
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.id == id)
        .ok_or(AppError::CheckpointNotFound)?;
    if checkpoint.kind == CheckpointKind::Git {
        return Err(AppError::CheckpointReadOnly);
    }
    Ok(checkpoint)
}

pub(super) fn deletion_blocks(metadata: &Metadata) -> HashMap<&str, CheckpointDeleteBlock> {
    let mut blocks = HashMap::new();
    for checkpoint in &metadata.checkpoints {
        for parent in checkpoint
            .parent_id
            .iter()
            .chain(&checkpoint.merge_parent_ids)
        {
            blocks.insert(parent.as_str(), CheckpointDeleteBlock::HasChildren);
        }
    }
    for checkpoint in metadata
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.kind == CheckpointKind::Git)
    {
        blocks.insert(checkpoint.id.as_str(), CheckpointDeleteBlock::ReadOnly);
    }
    if let Some(id) = &metadata.last_restore_backup {
        blocks.insert(id, CheckpointDeleteBlock::Recovery);
    }
    if let Some(id) = &metadata.current_checkpoint {
        blocks.insert(id, CheckpointDeleteBlock::Current);
    }
    blocks
}

pub(super) fn delete_checkpoint(
    backend: &SystemGitBackend,
    root: &Path,
    id: &str,
) -> AppResult<()> {
    let (metadata_path, mut metadata) = backend.load_metadata(root)?;
    editable_checkpoint(&metadata, id)?;
    if deletion_blocks(&metadata).contains_key(id) {
        return Err(AppError::CheckpointInUse);
    }
    let commit = commands::checkpoint_commit(root, id)?.ok_or(AppError::CheckpointNotFound)?;
    let envs = commands::app_environment(root)?;
    let reference = commands::checkpoint_ref(id);
    commands::run_git_with_env(root, &["update-ref", "-d", &reference, &commit], &envs)?;
    metadata
        .checkpoints
        .retain(|checkpoint| checkpoint.id != id);
    if let Err(error) = backend.save_metadata(&metadata_path, &metadata) {
        commands::run_git_with_env(root, &["update-ref", &reference, &commit], &envs)?;
        return Err(error);
    }
    Ok(())
}

pub(super) fn merge_checkpoints(
    backend: &SystemGitBackend,
    root: &Path,
    source: &str,
    target: &str,
    label: &str,
) -> AppResult<()> {
    let (metadata_path, mut metadata) = backend.load_metadata(root)?;
    editable_checkpoint(&metadata, source)?;
    editable_checkpoint(&metadata, target)?;
    if source == target {
        return Err(AppError::MergeNotAllowed("请选择两个不同的版本。".into()));
    }
    let envs = commands::app_environment(root)?;
    let source_commit = commands::checkpoint_commit_with_env(root, source, &envs)?
        .ok_or(AppError::CheckpointNotFound)?;
    let target_commit = commands::checkpoint_commit_with_env(root, target, &envs)?
        .ok_or(AppError::CheckpointNotFound)?;
    for (first, second) in [
        (&source_commit, &target_commit),
        (&target_commit, &source_commit),
    ] {
        let ancestor =
            commands::output(root, &["merge-base", "--is-ancestor", first, second], &envs)?;
        match ancestor.status.code() {
            Some(0) => {
                return Err(AppError::MergeNotAllowed(
                    "其中一个版本已包含另一个版本，请直接使用较新的版本。".into(),
                ));
            }
            Some(1) => {}
            _ => {
                return Err(AppError::Git(
                    String::from_utf8_lossy(&ancestor.stderr).into_owned(),
                ));
            }
        }
    }
    // merge-tree writes objects only. It never checks out files or uses the
    // user's index, including when the merge reports conflicts.
    let merged = commands::output(
        root,
        &[
            "-c",
            "merge.renames=false",
            "merge-tree",
            "--write-tree",
            &target_commit,
            &source_commit,
        ],
        &envs,
    )?;
    if merged.status.code() == Some(1) {
        return Err(AppError::MergeConflict(
            String::from_utf8_lossy(&merged.stdout)
                .lines()
                .skip(1)
                .take(12)
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    if !merged.status.success() {
        return Err(AppError::MergeNotAllowed(format!(
            "需要 Git 2.38 或更新版本，且两个版本必须具有共同来源。{}",
            String::from_utf8_lossy(&merged.stderr).trim(),
        )));
    }
    let tree = std::str::from_utf8(&merged.stdout)
        .map_err(|_| AppError::InvalidPath)?
        .lines()
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| AppError::Git("Merge returned no tree".into()))?;
    let label = if label.trim().is_empty() {
        "合并版本"
    } else {
        label.trim()
    };
    let mut identity = envs.clone();
    identity.extend(commands::identity_env());
    let commit = commands::run_git_with_env(
        root,
        &[
            "commit-tree",
            tree,
            "-p",
            &target_commit,
            "-p",
            &source_commit,
            "-m",
            label,
        ],
        &identity,
    )?;
    let commit = String::from_utf8(commit).map_err(|_| AppError::InvalidPath)?;
    let commit = commit.trim();
    let created_at =
        commands::run_git_with_env(root, &["show", "-s", "--format=%cI", commit], &envs)?;
    let summary =
        history::checkpoint_details_for_commit(root, commit, Some(&target_commit), &envs)?.summary;
    let id = Uuid::new_v4().simple().to_string();
    let reference = commands::checkpoint_ref(&id);
    commands::run_git_with_env(root, &["update-ref", &reference, commit], &envs)?;
    metadata.checkpoints.push(Checkpoint {
        id,
        label: label.to_owned(),
        created_at: String::from_utf8_lossy(&created_at).trim().to_owned(),
        kind: CheckpointKind::User,
        parent_id: Some(target.to_owned()),
        merge_parent_ids: vec![source.to_owned()],
        summary: Some(summary),
        delete_blocked_reason: None,
    });
    if let Err(error) = backend.save_metadata(&metadata_path, &metadata) {
        commands::run_git_with_env(root, &["update-ref", "-d", &reference, commit], &envs)?;
        return Err(error);
    }
    Ok(())
}
