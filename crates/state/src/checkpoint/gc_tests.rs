use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;

fn fixture(retain: usize, quota: u64) -> (TempDir, TempDir, PathBuf, Store) {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let store = Store::new_with_config(
        Some(&dir),
        root.path(),
        StoreConfig {
            retain,
            blob_quota_bytes: quota,
            ..StoreConfig::default()
        },
    );
    (temp, root, dir, store)
}

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn retention_expires_oldest_payload_first_and_writes_expired_wire_copy() {
    let (_temp, root, dir, store) = fixture(2, DEFAULT_BLOB_QUOTA_BYTES);
    for (turn, path, content) in [
        (0, "a.txt", "aaaa"),
        (1, "b.txt", "bbbb"),
        (2, "c.txt", "cccc"),
    ] {
        store.begin(turn, "turn", turn).unwrap();
        write(root.path(), path, content);
        store.capture_before(path, CaptureBeforeOpts::default());
    }
    store.gc().unwrap();

    assert!(!dir.join("turn-0.json").exists());
    assert!(dir.join("expired/turn-0.json").exists());
    assert!(dir.join("turn-1.json").exists());
    assert!(dir.join("turn-2.json").exists());
    let expired: Value =
        serde_json::from_slice(&fs::read(dir.join("expired/turn-0.json")).unwrap()).unwrap();
    assert_eq!(expired["coverage"], "partial");
    assert_eq!(expired["expiredFilePayload"], true);
    assert_eq!(expired["files"][0]["payloadExpired"], true);
    assert!(expired["files"][0]["content"].is_null());
    assert_eq!(expired["files"][0]["blobRef"].as_str().unwrap_or(""), "");
    assert!(
        expired["coverageGaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["reason"] == GAP_EXPIRED_PAYLOAD)
    );
    assert!(store.file_state("a.txt").is_none());
    assert!(store.file_state("b.txt").is_some());
}

#[test]
fn protected_turn_is_skipped_without_consuming_another_retention_slot() {
    let (_temp, root, dir, store) = fixture(1, DEFAULT_BLOB_QUOTA_BYTES);
    store.protect_turn(0);
    for (turn, path, content) in [
        (0, "a.txt", "aaaa"),
        (1, "b.txt", "bbbb"),
        (2, "c.txt", "cccc"),
    ] {
        store.begin(turn, "turn", turn).unwrap();
        write(root.path(), path, content);
        store.capture_before(path, CaptureBeforeOpts::default());
    }
    store.gc().unwrap();
    assert!(dir.join("turn-0.json").exists());
    assert!(!dir.join("turn-1.json").exists());
    assert!(dir.join("expired/turn-1.json").exists());
    assert!(dir.join("turn-2.json").exists());

    store.unprotect_turn(0);
    store.gc().unwrap();
    assert!(!dir.join("turn-0.json").exists());
    assert!(dir.join("expired/turn-0.json").exists());
    assert!(dir.join("turn-2.json").exists());
}

#[test]
fn protected_only_candidate_does_not_expire_a_newer_turn() {
    let (_temp, root, dir, store) = fixture(2, DEFAULT_BLOB_QUOTA_BYTES);
    store.protect_turn(0);
    for (turn, path, content) in [
        (0, "a.txt", "aaaa"),
        (1, "b.txt", "bbbb"),
        (2, "c.txt", "cccc"),
    ] {
        store.begin(turn, "turn", turn).unwrap();
        write(root.path(), path, content);
        store.capture_before(path, CaptureBeforeOpts::default());
    }
    store.gc().unwrap();
    assert!(dir.join("turn-0.json").exists());
    assert!(dir.join("turn-1.json").exists());
    assert!(dir.join("turn-2.json").exists());
}

#[test]
fn quota_expires_payloads_until_blob_size_fits() {
    let (_temp, root, dir, store) = fixture(100, 5);
    write(root.path(), "a.txt", "aaaa");
    write(root.path(), "b.txt", "bbbb");
    store.begin(0, "first", 0).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    store.begin(1, "second", 1).unwrap();
    store.capture_before("b.txt", CaptureBeforeOpts::default());
    store.gc().unwrap();

    assert!(!dir.join("turn-0.json").exists());
    assert!(dir.join("expired/turn-0.json").exists());
    assert!(dir.join("turn-1.json").exists());
    assert_eq!(store.blobs().unwrap().size().unwrap(), 4);
    assert!(store.file_state("a.txt").is_none());
    assert!(store.file_state("b.txt").is_some());
}

#[test]
fn pruning_keeps_shared_blobs_and_removes_orphans() {
    let (_temp, root, dir, store) = fixture(1, DEFAULT_BLOB_QUOTA_BYTES);
    write(root.path(), "a.txt", "shared");
    write(root.path(), "b.txt", "shared");
    let blobs = store.blobs().unwrap();
    let live = blobs.put(b"shared").unwrap();
    let orphan = blobs.put(b"orphan").unwrap();

    store.begin(0, "first", 0).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    store.begin(1, "second", 1).unwrap();
    store.capture_before("b.txt", CaptureBeforeOpts::default());
    store.gc().unwrap();

    assert!(!dir.join("turn-0.json").exists());
    assert!(blobs.has(&live));
    assert!(!blobs.has(&orphan));
    assert_eq!(blobs.size().unwrap(), 6);
    assert!(store.file_state("b.txt").is_some());
}

#[test]
fn transaction_manifest_references_keep_blobs_live_during_pruning() {
    let (_temp, root, dir, store) = fixture(1, DEFAULT_BLOB_QUOTA_BYTES);
    write(root.path(), "a.txt", "transaction payload");
    let blobs = store.blobs().unwrap();
    let restore_reference = blobs.put(b"transaction payload").unwrap();
    let forward_reference = blobs.put(b"forward payload").unwrap();
    fs::create_dir_all(dir.join("transactions")).unwrap();
    let manifest = TransactionManifest {
        id: "tx-test".into(),
        targets: vec![TransactionTarget {
            restore_blob: restore_reference.clone(),
            forward_blob: forward_reference.clone(),
            ..TransactionTarget::default()
        }],
        ..TransactionManifest::default()
    };
    fs::write(
        dir.join("transactions/tx-test.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    store.begin(0, "first", 0).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    write(root.path(), "b.txt", "newer");
    store.begin(1, "second", 1).unwrap();
    store.capture_before("b.txt", CaptureBeforeOpts::default());
    store.gc().unwrap();

    assert!(!dir.join("turn-0.json").exists());
    assert!(blobs.has(&restore_reference));
    assert!(blobs.has(&forward_reference));
}

#[test]
fn expired_metadata_reload_keeps_gap_and_disabled_reason() {
    let (_temp, root, dir, store) = fixture(1, DEFAULT_BLOB_QUOTA_BYTES);
    write(root.path(), "a.txt", "before");
    store.begin(0, "first", 0).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    write(root.path(), "b.txt", "next");
    store.begin(1, "second", 1).unwrap();
    store.capture_before("b.txt", CaptureBeforeOpts::default());
    store.gc().unwrap();

    let reloaded = Store::new(Some(&dir), root.path());
    let expired = reloaded
        .list()
        .into_iter()
        .find(|meta| meta.turn == 0)
        .unwrap();
    assert!(expired.expired_file_payload);
    assert_eq!(expired.coverage, Coverage::Partial);
    assert_eq!(expired.disabled_reason, "file recovery payload expired");
    assert!(
        expired
            .coverage_gaps
            .iter()
            .any(|gap| gap.reason == GAP_EXPIRED_PAYLOAD)
    );
    assert!(reloaded.unreadable_turns().is_empty());
}

#[test]
fn zero_retention_disables_expiration_but_not_explicit_pruning() {
    let (_temp, root, dir, store) = fixture(0, 0);
    write(root.path(), "a.txt", "before");
    store.begin(0, "first", 0).unwrap();
    store.capture_before("a.txt", CaptureBeforeOpts::default());
    let blobs = store.blobs().unwrap();
    let live = digest(b"before");
    let orphan = blobs.put(b"orphan").unwrap();
    store.gc().unwrap();
    assert!(dir.join("turn-0.json").exists());
    assert!(!store.list()[0].expired_file_payload);
    assert!(blobs.has(&orphan));

    store.prune_blobs().unwrap();
    assert!(!blobs.has(&orphan));
    assert!(blobs.has(&live));
    assert!(!store.list()[0].expired_file_payload);
}

#[test]
fn transaction_wire_defaults_round_trip_for_future_protocol() {
    let manifest = TransactionManifest {
        schema_version: SCHEMA_V2,
        id: "tx-1".into(),
        state: TransactionState::Prepared,
        scope: RewindScope::Code,
        targets: vec![TransactionTarget::default()],
        ..TransactionManifest::default()
    };
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let decoded: TransactionManifest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, manifest);
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["state"], "prepared");
    assert_eq!(value["scope"], 0);
    assert_eq!(value["targets"][0]["published"], false);
}

#[test]
fn gap_constants_cover_the_contract_reasons() {
    let reasons = [
        GAP_BASH_SIDE_EFFECT,
        GAP_HOOK_WRITE,
        GAP_MCP_EXTERNAL,
        GAP_OUTSIDE_WORKSPACE,
        GAP_SYMLINK,
        GAP_HARDLINK,
        GAP_UNREADABLE,
        GAP_OVERSIZED,
        GAP_BACKGROUND_WRITER,
        GAP_WORKSPACE_CLAIM_YIELDED,
        GAP_LEGACY_UNVERIFIED,
        GAP_CAPTURE_FAILED,
        GAP_EXPIRED_PAYLOAD,
    ];
    assert_eq!(reasons.len(), 13);
    assert_eq!(json!(reasons[0]), json!("bash_side_effect"));
    assert_eq!(json!(reasons[12]), json!("expired_file_payload"));
}
