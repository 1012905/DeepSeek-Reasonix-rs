use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;

fn fixture() -> (TempDir, TempDir, PathBuf, Store) {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let store = Store::new(Some(&dir), root.path());
    (temp, root, dir, store)
}

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn read_turn(dir: &Path, turn: i64) -> Value {
    serde_json::from_slice(&fs::read(dir.join(format!("turn-{turn}.json"))).unwrap()).unwrap()
}

#[test]
fn checkpoint_persistence_uses_compact_go_json_bytes() {
    let (_temp, _root, dir, store) = fixture();
    let sensitive = "<>&\u{2028}\u{2029}";
    let escaped = "\\u003c\\u003e\\u0026\\u2028\\u2029";
    let checkpoint = Checkpoint {
        schema_version: SCHEMA_V2,
        turn: 3,
        time: serde_json::from_value::<Checkpoint>(json!({
            "turn": 0,
            "time": "2026-02-03T04:05:06.123456789Z"
        }))
        .unwrap()
        .time,
        prompt: format!("prompt {sensitive}"),
        msg_index: 4,
        session_id: format!("session {sensitive}"),
        files: vec![FileSnap {
            path: format!("dir/{sensitive}/file.txt"),
            content: Some(format!("content {sensitive}")),
            mode: 0o640,
            ..FileSnap::default()
        }],
        coverage: Coverage::Complete,
        ..Checkpoint::default()
    };

    store.persist_checkpoint(&checkpoint).unwrap();

    let bytes = fs::read(turn_path(&dir, checkpoint.turn, false)).unwrap();
    let expected = format!(
        r#"{{"schemaVersion":2,"turn":3,"time":"2026-02-03T04:05:06.123456789Z","prompt":"prompt {escaped}","msgIndex":4,"sessionId":"session {escaped}","files":[{{"path":"dir/{escaped}/file.txt","content":"content {escaped}","mode":416}}],"coverage":"complete"}}"#
    );
    assert_eq!(bytes.as_slice(), expected.as_bytes());
    assert!(!bytes.contains(&b'\n'));
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(
        !text
            .chars()
            .any(|character| matches!(character, '<' | '>' | '&' | '\u{2028}' | '\u{2029}'))
    );
    let decoded: Checkpoint = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, checkpoint);
}

#[test]
fn v1_load_marks_legacy_and_preserves_inline_preimage() {
    let (temp, root, dir, _) = fixture();
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("turn-4.json"),
        json!({
            "turn": 4,
            "time": "2026-01-02T03:04:05Z",
            "prompt": "legacy",
            "msgIndex": 9,
            "files": [{"path": "a.txt", "content": "before"}]
        })
        .to_string(),
    )
    .unwrap();

    let store = Store::new(Some(&dir), root.path());
    let meta = &store.list()[0];
    assert_eq!(meta.turn, 4);
    assert_eq!(meta.msg_index, 9);
    assert!(meta.legacy);
    assert_eq!(meta.coverage, Coverage::Legacy);
    assert_eq!(
        meta.disabled_reason,
        "legacy checkpoint cannot verify later manual edits"
    );
    let legacy_gap = meta
        .coverage_gaps
        .iter()
        .find(|gap| gap.reason == GAP_LEGACY_UNVERIFIED)
        .unwrap();
    assert_eq!(
        legacy_gap.detail,
        "v1 checkpoint cannot verify later manual edits"
    );
    let state = store.file_state("a.txt").unwrap();
    assert_eq!(state.content.as_deref(), Some("before"));
    assert!(!state.owned);
    assert_eq!(store.bounds().get(&4), Some(&9));
    assert_eq!(store.next_turn(), 5);
    drop(temp);
}

