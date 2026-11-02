use std::{
    ffi::OsString,
    io,
    path::Path,
    process::{Command, Output},
};

use easilygit_domain::{AppError, AppResult};

pub(super) fn output(
    root: &Path,
    args: &[&str],
    envs: &[(OsString, OsString)],
) -> AppResult<Output> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX");
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => AppError::GitUnavailable,
        io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        _ => AppError::Git(error.to_string()),
    })
}

pub(super) fn run_git(root: &Path, args: &[&str]) -> AppResult<Vec<u8>> {
    run_git_with_env(root, args, &[])
}

pub(super) fn run_git_with_env(
    root: &Path,
    args: &[&str],
    envs: &[(OsString, OsString)],
) -> AppResult<Vec<u8>> {
    let output = output(root, args, envs)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if detail.to_ascii_lowercase().contains("permission denied") {
            Err(AppError::PermissionDenied)
        } else {
            Err(AppError::Git(if detail.is_empty() {
                format!("git exited with status {}", output.status)
            } else {
                detail
            }))
        }
    }
}

pub(super) fn index_env(path: &Path) -> (OsString, OsString) {
    (
        OsString::from("GIT_INDEX_FILE"),
        path.as_os_str().to_owned(),
    )
}

pub(super) fn identity_env() -> [(OsString, OsString); 4] {
    [
        (
            OsString::from("GIT_AUTHOR_NAME"),
            OsString::from("EasilyGit"),
        ),
        (
            OsString::from("GIT_AUTHOR_EMAIL"),
            OsString::from("checkpoint@easilygit.local"),
        ),
        (
            OsString::from("GIT_COMMITTER_NAME"),
            OsString::from("EasilyGit"),
        ),
        (
            OsString::from("GIT_COMMITTER_EMAIL"),
            OsString::from("checkpoint@easilygit.local"),
        ),
    ]
}

pub(super) fn reference_exists(root: &Path, id: &str) -> AppResult<bool> {
    let reference = checkpoint_ref(id);
    let output = output(root, &["show-ref", "--verify", "--quiet", &reference], &[])?;
    Ok(output.status.success())
}

pub(super) fn checkpoint_ref(id: &str) -> String {
    format!("refs/easilygit/checkpoints/{id}")
}

pub(super) fn map_io_error(error: &io::Error) -> AppError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        io::ErrorKind::NotFound => AppError::ProjectNotFound,
        _ => AppError::Git(error.to_string()),
    }
}
