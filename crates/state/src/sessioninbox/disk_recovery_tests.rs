use reasonix_contract::gojson::marshal;
use std::fs;

use super::disk::Store;
use super::disk_tests::support;
use super::{
    BLOCK_MANIFEST_SALVAGED, BLOCK_OWNER_INACTIVE, Error, Limits, PAUSE_RECOVERED, PromptEnvelope,
    SCHEMA_VERSION, STATE_BLOCKED, STATE_QUEUED, STATE_RUNNING, STATE_STEER_ACCEPTED,
    STATE_STEER_CONSUMED, STATE_UNCERTAIN, decode_manifest, empty_manifest,
    idempotency_request_hash, process_run_id, sha256_hex,
};
use crate::store::naming::session_inbox_dir;

#[test]
fn missing_manifest_opens_as_an_empty_writable_queue() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert_eq!(snap.schema_version, SCHEMA_VERSION);
    assert_eq!(snap.revision, 0);
    assert_eq!(snap.run_id, process_run_id());
    assert!(!snap.paused);
    assert!(!snap.recovered);
    assert!(!snap.readonly);
    assert!(!store.dir().join("manifest.json").exists());
}

#[test]
fn current_schema_adopts_the_process_run_without_rewriting() {
    let temp = tempfile::tempdir().unwrap();
    let mut manifest = empty_manifest("");
    manifest.revision = 9;
    support::write_manifest(temp.path(), "session", &manifest);
    let before =
        fs::read(session_inbox_dir(&temp.path().join("session.jsonl")).join("manifest.json"))
            .unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert_eq!(snap.revision, 9);
    assert_eq!(snap.run_id, process_run_id());
    assert_eq!(fs::read(store.dir().join("manifest.json")).unwrap(), before);
}

#[test]
fn schema_zero_and_one_backfill_hashes_from_blobs() {
    for schema in [0, 1] {
        let temp = tempfile::tempdir().unwrap();
        let env = PromptEnvelope {
            display_text: "display".to_owned(),
            raw_text: "raw".to_owned(),
            submit_text: "submit".to_owned(),
            ..PromptEnvelope::default()
        };
        support::write_envelope(temp.path(), "session", "item", &env);
        let mut item = support::item("item", "queued");
        item.byte_size = i64::try_from(marshal(&env).unwrap().len()).unwrap();
        item.checksum = sha256_hex(marshal(&env).unwrap().as_bytes());
        let mut manifest = empty_manifest("");
        manifest.schema_version = schema;
        manifest.revision = 5;
        manifest.items.push(item);
        manifest
            .idempotency
            .insert("request-key".to_owned(), "item".to_owned());
        support::write_manifest(temp.path(), "session", &manifest);

        let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
        let disk = decode_manifest(&fs::read(store.dir().join("manifest.json")).unwrap()).unwrap();
        assert_eq!(disk.schema_version, SCHEMA_VERSION);
        assert_eq!(disk.revision, 6);
        assert_eq!(disk.run_id, process_run_id());
        assert_eq!(
            disk.idempotency_hashes.get("request-key").unwrap(),
            &idempotency_request_hash(&env).unwrap()
        );
    }
}

#[test]
fn schema_two_missing_hash_is_rejected_before_migration() {
    let temp = tempfile::tempdir().unwrap();
    let env = PromptEnvelope {
        display_text: "body".to_owned(),
        ..PromptEnvelope::default()
    };
    support::write_envelope(temp.path(), "session", "item", &env);
    let mut manifest = empty_manifest("");
    manifest.schema_version = 2;
    manifest.items.push(support::item("item", "queued"));
    manifest
        .idempotency
        .insert("request-key".to_owned(), "item".to_owned());
    let bytes = support::manifest_bytes(&manifest);
    assert!(matches!(
        decode_manifest(&bytes),
        Err(Error::MissingIdempotencyRequestHash)
    ));

    let dir = support::inbox(temp.path(), "session");
    fs::create_dir_all(dir.join("blobs")).unwrap();
    fs::write(dir.join("manifest.json"), &bytes).unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert!(snap.paused);
    assert!(snap.recovered);
    assert_eq!(snap.recovered_n, 1);
    assert!(
        store
            .state
            .lock()
            .unwrap()
            .queue
            .manifest
            .idempotency
            .is_empty()
    );
}

