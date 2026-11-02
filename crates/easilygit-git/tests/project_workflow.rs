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

fn git_store(root: &Path, args: &[&str]) -> Vec<u8> {
    let store = root.join(".git/easilygit/store.git");
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(store)
        .args(args)
        .current_dir(root)
        .output()
        .expect("git must be installed for integration tests");
    assert!(
        output.status.success(),
        "git --git-dir <EasilyGit store> {args:?} failed: {}",
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
    let overview = service.project_overview(root).unwrap();
    assert!(overview.initialized);
    assert_eq!(overview.change_count, 0);
    let initial_id = initial
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.kind == easilygit_domain::CheckpointKind::User)
        .unwrap()
        .id
        .clone();
    assert_eq!(
        initial.current_checkpoint_id.as_deref(),
        Some(initial_id.as_str())
    );

    fs::write(root.join("notes.txt"), "edited after initial\n").unwrap();
    fs::remove_file(root.join("to-remove.txt")).unwrap();
    fs::write(root.join("added.txt"), "new file\n").unwrap();
    fs::write(root.join("ignored.txt"), "leave this alone\n").unwrap();

    let changed = service.refresh(root).unwrap();
    assert_eq!(changed.current_changes.len(), 3);
    let overview = service.project_overview(root).unwrap();
    assert_eq!(overview.change_count, 3);
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
    let saved_id = saved
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.label == "second version")
        .unwrap()
        .id
        .clone();
    assert_eq!(
        saved.current_checkpoint_id.as_deref(),
        Some(saved_id.as_str())
    );

    fs::write(root.join("notes.txt"), "later work\n").unwrap();
    fs::write(root.join("later.txt"), "another file\n").unwrap();
    let before_restore = service.refresh(root).unwrap();
    assert_eq!(before_restore.current_changes.len(), 2);

    let restored = service.restore(root, &initial_id).unwrap();
    assert!(restored.current_changes.is_empty());
    assert!(restored.can_undo_restore);
    assert_eq!(
        restored.current_checkpoint_id.as_deref(),
        Some(initial_id.as_str())
    );
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
    let overview = service.project_overview(root).unwrap();
    assert!(!overview.initialized);
    assert!(overview.is_git_repository);
    assert_eq!(overview.change_count, 0);
    let enabled = service.enable_protection(root).unwrap();
    assert!(enabled.initialized);
    assert_eq!(enabled.checkpoints.len(), 1);
    assert!(enabled.current_changes.is_empty());
}

#[test]
fn repository_info_excludes_are_respected_by_easilygit() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "--quiet"]);
    fs::write(root.join(".git/info/exclude"), "local-ignored.txt\n").unwrap();
    fs::write(root.join("local-ignored.txt"), "ignored local file\n").unwrap();
    fs::write(root.join("notes.txt"), "tracked by EasilyGit\n").unwrap();

    let service = ProjectService::new(SystemGitBackend);
    let enabled = service.enable_protection(root).unwrap();
    assert!(enabled.current_changes.is_empty());
    let initial = enabled
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.kind == easilygit_domain::CheckpointKind::User)
        .unwrap()
        .id
        .clone();
    fs::write(root.join("notes.txt"), "later content\n").unwrap();
    service.restore(root, &initial).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("local-ignored.txt")).unwrap(),
        "ignored local file\n"
    );
}

#[test]
fn local_git_excludes_added_after_setup_still_apply_to_fast_status_checks() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "tracked by MosSaver\n").unwrap();
    git(root, &["init", "--quiet"]);

    let service = ProjectService::new(SystemGitBackend);
    service.enable_protection(root).unwrap();

    fs::write(root.join(".git/info/exclude"), "local-ignored.txt\n").unwrap();
    fs::write(root.join("local-ignored.txt"), "locally ignored\n").unwrap();

    let refreshed = service.refresh(root).unwrap();
    assert!(refreshed.current_changes.is_empty());
}

#[test]
fn fast_status_checks_repair_object_alternates_after_a_repository_is_moved() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("project");
    let moved = directory.path().join("renamed-project");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("notes.txt"), "saved content\n").unwrap();
    let service = ProjectService::new(SystemGitBackend);
    service.enable_protection(&root).unwrap();

    fs::rename(&root, &moved).unwrap();

    let refreshed = service.refresh(&moved).unwrap();
    assert!(refreshed.current_changes.is_empty());
}

