use std::{fs, io::Write, path::Path};

use easilygit_domain::{AppError, AppResult, Checkpoint};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Metadata {
    #[serde(default = "metadata_version")]
    version: u32,
    #[serde(default)]
    pub git_history_imported: bool,
    #[serde(default)]
    pub git_history_tips: Vec<String>,
    #[serde(default)]
    pub last_observed_head: Option<String>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    pub current_checkpoint: Option<String>,
    #[serde(default)]
    pub last_restore_backup: Option<String>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            version: metadata_version(),
            git_history_imported: false,
            git_history_tips: Vec::new(),
            last_observed_head: None,
            checkpoints: Vec::new(),
            current_checkpoint: None,
            last_restore_backup: None,
        }
    }
}

fn metadata_version() -> u32 {
    2
}

pub(super) fn load(path: &Path) -> AppResult<Metadata> {
    match fs::read(path) {
        Ok(contents) => {
            serde_json::from_slice(&contents).map_err(|error| AppError::Metadata(error.to_string()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Metadata::default()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(AppError::PermissionDenied)
        }
        Err(error) => Err(AppError::Metadata(error.to_string())),
    }
}

pub(super) fn save(path: &Path, metadata: &Metadata) -> AppResult<()> {
    let parent = path.parent().ok_or(AppError::InvalidPath)?;
    fs::create_dir_all(parent).map_err(map_io_error)?;
    let contents = serde_json::to_vec_pretty(metadata)
        .map_err(|error| AppError::Metadata(error.to_string()))?;
    let mut temporary =
        NamedTempFile::new_in(parent).map_err(|error| AppError::Metadata(error.to_string()))?;
    temporary
        .write_all(&contents)
        .map_err(|error| AppError::Metadata(error.to_string()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| AppError::Metadata(error.to_string()))?;
    temporary
        .persist(path)
        .map_err(|error| AppError::Metadata(error.error.to_string()))?;
    Ok(())
}

fn map_io_error(error: std::io::Error) -> AppError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        _ => AppError::Metadata(error.to_string()),
    }
}
