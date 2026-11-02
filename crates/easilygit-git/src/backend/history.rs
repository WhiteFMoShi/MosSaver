use std::path::Path;

use easilygit_domain::{AppError, AppResult, Checkpoint, CheckpointKind};

use crate::metadata::Metadata;

use super::{commands, snapshot};

const HISTORY_FORMAT: &str = "--format=tformat:%H%x00%cI%x00%s";

pub(super) fn update_existing_history(root: &Path, metadata: &mut Metadata) -> AppResult<bool> {
    let head = snapshot::head_commit(root);
    let previous_head = metadata.last_observed_head.clone();
    let first_history_sync = !metadata.git_history_imported && previous_head.is_none();
    let mut tips = repository_tips(root)?;
    if let Some(head) = &head {
        tips.push(head.clone());
    }
    tips.sort();
    tips.dedup();

    if metadata.git_history_imported
        && metadata.git_history_tips == tips
        && metadata.last_observed_head == head
    {
        return Ok(false);
    }

    let mut args = vec!["log", "--branches", "--remotes", "--tags", HISTORY_FORMAT];
    if head.is_some() {
        args.push("HEAD");
    }
    let output = commands::run_git(root, &args)?;
    for line in output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: Vec<&[u8]> = line.split(|byte| *byte == 0).collect();
        if entry.len() != 3 {
            return Err(AppError::Git(
                "Git returned incomplete commit history".into(),
            ));
        }
        let commit = String::from_utf8(entry[0].to_vec()).map_err(|_| AppError::InvalidPath)?;
        let created_at = String::from_utf8(entry[1].to_vec()).map_err(|_| AppError::InvalidPath)?;
        let id = format!("git-{commit}");
        let reference = commands::checkpoint_ref(&id);
        let checkpoint = Checkpoint {
            id: id.clone(),
            label: nonempty_subject(entry[2]),
            created_at,
            kind: CheckpointKind::Git,
        };

        if !metadata.checkpoints.iter().any(|saved| saved.id == id) {
            commands::run_git(root, &["update-ref", &reference, &commit])?;
            metadata.checkpoints.push(checkpoint);
        }
    }

    if previous_head != head && !first_history_sync {
        metadata.current_checkpoint = head.as_ref().map(|head| format!("git-{head}"));
    } else if metadata.current_checkpoint.is_none()
        && let Some(head) = &head
    {
        metadata.current_checkpoint = Some(format!("git-{head}"));
    }
    metadata.git_history_imported = true;
    metadata.git_history_tips = tips;
    metadata.last_observed_head = head;
    Ok(true)
}

fn repository_tips(root: &Path) -> AppResult<Vec<String>> {
    let output = commands::run_git(
        root,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?;
    Ok(String::from_utf8_lossy(&output)
        .lines()
        .map(str::trim)
        .filter(|tip| !tip.is_empty())
        .map(str::to_owned)
        .collect())
}

fn nonempty_subject(subject: &[u8]) -> String {
    let subject = String::from_utf8_lossy(subject).trim().to_owned();
    if subject.is_empty() {
        "无提交说明".into()
    } else {
        subject
    }
}
