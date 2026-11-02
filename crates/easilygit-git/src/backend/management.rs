use std::{fs, path::Path};

use easilygit_domain::{AppError, AppResult};

use super::commands;

pub(super) fn disable_protection(root: &Path, metadata_path: &Path) -> AppResult<()> {
    // Ensure any EasilyGit refs left by older versions are migrated out of
    // the user's repository before removing the app-owned storage directory.
    let _ = commands::app_environment(root)?;
    let directory = metadata_path.parent().ok_or(AppError::InvalidPath)?;
    match fs::remove_dir_all(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(commands::map_io_error(&error)),
    }
    Ok(())
}
