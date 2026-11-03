use std::{fs, path::Path, process::Command};

use easilygit_app::ProjectService;
use easilygit_domain::{CheckpointDeleteBlock, CheckpointKind, ProjectState};
use easilygit_git::SystemGitBackend;
use tempfile::tempdir;

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn store(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(root.join(".git/easilygit/store.git"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn initialize(root: &Path) -> (ProjectService<SystemGitBackend>, String) {
    fs::write(root.join("left.txt"), "left base\n").unwrap();
    fs::write(root.join("right.txt"), "right base\n").unwrap();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "Native base"]);
    let service = ProjectService::new(SystemGitBackend);
    let initial = service
        .enable_protection(root)
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    (service, initial)
}

fn fork(
    service: &ProjectService<SystemGitBackend>,
    root: &Path,
    initial: &str,
    conflict: bool,
) -> (String, String) {
    fs::write(root.join("left.txt"), "left branch\n").unwrap();
    let left = service
        .save_version(root, "Left branch")
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    service.restore(root, initial).unwrap();
    fs::write(
        root.join(if conflict { "left.txt" } else { "right.txt" }),
        "right branch\n",
    )
    .unwrap();
    let right = service
        .save_version(root, "Right branch")
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    (left, right)
}

fn merged_id(state: &ProjectState) -> String {
    state
        .checkpoints
        .iter()
        .find(|checkpoint| !checkpoint.merge_parent_ids.is_empty())
        .unwrap()
        .id
        .clone()
}

#[test]
fn merging_siblings_preserves_two_parents_worktree_native_git_and_unsaved_changes() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, initial) = initialize(root);
    let (left, right) = fork(&service, root, &initial, false);
    fs::write(root.join("unsaved.txt"), "do not lose me\n").unwrap();
    git(root, &["add", "unsaved.txt"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let head = git(root, &["rev-parse", "HEAD"]);
    let refs = git(root, &["show-ref"]);
    let before = service.refresh(root).unwrap();
    let merged = service
        .merge_checkpoints(root, &left, &right, "Combined")
        .unwrap();
    let merged_id = merged_id(&merged);
    let checkpoint = merged
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.id == merged_id)
        .unwrap();
    assert_eq!(checkpoint.parent_id.as_deref(), Some(right.as_str()));
    assert_eq!(checkpoint.merge_parent_ids, vec![left.clone()]);
    assert_eq!(checkpoint.summary.as_ref().unwrap().modified, 1);
    assert_eq!(merged.current_checkpoint_id, before.current_checkpoint_id);
    assert_eq!(merged.current_changes, before.current_changes);
    assert_eq!(
        fs::read_to_string(root.join("left.txt")).unwrap(),
        "left base\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("unsaved.txt")).unwrap(),
        "do not lose me\n"
    );
    let parents = store(
        root,
        &[
            "rev-list",
            "--parents",
            "-n",
            "1",
            &format!("refs/easilygit/checkpoints/{merged_id}"),
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&parents).split_whitespace().count(),
        3
    );
    let target_commit = store(
        root,
        &["rev-parse", &format!("refs/easilygit/checkpoints/{right}")],
    );
    let source_commit = store(
        root,
        &["rev-parse", &format!("refs/easilygit/checkpoints/{left}")],
    );
    assert_eq!(
        String::from_utf8_lossy(&parents)
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>(),
        vec![
            String::from_utf8_lossy(&target_commit).trim(),
            String::from_utf8_lossy(&source_commit).trim()
        ]
    );
    let reopened = service.open_project(root).unwrap();
    assert_eq!(
        reopened
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == merged_id)
            .unwrap()
            .merge_parent_ids,
        vec![left]
    );
    let preview = service.file_preview(root, &merged_id, "left.txt").unwrap();
    assert!(preview.patch.contains("+left branch"));
    let restored = service.restore(root, &merged_id).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("left.txt")).unwrap(),
        "left branch\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("right.txt")).unwrap(),
        "right branch\n"
    );
    assert!(restored.current_changes.is_empty());
    service.undo_restore(root).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("unsaved.txt")).unwrap(),
        "do not lose me\n"
    );
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(root, &["show-ref"]), refs);
}

#[test]
fn a_conflicting_merge_changes_neither_metadata_refs_nor_files() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, initial) = initialize(root);
    let (left, right) = fork(&service, root, &initial, true);
    let metadata = fs::read(root.join(".git/easilygit/metadata.json")).unwrap();
    let refs = store(root, &["show-ref"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let error = service
        .merge_checkpoints(root, &left, &right, "Conflict")
        .unwrap_err();
    assert_eq!(error.code(), "MERGE_CONFLICT");
    assert_eq!(
        fs::read(root.join(".git/easilygit/metadata.json")).unwrap(),
        metadata
    );
    assert_eq!(store(root, &["show-ref"]), refs);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(root.join("left.txt")).unwrap(),
        "right branch\n"
    );
}

