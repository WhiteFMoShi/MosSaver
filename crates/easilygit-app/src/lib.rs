use std::path::{Path, PathBuf};

use easilygit_domain::{
    AppError, AppResult, ChangedFile, Checkpoint, CheckpointKind, ProjectState,
};

pub trait ProjectPort: Send + Sync + 'static {
    fn repository_root(&self, path: &Path) -> AppResult<Option<PathBuf>>;
    fn initialize_repository(&self, path: &Path) -> AppResult<()>;
    fn is_protected(&self, root: &Path) -> AppResult<bool>;
    fn current_changes(&self, root: &Path) -> AppResult<Vec<ChangedFile>>;
    fn checkpoints(&self, root: &Path) -> AppResult<Vec<Checkpoint>>;
    fn create_checkpoint(&self, root: &Path, label: &str, automatic: bool)
    -> AppResult<Checkpoint>;
    fn restore_checkpoint(&self, root: &Path, id: &str) -> AppResult<()>;
    fn undo_restore(&self, root: &Path) -> AppResult<()>;
    fn can_undo_restore(&self, root: &Path) -> AppResult<bool>;
}

#[derive(Clone)]
pub struct ProjectService<P> {
    port: P,
}

impl<P: ProjectPort> ProjectService<P> {
    pub fn new(port: P) -> Self {
        Self { port }
    }

    pub fn open_project(&self, path: &Path) -> AppResult<ProjectState> {
        let selected = validate_folder(path)?;
        let root = self.port.repository_root(&selected)?.unwrap_or(selected);
        self.state_for_root(root)
    }

    pub fn enable_protection(&self, path: &Path) -> AppResult<ProjectState> {
        let selected = validate_folder(path)?;
        if self.port.repository_root(&selected)?.is_none() {
            self.port.initialize_repository(&selected)?;
        }
        let root = self
            .port
            .repository_root(&selected)?
            .ok_or(AppError::NotARepository)?;
        if !self
            .port
            .checkpoints(&root)?
            .iter()
            .any(|checkpoint| checkpoint.kind == CheckpointKind::User)
        {
            self.port.create_checkpoint(&root, "项目初始版本", false)?;
        }
        self.state_for_root(root)
    }

    pub fn refresh(&self, path: &Path) -> AppResult<ProjectState> {
        let root = self.protected_root(path)?;
        self.state_for_root(root)
    }

    pub fn save_version(&self, path: &Path, label: &str) -> AppResult<ProjectState> {
        let root = self.protected_root(path)?;
        self.port.create_checkpoint(&root, label, false)?;
        self.state_for_root(root)
    }

    pub fn restore(&self, path: &Path, id: &str) -> AppResult<ProjectState> {
        let root = self.protected_root(path)?;
        self.port.restore_checkpoint(&root, id)?;
        self.state_for_root(root)
    }

    pub fn undo_restore(&self, path: &Path) -> AppResult<ProjectState> {
        let root = self.protected_root(path)?;
        self.port.undo_restore(&root)?;
        self.state_for_root(root)
    }

    fn protected_root(&self, path: &Path) -> AppResult<PathBuf> {
        let selected = validate_folder(path)?;
        let root = self
            .port
            .repository_root(&selected)?
            .ok_or(AppError::NotInitialized)?;
        if !self.port.is_protected(&root)? {
            return Err(AppError::NotInitialized);
        }
        Ok(root)
    }

    fn state_for_root(&self, root: PathBuf) -> AppResult<ProjectState> {
        let repository_root = self.port.repository_root(&root)?;
        let initialized = match &repository_root {
            Some(repository_root) => self.port.is_protected(repository_root)?,
            None => false,
        };
        let is_git_repository = repository_root.is_some();
        let root = repository_root.unwrap_or(root);
        let name = root
            .file_name()
            .map(|part| part.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());

        if !initialized {
            return Ok(ProjectState {
                root: root.to_string_lossy().into_owned(),
                name,
                initialized: false,
                is_git_repository,
                current_changes: Vec::new(),
                checkpoints: Vec::new(),
                can_undo_restore: false,
            });
        }

        let checkpoints = self
            .port
            .checkpoints(&root)?
            .into_iter()
            .filter(|checkpoint| checkpoint.kind != CheckpointKind::Automatic)
            .collect();

        Ok(ProjectState {
            root: root.to_string_lossy().into_owned(),
            name,
            initialized: true,
            is_git_repository,
            current_changes: self.port.current_changes(&root)?,
            checkpoints,
            can_undo_restore: self.port.can_undo_restore(&root)?,
        })
    }
}

fn validate_folder(path: &Path) -> AppResult<PathBuf> {
    let canonical = path.canonicalize().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => AppError::ProjectNotFound,
        std::io::ErrorKind::PermissionDenied => AppError::PermissionDenied,
        _ => AppError::ProjectNotFound,
    })?;
    if !canonical.is_dir() {
        return Err(AppError::ProjectNotFound);
    }
    Ok(canonical)
}
