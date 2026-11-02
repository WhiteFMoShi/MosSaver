use std::{ffi::OsString, path::Path};

use easilygit_domain::{AppError, AppResult, ChangeKind, ChangedFile};
use tempfile::tempdir;

use super::commands;

pub(super) fn current_changes(root: &Path, base: Option<&str>) -> AppResult<Vec<ChangedFile>> {
    let temporary = tempdir().map_err(|error| AppError::Internal(error.to_string()))?;
    let index_path = temporary.path().join("status-index");
    let index_env = commands::index_env(&index_path);
    let envs = [index_env];

    if let Some(base) = base {
        commands::run_git_with_env(root, &["read-tree", base], &envs)?;
    } else {
        commands::run_git_with_env(root, &["read-tree", "--empty"], &envs)?;
    }
    commands::run_git_with_env(root, &["add", "--all", "--", "."], &envs)?;
    let tree = output_text(root, &["write-tree"], &envs)?;
    let empty_tree;
    let base = match base {
        Some(base) => base,
        None => {
            empty_tree = output_text(root, &["mktree"], &[])?;
            &empty_tree
        }
    };

    let diff = commands::run_git(
        root,
        &["diff", "--no-renames", "--name-status", "-z", base, &tree],
    )?;
    parse_changes(&diff)
}

fn output_text(root: &Path, args: &[&str], envs: &[(OsString, OsString)]) -> AppResult<String> {
    let output = commands::run_git_with_env(root, args, envs)?;
    String::from_utf8(output)
        .map(|value| value.trim().to_owned())
        .map_err(|_| AppError::Git("Git returned invalid text output".into()))
}

fn parse_changes(diff: &[u8]) -> AppResult<Vec<ChangedFile>> {
    let mut fields = diff.split(|byte| *byte == 0);
    let mut changes = Vec::new();

    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        let path = fields
            .next()
            .ok_or_else(|| AppError::Git("Git returned an incomplete file change".into()))?;
        let kind = match status.first() {
            Some(b'A') => ChangeKind::Added,
            Some(b'D') => ChangeKind::Deleted,
            Some(b'M' | b'T') => ChangeKind::Modified,
            _ => return Err(AppError::Git("Git returned an unknown file change".into())),
        };
        changes.push(ChangedFile {
            path: String::from_utf8_lossy(path).into_owned(),
            kind,
        });
    }

    changes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(changes)
}