#[test]
fn sha256_git_repositories_can_be_protected() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "first content\n").unwrap();
    let initialized = Command::new("git")
        .args(["init", "--quiet", "--object-format=sha256"])
        .current_dir(root)
        .output()
        .unwrap();
    if !initialized.status.success() {
        eprintln!(
            "Skipping SHA-256 repository test: {}",
            String::from_utf8_lossy(&initialized.stderr)
        );
        return;
    }

    let service = ProjectService::new(SystemGitBackend);
    let initial = service.enable_protection(root).unwrap();
    let initial_id = initial
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.kind == easilygit_domain::CheckpointKind::User)
        .unwrap()
        .id
        .clone();
    assert_eq!(
        String::from_utf8(git_store(root, &["rev-parse", "--show-object-format"]))
            .unwrap()
            .trim(),
        "sha256"
    );
    fs::write(root.join("notes.txt"), "second content\n").unwrap();
    service
        .save_version(root, "SHA-256 EasilyGit version")
        .unwrap();
    let native_log = String::from_utf8(git(root, &["log", "--all", "--format=%s"])).unwrap();
    assert!(!native_log.contains("SHA-256 EasilyGit version"));
    service.restore(root, &initial_id).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "first content\n"
    );
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
        .find(|checkpoint| checkpoint.label == "开始使用 MosSaver")
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
    let native_log = String::from_utf8(git(root, &["log", "--all", "--format=%s"])).unwrap();
    assert!(!native_log.contains("EasilyGit version"));
    let app_log = String::from_utf8(git_store(root, &["log", "--all", "--format=%s"])).unwrap();
    assert!(app_log.contains("EasilyGit version"));

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
fn native_git_reset_stays_a_change_against_the_latest_mossaver_version() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "project initial version\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Existing Git User"]);
    git(root, &["config", "user.email", "existing@example.test"]);
    git(root, &["add", "notes.txt"]);
    git(
        root,
        &["commit", "--quiet", "-m", "Project initial version"],
    );
    let initial_git_commit = String::from_utf8(git(root, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();

    let service = ProjectService::new(SystemGitBackend);
    service.enable_protection(root).unwrap();
    fs::write(root.join("notes.txt"), "latest MosSaver version\n").unwrap();
    let saved = service
        .save_version(root, "Latest MosSaver version")
        .unwrap();
    assert!(saved.current_changes.is_empty());
    let saved_checkpoint_id = saved
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.label == "Latest MosSaver version")
        .unwrap()
        .id
        .clone();

    // Older builds could store the native HEAD in this field after a Git
    // history refresh. Keep that stale value to verify both overview and full
    // refresh recover the MosSaver comparison point.
    let metadata_path = root.join(".git/easilygit/metadata.json");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    let checkpoints = metadata["checkpoints"].as_array_mut().unwrap();
    let initial_mossaver_checkpoint = checkpoints
        .iter_mut()
        .find(|checkpoint| checkpoint["label"] == "开始使用 MosSaver")
        .unwrap();
    // A wall-clock timestamp must not make an older MosSaver version appear
    // newer than the one most recently saved.
    initial_mossaver_checkpoint["created_at"] =
        serde_json::Value::String("9999-12-31T23:59:59+00:00".into());
    let latest_mossaver_checkpoint = checkpoints
        .iter_mut()
        .find(|checkpoint| checkpoint["label"] == "Latest MosSaver version")
        .unwrap();
    latest_mossaver_checkpoint["created_at"] =
        serde_json::Value::String("2000-01-01T00:00:00+00:00".into());
    metadata["current_checkpoint"] = serde_json::Value::String(format!("git-{initial_git_commit}"));
    fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();

    git(root, &["reset", "--hard", &initial_git_commit]);
    assert_eq!(service.project_overview(root).unwrap().change_count, 1);

    let refreshed = service.refresh(root).unwrap();
    assert_eq!(refreshed.current_changes.len(), 1);
    assert_eq!(refreshed.current_changes[0].path, "notes.txt");
    assert_eq!(refreshed.current_changes[0].kind, ChangeKind::Modified);

    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    assert_eq!(persisted["current_checkpoint"], saved_checkpoint_id);
}