#[test]
fn missing_migration_target_has_typed_not_found_identity() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let mut constructed = empty_manifest("");
    constructed.schema_version = 1;
    constructed
        .idempotency
        .insert("request-key".to_owned(), "absent".to_owned());

    let state = store.state.lock().unwrap();
    assert!(matches!(
        store.backfill_idempotency_hashes_at(&state, &mut constructed),
        Err(Error::NotFound)
    ));
}

#[test]
fn foreign_run_recovers_all_rows_and_pauses_for_review() {
    let temp = tempfile::tempdir().unwrap();
    let states = [
        STATE_RUNNING,
        STATE_STEER_ACCEPTED,
        STATE_STEER_CONSUMED,
        STATE_QUEUED,
        STATE_BLOCKED,
        STATE_UNCERTAIN,
    ];
    let mut manifest = empty_manifest("foreign-process");
    manifest.revision = 3;
    for (index, state) in states.iter().enumerate() {
        manifest
            .items
            .push(support::item(&format!("item-{index}"), state.as_str()));
    }
    support::write_manifest(temp.path(), "session", &manifest);

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert!(snap.paused);
    assert_eq!(snap.pause_code, PAUSE_RECOVERED);
    assert!(snap.recovered);
    assert_eq!(snap.recovered_n, 6);
    assert_eq!(snap.run_id, process_run_id());
    let items = snap.items.unwrap();
    for item in &items[..3] {
        assert_eq!(item.state, STATE_UNCERTAIN);
        assert_eq!(item.block_code, BLOCK_OWNER_INACTIVE);
        assert_eq!(item.block_reason, "in-flight owner is no longer active");
        assert!(!item.updated_at.is_zero());
    }
    assert_eq!(items[3].state, STATE_QUEUED);
    assert_eq!(items[4].state, STATE_BLOCKED);
    assert_eq!(items[5].state, STATE_UNCERTAIN);
    let disk = decode_manifest(&fs::read(store.dir().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(disk.run_id, process_run_id());
    assert_eq!(disk.revision, 4);
}

#[test]
fn empty_foreign_run_is_adopted_in_memory_without_a_commit() {
    let temp = tempfile::tempdir().unwrap();
    let mut manifest = empty_manifest("foreign-process");
    manifest.revision = 8;
    support::write_manifest(temp.path(), "session", &manifest);
    let before =
        fs::read(session_inbox_dir(&temp.path().join("session.jsonl")).join("manifest.json"))
            .unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert_eq!(snap.run_id, process_run_id());
    assert_eq!(snap.revision, 8);
    assert!(!snap.paused);
    assert!(!snap.recovered);
    assert_eq!(fs::read(store.dir().join("manifest.json")).unwrap(), before);
}

#[test]
fn newer_schema_is_paused_readonly_and_never_rewritten() {
    let temp = tempfile::tempdir().unwrap();
    let mut item = support::item("future", "future_state");
    item.intent = super::INTENT_FOLLOWUP.clone();
    let mut manifest = empty_manifest("future-process");
    manifest.schema_version = SCHEMA_VERSION + 1;
    manifest.revision = 12;
    manifest.items.push(item);
    support::write_manifest(temp.path(), "session", &manifest);
    let before =
        fs::read(session_inbox_dir(&temp.path().join("session.jsonl")).join("manifest.json"))
            .unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert_eq!(snap.schema_version, SCHEMA_VERSION + 1);
    assert_eq!(snap.revision, 12);
    assert!(snap.paused);
    assert!(snap.readonly);
    assert_eq!(fs::read(store.dir().join("manifest.json")).unwrap(), before);
    let state = store.state.lock().unwrap();
    assert!(matches!(
        store.mutable_locked(&state),
        Err(Error::SchemaReadonly)
    ));
}

#[test]
fn orphan_gc_quarantines_unknown_and_unreferenced_entries() {
    let temp = tempfile::tempdir().unwrap();
    let mut manifest = empty_manifest(process_run_id());
    manifest.items.push(support::item("live", "queued"));
    support::write_manifest(temp.path(), "session", &manifest);
    let blobs = support::inbox(temp.path(), "session").join("blobs");
    fs::write(blobs.join("live.json"), b"{}").unwrap();
    fs::write(blobs.join("orphan.json"), b"{}").unwrap();
    fs::write(blobs.join("notes.txt"), b"notes").unwrap();
    fs::create_dir(blobs.join("directory.json")).unwrap();
    fs::write(blobs.join("directory.json/keep"), b"keep").unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    assert!(blobs.join("live.json").exists());
    assert!(!blobs.join("orphan.json").exists());
    assert!(!blobs.join("notes.txt").exists());
    assert!(blobs.join("directory.json/keep").exists());
    let quarantine = store.dir().join("quarantine");
    let names: Vec<String> = fs::read_dir(quarantine)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|name| name.starts_with("orphan.json.")));
    assert!(names.iter().any(|name| name.starts_with("notes.txt.")));
}

