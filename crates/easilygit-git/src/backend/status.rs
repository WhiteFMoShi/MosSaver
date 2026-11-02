use std::{ffi::OsString, path::Path};

use easilygit_domain::{AppError, AppResult, ChangeKind, ChangedFile};
use tempfile::tempdir;

use super::{commands, files};

pub(super) fn current_changes(
    root: &Path,
    base: Option<&str>,
    excluded_paths: &[String],
    app_env: &[(OsString, OsString)],
) -> AppResult<Vec<ChangedFile>> {
    let temporary = tempdir().map_err(|error| AppError::Internal(error.to_string()))?;
    let base_index_path = temporary.path().join("base-index");
    let current_index_path = temporary.path().join("current-index");
    let mut base_env = app_env.to_vec();
    base_env.push(commands::index_env(&base_index_path));
    let mut current_env = app_env.to_vec();
    current_env.push(commands::index_env(&current_index_path));

    if let Some(base) = base {
        commands::run_git_with_env(root, &["read-tree", base], &base_env)?;
        commands::run_git_with_env(root, &["read-tree", base], &current_env)?;
    } else {
        commands::run_git_with_env(root, &["read-tree", "--empty"], &base_env)?;
        commands::run_git_with_env(root, &["read-tree", "--empty"], &current_env)?;
    }
    commands::run_git_with_env(root, &["add", "--all", "--", "."], &current_env)?;
    files::apply_exclusions(root, &base_env, excluded_paths)?;
    files::apply_exclusions(root, &current_env, excluded_paths)?;
    let base_tree = output_text(root, &["write-tree"], &base_env)?;
    let current_tree = output_text(root, &["write-tree"], &current_env)?;

    let diff = commands::run_git_with_env(
        root,
        &[
            "diff",
            "--no-renames",
            "--name-status",
            "-z",
            &base_tree,
            &current_tree,
        ],
        &base_env,
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