#[test]
fn expired_copy_loads_without_payload_and_keeps_boundary() {
    let (temp, root, dir, _) = fixture();
    fs::create_dir_all(dir.join("expired")).unwrap();
    fs::write(
        dir.join("expired/turn-3.json"),
        json!({
            "schemaVersion": 2,
            "turn": 3,
            "time": "2026-01-02T03:04:05Z",
            "prompt": "expired",
            "msgIndex": 12,
            "files": [{"path": "a.txt", "content": "stale", "blobRef": digest(b"stale")}],
            "coverage": "complete"
        })
        .to_string(),
    )
    .unwrap();

    let store = Store::new(Some(&dir), root.path());
    let meta = &store.list()[0];
    assert!(meta.expired_file_payload);
    assert_eq!(meta.coverage, Coverage::Partial);
    assert_eq!(meta.disabled_reason, "file recovery payload expired");
    assert!(
        meta.coverage_gaps
            .iter()
            .any(|gap| gap.reason == GAP_EXPIRED_PAYLOAD)
    );
    assert!(store.file_state("a.txt").is_none());
    assert_eq!(store.bounds().get(&3), Some(&12));
    assert!(store.unreadable_turns().is_empty());
    assert_eq!(store.next_turn(), 4);
    drop(temp);
}

#[test]
fn unreadable_turn_floor_and_readable_copy_precedence() {
    let (temp, root, dir, _) = fixture();
    fs::create_dir_all(dir.join("expired")).unwrap();
    fs::write(dir.join("turn-7.json"), b"{truncated").unwrap();
    fs::write(
        dir.join("expired/turn-7.json"),
        json!({
            "schemaVersion": 2,
            "turn": 7,
            "time": "2026-01-02T03:04:05Z",
            "prompt": "readable expired copy",
            "msgIndex": 1,
            "files": [],
            "expiredFilePayload": true
        })
        .to_string(),
    )
    .unwrap();
    fs::write(dir.join("turn-11.json"), b"not json").unwrap();

    let store = Store::new(Some(&dir), root.path());
    assert_eq!(store.unreadable_turns(), vec![11]);
    assert_eq!(store.bounds().get(&7), Some(&1));
    assert_eq!(store.next_turn(), 12);
    let corrupt = fs::read(dir.join("turn-11.json")).unwrap();
    store
        .begin(store.next_turn(), "after corruption", 13)
        .unwrap();
    assert_eq!(fs::read(dir.join("turn-11.json")).unwrap(), corrupt);
    assert!(dir.join("turn-12.json").exists());
    let reloaded = Store::new(Some(&dir), root.path());
    assert_eq!(reloaded.unreadable_turns(), vec![11]);
    assert_eq!(reloaded.next_turn(), 13);
    assert_eq!(fs::read(dir.join("turn-11.json")).unwrap(), corrupt);
    drop(temp);
}

#[test]
fn absent_turns_do_not_create_reservations() {
    let (temp, root, dir, _) = fixture();
    fs::create_dir_all(&dir).unwrap();
    for (turn, index) in [(0, 3), (2, 9)] {
        fs::write(
            dir.join(format!("turn-{turn}.json")),
            json!({
                "schemaVersion": 2,
                "turn": turn,
                "time": "2026-01-02T03:04:05Z",
                "prompt": "p",
                "msgIndex": index,
                "files": [],
                "coverage": "none"
            })
            .to_string(),
        )
        .unwrap();
    }
    let store = Store::new(Some(&dir), root.path());
    assert!(store.unreadable_turns().is_empty());
    assert_eq!(store.next_turn(), 3);
    assert_eq!(store.bounds(), BTreeMap::from([(0, 3), (2, 9)]));
    drop(temp);
}