#[test]
fn existing_branch_history_is_read_only_and_never_checks_out_a_branch() {
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
    let easilygit_git_refs = String::from_utf8(git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/easilygit/checkpoints/git-",
        ],
    ))
    .unwrap();
    assert!(easilygit_git_refs.trim().is_empty());
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
    let restore_error = service.restore(root, &alternate_id).unwrap_err();
    assert_eq!(restore_error.code(), "CHECKPOINT_READ_ONLY");
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "main branch version\n"
    );
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(root, &["symbolic-ref", "HEAD"]), branch_before);

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
    assert_eq!(refreshed.current_changes.len(), 1);
    assert_eq!(refreshed.current_changes[0].path, "later.txt");
    assert_eq!(refreshed.current_changes[0].kind, ChangeKind::Added);

    git(root, &["checkout", "--quiet", "alternate"]);
    let switched = service.refresh(root).unwrap();
    assert_eq!(switched.current_changes.len(), 1);
    assert_eq!(switched.current_changes[0].path, "notes.txt");
    assert_eq!(switched.current_changes[0].kind, ChangeKind::Modified);
    assert_eq!(
        String::from_utf8(git(root, &["symbolic-ref", "--short", "HEAD"]))
            .unwrap()
            .trim(),
        "alternate"
    );
}

#[test]
fn stopping_protection_removes_easilygit_records_and_preserves_the_git_repository() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "keep this file\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Existing Git User"]);
    git(root, &["config", "user.email", "existing@example.test"]);
    git(root, &["add", "notes.txt"]);
    git(root, &["commit", "--quiet", "-m", "Original Git commit"]);

    let service = ProjectService::new(SystemGitBackend);
    service.enable_protection(root).unwrap();
    service
        .save_version(root, "EasilyGit version to remove")
        .unwrap();

    let head_before = git(root, &["rev-parse", "HEAD"]);
    let branch_before = git(root, &["symbolic-ref", "HEAD"]);
    let index_before = fs::read(root.join(".git/index")).unwrap();
    let metadata_path = root.join(".git/easilygit/metadata.json");
    assert!(metadata_path.exists());
    assert!(
        String::from_utf8(git(
            root,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/easilygit/checkpoints"
            ]
        ))
        .unwrap()
        .trim()
        .is_empty()
    );
    assert!(
        !String::from_utf8(git_store(
            root,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/easilygit/checkpoints"
            ]
        ))
        .unwrap()
        .trim()
        .is_empty()
    );
    let native_log = String::from_utf8(git(root, &["log", "--all", "--format=%s"])).unwrap();
    assert!(!native_log.contains("EasilyGit version to remove"));

    let stopped = service.disable_protection(root).unwrap();
    assert!(!stopped.initialized);
    assert!(stopped.is_git_repository);
    assert!(!metadata_path.exists());
    assert!(!root.join(".git/easilygit").exists());
    assert!(
        String::from_utf8(git(
            root,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/easilygit/checkpoints"
            ]
        ))
        .unwrap()
        .trim()
        .is_empty()
    );
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(root, &["symbolic-ref", "HEAD"]), branch_before);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "keep this file\n"
    );
    let remaining_log = String::from_utf8(git(root, &["log", "--all", "--format=%s"])).unwrap();
    assert!(remaining_log.contains("Original Git commit"));
    assert!(!remaining_log.contains("EasilyGit version to remove"));
}

#[test]
fn legacy_easilygit_refs_are_moved_out_of_the_users_repository() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "original content\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Existing Git User"]);
    git(root, &["config", "user.email", "existing@example.test"]);
    git(root, &["add", "notes.txt"]);
    git(root, &["commit", "--quiet", "-m", "Original Git commit"]);
    let head = String::from_utf8(git(root, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    let tree = String::from_utf8(git(root, &["rev-parse", "HEAD^{tree}"]))
        .unwrap()
        .trim()
        .to_owned();
    let legacy_commit = Command::new("git")
        .args([
            "commit-tree",
            &tree,
            "-p",
            &head,
            "-m",
            "Legacy EasilyGit version",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        legacy_commit.status.success(),
        "git commit-tree failed: {}",
        String::from_utf8_lossy(&legacy_commit.stderr)
    );
    let legacy_commit = String::from_utf8(legacy_commit.stdout)
        .unwrap()
        .trim()
        .to_owned();
    git(
        root,
        &[
            "update-ref",
            "refs/easilygit/checkpoints/legacy-id",
            &legacy_commit,
        ],
    );
    fs::create_dir_all(root.join(".git/easilygit")).unwrap();
    fs::write(
        root.join(".git/easilygit/metadata.json"),
        r#"{"version":2,"checkpoints":[{"id":"legacy-id","label":"Legacy EasilyGit version","createdAt":"2026-01-01T00:00:00Z","kind":"user"}],"currentCheckpoint":"legacy-id"}"#,
    )
    .unwrap();

    let service = ProjectService::new(SystemGitBackend);
    let state = service.refresh(root).unwrap();
    assert!(state.checkpoints.iter().any(|checkpoint| {
        checkpoint.id == "legacy-id" && checkpoint.label == "Legacy EasilyGit version"
    }));
    assert!(
        String::from_utf8(git(
            root,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/easilygit/checkpoints",
            ]
        ))
        .unwrap()
        .trim()
        .is_empty()
    );
    assert!(
        String::from_utf8(git_store(
            root,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/easilygit/checkpoints",
            ]
        ))
        .unwrap()
        .contains("refs/easilygit/checkpoints/legacy-id")
    );
    let native_log = String::from_utf8(git(root, &["log", "--all", "--format=%s"])).unwrap();
    assert!(!native_log.contains("Legacy EasilyGit version"));
    service.restore(root, "legacy-id").unwrap();
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "original content\n"
    );
}