#[cfg(unix)]
#[test]
fn orphan_gc_quarantines_non_utf8_entries_without_lossy_rename() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().unwrap();
    support::write_manifest(temp.path(), "session", &empty_manifest(process_run_id()));
    let blobs = support::inbox(temp.path(), "session").join("blobs");
    let invalid = OsString::from_vec(b"orphan-\xff.json".to_vec());
    fs::write(blobs.join(&invalid), b"{}").unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    assert!(!blobs.join(&invalid).exists());
    assert!(
        fs::read_dir(store.dir().join("quarantine"))
            .unwrap()
            .any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .as_bytes()
                    .starts_with(invalid.as_bytes())
            })
    );
}

#[cfg(unix)]
#[test]
fn corrupt_manifest_with_non_utf8_blob_name_stays_unrecovered() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().unwrap();
    let dir = support::inbox(temp.path(), "session");
    fs::create_dir_all(dir.join("blobs")).unwrap();
    let corrupt = b"{corrupt";
    fs::write(dir.join("manifest.json"), corrupt).unwrap();
    let invalid = OsString::from_vec(b"orphan-\xff.json".to_vec());
    fs::write(dir.join("blobs").join(invalid), b"{}").unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert!(snap.unrecovered);
    assert!(!snap.recovered);
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), corrupt);
}

#[test]
fn corrupt_manifest_salvages_bodies_with_complete_stems() {
    let temp = tempfile::tempdir().unwrap();
    let dir = support::inbox(temp.path(), "session");
    fs::create_dir_all(dir.join("blobs")).unwrap();
    fs::write(dir.join("manifest.json"), b"{corrupt").unwrap();
    let env = PromptEnvelope {
        raw_text: "salvaged text".to_owned(),
        ..PromptEnvelope::default()
    };
    let bytes = marshal(&env).unwrap();
    support::write_blob(temp.path(), "session", "owner.part", bytes.as_bytes());
    support::write_envelope(temp.path(), "session", "empty", &PromptEnvelope::default());
    support::write_blob(temp.path(), "session", "invalid", b"not json");
    fs::create_dir(dir.join("blobs/directory.json")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(
            dir.join("blobs/owner.part.json"),
            dir.join("blobs/link.json"),
        )
        .unwrap();
    }

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert!(snap.paused);
    assert_eq!(snap.pause_code, PAUSE_RECOVERED);
    assert!(snap.recovered);
    assert!(!snap.unrecovered);
    assert_eq!(snap.recovered_n, 2);
    let items = snap.items.unwrap();
    let owner = items.iter().find(|item| item.id == "owner.part").unwrap();
    assert_eq!(owner.blob_name, "owner.part");
    assert_eq!(owner.state, STATE_UNCERTAIN);
    assert_eq!(owner.intent, super::INTENT_FOLLOWUP);
    assert_eq!(owner.preview, "salvaged text");
    assert_eq!(owner.byte_size, i64::try_from(bytes.len()).unwrap());
    assert_eq!(owner.checksum, sha256_hex(bytes.as_bytes()));
    assert_eq!(owner.run_id, process_run_id());
    assert_eq!(owner.block_code, BLOCK_MANIFEST_SALVAGED);
    assert_eq!(owner.block_reason, "salvaged after corrupt manifest");
    let empty = items.iter().find(|item| item.id == "empty").unwrap();
    assert_eq!(empty.preview, "(salvaged body)");
    assert!(
        store
            .dir()
            .join("quarantine")
            .read_dir()
            .unwrap()
            .any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("manifest.json.manifest-corrupt.")
            })
    );
}