#[test]
fn first_touch_dedup_uses_normalized_workspace_paths() {
    let (_temp, root, _dir, store) = fixture();
    write(root.path(), "a.txt", "original");
    store.begin(0, "first", 0).unwrap();
    store.snapshot(PreviewChange {
        path: root.path().join("a.txt").to_string_lossy().into_owned(),
        kind: ChangeKind::Modify,
        old_text: "original".into(),
        ..PreviewChange::default()
    });
    write(root.path(), "a.txt", "edited once");
    store.snapshot(PreviewChange {
        path: "./a.txt".into(),
        kind: ChangeKind::Modify,
        old_text: "edited once".into(),
        ..PreviewChange::default()
    });

    let metas = store.list();
    assert_eq!(metas.len(), 1);
    assert_eq!(metas[0].paths.len(), 1);
    assert_eq!(metas[0].rewind_files, 1);
    assert_eq!(
        store.file_state("a.txt").unwrap().content.as_deref(),
        Some("original")
    );
}

#[test]
fn inline_content_and_blob_persist_together_with_create_sentinel() {
    let (_temp, root, dir, store) = fixture();
    write(root.path(), "existing.txt", "before");
    store.begin(0, "compat", 0).unwrap();
    store.capture_before("existing.txt", CaptureBeforeOpts::default());
    store.capture_before("created.txt", CaptureBeforeOpts::default());
    store.persist().unwrap();

    let wire = read_turn(&dir, 0);
    let files = wire["files"].as_array().unwrap();
    let existing = files.iter().find(|f| f["path"] == "existing.txt").unwrap();
    let created = files.iter().find(|f| f["path"] == "created.txt").unwrap();
    assert_eq!(existing["content"], "before");
    assert_eq!(existing["sha256"], digest(b"before"));
    assert!(valid_blob_ref(existing["blobRef"].as_str().unwrap()));
    assert!(created["content"].is_null());
    assert!(created.get("blobRef").is_none());

    let blobs = store.blobs().unwrap();
    assert_eq!(
        blobs.get(existing["blobRef"].as_str().unwrap()).unwrap(),
        b"before"
    );
    let reloaded = Store::new(Some(&dir), root.path());
    assert_eq!(
        reloaded
            .file_state("existing.txt")
            .unwrap()
            .content
            .as_deref(),
        Some("before")
    );
    assert_eq!(reloaded.file_state("created.txt").unwrap().content, None);
}