#[test]
fn merge_rejects_identical_ancestral_unknown_and_native_git_versions() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, initial) = initialize(root);
    let (left, right) = fork(&service, root, &initial, false);
    for (source, target) in [(&left, &left), (&initial, &right), (&right, &initial)] {
        assert_eq!(
            service
                .merge_checkpoints(root, source, target, "Invalid")
                .unwrap_err()
                .code(),
            "MERGE_NOT_ALLOWED"
        );
    }
    assert_eq!(
        service
            .merge_checkpoints(root, "unknown", &right, "Invalid")
            .unwrap_err()
            .code(),
        "CHECKPOINT_NOT_FOUND"
    );
    let state = service.open_project(root).unwrap();
    let native = state
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.kind == CheckpointKind::Git)
        .unwrap();
    assert_eq!(
        service
            .merge_checkpoints(root, &native.id, &right, "Invalid")
            .unwrap_err()
            .code(),
        "CHECKPOINT_READ_ONLY"
    );
    assert_eq!(
        service
            .delete_checkpoint(root, &native.id)
            .unwrap_err()
            .code(),
        "CHECKPOINT_READ_ONLY"
    );
}

#[test]
fn inactive_leaf_can_be_deleted_but_current_ancestors_merge_sources_and_recovery_points_are_protected()
 {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, initial) = initialize(root);
    let (left, right) = fork(&service, root, &initial, false);
    let before_merge = service.open_project(root).unwrap();
    // A hidden safety checkpoint still references this visible leaf.
    assert!(
        !before_merge
            .checkpoints
            .iter()
            .any(|checkpoint| checkpoint.parent_id.as_deref() == Some(left.as_str()))
    );
    assert_eq!(
        before_merge
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == left)
            .unwrap()
            .delete_blocked_reason,
        Some(CheckpointDeleteBlock::HasChildren)
    );
    assert_eq!(
        before_merge
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == right)
            .unwrap()
            .delete_blocked_reason,
        Some(CheckpointDeleteBlock::Current)
    );
    assert_eq!(
        before_merge
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.kind == CheckpointKind::Git)
            .unwrap()
            .delete_blocked_reason,
        Some(CheckpointDeleteBlock::ReadOnly)
    );
    let state = service
        .merge_checkpoints(root, &left, &right, "Combined")
        .unwrap();
    let merged = merged_id(&state);
    assert_eq!(
        state
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == merged)
            .unwrap()
            .delete_blocked_reason,
        None
    );
    for id in [&initial, &left, &right] {
        assert_eq!(
            service.delete_checkpoint(root, id).unwrap_err().code(),
            "CHECKPOINT_IN_USE"
        );
    }
    let before_refs = git(root, &["show-ref"]);
    let state = service.delete_checkpoint(root, &merged).unwrap();
    assert!(
        !state
            .checkpoints
            .iter()
            .any(|checkpoint| checkpoint.id == merged)
    );
    assert!(!String::from_utf8_lossy(&store(root, &["show-ref"])).contains(&merged));
    assert_eq!(git(root, &["show-ref"]), before_refs);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(".git/easilygit/metadata.json")).unwrap())
            .unwrap();
    let backup = metadata["last_restore_backup"].as_str().unwrap();
    assert_eq!(
        service.delete_checkpoint(root, backup).unwrap_err().code(),
        "CHECKPOINT_IN_USE"
    );
    service.undo_restore(root).unwrap();
}

#[test]
fn merge_restore_preserves_files_excluded_after_the_source_versions_were_saved() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, initial) = initialize(root);
    let (left, right) = fork(&service, root, &initial, false);
    service.set_file_managed(root, "left.txt", false).unwrap();
    fs::write(root.join("left.txt"), "local excluded content\n").unwrap();
    let state = service
        .merge_checkpoints(root, &left, &right, "Combined")
        .unwrap();
    service.restore(root, &merged_id(&state)).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("left.txt")).unwrap(),
        "local excluded content\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("right.txt")).unwrap(),
        "right branch\n"
    );
}

#[test]
fn previews_show_add_modify_delete_and_initial_versions_without_touching_files() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("root.txt"), "initial line\n").unwrap();
    let service = ProjectService::new(SystemGitBackend);
    let initial = service
        .enable_protection(root)
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    assert!(
        service
            .file_preview(root, &initial, "root.txt")
            .unwrap()
            .patch
            .contains("+initial line")
    );
    fs::write(root.join("root.txt"), "changed line\n").unwrap();
    fs::write(root.join("added file 中文.txt"), "new file\n").unwrap();
    let modified = service
        .save_version(root, "Modified")
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    let preview = service.file_preview(root, &modified, "root.txt").unwrap();
    assert!(preview.patch.contains("-initial line"));
    assert!(preview.patch.contains("+changed line"));
    assert!(
        service
            .file_preview(root, &modified, "added file 中文.txt")
            .unwrap()
            .patch
            .contains("+new file")
    );
    fs::remove_file(root.join("root.txt")).unwrap();
    let deleted = service
        .save_version(root, "Deleted")
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    assert!(
        service
            .file_preview(root, &deleted, "root.txt")
            .unwrap()
            .patch
            .contains("-changed line")
    );
    assert!(!root.join("root.txt").exists());
}

