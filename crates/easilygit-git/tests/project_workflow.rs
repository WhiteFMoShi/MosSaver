use std::{fs, path::Path, process::Command};

use easilygit_app::ProjectService;
use easilygit_domain::ChangeKind;
use easilygit_git::SystemGitBackend;
use tempfile::tempdir;

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git must be installed for integration tests");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn save_restore_and_undo_follow_the_saved_version() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "first version\n").unwrap();
    fs::write(root.join("to-remove.txt"), "remove me\n").unwrap();
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();

    let service = ProjectService::new(SystemGitBackend);
    let opened = service.open_project(root).unwrap();
    assert!(!opened.initialized);
    assert!(!opened.is_git_repository);

    let initial = service.enable_protection(root).unwrap();
    assert!(initial.initialized);
    assert_eq!(initial.checkpoints.len(), 1);
    assert!(initial.current_changes.is_empty());
    let initial_id = initial.checkpoints[0].id.clone();

    fs::write(root.join("notes.txt"), "edited after initial\n").unwrap();
    fs::remove_file(root.join("to-remove.txt")).unwrap();
    fs::write(root.join("added.txt"), "new file\n").unwrap();
    fs::write(root.join("ignored.txt"), "leave this alone\n").unwrap();

    let changed = service.refresh(root).unwrap();
    assert_eq!(changed.current_changes.len(), 3);
    assert!(
        changed
            .current_changes
            .iter()
            .any(|file| { file.path == "notes.txt" && file.kind == ChangeKind::Modified })
    );
    assert!(
        changed
            .current_changes
            .iter()
            .any(|file| { file.path == "to-remove.txt" && file.kind == ChangeKind::Deleted })
    );
    assert!(
        changed
            .current_changes
            .iter()
            .any(|file| { file.path == "added.txt" && file.kind == ChangeKind::Added })
    );
    assert!(
        changed
            .current_changes
            .iter()
            .all(|file| file.path != "ignored.txt")
    );

    let saved = service.save_version(root, "second version").unwrap();
    assert!(saved.current_changes.is_empty());
    assert_eq!(saved.checkpoints.len(), 2);

    fs::write(root.join("notes.txt"), "later work\n").unwrap();
    fs::write(root.join("later.txt"), "another file\n").unwrap();
    let before_restore = service.refresh(root).unwrap();
    assert_eq!(before_restore.current_changes.len(), 2);

    let restored = service.restore(root, &initial_id).unwrap();
    assert!(restored.current_changes.is_empty());
    assert!(restored.can_undo_restore);
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "first version\n"
    );
    assert!(!root.join("added.txt").exists());
    assert!(!root.join("later.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("ignored.txt")).unwrap(),
        "leave this alone\n"
    );

    let undone = service.undo_restore(root).unwrap();
    assert!(undone.current_changes.is_empty());
    assert!(!undone.can_undo_restore);
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "later work\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("later.txt")).unwrap(),
        "another file\n"
    );
}

#[test]
fn saving_a_version_does_not_change_the_users_git_index() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "first version\n").unwrap();
    let service = ProjectService::new(SystemGitBackend);
    service.enable_protection(root).unwrap();

    git(root, &["add", "notes.txt"]);
    let index_path = root.join(".git/index");
    let index_before = fs::read(&index_path).unwrap();
    fs::write(root.join("notes.txt"), "second version\n").unwrap();

    service
        .save_version(root, "saved without touching the index")
        .unwrap();

    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert!(service.refresh(root).unwrap().current_changes.is_empty());
}

#[test]
fn an_existing_git_repository_requires_an_explicit_start() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "project file\n").unwrap();
    git(root, &["init", "--quiet"]);

    let service = ProjectService::new(SystemGitBackend);
    let opened = service.open_project(root).unwrap();
    assert!(!opened.initialized);
    assert!(opened.is_git_repository);
    let enabled = service.enable_protection(root).unwrap();
    assert!(enabled.initialized);
    assert_eq!(enabled.checkpoints.len(), 1);
    assert!(enabled.current_changes.is_empty());
}

