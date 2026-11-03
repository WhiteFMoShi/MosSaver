use std::collections::HashMap;
use std::path::Path;

use easilygit_domain::{
    AppError, AppResult, ChangeKind, Checkpoint, CheckpointDetails, CheckpointKind,
    CheckpointSummary,
};

use crate::metadata::Metadata;

use super::{commands, snapshot};

pub(super) const HISTORY_PAGE_SIZE: usize = 50;

const HISTORY_FORMAT: &str = "--format=tformat:%H%x00%cI%x00%s";

/// Fill the parent links for versions written by older MosSaver builds. The
/// parent is taken from the stored commit graph, then mapped back to another
/// MosSaver id when that commit belongs to our own history. Native Git
/// commits remain roots in the MosSaver graph because they are read-only.
pub(super) fn hydrate_checkpoint_graph(root: &Path, metadata: &mut Metadata) -> AppResult<bool> {
    if metadata
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.kind != CheckpointKind::Git)
        .all(|checkpoint| checkpoint.summary.is_some())
    {
        return Ok(false);
    }
    let environment = commands::app_environment(root)?;
    let mut commits = HashMap::new();
    for checkpoint in metadata
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.kind != CheckpointKind::Git)
    {
        if let Some(commit) =
            commands::checkpoint_commit_with_env(root, &checkpoint.id, &environment)?
        {
            commits.insert(commit, checkpoint.id.clone());
        }
    }
    let mut changed = false;
    for checkpoint in metadata
        .checkpoints
        .iter_mut()
        .filter(|checkpoint| checkpoint.kind != CheckpointKind::Git && checkpoint.summary.is_none())
    {
        let Some(commit) =
            commands::checkpoint_commit_with_env(root, &checkpoint.id, &environment)?
        else {
            continue;
        };
        let parent_commit = first_parent(root, &commit, &environment)?;
        let parent = parent_commit.as_ref().map(|parent_commit| {
            commits
                .get(parent_commit)
                .cloned()
                .unwrap_or_else(|| format!("git-{parent_commit}"))
        });
        let details =
            checkpoint_details_for_commit(root, &commit, parent_commit.as_deref(), &environment)?;
        if checkpoint.parent_id != parent {
            checkpoint.parent_id = parent;
        }
        checkpoint.summary = Some(details.summary);
        changed = true;
    }
    Ok(changed)
}

pub(super) fn checkpoint_details(
    root: &Path,
    checkpoint: &Checkpoint,
) -> AppResult<CheckpointDetails> {
    let app_checkpoint = checkpoint.kind != CheckpointKind::Git;
    let environment = if app_checkpoint {
        commands::app_environment(root)?
    } else {
        Vec::new()
    };
    let commit = commands::checkpoint_commit_with_env(root, &checkpoint.id, &environment)?
        .ok_or(AppError::CheckpointNotFound)?;
    let parent = match checkpoint.parent_id.as_deref() {
        Some(parent_id) => commands::checkpoint_commit(root, parent_id)?,
        None => first_parent(root, &commit, &environment)?,
    };
    checkpoint_details_for_commit(root, &commit, parent.as_deref(), &environment)
}

pub(super) fn checkpoint_details_for_commit(
    root: &Path,
    commit: &str,
    parent: Option<&str>,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) -> AppResult<CheckpointDetails> {
    let tree = commands::run_git_with_env(
        root,
        &["ls-tree", "-r", "-z", "--name-only", commit],
        environment,
    )?;
    let file_count = tree
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .count();
    let diff = if let Some(parent) = parent {
        commands::run_git_with_env(
            root,
            &[
                "diff",
                "--no-renames",
                "--name-status",
                "-z",
                parent,
                commit,
            ],
            environment,
        )?
    } else {
        commands::run_git_with_env(
            root,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--no-renames",
                "--name-status",
                "-r",
                "-z",
                commit,
            ],
            environment,
        )?
    };
    let files = super::status::parse_changes(&diff)?;
    let summary = CheckpointSummary {
        file_count,
        added: files
            .iter()
            .filter(|file| file.kind == ChangeKind::Added)
            .count(),
        modified: files
            .iter()
            .filter(|file| file.kind == ChangeKind::Modified)
            .count(),
        deleted: files
            .iter()
            .filter(|file| file.kind == ChangeKind::Deleted)
            .count(),
    };
    Ok(CheckpointDetails { summary, files })
}

pub(super) fn first_parent(
    root: &Path,
    commit: &str,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) -> AppResult<Option<String>> {
    let output = commands::run_git_with_env(
        root,
        &["rev-list", "--parents", "-n", "1", commit],
        environment,
    )?;
    let parent = String::from_utf8_lossy(&output)
        .split_whitespace()
        .nth(1)
        .map(str::to_owned);
    Ok(parent)
}

