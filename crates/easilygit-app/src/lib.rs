use std::path::{Path, PathBuf};

use easilygit_domain::{
    AppError, AppResult, ChangedFile, Checkpoint, CheckpointDetails, CheckpointPage, ManagedFile,
    ProjectOverview, ProjectState,
};

const CHECKPOINT_PAGE_SIZE: usize = 50;

pub trait ProjectPort: Send + Sync + 'static {
    fn repository_root(&self, path: &Path) -> AppResult<Option<PathBuf>>;
    fn initialize_repository(&self, path: &Path) -> AppResult<()>;
    fn is_protected(&self, root: &Path) -> AppResult<bool>;
    fn current_changes(&self, root: &Path) -> AppResult<Vec<ChangedFile>>;
    fn checkpoints_page(
        &self,
        root: &Path,
        offset: usize,
        limit: usize,
    ) -> AppResult<CheckpointPage>;
    fn checkpoint_details(&self, root: &Path, id: &str) -> AppResult<CheckpointDetails>;
    fn current_checkpoint_id(&self, root: &Path) -> AppResult<Option<String>>;
    fn has_user_checkpoint(&self, root: &Path) -> AppResult<bool>;
    fn create_checkpoint(&self, root: &Path, label: &str, automatic: bool)
    -> AppResult<Checkpoint>;
    fn restore_checkpoint(&self, root: &Path, id: &str) -> AppResult<()>;
    fn undo_restore(&self, root: &Path) -> AppResult<()>;
    fn can_undo_restore(&self, root: &Path) -> AppResult<bool>;
    fn managed_files(&self, root: &Path) -> AppResult<Vec<ManagedFile>>;
    fn set_file_managed(&self, root: &Path, file_path: &str, managed: bool) -> AppResult<()>;
    fn disable_protection(&self, root: &Path) -> AppResult<()>;
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

    pub fn project_overview(&self, path: &Path) -> AppResult<ProjectOverview> {
        let selected = validate_folder(path)?;
        let repository_root = self.port.repository_root(&selected)?;
        let initialized = match &repository_root {
            Some(root) => self.port.is_protected(root)?,
            None => false,
        };
        let is_git_repository = repository_root.is_some();
        let root = repository_root.unwrap_or(selected);
        let name = root
            .file_name()
            .map(|part| part.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());
        let change_count = if initialized {
            self.port.current_changes(&root)?.len()
        } else {
            0
        };

        Ok(ProjectOverview {
            root: root.to_string_lossy().into_owned(),
            name,
            initialized,
            is_git_repository,
            change_count,
        })
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
        if !self.port.has_user_checkpoint(&root)? {
            self.port
                .create_checkpoint(&root, "开始使用 MosSaver", false)?;
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

    pub fn load_more_checkpoints(&self, path: &Path, offset: usize) -> AppResult<CheckpointPage> {
        let root = self.protected_root(path)?;
        self.port
            .checkpoints_page(&root, offset, CHECKPOINT_PAGE_SIZE)
    }

    pub fn checkpoint_details(&self, path: &Path, id: &str) -> AppResult<CheckpointDetails> {
        let root = self.protected_root(path)?;
        self.port.checkpoint_details(&root, id)
    }

    pub fn managed_files(&self, path: &Path) -> AppResult<Vec<ManagedFile>> {
        let root = self.protected_root(path)?;
        self.port.managed_files(&root)
    }

    pub fn set_file_managed(
        &self,
        path: &Path,
        file_path: &str,
        managed: bool,
    ) -> AppResult<ProjectState> {
        let root = self.protected_root(path)?;
        self.port.set_file_managed(&root, file_path, managed)?;
        self.state_for_root(root)
    }

    pub fn disable_protection(&self, path: &Path) -> AppResult<ProjectState> {
        let root = self.protected_root(path)?;
        self.port.disable_protection(&root)?;
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
                has_more_checkpoints: false,
                current_checkpoint_id: None,
                can_undo_restore: false,
            });
        }

        let page = self.port.checkpoints_page(&root, 0, CHECKPOINT_PAGE_SIZE)?;
        let current_checkpoint_id = self.port.current_checkpoint_id(&root)?;

        Ok(ProjectState {
            root: root.to_string_lossy().into_owned(),
            name,
            initialized: true,
            is_git_repository,
            current_changes: self.port.current_changes(&root)?,
            checkpoints: page.checkpoints,
            has_more_checkpoints: page.has_more,
            current_checkpoint_id,
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