#[test]
fn after_fingerprints_track_latest_ownership_and_earlier_turns() {
    let (_temp, root, dir, store) = fixture();
    write(root.path(), "a.txt", "v0");
    store.begin(0, "first", 0).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    write(root.path(), "a.txt", "v1");
    store
        .capture_after(
            "a.txt",
            CaptureAfterOpts {
                seq: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(store.file_state("a.txt").unwrap().owned);

    store.begin(1, "second", 2).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    assert!(!store.file_state("a.txt").unwrap().owned);
    write(root.path(), "a.txt", "v2");
    store
        .capture_after(
            "a.txt",
            CaptureAfterOpts {
                seq: 2,
                ..Default::default()
            },
        )
        .unwrap();
    let state = store.file_state("a.txt").unwrap();
    assert_eq!(state.content.as_deref(), Some("v0"));
    assert!(state.owned);

    store.begin(2, "empty", 4).unwrap();
    write(root.path(), "a.txt", "v3");
    store
        .capture_after(
            "a.txt",
            CaptureAfterOpts {
                seq: 3,
                ..Default::default()
            },
        )
        .unwrap();
    let wire = read_turn(&dir, 1);
    let file = &wire["files"][0];
    assert_eq!(file["afterExisted"], true);
    assert_eq!(file["afterSha256"], digest(b"v3"));
    assert_eq!(wire["lastMutationSeq"], 2);
}

#[test]
fn coverage_recomputation_follows_legacy_expired_files_and_gaps() {
    let (_temp, root, _dir, store) = fixture();
    store.begin(0, "empty", 0).unwrap();
    assert_eq!(store.list()[0].coverage, Coverage::None);
    store.record_gap(CoverageGap::new(GAP_BASH_SIDE_EFFECT, "unknown"));
    store.record_gap(CoverageGap::new(GAP_BASH_SIDE_EFFECT, "unknown"));
    assert_eq!(store.list()[0].coverage, Coverage::None);
    assert_eq!(store.list()[0].coverage_gaps.len(), 1);

    write(root.path(), "a.txt", "before");
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    assert_eq!(store.list()[0].coverage, Coverage::Partial);
    let mut complete = Checkpoint {
        schema_version: SCHEMA_V2,
        files: vec![FileSnap {
            path: "a.txt".into(),
            content: Some("before".into()),
            ..FileSnap::default()
        }],
        ..Checkpoint::default()
    };
    complete.recompute_coverage();
    assert_eq!(complete.coverage, Coverage::Complete);
    complete.expired_file_payload = true;
    complete.recompute_coverage();
    assert_eq!(complete.coverage, Coverage::Partial);
    complete.legacy = true;
    complete.recompute_coverage();
    assert_eq!(complete.coverage, Coverage::Legacy);
}

#[test]
fn outside_workspace_after_is_a_gap_not_a_failure() {
    let (temp, root, _dir, store) = fixture();
    let outside = temp.path().join("outside.txt");
    fs::write(&outside, "outside").unwrap();
    store.begin(0, "outside", 0).unwrap();
    store
        .capture_after(&outside, CaptureAfterOpts::default())
        .unwrap();
    let meta = &store.list()[0];
    assert_eq!(meta.coverage, Coverage::None);
    assert_eq!(meta.coverage_gaps[0].reason, GAP_OUTSIDE_WORKSPACE);
    assert!(matches!(
        capture_path(
            &outside,
            &CaptureOptions {
                workspace_root: root.path().to_path_buf(),
                read_content: true,
                ..CaptureOptions::default()
            }
        ),
        Err(CaptureError::Path(PathError::OutsideWorkspace { .. }))
    ));
}

#[test]
fn file_state_combines_earliest_preimage_with_latest_after_identity() {
    let (_temp, root, _dir, store) = fixture();
    write(root.path(), "nested/a.txt", "original");
    store.begin(0, "first", 0).unwrap();
    store.snapshot(PreviewChange {
        path: root
            .path()
            .join("nested/a.txt")
            .to_string_lossy()
            .into_owned(),
        kind: ChangeKind::Modify,
        old_text: "original".into(),
        ..PreviewChange::default()
    });
    write(root.path(), "nested/a.txt", "middle");
    store
        .capture_after("nested/a.txt", CaptureAfterOpts::default())
        .unwrap();
    store.begin(1, "second", 2).unwrap();
    store.snapshot(PreviewChange {
        path: "nested/./a.txt".into(),
        kind: ChangeKind::Modify,
        old_text: "middle".into(),
        ..PreviewChange::default()
    });
    write(root.path(), "nested/a.txt", "latest");
    store
        .capture_after("nested/a.txt", CaptureAfterOpts::default())
        .unwrap();

    let state = store.file_state("nested/a.txt").unwrap();
    assert_eq!(state.content.as_deref(), Some("original"));
    assert_eq!(state.sha256, digest(b"original"));
    assert!(state.owned);
    assert!(store.file_state("../escape.txt").is_none());
}

#[test]
fn truncate_from_removes_root_expired_and_current_turns() {
    let (_temp, root, dir, store) = fixture();
    for turn in 0..3 {
        store.begin(turn, &format!("turn {turn}"), turn).unwrap();
        write(root.path(), "a.txt", &format!("v{turn}"));
        store.capture_before("a.txt", CaptureBeforeOpts::default());
    }
    store.truncate_from(1).unwrap();
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.list()[0].turn, 0);
    assert_eq!(store.next_turn(), 1);
    assert!(!dir.join("turn-1.json").exists());
    assert!(!dir.join("turn-2.json").exists());
    assert!(dir.join("turn-0.json").exists());

    let reloaded = Store::new(Some(&dir), root.path());
    assert_eq!(reloaded.list().len(), 1);
    assert_eq!(reloaded.next_turn(), 1);
}

#[test]
fn truncate_failure_leaves_in_memory_checkpoints_unchanged() {
    let (_temp, _root, dir, store) = fixture();
    store.begin(0, "keep", 0).unwrap();
    store.begin(1, "drop", 1).unwrap();
    let path = dir.join("turn-1.json");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("marker"), b"keep").unwrap();

    assert!(store.truncate_from(1).is_err());
    assert_eq!(store.list().len(), 2);
    assert_eq!(store.list()[1].turn, 1);
    assert!(path.join("marker").exists());
}