pub(super) fn update_existing_history(root: &Path, metadata: &mut Metadata) -> AppResult<bool> {
    let mut changed = false;
    // Older builds temporarily used the native Git HEAD as the active
    // comparison point. Migrate that value back to MosSaver's own latest
    // checkpoint so a Git checkout/reset is reported as a file change.
    if metadata
        .current_checkpoint
        .as_deref()
        .is_some_and(|id| id.starts_with("git-"))
    {
        if let Some(checkpoint) = latest_mossaver_checkpoint(metadata) {
            metadata.current_checkpoint = Some(checkpoint.id.clone());
            changed = true;
        }
    }
    if !metadata.git_history_refs_cleaned {
        remove_legacy_git_references(root)?;
        initialize_history_order(metadata);
        metadata.git_history_refs_cleaned = true;
        changed = true;
    }

    let head = snapshot::head_commit(root);
    let mut tips = repository_tips(root)?;
    if let Some(head) = &head {
        tips.push(head.clone());
    }
    tips.sort();
    tips.dedup();

    let refs_changed = !metadata.git_history_imported || metadata.git_history_tips != tips;
    let head_changed = metadata.last_observed_head != head;
    if !refs_changed && !head_changed {
        return Ok(changed);
    }

    if refs_changed {
        let (page, has_more) = read_history_page(root, 0, HISTORY_PAGE_SIZE, head.is_some())?;
        add_history_page(metadata, &page, true);
        metadata.git_history_scan_offset = page.len();
        metadata.git_history_has_more = has_more;
        metadata.git_history_tips = tips;
        metadata.git_history_imported = true;
    }

    // Native Git history is read-only context. It must never replace the
    // MosSaver checkpoint used to calculate current file changes.
    metadata.last_observed_head = head;
    Ok(true)
}

fn latest_mossaver_checkpoint(metadata: &Metadata) -> Option<&Checkpoint> {
    metadata
        .checkpoints
        .iter()
        .rev()
        .find(|checkpoint| checkpoint.kind != CheckpointKind::Git)
}

pub(super) fn load_next_history_page(root: &Path, metadata: &mut Metadata) -> AppResult<bool> {
    if !metadata.git_history_has_more {
        return Ok(false);
    }
    let head = snapshot::head_commit(root);
    let (page, has_more) = read_history_page(
        root,
        metadata.git_history_scan_offset,
        HISTORY_PAGE_SIZE,
        head.is_some(),
    )?;
    if page.is_empty() {
        metadata.git_history_has_more = false;
        return Ok(true);
    }
    add_history_page(metadata, &page, false);
    metadata.git_history_scan_offset += page.len();
    metadata.git_history_has_more = has_more;
    Ok(true)
}

fn read_history_page(
    root: &Path,
    offset: usize,
    limit: usize,
    include_head: bool,
) -> AppResult<(Vec<Checkpoint>, bool)> {
    let skip = format!("--skip={offset}");
    let max_count = format!("--max-count={}", limit + 1);
    let mut args = vec![
        "log",
        "--branches",
        "--remotes",
        "--tags",
        HISTORY_FORMAT,
        skip.as_str(),
        max_count.as_str(),
    ];
    if include_head {
        args.push("HEAD");
    }
    let output = commands::run_git(root, &args)?;
    let mut page = output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(parse_history_line)
        .collect::<AppResult<Vec<_>>>()?;
    let has_more = page.len() > limit;
    page.truncate(limit);
    Ok((page, has_more))
}

fn parse_history_line(line: &[u8]) -> AppResult<Checkpoint> {
    let entry: Vec<&[u8]> = line.split(|byte| *byte == 0).collect();
    if entry.len() != 3 {
        return Err(AppError::Git(
            "Git returned incomplete commit history".into(),
        ));
    }
    let commit = String::from_utf8(entry[0].to_vec()).map_err(|_| AppError::InvalidPath)?;
    let created_at = String::from_utf8(entry[1].to_vec()).map_err(|_| AppError::InvalidPath)?;
    Ok(Checkpoint {
        id: format!("git-{commit}"),
        label: nonempty_subject(entry[2]),
        created_at,
        kind: CheckpointKind::Git,
        parent_id: None,
        merge_parent_ids: Vec::new(),
        summary: None,
        delete_blocked_reason: None,
    })
}

fn add_history_page(metadata: &mut Metadata, page: &[Checkpoint], prepend: bool) {
    let page_ids: Vec<String> = page
        .iter()
        .map(|checkpoint| checkpoint.id.clone())
        .collect();
    for checkpoint in page {
        if metadata
            .checkpoints
            .iter()
            .any(|saved| saved.id == checkpoint.id)
        {
            continue;
        }
        metadata.checkpoints.push(checkpoint.clone());
    }
    if prepend {
        metadata
            .git_history_order
            .retain(|id| !page_ids.contains(id));
        metadata.git_history_order.splice(0..0, page_ids);
    } else {
        for id in page_ids {
            if !metadata.git_history_order.contains(&id) {
                metadata.git_history_order.push(id);
            }
        }
    }
}

fn initialize_history_order(metadata: &mut Metadata) {
    if !metadata.git_history_order.is_empty() {
        return;
    }
    let mut git_checkpoints: Vec<&Checkpoint> = metadata
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.kind == CheckpointKind::Git)
        .collect();
    git_checkpoints.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    metadata.git_history_order = git_checkpoints
        .into_iter()
        .map(|checkpoint| checkpoint.id.clone())
        .collect();
}

fn remove_legacy_git_references(root: &Path) -> AppResult<()> {
    let output = commands::run_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/easilygit/checkpoints",
        ],
    )?;
    let references: Vec<String> = String::from_utf8_lossy(&output)
        .lines()
        .filter(|reference| reference.starts_with("refs/easilygit/checkpoints/git-"))
        .map(str::to_owned)
        .collect();
    commands::delete_references(root, &references)
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
