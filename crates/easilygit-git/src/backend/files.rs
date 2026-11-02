use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use easilygit_domain::{AppError, AppResult, ManagedFile};

use crate::metadata::Metadata;

use super::commands;

pub(super) fn list_managed_files(root: &Path, metadata: &Metadata) -> AppResult<Vec<ManagedFile>> {
    let output = commands::run_git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    let mut paths: BTreeSet<String> = output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect();
    paths.extend(metadata.excluded_paths.iter().cloned());

    Ok(paths
        .into_iter()
        .map(|path| ManagedFile {
            managed: !is_excluded(&path, &metadata.excluded_paths),
            path,
        })
        .collect())
}

pub(super) fn set_managed(
    metadata: &mut Metadata,
    file_path: &str,
    managed: bool,
) -> AppResult<bool> {
    validate_relative_path(file_path)?;
    let was_excluded = is_excluded(file_path, &metadata.excluded_paths);
    if managed {
        metadata
            .excluded_paths
            .retain(|excluded| excluded != file_path);
    } else if !was_excluded {
        metadata.excluded_paths.push(file_path.to_owned());
        metadata.excluded_paths.sort();
    }
    Ok(was_excluded != !managed)
}

pub(super) fn apply_exclusions(
    root: &Path,
    envs: &[(std::ffi::OsString, std::ffi::OsString)],
    excluded_paths: &[String],
) -> AppResult<()> {
    for path in excluded_paths {
        validate_relative_path(path)?;
        commands::run_git_with_env(
            root,
            &[
                "rm",
                "--cached",
                "--force",
                "--ignore-unmatch",
                "--quiet",
                "--",
                path,
            ],
            envs,
        )?;
    }
    Ok(())
}

pub(super) fn is_excluded(file_path: &str, excluded_paths: &[String]) -> bool {
    excluded_paths.iter().any(|excluded| {
        file_path == excluded
            || file_path
                .strip_prefix(excluded)
                .is_some_and(|tail| tail.starts_with('/') || tail.starts_with('\\'))
    })
}

pub(super) fn validate_relative_path(path: &str) -> AppResult<()> {
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
