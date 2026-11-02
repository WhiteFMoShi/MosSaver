use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
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

pub(super) fn app_environment(root: &Path) -> AppResult<Vec<(OsString, OsString)>> {
    let store_path = git_path(root, "easilygit/store.git")?;
    let object_format = repository_object_format(root);
    if !store_path.join("HEAD").is_file() {
        let store = store_path.to_str().ok_or(AppError::InvalidPath)?;
        let mut args = vec!["init", "--bare", "--quiet"];
        if object_format != "sha1" {
            args.extend(["--object-format", object_format.as_str()]);
        }
        args.push(store);
        run_git(root, &args)?;
    }

    let environment = vec![
        (OsString::from("GIT_DIR"), store_path.as_os_str().to_owned()),
        (OsString::from("GIT_WORK_TREE"), root.as_os_str().to_owned()),
    ];

    let object_dir = git_path(root, "objects")?;
    let alternate_file = store_path.join("objects/info/alternates");
    let alternate = format!("{}\n", object_dir.display());
    let current_alternate = fs::read_to_string(&alternate_file).ok();
    if current_alternate.as_deref() != Some(alternate.as_str()) {
        fs::write(&alternate_file, alternate).map_err(map_storage_io)?;
    }
    sync_exclude_file(root, &store_path)?;

    let bare_config = output(root, &["config", "--bool", "core.bare"], &environment)?;
    if !bare_config.status.success() || bare_config.stdout.trim_ascii() != b"false" {
        run_git_with_env(root, &["config", "core.bare", "false"], &environment)?;
    }

    let store_format = output(root, &["rev-parse", "--show-object-format"], &environment)?;
    if store_format.status.success()
        && String::from_utf8_lossy(&store_format.stdout).trim() != object_format
    {
        return Err(AppError::Git(
            "EasilyGit storage uses a different Git object format from this repository.".into(),
        ));
    }

    migrate_legacy_references(root, &store_path, &environment)?;
    Ok(environment)
}

fn repository_object_format(root: &Path) -> String {
    output(root, &["rev-parse", "--show-object-format"], &[])
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|format| format.trim().to_owned())
        .filter(|format| !format.is_empty())
        .unwrap_or_else(|| "sha1".into())
}

fn sync_exclude_file(root: &Path, store_path: &Path) -> AppResult<()> {
    let source = git_path(root, "info/exclude")?;
    let destination = store_path.join("info/exclude");
    let contents = match fs::read(&source) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(map_storage_io(error)),
    };
    if fs::read(&destination).ok().as_deref() != Some(contents.as_slice()) {
        fs::write(destination, contents).map_err(map_storage_io)?;
    }
    Ok(())
}

pub(super) fn app_output(
    root: &Path,
    args: &[&str],
    extra_envs: &[(OsString, OsString)],
) -> AppResult<Output> {
    let mut envs = app_environment(root)?;
    envs.extend_from_slice(extra_envs);
    output(root, args, &envs)
}

pub(super) fn app_run_git(root: &Path, args: &[&str]) -> AppResult<Vec<u8>> {
    app_run_git_with_env(root, args, &[])
}

pub(super) fn app_run_git_with_env(
    root: &Path,
    args: &[&str],
    extra_envs: &[(OsString, OsString)],
) -> AppResult<Vec<u8>> {
    let output = app_output(root, args, extra_envs)?;
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
    let output = app_output(root, &["show-ref", "--verify", "--quiet", &reference], &[])?;
    Ok(output.status.success())
}

pub(super) fn checkpoint_reference_ids(root: &Path) -> AppResult<HashSet<String>> {
    let output = app_run_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/easilygit/checkpoints",
        ],
    )?;
    Ok(String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|reference| reference.strip_prefix("refs/easilygit/checkpoints/"))
        .map(str::to_owned)
        .collect())
}

pub(super) fn checkpoint_commit(root: &Path, id: &str) -> AppResult<Option<String>> {
    let reference = if let Some(commit) = id.strip_prefix("git-") {
        format!("{commit}^{{commit}}")
    } else {
        if !reference_exists(root, id)? {
            return Ok(None);
        }
        checkpoint_ref(id)
    };
    let output = if id.starts_with("git-") {
        output(root, &["rev-parse", "--verify", &reference], &[])?
    } else {
        app_output(root, &["rev-parse", "--verify", &reference], &[])?
    };
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .map(|value| Some(value.trim().to_owned()))
        .map_err(|_| AppError::Git("Git returned an invalid checkpoint id".into()))
}

pub(super) fn delete_references(root: &Path, references: &[String]) -> AppResult<()> {
    delete_references_with_env(root, references, &[])
}

pub(super) fn delete_references_with_env(
    root: &Path,
    references: &[String],
    envs: &[(OsString, OsString)],
) -> AppResult<()> {
    if references.is_empty() {
        return Ok(());
    }
    let mut command = Command::new("git");
    command
        .args(["update-ref", "--stdin"])
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => AppError::GitUnavailable,
        io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        _ => AppError::Git(error.to_string()),
    })?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| AppError::Git("Could not write Git reference transaction".into()))?;
        stdin
            .write_all(b"start\n")
            .map_err(|error| AppError::Git(error.to_string()))?;
        for reference in references {
            stdin
                .write_all(format!("delete {reference}\n").as_bytes())
                .map_err(|error| AppError::Git(error.to_string()))?;
        }
        stdin
            .write_all(b"prepare\ncommit\n")
            .map_err(|error| AppError::Git(error.to_string()))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| AppError::Git(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(AppError::Git(if detail.is_empty() {
            format!("git exited with status {}", output.status)
        } else {
            detail
        }))
    }
}

fn git_path(root: &Path, path: &str) -> AppResult<PathBuf> {
    let output = run_git(root, &["rev-parse", "--git-path", path])?;
    let value = String::from_utf8(output).map_err(|_| AppError::InvalidPath)?;
    let value = PathBuf::from(value.trim());
    let value = if value.is_absolute() {
        value
    } else {
        root.join(value)
    };
    if path == "objects" {
        value.canonicalize().map_err(map_storage_io)
    } else {
        Ok(value)
    }
}

fn migrate_legacy_references(
    root: &Path,
    store_path: &Path,
    app_env: &[(OsString, OsString)],
) -> AppResult<()> {
    let marker = store_path.join("easilygit-storage-v1");
    if marker.exists() {
        return Ok(());
    }

    let output = run_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/easilygit/checkpoints",
        ],
    )?;
    let mut source_references = Vec::new();
    for line in String::from_utf8_lossy(&output).lines() {
        let Some((reference, object)) = line.split_once(' ') else {
            continue;
        };
        source_references.push(reference.to_owned());
        if reference
            .strip_prefix("refs/easilygit/checkpoints/")
            .is_some_and(|id| !id.starts_with("git-"))
        {
            run_git_with_env(root, &["update-ref", reference, object], app_env)?;
        }
    }
    delete_references(root, &source_references)?;
    fs::write(marker, b"isolated\n").map_err(map_storage_io)
}

fn map_storage_io(error: io::Error) -> AppError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        _ => AppError::Metadata(error.to_string()),
    }
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