#[test]
fn excluded_files_stay_on_disk_and_are_untouched_by_easilygit_saves_and_restores() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "original notes\n").unwrap();
    fs::write(root.join("generated.txt"), "original generated\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Existing Git User"]);
    git(root, &["config", "user.email", "existing@example.test"]);
    git(root, &["add", "--all"]);
    git(root, &["commit", "--quiet", "-m", "Initial Git commit"]);

    let service = ProjectService::new(SystemGitBackend);
    let initial = service.enable_protection(root).unwrap();
    let initial_id = initial
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.kind == easilygit_domain::CheckpointKind::User)
        .unwrap()
        .id
        .clone();
    let index_path = root.join(".git/index");
    let index_before = fs::read(&index_path).unwrap();

    let after_excluding = service
        .set_file_managed(root, "generated.txt", false)
        .unwrap();
    assert!(after_excluding.current_changes.is_empty());
    let files = service.managed_files(root).unwrap();
    assert!(
        files
            .iter()
            .any(|file| file.path == "generated.txt" && !file.managed)
    );

    fs::write(root.join("generated.txt"), "outside EasilyGit\n").unwrap();
    fs::write(root.join("notes.txt"), "saved notes\n").unwrap();
    let changed = service.refresh(root).unwrap();
    assert_eq!(changed.current_changes.len(), 1);
    assert_eq!(changed.current_changes[0].path, "notes.txt");
    let saved = service.save_version(root, "Notes update").unwrap();
    assert!(saved.current_changes.is_empty());

    fs::write(root.join("generated.txt"), "keep this latest content\n").unwrap();
    fs::write(root.join("notes.txt"), "unsaved notes\n").unwrap();
    service.restore(root, &initial_id).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("generated.txt")).unwrap(),
        "keep this latest content\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "original notes\n"
    );
    assert_eq!(fs::read(&index_path).unwrap(), index_before);

    let reenabled = service
        .set_file_managed(root, "generated.txt", true)
        .unwrap();
    assert!(
        reenabled
            .current_changes
            .iter()
            .any(|file| file.path == "generated.txt")
    );
}

#[test]
fn large_git_histories_are_loaded_in_pages() {
    let directory = tempdir().unwrap();
    let root = directory.path();
    fs::write(root.join("notes.txt"), "history\n").unwrap();
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "History User"]);
    git(root, &["config", "user.email", "history@example.test"]);
    git(root, &["add", "notes.txt"]);
    git(root, &["commit", "--quiet", "-m", "History 000"]);
    for index in 1..125 {
        let label = format!("History {index:03}");
        git(root, &["commit", "--quiet", "--allow-empty", "-m", &label]);
    }

    let service = ProjectService::new(SystemGitBackend);
    let first = service.enable_protection(root).unwrap();
    assert_eq!(first.checkpoints.len(), 50);
    assert!(first.has_more_checkpoints);

    let second = service
        .load_more_checkpoints(root, first.checkpoints.len())
        .unwrap();
    assert_eq!(second.checkpoints.len(), 50);
    assert!(second.has_more);
    let third = service
        .load_more_checkpoints(root, first.checkpoints.len() + second.checkpoints.len())
        .unwrap();
    assert_eq!(third.checkpoints.len(), 26);
    assert!(!third.has_more);

    let mut ids: Vec<String> = first
        .checkpoints
        .iter()
        .chain(second.checkpoints.iter())
        .chain(third.checkpoints.iter())
        .map(|checkpoint| checkpoint.id.clone())
        .collect();
    let count_with_duplicates = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count_with_duplicates);
}
