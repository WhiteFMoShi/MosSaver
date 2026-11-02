use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    pub root: String,
    pub name: String,
    pub initialized: bool,
    pub is_git_repository: bool,
    pub current_changes: Vec<ChangedFile>,
    pub checkpoints: Vec<Checkpoint>,
    pub has_more_checkpoints: bool,
    pub current_checkpoint_id: Option<String>,
    pub can_undo_restore: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointDetails {
    pub summary: CheckpointSummary,
    pub files: Vec<ChangedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointSummary {
    pub file_count: usize,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOverview {
    pub root: String,
    pub name: String,
    pub initialized: bool,
    pub is_git_repository: bool,
    pub change_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointPage {
    pub checkpoints: Vec<Checkpoint>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub id: String,
    pub label: String,
    pub created_at: String,
    pub kind: CheckpointKind,
    /// The MosSaver version or read-only Git commit this version was created
    /// from. Native Git history entries leave this empty.
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub summary: Option<CheckpointSummary>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CheckpointKind {
    User,
    Git,
    Automatic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedFile {
    pub path: String,
    pub managed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}