#[test]
fn unreadable_salvage_preserves_corrupt_bytes_and_blocks_commits() {
    let temp = tempfile::tempdir().unwrap();
    let dir = support::inbox(temp.path(), "session");
    fs::create_dir_all(dir.join("blobs")).unwrap();
    let corrupt = b"{corrupt";
    fs::write(dir.join("manifest.json"), corrupt).unwrap();
    fs::write(dir.join("blobs/large.json"), b"123456").unwrap();
    let limits = Limits {
        max_items: 64,
        max_item_bytes: 4,
        max_total_bytes: 64,
    };

    let store = Store::open(&temp.path().join("session.jsonl"), limits).unwrap();
    let snap = store.snapshot();
    assert!(snap.paused);
    assert!(snap.unrecovered);
    assert!(!snap.recovered);
    assert_eq!(snap.items, None);
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), corrupt);
    assert!(!dir.join("quarantine").exists());
    let mut state = store.state.lock().unwrap();
    assert!(matches!(
        store.commit_manifest_locked(&mut state, empty_manifest(process_run_id())),
        Err(Error::Unrecovered)
    ));
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), corrupt);
}

#[test]
fn missing_blob_directory_is_a_legitimate_recovered_empty_queue() {
    let temp = tempfile::tempdir().unwrap();
    let dir = support::inbox(temp.path(), "session");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("manifest.json"), b"{corrupt").unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert!(snap.paused);
    assert!(snap.recovered);
    assert!(!snap.unrecovered);
    assert_eq!(snap.recovered_n, 0);
    assert_eq!(snap.items, None);
    assert!(decode_manifest(&fs::read(dir.join("manifest.json")).unwrap()).is_ok());
}

#[test]
fn corrupt_manifest_quarantine_errors_are_ignored_like_go() {
    let temp = tempfile::tempdir().unwrap();
    let dir = support::inbox(temp.path(), "session");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("manifest.json"), b"{corrupt").unwrap();
    fs::write(dir.join("quarantine"), b"not a directory").unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snap = store.snapshot();
    assert!(snap.recovered);
    assert!(!snap.unrecovered);
    assert_eq!(
        fs::read(dir.join("quarantine")).unwrap(),
        b"not a directory"
    );
    assert!(decode_manifest(&fs::read(dir.join("manifest.json")).unwrap()).is_ok());
}

#[test]
fn lock_free_read_reports_foreign_run_without_repair() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let mut manifest = empty_manifest("foreign-process");
    manifest.schema_version = 1;
    manifest.revision = 4;
    support::write_manifest(temp.path(), "session", &manifest);
    let manifest_path = store.dir().join("manifest.json");
    let before = fs::read(&manifest_path).unwrap();

    let mut state = store.state.lock().unwrap();
    assert!(matches!(
        store.read_manifest_without_transaction(&mut state),
        Err(Error::ForeignRun)
    ));
    assert_eq!(fs::read(&manifest_path).unwrap(), before);
    assert_eq!(state.queue.manifest.revision, 0);
}

#[test]
fn lock_free_missing_manifest_resets_like_go() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let mut state = store.state.lock().unwrap();
    state.queue.manifest.revision = 11;
    state.queue.readonly = true;
    state.queue.unrecovered = true;

    store.read_manifest_without_transaction(&mut state).unwrap();
    assert_eq!(state.queue.manifest.revision, 0);
    assert_eq!(state.queue.manifest.run_id, process_run_id());
    assert!(!state.queue.readonly);
    assert!(state.queue.unrecovered);
}

#[test]
fn lock_free_corrupt_read_does_not_quarantine_or_salvage() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let manifest_path = store.dir().join("manifest.json");
    let quarantine_path = store.dir().join("quarantine");
    fs::write(&manifest_path, b"{corrupt").unwrap();
    let mut state = store.state.lock().unwrap();
    assert!(matches!(
        store.read_manifest_without_transaction(&mut state),
        Err(Error::Json(_))
    ));
    assert_eq!(fs::read(&manifest_path).unwrap(), b"{corrupt");
    assert!(!quarantine_path.exists());
}