#[test]
fn list_counts_cumulative_distinct_paths_oldest_first() {
    let (_temp, root, _dir, store) = fixture();
    write(root.path(), "a.txt", "a0");
    write(root.path(), "b.txt", "b0");
    store.begin(0, "no edits", 0).unwrap();
    store.begin(1, "a and b", 2).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    store.capture_before("b.txt", CaptureBeforeOpts::default());
    store.begin(2, "a and c", 4).unwrap();
    store.capture_before("./a.txt", CaptureBeforeOpts::default());
    store.capture_before("c.txt", CaptureBeforeOpts::default());
    store.begin(3, "no later edits", 6).unwrap();

    let metas = store.list();
    assert_eq!(
        metas.iter().map(|m| m.turn).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(
        metas.iter().map(|m| m.rewind_files).collect::<Vec<_>>(),
        vec![3, 3, 2, 0]
    );
    assert_eq!(metas[0].msg_index, 0);
    assert_eq!(metas[3].msg_index, 6);
}

#[test]
fn persistence_round_trip_preserves_prompts_bounds_and_next_turn() {
    let (_temp, root, dir, store) = fixture();
    store.begin(0, "hello", 1).unwrap();
    store.begin(1, "world", 5).unwrap();
    let reloaded = Store::new(Some(&dir), root.path());
    assert_eq!(
        reloaded
            .list()
            .iter()
            .map(|m| m.prompt.as_str())
            .collect::<Vec<_>>(),
        vec!["hello", "world"]
    );
    assert_eq!(reloaded.bounds(), BTreeMap::from([(0, 1), (1, 5)]));
    assert_eq!(reloaded.next_turn(), 2);
}

#[test]
fn new_is_lazy_and_begin_creates_the_checkpoint_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("missing/session.ckpt");
    let store = Store::new(Some(&dir), root.path());
    assert!(!dir.exists());
    store.begin(0, "first", 0).unwrap();
    assert!(dir.join("turn-0.json").is_file());
}

#[test]
fn encoding_kind_uses_go_numeric_wire_values() {
    let file = FileSnap {
        encoding: Some(Kind::Gb18030),
        ..FileSnap::default()
    };
    let value = serde_json::to_value(file).unwrap();
    assert_eq!(value["encoding"], json!(4));
    let decoded: FileSnap =
        serde_json::from_value(json!({"path": "a", "content": null, "encoding": 8})).unwrap();
    assert_eq!(decoded.encoding, Some(Kind::Gbk));
    assert!(
        serde_json::from_value::<FileSnap>(json!({"path": "a", "content": null, "encoding": 99}))
            .is_err()
    );
}

#[test]
fn rewind_wire_types_keep_numeric_scope_and_conflict_fields() {
    let conflict = RewindConflict {
        path: "a.txt".into(),
        reason: CONFLICT_MANUAL_EDIT.into(),
        current_existed: true,
        checkpoint_existed: true,
        ..RewindConflict::default()
    };
    let value = serde_json::to_value(&conflict).unwrap();
    assert_eq!(value["reason"], "manual_edit");
    assert_eq!(value["currentExisted"], true);
    assert_eq!(serde_json::to_value(RewindScope::Both).unwrap(), json!(2));
    let target = TransactionTarget {
        restore_inline: b"restore".to_vec(),
        forward_inline: b"forward".to_vec(),
        ..TransactionTarget::default()
    };
    let value = serde_json::to_value(&target).unwrap();
    assert_eq!(value["restoreInline"], "cmVzdG9yZQ==");
}

