mod commands;
mod files;
mod history;
mod management;
mod restore;
mod snapshot;
mod status;

use std::path::{Path, PathBuf};

use easilygit_app::ProjectPort;
use easilygit_domain::{
    AppError, AppResult, ChangedFile, Checkpoint, CheckpointDetails, CheckpointKind,
    CheckpointPage, ManagedFile,
};

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
        let app_env = commands::app_environment(root)?;
        let current_checkpoint_id = metadata
            .current_checkpoint
            .as_deref()
            .filter(|id| !id.starts_with("git-"));
        let mut checkpoint = match current_checkpoint_id {
            Some(id) => commands::checkpoint_commit_with_env(root, id, &app_env)?,
            None => None,
        };
        if checkpoint.is_none() {
            for saved_checkpoint in metadata.checkpoints.iter().rev() {
                if saved_checkpoint.kind == CheckpointKind::Git {
                    continue;
                }
                if let Some(commit) =
                    commands::checkpoint_commit_with_env(root, &saved_checkpoint.id, &app_env)?
                {
                    checkpoint = Some(commit);
                    break;
                }
            }
        }
        let base = checkpoint.or_else(|| snapshot::head_commit(root));
        status::current_changes(root, base.as_deref(), &metadata.excluded_paths, &app_env)
    }

    fn checkpoints_page(
        &self,
        root: &Path,
        offset: usize,
        limit: usize,
    ) -> AppResult<CheckpointPage> {
        let (metadata_path, mut metadata) = self.load_metadata(root)?;
        let mut changed = history::update_existing_history(root, &mut metadata)?;
        changed |= history::hydrate_checkpoint_graph(root, &mut metadata)?;
        let required = offset.saturating_add(limit);
        let mut visible = valid_visible_checkpoints(root, &metadata)?;
        while visible.len() < required && metadata.git_history_has_more {
            if !history::load_next_history_page(root, &mut metadata)? {
                break;
            }
            changed = true;
            visible = valid_visible_checkpoints(root, &metadata)?;
        }
        if changed {
            self.save_metadata(&metadata_path, &metadata)?;
        }
        let has_more =
            visible.len() > offset.saturating_add(limit) || metadata.git_history_has_more;
        Ok(CheckpointPage {
            checkpoints: visible.into_iter().skip(offset).take(limit).collect(),
            has_more,
        })
    }

    fn checkpoint_details(&self, root: &Path, id: &str) -> AppResult<CheckpointDetails> {
        let (_, metadata) = self.load_metadata(root)?;
        let checkpoint = metadata
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == id)
            .ok_or(AppError::CheckpointNotFound)?;
        history::checkpoint_details(root, checkpoint)
    }

    fn current_checkpoint_id(&self, root: &Path) -> AppResult<Option<String>> {
        let (_, metadata) = self.load_metadata(root)?;
        Ok(metadata
            .current_checkpoint
            .filter(|id| !id.starts_with("git-")))
    }

    fn has_user_checkpoint(&self, root: &Path) -> AppResult<bool> {
        let (_, metadata) = self.load_metadata(root)?;
        let references = commands::checkpoint_reference_ids(root)?;
        Ok(metadata.checkpoints.iter().any(|checkpoint| {
            checkpoint.kind == CheckpointKind::User && references.contains(&checkpoint.id)
        }))
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

    fn managed_files(&self, root: &Path) -> AppResult<Vec<ManagedFile>> {
        let (_, metadata) = self.load_metadata(root)?;
        files::list_managed_files(root, &metadata)
    }

    fn set_file_managed(&self, root: &Path, file_path: &str, managed: bool) -> AppResult<()> {
        let (metadata_path, mut metadata) = self.load_metadata(root)?;
        if files::set_managed(&mut metadata, file_path, managed)? {
            self.save_metadata(&metadata_path, &metadata)?;
        }
        Ok(())
    }

    fn disable_protection(&self, root: &Path) -> AppResult<()> {
        let (metadata_path, _) = self.load_metadata(root)?;
        management::disable_protection(root, &metadata_path)
    }
}

fn valid_visible_checkpoints(
    root: &Path,
    metadata: &crate::metadata::Metadata,
) -> AppResult<Vec<Checkpoint>> {
    let references = commands::checkpoint_reference_ids(root)?;
    let history_order: std::collections::HashMap<&str, usize> = metadata
        .git_history_order
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect();
    let mut checkpoints = Vec::new();
    for checkpoint in &metadata.checkpoints {
        if checkpoint.kind != CheckpointKind::Automatic
            && (checkpoint.kind == CheckpointKind::Git || references.contains(&checkpoint.id))
        {
            checkpoints.push(checkpoint.clone());
        }
    }
    checkpoints.sort_by(|left, right| {
        if left.kind == CheckpointKind::Git && right.kind == CheckpointKind::Git {
            return history_order
                .get(left.id.as_str())
                .cmp(&history_order.get(right.id.as_str()));
        }
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(checkpoints)
}