#[test]
fn using_an_existing_repository_preserves_head_branch_and_git_index() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "original version\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Existing Git User"]);
    git(root, &["config", "user.email", "existing@example.test"]);
    git(root, &["add", "notes.txt"]);
    git(root, &["commit", "--quiet", "-m", "Existing Git history"]);

    let head_before = git(root, &["rev-parse", "HEAD"]);
    let branch_before = git(root, &["symbolic-ref", "HEAD"]);
    let service = ProjectService::new(SystemGitBackend);
    let opened = service.open_project(root).unwrap();
    assert!(!opened.initialized);
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(root, &["symbolic-ref", "HEAD"]), branch_before);

    let enabled = service.enable_protection(root).unwrap();
    let initial_checkpoint = enabled
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.label == "项目初始版本")
        .unwrap()
        .id
        .clone();
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(root, &["symbolic-ref", "HEAD"]), branch_before);

    fs::write(root.join("notes.txt"), "staged version\n").unwrap();
    git(root, &["add", "notes.txt"]);
    let index_path = root.join(".git/index");
    let staged_index = fs::read(&index_path).unwrap();
    service.save_version(root, "EasilyGit version").unwrap();
    assert_eq!(fs::read(&index_path).unwrap(), staged_index);

    let restored = service.restore(root, &initial_checkpoint).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "original version\n"
    );
    assert_eq!(fs::read(&index_path).unwrap(), staged_index);
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(root, &["symbolic-ref", "HEAD"]), branch_before);
    assert!(restored.can_undo_restore);
}

#[test]
fn existing_branch_history_is_shown_and_can_be_restored_without_checkout() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "base version\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Existing Git User"]);
    git(root, &["config", "user.email", "existing@example.test"]);
    git(root, &["add", "notes.txt"]);
    git(root, &["commit", "--quiet", "-m", "Base commit"]);
    let main_branch = String::from_utf8(git(root, &["symbolic-ref", "--short", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();

    git(root, &["checkout", "--quiet", "-b", "alternate"]);
    fs::write(root.join("notes.txt"), "alternate branch version\n").unwrap();
    git(root, &["add", "notes.txt"]);
    git(
        root,
        &["commit", "--quiet", "-m", "Alternate branch commit"],
    );

    git(root, &["checkout", "--quiet", &main_branch]);
    fs::write(root.join("notes.txt"), "main branch version\n").unwrap();
    git(root, &["add", "notes.txt"]);
    git(root, &["commit", "--quiet", "-m", "Main branch commit"]);
    git(root, &["tag", "release-one"]);

    let head_before = git(root, &["rev-parse", "HEAD"]);
    let branch_before = git(root, &["symbolic-ref", "HEAD"]);
    let service = ProjectService::new(SystemGitBackend);
    let enabled = service.enable_protection(root).unwrap();
    let labels: Vec<&str> = enabled
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.kind == easilygit_domain::CheckpointKind::Git)
        .map(|checkpoint| checkpoint.label.as_str())
        .collect();
    assert!(labels.contains(&"Base commit"));
    assert!(labels.contains(&"Alternate branch commit"));
    assert!(labels.contains(&"Main branch commit"));

    let alternate_id = enabled
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.label == "Alternate branch commit")
        .unwrap()
        .id
        .clone();
    let index_path = root.join(".git/index");
    let index_before = fs::read(&index_path).unwrap();
    let restored = service.restore(root, &alternate_id).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "alternate branch version\n"
    );
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(root, &["symbolic-ref", "HEAD"]), branch_before);
    assert!(restored.can_undo_restore);

    fs::write(root.join("later.txt"), "later change\n").unwrap();
    git(root, &["add", "--all"]);
    git(
        root,
        &["commit", "--quiet", "-m", "Git commit after enabling"],
    );
    let refreshed = service.refresh(root).unwrap();
    let refreshed_labels: Vec<&str> = refreshed
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.label.as_str())
        .collect();
    assert!(refreshed_labels.contains(&"Base commit"));
    assert!(refreshed_labels.contains(&"Alternate branch commit"));
    assert!(refreshed_labels.contains(&"Main branch commit"));
    assert!(refreshed_labels.contains(&"Git commit after enabling"));
    assert!(refreshed.current_changes.is_empty());

    git(root, &["checkout", "--quiet", "alternate"]);
    let switched = service.refresh(root).unwrap();
    assert!(switched.current_changes.is_empty());
    assert_eq!(
        String::from_utf8(git(root, &["symbolic-ref", "--short", "HEAD"]))
            .unwrap()
            .trim(),
        "alternate"
    );
}
