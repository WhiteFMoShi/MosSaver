use std::path::Path;

use easilygit_domain::{AppError, AppResult, CheckpointKind, FilePreview};

use super::{SystemGitBackend, commands, files, history};

const MAX_BLOB_BYTES: usize = 512 * 1024;
const MAX_PATCH_BYTES: usize = 160 * 1024;
const MAX_PATCH_LINES: usize = 1200;

pub(super) fn file_preview(
    backend: &SystemGitBackend,
    root: &Path,
    id: &str,
    file_path: &str,
) -> AppResult<FilePreview> {
    files::validate_relative_path(file_path)?;
    let (_, metadata) = backend.load_metadata(root)?;
    let checkpoint = metadata
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.id == id)
        .ok_or(AppError::CheckpointNotFound)?;
    let mut envs = if checkpoint.kind == CheckpointKind::Git {
        Vec::new()
    } else {
        commands::app_environment(root)?
    };
    // A filename such as :(glob)* must remain a filename, never a pathspec.
    envs.push(("GIT_LITERAL_PATHSPECS".into(), "1".into()));
    let commit = commands::checkpoint_commit_with_env(root, id, &envs)?
        .ok_or(AppError::CheckpointNotFound)?;
    let parent = history::first_parent(root, &commit, &envs)?;
    let mut preview = FilePreview {
        path: file_path.to_owned(),
        patch: String::new(),
        unavailable_reason: None,
        truncated: false,
    };
    let mut found = false;
    for version in [Some(commit.as_str()), parent.as_deref()]
        .into_iter()
        .flatten()
    {
        let entry =
            commands::run_git_with_env(root, &["ls-tree", "-z", version, "--", file_path], &envs)?;
        if entry.is_empty() {
            continue;
        }
        found = true;
        let header = entry
            .split(|byte| *byte == b'\t')
            .next()
            .unwrap_or_default();
        let header = String::from_utf8_lossy(header);
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || fields[1] != "blob" {
            preview.unavailable_reason = Some("文件夹或子模块不支持文本预览。".into());
            return Ok(preview);
        }
        let size = commands::run_git_with_env(root, &["cat-file", "-s", fields[2]], &envs)?;
        let size: usize = String::from_utf8_lossy(&size)
            .trim()
            .parse()
            .map_err(|_| AppError::Git("Invalid blob size".into()))?;
        if size > MAX_BLOB_BYTES {
            preview.unavailable_reason = Some("文件超过 512 KB，请使用外部编辑器查看。".into());
            return Ok(preview);
        }
        let blob = commands::run_git_with_env(root, &["cat-file", "blob", fields[2]], &envs)?;
        if blob.contains(&0) || std::str::from_utf8(&blob).is_err() {
            preview.unavailable_reason = Some("二进制文件或非 UTF-8 文本暂不支持差异预览。".into());
            return Ok(preview);
        }
    }
    if !found {
        return Err(AppError::InvalidPath);
    }
    let mut args = vec!["-c", "core.quotePath=false"];
    if let Some(parent) = &parent {
        args.extend(["diff", parent.as_str(), commit.as_str()]);
    } else {
        args.extend([
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            commit.as_str(),
        ]);
    }
    args.extend([
        "--patch",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "--unified=3",
        "--",
        file_path,
    ]);
    let patch = commands::run_git_with_env(root, &args, &envs)?;
    let patch = String::from_utf8_lossy(&patch);
    for (index, line) in patch.lines().enumerate() {
        if index >= MAX_PATCH_LINES || preview.patch.len() + line.len() + 1 > MAX_PATCH_BYTES {
            preview.truncated = true;
            break;
        }
        preview.patch.push_str(line);
        preview.patch.push('\n');
    }
    Ok(preview)
}
