mod commands;
mod history;
mod restore;
mod snapshot;
mod status;

use std::path::{Path, PathBuf};

use easilygit_app::ProjectPort;
use easilygit_domain::{AppError, AppResult, ChangedFile, Checkpoint};

use crate::metadata::{self, Metadata};

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemGitBackend;

impl SystemGitBackend {
    fn metadata_path(&self, root: &Path) -> AppResult<PathBuf> {
        let output = commands::run_git(
            root,
            &["rev-parse", "--git-path", "easilygit/metadata.json"],
        )?;
        let value = String::from_utf8(output).map_err(|_| AppError::InvalidPath)?;
        let value = PathBuf::from(value.trim());
        Ok(if value.is_absolute() {
            value
        } else {
            root.join(value)
        })
    }

    fn load_metadata(&self, root: &Path) -> AppResult<(PathBuf, Metadata)> {
        let path = self.metadata_path(root)?;
        let metadata = metadata::load(&path)?;
        Ok((path, metadata))
    }

    fn save_metadata(&self, path: &Path, metadata: &Metadata) -> AppResult<()> {
        metadata::save(path, metadata)
    }
}

impl ProjectPort for SystemGitBackend {
    fn repository_root(&self, path: &Path) -> AppResult<Option<PathBuf>> {
        let output = commands::output(path, &["rev-parse", "--show-toplevel"], &[])?;
        if !output.status.success() {
            return Ok(None);
        }
        let root = String::from_utf8(output.stdout).map_err(|_| AppError::InvalidPath)?;
        let root = PathBuf::from(root.trim());
        root.canonicalize()
            .map(Some)
            .map_err(|error| commands::map_io_error(&error))
    }

    fn initialize_repository(&self, path: &Path) -> AppResult<()> {
        commands::run_git(path, &["init", "--quiet"])?;
        Ok(())
    }

    fn is_protected(&self, root: &Path) -> AppResult<bool> {
        let path = self.metadata_path(root)?;
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(AppError::PermissionDenied)
            }
            Err(error) => Err(AppError::Metadata(error.to_string())),
        }
    }

    fn current_changes(&self, root: &Path) -> AppResult<Vec<ChangedFile>> {
        let (_, metadata) = self.load_metadata(root)?;
        let mut checkpoint_id = match metadata.current_checkpoint {
            Some(id) if commands::reference_exists(root, &id)? => Some(id),
            _ => None,
        };
        if checkpoint_id.is_none() {
            let mut checkpoints = Vec::new();
            for checkpoint in metadata.checkpoints {
                if commands::reference_exists(root, &checkpoint.id)? {
                    checkpoints.push(checkpoint);
                }
            }
            checkpoint_id = checkpoints
                .iter()
                .max_by(|left, right| {
                    left.created_at
                        .cmp(&right.created_at)
                        .then_with(|| left.id.cmp(&right.id))
                })
                .map(|checkpoint| checkpoint.id.clone());
        }
        let base = checkpoint_id
            .map(|id| commands::checkpoint_ref(&id))
            .or_else(|| snapshot::head_commit(root));
        status::current_changes(root, base.as_deref())
    }

    fn checkpoints(&self, root: &Path) -> AppResult<Vec<Checkpoint>> {
        let (metadata_path, mut metadata) = self.load_metadata(root)?;
        if history::update_existing_history(root, &mut metadata)? {
            self.save_metadata(&metadata_path, &metadata)?;
        }
        let mut valid_checkpoints = Vec::with_capacity(metadata.checkpoints.len());
        for checkpoint in metadata.checkpoints {
            if commands::reference_exists(root, &checkpoint.id)? {
                valid_checkpoints.push(checkpoint);
            }
        }
        metadata.checkpoints = valid_checkpoints;
        metadata.checkpoints.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(metadata.checkpoints)
    }

    fn create_checkpoint(
        &self,
        root: &Path,
        label: &str,
        automatic: bool,
    ) -> AppResult<Checkpoint> {
        snapshot::create_checkpoint(self, root, label, automatic)
    }

    fn restore_checkpoint(&self, root: &Path, id: &str) -> AppResult<()> {
        restore::restore_checkpoint(self, root, id)
    }

    fn undo_restore(&self, root: &Path) -> AppResult<()> {
        restore::undo_restore(self, root)
    }

    fn can_undo_restore(&self, root: &Path) -> AppResult<bool> {
        let (_, metadata) = self.load_metadata(root)?;
        match metadata.last_restore_backup.as_deref() {
            Some(id) => commands::reference_exists(root, id),
            None => Ok(false),
        }
    }
}