#[test]
fn previews_limit_large_binary_and_long_patches_and_treat_pathspec_characters_literally() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("binary.bin"), [0, 1, 2, 3]).unwrap();
    fs::write(root.join("non-utf8.txt"), [0xff, 0xfe, b'a']).unwrap();
    fs::write(root.join("large.txt"), vec![b'a'; 512 * 1024 + 1]).unwrap();
    fs::write(root.join("long.txt"), "line\n".repeat(1500)).unwrap();
    fs::write(root.join("[ab].txt"), "literal filename\n").unwrap();
    fs::write(root.join("a.txt"), "another filename\n").unwrap();
    #[cfg(unix)]
    fs::write(root.join(":(glob)*"), "special pathspec filename\n").unwrap();
    let service = ProjectService::new(SystemGitBackend);
    let id = service
        .enable_protection(root)
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    for path in ["binary.bin", "large.txt", "non-utf8.txt"] {
        let preview = service.file_preview(root, &id, path).unwrap();
        assert!(preview.unavailable_reason.is_some());
        assert!(preview.patch.is_empty());
    }
    let long = service.file_preview(root, &id, "long.txt").unwrap();
    assert!(long.truncated);
    assert!(long.patch.lines().count() <= 1200);
    let literal = service.file_preview(root, &id, "[ab].txt").unwrap();
    assert!(literal.patch.contains("+literal filename"));
    assert!(!literal.patch.contains("+another filename"));
    #[cfg(unix)]
    assert!(
        service
            .file_preview(root, &id, ":(glob)*")
            .unwrap()
            .patch
            .contains("+special pathspec filename")
    );
    for path in ["../outside", "/etc/passwd", "missing.txt"] {
        assert_eq!(
            service.file_preview(root, &id, path).unwrap_err().code(),
            "INVALID_PATH"
        );
    }
}

#[test]
fn native_git_diff_preview_does_not_modify_native_index_or_objects() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, _) = initialize(root);
    let state = service.open_project(root).unwrap();
    let native = state
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.kind == CheckpointKind::Git)
        .unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    let objects = git(root, &["count-objects", "-v"]);
    assert!(
        service
            .file_preview(root, &native.id, "left.txt")
            .unwrap()
            .patch
            .contains("+left base")
    );
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(git(root, &["count-objects", "-v"]), objects);
}

#[test]
fn merge_combines_non_overlapping_edits_in_the_same_file_and_file_additions_and_deletions() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, _) = initialize(root);
    let base = (1..=20)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    fs::write(root.join("shared.txt"), &base).unwrap();
    let initial = service
        .save_version(root, "Common base")
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    fs::write(
        root.join("shared.txt"),
        base.replacen("line 1\n", "edited first line\n", 1),
    )
    .unwrap();
    fs::write(root.join("new.txt"), "from left\n").unwrap();
    let left = service
        .save_version(root, "Left edits")
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    service.restore(root, &initial).unwrap();
    fs::write(
        root.join("shared.txt"),
        base.replace("line 20\n", "edited last line\n"),
    )
    .unwrap();
    fs::remove_file(root.join("right.txt")).unwrap();
    let right = service
        .save_version(root, "Right edits")
        .unwrap()
        .current_checkpoint_id
        .unwrap();
    let state = service
        .merge_checkpoints(root, &left, &right, "Combined text edits")
        .unwrap();
    let merged = merged_id(&state);
    let details = service.checkpoint_details(root, &merged).unwrap();
    assert_eq!(details.summary.added, 1);
    assert_eq!(details.summary.modified, 1);
    service.restore(root, &merged).unwrap();
    let text = fs::read_to_string(root.join("shared.txt")).unwrap();
    assert!(text.starts_with("edited first line\n"));
    assert!(text.ends_with("edited last line\n"));
    assert_eq!(
        fs::read_to_string(root.join("new.txt")).unwrap(),
        "from left\n"
    );
    assert!(!root.join("right.txt").exists());
}

#[test]
fn concurrent_commands_on_service_clones_do_not_lose_checkpoints() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    let (service, _) = initialize(root);
    std::thread::scope(|scope| {
        for index in 0..6 {
            let service = service.clone();
            scope.spawn(move || {
                service
                    .save_version(root, &format!("Concurrent {index}"))
                    .unwrap();
            });
        }
    });
    let state = service.open_project(root).unwrap();
    let saved: Vec<_> = state
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.label.starts_with("Concurrent"))
        .collect();
    assert_eq!(saved.len(), 6);
    let mut current = state.current_checkpoint_id.as_deref();
    let mut count = 0;
    while let Some(id) = current {
        let checkpoint = state
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == id)
            .unwrap();
        if !checkpoint.label.starts_with("Concurrent") {
            break;
        }
        count += 1;
        current = checkpoint.parent_id.as_deref();
    }
    assert_eq!(count, 6);
}
