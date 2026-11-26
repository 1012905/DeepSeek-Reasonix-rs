use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::*;

fn checkpoint(turn: i64, path: &str, before: &str, after: &str) -> Checkpoint {
    Checkpoint {
        schema_version: SCHEMA_V2,
        turn,
        time: CreatedAt::now(),
        coverage: Coverage::Complete,
        files: vec![FileSnap {
            path: path.to_owned(),
            content: Some(before.to_owned()),
            sha256: digest(before.as_bytes()),
            mode: 0o644,
            after_existed: Some(true),
            after_sha256: digest(after.as_bytes()),
            after_mode: 0o644,
            ..FileSnap::default()
        }],
        ..Checkpoint::default()
    }
}

fn load(root: &Path, dir: &Path, checkpoints: &[Checkpoint]) -> Store {
    fs::create_dir_all(dir).unwrap();
    for checkpoint in checkpoints {
        fs::write(
            turn_path(dir, checkpoint.turn, false),
            serde_json::to_vec(checkpoint).unwrap(),
        )
        .unwrap();
    }
    Store::new(Some(dir), root)
}

fn two_turn_fixture() -> (TempDir, TempDir, PathBuf, Store) {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let checkpoints = [
        checkpoint(0, "./nested/a.txt", "v0", "v1"),
        checkpoint(1, "nested/a.txt", "v1", "v2"),
    ];
    let store = load(root.path(), &dir, &checkpoints);
    (temp, root, dir, store)
}

fn target(path: &str, restore: &str, forward: &str) -> TransactionTarget {
    TransactionTarget {
        path: path.to_owned(),
        restore_existed: true,
        restore_sha: digest(restore.as_bytes()),
        restore_mode: 0o600,
        forward_existed: true,
        forward_sha: digest(forward.as_bytes()),
        forward_mode: 0o644,
        ..TransactionTarget::default()
    }
}

#[test]
fn sync_restored_ownership_updates_latest_checkpoint_and_preserves_preimages() {
    let (_temp, root, dir, store) = two_turn_fixture();
    let targets = [target("nested/./a.txt", "v0", "v2")];

    store.sync_restored_ownership(&targets).unwrap();

    let checkpoints = store.state().all();
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[0].files[0].content.as_deref(), Some("v0"));
    assert_eq!(checkpoints[0].files[0].sha256, digest(b"v0"));
    assert_eq!(checkpoints[0].files[0].mode, 0o644);
    assert_eq!(checkpoints[0].files[0].after_sha256, digest(b"v1"));
    assert_eq!(checkpoints[0].files[0].after_existed, Some(true));
    assert_eq!(checkpoints[0].files[0].after_mode, 0o644);
    assert_eq!(checkpoints[1].files[0].content.as_deref(), Some("v1"));
    assert_eq!(checkpoints[1].files[0].sha256, digest(b"v1"));
    assert_eq!(checkpoints[1].files[0].mode, 0o644);
    assert_eq!(checkpoints[1].files[0].after_sha256, digest(b"v0"));
    assert_eq!(checkpoints[1].files[0].after_existed, Some(true));
    assert_eq!(checkpoints[1].files[0].after_mode, 0o600);

    let persisted: Checkpoint = read_json_file(&turn_path(&dir, 1, false)).unwrap();
    assert_eq!(persisted.files[0].after_sha256, digest(b"v0"));
    assert_eq!(persisted.files[0].after_mode, 0o600);
    let persisted_first: Checkpoint = read_json_file(&turn_path(&dir, 0, false)).unwrap();
    assert_eq!(persisted_first.files[0].after_sha256, digest(b"v1"));
    assert_eq!(
        store.file_state("nested/a.txt").unwrap().content.as_deref(),
        Some("v0")
    );
    drop(root);
}

#[test]
fn sync_forward_ownership_maps_forward_identity_without_rewriting_preimages() {
    let (_temp, _root, dir, store) = two_turn_fixture();
    let targets = [target("nested/a.txt", "v0", "v2")];

    store.sync_forward_ownership(&targets).unwrap();

    let checkpoints = store.state().all();
    assert_eq!(checkpoints[0].files[0].after_sha256, digest(b"v1"));
    assert_eq!(checkpoints[1].files[0].content.as_deref(), Some("v1"));
    assert_eq!(checkpoints[1].files[0].sha256, digest(b"v1"));
    assert_eq!(checkpoints[1].files[0].after_sha256, digest(b"v2"));
    assert_eq!(checkpoints[1].files[0].after_existed, Some(true));
    assert_eq!(checkpoints[1].files[0].after_mode, 0o644);
    let persisted: Checkpoint = read_json_file(&turn_path(&dir, 1, false)).unwrap();
    assert_eq!(persisted.files[0].after_sha256, digest(b"v2"));
}

#[test]
fn ownership_synchronization_tracks_existence_in_both_directions() {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let store = load(
        root.path(),
        &dir,
        &[checkpoint(0, "a.txt", "before", "after")],
    );
    let targets = [TransactionTarget {
        path: "a.txt".into(),
        restore_existed: false,
        forward_existed: true,
        forward_sha: digest(b"after"),
        forward_mode: 0o644,
        ..TransactionTarget::default()
    }];

    store.sync_restored_ownership(&targets).unwrap();
    let restored = &store.state().all()[0].files[0];
    assert_eq!(restored.after_existed, Some(false));
    assert!(restored.after_sha256.is_empty());
    assert_eq!(restored.after_mode, 0);
    assert_eq!(restored.content.as_deref(), Some("before"));

    store.sync_forward_ownership(&targets).unwrap();
    let forward = &store.state().all()[0].files[0];
    assert_eq!(forward.after_existed, Some(true));
    assert_eq!(forward.after_sha256, digest(b"after"));
    assert_eq!(forward.after_mode, 0o644);
    assert_eq!(forward.content.as_deref(), Some("before"));
}

#[test]
fn ownership_save_failure_keeps_all_in_memory_checkpoints_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let checkpoints = [
        checkpoint(0, "a.txt", "a0", "a1"),
        checkpoint(1, "b.txt", "b0", "b1"),
    ];
    let store = load(root.path(), &dir, &checkpoints);
    let before = store.state().all();
    let targets = [target("a.txt", "a0", "a1"), target("b.txt", "b0", "b1")];
    let blocked = turn_path(&dir, 1, false);
    fs::remove_file(&blocked).unwrap();
    fs::create_dir(&blocked).unwrap();

    let error = store.sync_restored_ownership(&targets).unwrap_err();

    assert!(matches!(
        error,
        StoreError::Io {
            op: "write checkpoint",
            path,
            ..
        } if path == blocked
    ));
    assert_eq!(store.state().all(), before);
    let persisted_first: Checkpoint = read_json_file(&turn_path(&dir, 0, false)).unwrap();
    assert_eq!(persisted_first.files[0].after_sha256, digest(b"a0"));
    assert_eq!(persisted_first.files[0].after_mode, 0o600);
    assert!(blocked.is_dir());
}
