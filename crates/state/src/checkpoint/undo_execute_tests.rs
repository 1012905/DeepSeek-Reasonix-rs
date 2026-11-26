use std::fs;
use std::path::Path;

use super::transaction_test_support::{
    ConversationCall, Fixture, MockApplier, active_writer, committed_fixture, fixture, load,
    manifest_path, modify_add_delete_fixture, plan, read, single_manifest, transaction_paths,
    write, write_bytes,
};
use super::*;

fn undo(store: &Store, transaction_id: &str) -> Result<RewindResult, TransactionError> {
    store.undo_rewind(transaction_id, None, None)
}

fn manifests(fixture: &Fixture) -> Vec<TransactionManifest> {
    transaction_paths(&fixture.dir)
        .into_iter()
        .map(|path| read_json_file(&path).unwrap())
        .collect()
}

fn inline_fixture(created_empty: bool) -> Fixture {
    let fixture = fixture();
    let store = Store::new(None, fixture.root.path());
    write(fixture.root.path(), "empty.txt", "before");
    store.begin(0, "empty forward", 0).unwrap();
    store.capture_before("empty.txt", CaptureBeforeOpts::default());
    write_bytes(fixture.root.path(), "empty.txt", b"");
    store
        .capture_after(
            "empty.txt",
            CaptureAfterOpts {
                seq: 1,
                ..Default::default()
            },
        )
        .unwrap();
    if created_empty {
        store.capture_before("created-empty.txt", CaptureBeforeOpts::default());
        write_bytes(fixture.root.path(), "created-empty.txt", b"");
        store
            .capture_after(
                "created-empty.txt",
                CaptureAfterOpts {
                    seq: 2,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    let rewind = plan(&store, RewindScope::Code);
    assert!(rewind.can_files, "{rewind:?}");
    let result = store
        .commit_rewind(&rewind.plan_id, 7, b"", None, None)
        .unwrap();
    assert!(result.ok);
    assert!(result.undo_available);
    Fixture { store, ..fixture }
}

#[test]
fn undo_rewind_swaps_forward_and_restore_payloads() {
    let (fixture, rewind) = committed_fixture();
    let original = single_manifest(&fixture.dir).0;

    let result = undo(&fixture.store, &rewind.transaction_id).unwrap();

    assert!(result.ok);
    assert!(!result.undo_available);
    assert!(!result.conversation_ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert!(!fixture.root.path().join("c.txt").exists());
    assert!(fixture.store.current_undo_offer().is_none());
    assert_eq!(fixture.store.last_undo_transaction_id(), "");

    let durable = manifests(&fixture);
    assert_eq!(durable.len(), 2);
    let inverse = durable
        .iter()
        .find(|transaction| transaction.kind == "undo")
        .expect("inverse manifest");
    assert_eq!(inverse.state, TransactionState::Committed);
    assert_eq!(inverse.parent_transaction, original.id);
    assert_eq!(inverse.targets.len(), original.targets.len());
    for (forward, inverse) in original.targets.iter().zip(&inverse.targets) {
        assert_eq!(inverse.path, forward.path);
        assert_eq!(inverse.restore_existed, forward.forward_existed);
        assert_eq!(inverse.restore_mode, forward.forward_mode);
        assert_eq!(inverse.restore_sha, forward.forward_sha);
        assert_eq!(inverse.restore_blob, forward.forward_blob);
        assert_eq!(inverse.restore_inline, forward.forward_inline);
        assert_eq!(inverse.forward_existed, forward.restore_existed);
        assert_eq!(inverse.forward_mode, forward.restore_mode);
        assert_eq!(inverse.forward_sha, forward.restore_sha);
        assert_eq!(inverse.forward_blob, forward.restore_blob);
        assert_eq!(inverse.forward_inline, forward.restore_inline);
        assert!(inverse.published);
        assert!(inverse.publish_tmp.is_empty() || !Path::new(&inverse.publish_tmp).exists());
        assert!(inverse.backup_path.is_empty() || !Path::new(&inverse.backup_path).exists());
    }
    let retired = durable
        .iter()
        .find(|transaction| transaction.id == original.id)
        .unwrap();
    assert_eq!(retired.state, TransactionState::Undone);
}

#[test]
fn undo_syncs_checkpoint_ownership_back_to_forward_images() {
    let (fixture, rewind) = committed_fixture();
    let owned = |path: &str| {
        fixture
            .store
            .revisions()
            .into_iter()
            .find(|file| file.path == path)
            .expect("owned file")
    };
    assert_eq!(owned("a.txt").after_sha256, digest(b"a-before"));
    assert_eq!(owned("b.txt").after_existed, Some(false));

    assert!(undo(&fixture.store, &rewind.transaction_id).unwrap().ok);

    assert_eq!(owned("a.txt").after_sha256, digest(b"a-after"));
    assert_eq!(owned("a.txt").after_existed, Some(true));
    assert_eq!(owned("b.txt").after_sha256, digest(b"b-created"));
    assert_eq!(owned("b.txt").after_existed, Some(true));
    assert_eq!(owned("c.txt").after_existed, Some(false));
    assert!(owned("c.txt").after_sha256.is_empty());
}

#[test]
fn undo_staging_distinguishes_missing_from_present_empty_inline() {
    let fixture = inline_fixture(true);
    let offer = fixture.store.current_undo_offer().unwrap();
    let empty = offer
        .targets
        .iter()
        .find(|target| target.path == "empty.txt")
        .unwrap();
    assert_eq!(empty.forward_inline, Some(Vec::new()));
    assert_eq!(empty.restore_inline, Some(b"before".to_vec()));
    let created = offer
        .targets
        .iter()
        .find(|target| target.path == "created-empty.txt")
        .unwrap();
    assert_eq!(created.restore_inline, None);
    assert_eq!(created.forward_inline, Some(Vec::new()));

    let result = undo(&fixture.store, &offer.id).unwrap();

    assert!(result.ok);
    assert!(read(fixture.root.path(), "empty.txt").is_empty());
    assert!(read(fixture.root.path(), "created-empty.txt").is_empty());
    assert!(fixture.store.available_undo().is_none());
}

#[test]
fn undo_staging_failure_aborts_inverse_and_preserves_parent_offer() {
    let (fixture, rewind) = committed_fixture();
    let original = single_manifest(&fixture.dir).0;
    let created = original
        .targets
        .iter()
        .find(|target| target.path == "b.txt")
        .expect("created target");
    assert!(!created.forward_blob.is_empty());
    fixture
        .store
        .blobs()
        .unwrap()
        .remove(&created.forward_blob)
        .unwrap();

    let error = undo(&fixture.store, &rewind.transaction_id).unwrap_err();

    assert!(matches!(
        error,
        TransactionError::MissingPayload { path, .. } if path == "b.txt"
    ));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert!(!fixture.root.path().join("b.txt").exists());
    assert_eq!(read(fixture.root.path(), "c.txt"), b"c-before");
    let leftovers: Vec<_> = fs::read_dir(fixture.root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().contains(".reasonix-"))
        .collect();
    assert!(leftovers.is_empty(), "stale undo temps: {leftovers:?}");

    let inverse = single_manifest_by_kind(&fixture, "undo");
    assert_eq!(inverse.state, TransactionState::Aborted);
    assert_eq!(inverse.parent_transaction, original.id);
    assert!(inverse.targets.iter().all(|target| !target.published));
    assert_eq!(
        fixture.store.available_undo().unwrap().transaction_id,
        rewind.transaction_id
    );
    let reloaded = Store::new(Some(&fixture.dir), fixture.root.path());
    assert_eq!(
        reloaded.available_undo().unwrap().transaction_id,
        rewind.transaction_id
    );
}

#[test]
fn undo_rewind_restores_empty_forward_file() {
    let fixture = inline_fixture(false);
    let offer = fixture.store.available_undo().unwrap();

    let result = fixture
        .store
        .undo_rewind(&offer.transaction_id, None, None)
        .unwrap();

    assert!(result.ok);
    assert_eq!(read(fixture.root.path(), "empty.txt"), b"");
    assert!(fixture.root.path().join("empty.txt").is_file());
}

#[test]
fn undo_rewind_rejects_permission_only_change() {
    let (fixture, rewind) = committed_fixture();
    chmod(fixture.root.path(), Path::new("a.txt"), 0o444).unwrap();
    let before = read(fixture.root.path(), "a.txt");

    let error = undo(&fixture.store, &rewind.transaction_id).unwrap_err();

    let TransactionError::UndoFileConflicts(conflicts) = error else {
        panic!("expected undo file conflicts");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "a.txt");
    assert_eq!(conflicts[0].reason, CONFLICT_MODE_CHANGE);
    assert_eq!(read(fixture.root.path(), "a.txt"), before);
    assert!(fixture.store.available_undo().is_some());
}

#[test]
fn undo_rewind_rejects_unknown_or_noncommitted_transaction() {
    let (fixture, rewind) = committed_fixture();

    for id in ["missing", &rewind.transaction_id] {
        if id == "missing" {
            let error = undo(&fixture.store, id).unwrap_err();
            assert!(matches!(
                error,
                TransactionError::UndoNotAvailable { transaction_id } if transaction_id == id
            ));
        }
    }
    fixture.store.invalidate_undo().unwrap();
    let error = undo(&fixture.store, &rewind.transaction_id).unwrap_err();
    assert!(matches!(
        error,
        TransactionError::UndoNotAvailable { transaction_id } if transaction_id == rewind.transaction_id
    ));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
}

#[test]
fn undo_offer_is_hidden_while_writer_or_recovery_pending() {
    let (fixture, rewind) = committed_fixture();
    assert_eq!(
        fixture.store.available_undo().unwrap().transaction_id,
        rewind.transaction_id
    );
    assert_eq!(
        fixture.store.last_undo_transaction_id(),
        rewind.transaction_id
    );

    let writer = fixture.store.barrier().try_enter_write().unwrap();
    assert!(fixture.store.available_undo().is_none());
    assert_eq!(
        fixture.store.last_undo_transaction_id(),
        rewind.transaction_id
    );
    drop(writer);

    fixture.store.state().active_writers.push(active_writer());
    assert!(fixture.store.available_undo().is_none());
    assert_eq!(
        fixture.store.last_undo_transaction_id(),
        rewind.transaction_id
    );
    fixture.store.state().active_writers.clear();

    let mut pending = fixture.store.current_undo_offer().unwrap();
    pending.id = "pending".to_owned();
    let mut tracked = fixture.store.track_undo_transaction(&mut pending).unwrap();
    assert!(fixture.store.available_undo().is_none());
    assert_eq!(fixture.store.last_undo_transaction_id(), "");
    tracked.state = TransactionState::Aborted;
    drop(tracked);
    assert_eq!(
        fixture.store.available_undo().unwrap().transaction_id,
        rewind.transaction_id
    );
}

#[test]
fn undo_offer_does_no_disk_io_and_defers_validation_to_execution() {
    let (fixture, rewind) = committed_fixture();
    let generation = fixture.store.barrier().generation();
    let held = fixture._temp.path().join("held-checkpoints");
    fs::rename(&fixture.dir, &held).unwrap();

    assert_eq!(
        fixture.store.available_undo().unwrap().transaction_id,
        rewind.transaction_id
    );
    assert_eq!(
        fixture.store.last_undo_transaction_id(),
        rewind.transaction_id
    );
    assert_eq!(fixture.store.barrier().generation(), generation);
    fs::rename(&held, &fixture.dir).unwrap();

    write(fixture.root.path(), "a.txt", "external edit");
    assert_eq!(
        fixture.store.available_undo().unwrap().transaction_id,
        rewind.transaction_id
    );
    let error = undo(&fixture.store, &rewind.transaction_id).unwrap_err();
    assert!(matches!(error, TransactionError::UndoFileConflicts(_)));
    assert_eq!(read(fixture.root.path(), "a.txt"), b"external edit");
}

#[test]
fn undo_execution_rechecks_barrier_active_writers_and_pending_recovery() {
    let (fixture, rewind) = committed_fixture();
    let writer = fixture.store.barrier().try_enter_write().unwrap();
    assert!(matches!(
        undo(&fixture.store, &rewind.transaction_id),
        Err(TransactionError::BarrierBusy)
    ));
    drop(writer);

    fixture.store.state().active_writers.push(active_writer());
    assert!(matches!(
        undo(&fixture.store, &rewind.transaction_id),
        Err(TransactionError::ActiveWriters)
    ));
    fixture.store.state().active_writers.clear();

    let mut pending = fixture.store.current_undo_offer().unwrap();
    pending.id = "pending".to_owned();
    let mut tracked = fixture.store.track_undo_transaction(&mut pending).unwrap();
    assert!(matches!(
        undo(&fixture.store, &rewind.transaction_id),
        Err(TransactionError::Store(StoreError::UndoRecoveryPending))
    ));
    tracked.state = TransactionState::Aborted;
    drop(tracked);
}

#[test]
fn undo_checkpoint_restore_failure_reapplies_original_rewind() {
    let fixture = modify_add_delete_fixture();
    let rewind = fixture
        .store
        .prepare_rewind(0, RewindScope::Both, 7, 5, true);
    let applier = MockApplier::default();
    fixture
        .store
        .commit_rewind(
            &rewind.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            None,
        )
        .unwrap();
    applier.calls.lock().unwrap().clear();
    applier.fail_at(ConversationPhase::RestoreCheckpoints);

    let error = fixture
        .store
        .undo_rewind(
            &fixture.store.last_undo_transaction_id(),
            Some(&applier),
            None,
        )
        .unwrap_err();

    assert!(
        error
            .conversation_phases()
            .contains(&ConversationPhase::RestoreCheckpoints)
    );
    assert_eq!(
        *applier.calls.lock().unwrap(),
        vec![
            ConversationCall::RestoreConversation(b"conversation-forward".to_vec()),
            ConversationCall::RestoreCheckpoints(
                single_manifest_by_kind(&fixture, "rewind").checkpoint_backup
            ),
            ConversationCall::ApplyConversationTruncate {
                boundary: 5,
                forward: b"conversation-forward".to_vec(),
            },
            ConversationCall::TruncateCheckpoints(0),
        ]
    );
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-before");
    assert!(!fixture.root.path().join("b.txt").exists());
    assert_eq!(read(fixture.root.path(), "c.txt"), b"c-before");
    assert!(fixture.store.available_undo().is_some());
    assert_eq!(
        single_manifest_by_kind(&fixture, "undo").state,
        TransactionState::Aborted
    );
}

fn single_manifest_by_kind(fixture: &Fixture, kind: &str) -> TransactionManifest {
    manifests(fixture)
        .into_iter()
        .find(|transaction| transaction.kind == kind)
        .unwrap_or_else(|| panic!("missing {kind} manifest"))
}

#[test]
fn undo_parent_mark_failure_still_suppresses_stale_offer_after_reload() {
    let (fixture, rewind) = committed_fixture();
    let injection = TransactionInjection::fail(TransactionPhase::PersistParentUndone);

    let result = fixture
        .store
        .undo_rewind(&rewind.transaction_id, None, Some(&injection))
        .unwrap();

    assert!(result.ok);
    assert!(!result.undo_available);
    assert!(fixture.store.available_undo().is_none());
    let parent: TransactionManifest =
        read_json_file(&manifest_path(&fixture.dir, &rewind.transaction_id)).unwrap();
    assert_eq!(parent.state, TransactionState::Committed);
    let inverse = single_manifest_by_kind(&fixture, "undo");
    assert_eq!(inverse.state, TransactionState::Committed);
    assert_eq!(inverse.parent_transaction, rewind.transaction_id);

    let reloaded = Store::new(Some(&fixture.dir), fixture.root.path());
    assert!(reloaded.available_undo().is_none());
    assert_eq!(reloaded.last_undo_transaction_id(), "");
}

struct StoreTruncatingApplier<'a> {
    store: &'a Store,
}

impl ConversationApplier for StoreTruncatingApplier<'_> {
    fn apply_conversation_truncate(
        &self,
        _boundary: i64,
        _forward: &[u8],
    ) -> Result<(), ConversationError> {
        Ok(())
    }

    fn restore_conversation(&self, _forward: &[u8]) -> Result<(), ConversationError> {
        Ok(())
    }

    fn truncate_checkpoints(&self, from_turn: i64) -> Result<(), ConversationError> {
        self.store.truncate_from(from_turn).map_err(|source| {
            ConversationError::external(ConversationPhase::TruncateCheckpoints, source)
        })
    }

    fn restore_checkpoints(&self, _backup: &[u8]) -> Result<(), ConversationError> {
        Ok(())
    }
}

#[test]
fn store_only_undo_restores_checkpoint_backup() {
    let fixture = modify_add_delete_fixture();
    let rewind = fixture
        .store
        .prepare_rewind(0, RewindScope::Both, 7, 5, true);
    let applier = StoreTruncatingApplier {
        store: &fixture.store,
    };
    let committed = fixture
        .store
        .commit_rewind(
            &rewind.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            None,
        )
        .unwrap();
    assert!(fixture.store.list().is_empty());
    assert!(!single_manifest(&fixture.dir).0.checkpoint_backup.is_empty());

    let result = undo(&fixture.store, &committed.transaction_id).unwrap();

    assert!(result.ok);
    assert!(!result.undo_available);
    assert!(!result.conversation_ok);
    assert_eq!(fixture.store.list().len(), 1);
    assert_eq!(fixture.store.list()[0].turn, 0);
    assert!(fixture.store.state().current.is_none());
    assert_eq!(fixture.store.state().done.len(), 1);
    assert!(turn_path(&fixture.dir, 0, false).is_file());
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert_eq!(read(fixture.root.path(), "b.txt"), b"b-created");
    assert!(!fixture.root.path().join("c.txt").exists());

    let reloaded = load(fixture.root.path(), &fixture.dir, &[]);
    assert_eq!(reloaded.list().len(), 1);
    assert_eq!(reloaded.list()[0].turn, 0);
}

#[test]
fn available_undo_survives_reload_and_restores_files() {
    let (fixture, rewind) = committed_fixture();
    let reloaded = Store::new(Some(&fixture.dir), fixture.root.path());
    let offer = reloaded.available_undo().unwrap();
    assert_eq!(offer.transaction_id, rewind.transaction_id);
    assert_eq!(offer.turn, 0);
    assert_eq!(offer.files, 3);

    assert!(undo(&reloaded, &offer.transaction_id).unwrap().ok);
    assert_eq!(read(fixture.root.path(), "a.txt"), b"a-after");
    assert!(
        Store::new(Some(&fixture.dir), fixture.root.path())
            .available_undo()
            .is_none()
    );
}

#[test]
fn successful_rewind_rejects_undo_with_a_newer_offer() {
    let (fixture, first) = committed_fixture();
    let second = fixture
        .store
        .prepare_rewind(0, RewindScope::Conversation, 8, 5, true);
    let committed = fixture
        .store
        .commit_rewind(&second.plan_id, 8, b"next-forward", None, None)
        .unwrap();
    assert_ne!(first.transaction_id, committed.transaction_id);

    let error = undo(&fixture.store, &first.transaction_id).unwrap_err();
    assert!(matches!(
        error,
        TransactionError::UndoNotAvailable { transaction_id } if transaction_id == first.transaction_id
    ));
}

#[test]
fn checkpoint_backup_restore_removes_stale_expired_counterpart() {
    let fixture = modify_add_delete_fixture();
    let restored = fixture.store.list()[0].turn;
    let rewind = fixture
        .store
        .prepare_rewind(0, RewindScope::Conversation, 7, 5, true);
    let applier = StoreTruncatingApplier {
        store: &fixture.store,
    };
    let committed = fixture
        .store
        .commit_rewind(
            &rewind.plan_id,
            7,
            b"conversation-forward",
            Some(&applier),
            None,
        )
        .unwrap();
    let backup: Vec<Checkpoint> =
        serde_json::from_slice(&single_manifest(&fixture.dir).0.checkpoint_backup).unwrap();
    let mut stale = backup[0].clone();
    stale.expired_file_payload = true;
    write_json_atomic(&turn_path(&fixture.dir, restored, true), &stale).unwrap();

    assert!(undo(&fixture.store, &committed.transaction_id).is_ok());
    assert!(!turn_path(&fixture.dir, restored, true).exists());
    assert!(turn_path(&fixture.dir, restored, false).exists());
}
