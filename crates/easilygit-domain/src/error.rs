use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("The selected folder does not exist.")]
    ProjectNotFound,
    #[error("Git is not installed or could not be started.")]
    GitUnavailable,
    #[error("This folder is not a Git repository.")]
    NotARepository,
    #[error("The project is not protected yet.")]
    NotInitialized,
    #[error("The requested checkpoint could not be found.")]
    CheckpointNotFound,
    #[error("Git history entries can be viewed but not restored by EasilyGit.")]
    CheckpointReadOnly,
    #[error("There is no restore operation to undo.")]
    NothingToUndo,
    #[error("A repository path or file name could not be represented safely.")]
    InvalidPath,
    #[error("Permission denied while accessing the project.")]
    PermissionDenied,
    #[error("The restore could not be completed safely: {0}")]
    RestoreFailed(String),
    #[error("Could not read or write EasilyGit metadata: {0}")]
    Metadata(String),
    #[error("Git operation failed: {0}")]
    Git(String),
    #[error("Unexpected internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProjectNotFound => "PROJECT_NOT_FOUND",
            Self::GitUnavailable => "GIT_NOT_INSTALLED",
            Self::NotARepository => "NOT_A_REPOSITORY",
            Self::NotInitialized => "NOT_INITIALIZED",
            Self::CheckpointNotFound => "CHECKPOINT_NOT_FOUND",
            Self::CheckpointReadOnly => "CHECKPOINT_READ_ONLY",
            Self::NothingToUndo => "NOTHING_TO_UNDO",
            Self::InvalidPath => "INVALID_PATH",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::RestoreFailed(_) => "RESTORE_FAILED",
            Self::Metadata(_) => "METADATA_ERROR",
            Self::Git(_) => "GIT_ERROR",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}