#[test]
fn no_op_operations_do_not_create_the_checkpoint_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("missing/session.ckpt");
    let store = Store::new(Some(&dir), root.path());

    store.load().unwrap();
    store.capture_before("missing.txt", CaptureBeforeOpts::default());
    store.snapshot(PreviewChange {
        path: "missing.txt".into(),
        ..PreviewChange::default()
    });
    store
        .capture_after("missing.txt", CaptureAfterOpts::default())
        .unwrap();
    store.record_gap(CoverageGap::new(GAP_BASH_SIDE_EFFECT, "unknown"));
    store.set_active_writers(vec![ActiveWriter {
        id: "writer".into(),
        ..ActiveWriter::default()
    }]);
    store.truncate_from(0).unwrap();
    store.gc().unwrap();
    store.prune_blobs().unwrap();
    assert!(store.list().is_empty());
    assert_eq!(store.next_turn(), 0);
    assert!(!dir.exists());
}

#[test]
fn empty_capture_before_keeps_inline_empty_without_blob() {
    let (_temp, root, dir, store) = fixture();
    write(root.path(), "empty.txt", "");
    store.begin(0, "empty", 0).unwrap();
    store.capture_before("empty.txt", CaptureBeforeOpts::default());

    let wire = read_turn(&dir, 0);
    let file = &wire["files"][0];
    assert_eq!(file["content"], "");
    assert_eq!(file["sha256"], digest(b""));
    assert!(file.get("blobRef").is_none());
    assert!(!store.blobs().unwrap().has(&digest(b"")));
    assert_eq!(
        store.file_state("empty.txt").unwrap().content.as_deref(),
        Some("")
    );
}

#[test]
fn partial_preview_capture_keeps_inline_mode_fallback_blob_and_gap_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let store = Store::new_with_config(
        Some(&dir),
        root.path(),
        StoreConfig {
            max_file_bytes: 3,
            ..StoreConfig::default()
        },
    );
    write(root.path(), "large.txt", "too large");
    store.begin(0, "partial", 0).unwrap();
    store.snapshot(PreviewChange {
        path: "large.txt".into(),
        kind: ChangeKind::Modify,
        old_text: "old".into(),
        ..PreviewChange::default()
    });

    let wire = read_turn(&dir, 0);
    let file = &wire["files"][0];
    assert_eq!(file["content"], "old");
    assert_eq!(file["sha256"], digest(b"old"));
    assert_eq!(file["blobRef"], digest(b"old"));
    assert!(file["mode"].as_u64().unwrap() > 0);
    let gap = wire["coverageGaps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gap| gap["reason"] == GAP_OVERSIZED)
        .unwrap();
    assert_eq!(gap["path"], "large.txt");
}

#[test]
fn preview_capture_keeps_inline_text_alongside_exact_disk_blob() {
    let (_temp, root, dir, store) = fixture();
    write(root.path(), "a.txt", "disk bytes");
    store.begin(0, "preview", 0).unwrap();
    store.snapshot(PreviewChange {
        path: "a.txt".into(),
        kind: ChangeKind::Modify,
        old_text: "preview text".into(),
        ..PreviewChange::default()
    });

    let file = &read_turn(&dir, 0)["files"][0];
    assert_eq!(file["content"], "preview text");
    assert_eq!(file["sha256"], digest(b"disk bytes"));
    assert_eq!(file["blobRef"], digest(b"disk bytes"));
    assert_eq!(
        store
            .blobs()
            .unwrap()
            .get(file["blobRef"].as_str().unwrap())
            .unwrap(),
        b"disk bytes"
    );
}
