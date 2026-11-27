use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::sessionstore::eventlog::record::CreatedAt;

use super::transaction_test_support::{committed_fixture, fixture, manifest_path, save_manifest};
use super::*;

fn terminal(id: &str, state: TransactionState, created_at: &str) -> TransactionManifest {
    TransactionManifest {
        id: id.into(),
        state,
        kind: "rewind".into(),
        created_at: CreatedAt::from_datetime(
            chrono::DateTime::parse_from_rfc3339(created_at).unwrap(),
        ),
        updated_at: CreatedAt::now(),
        ..TransactionManifest::default()
    }
}

#[test]
fn committed_rewind_advertises_public_undo_offer() {
    let (fixture, result) = committed_fixture();
    assert!(result.ok);
    assert!(result.undo_available);
    let offer = fixture.store.current_undo_offer().unwrap();
    assert_eq!(offer.id, result.transaction_id);
    assert_eq!(offer.state, TransactionState::Committed);
}

#[test]
fn tracked_offer_notification_observes_cleared_pending() {
    let (fixture, _) = committed_fixture();
    let store = Arc::new(Store::new(Some(&fixture.dir), fixture.root.path()));
    let notifications = Arc::new(AtomicUsize::new(0));
    let callback_notifications = notifications.clone();
    let callback_store = Arc::downgrade(&store);
    store.on_undo_change(move || {
        let store = callback_store.upgrade().expect("callback owns the store");
        store.require_undo_recovery_complete().unwrap();
        assert!(store.current_undo_offer().is_some());
        callback_notifications.fetch_add(1, Ordering::Relaxed);
    });
    let mut transaction = store.current_undo_offer().unwrap();
    transaction.id = "next".to_owned();

    let tracked = store.track_undo_transaction(&mut transaction).unwrap();
    assert!(store.current_undo_offer().is_none());
    store.publish_undo_offer(&tracked);
    assert_eq!(notifications.load(Ordering::Relaxed), 0);
    drop(tracked);
    assert_eq!(notifications.load(Ordering::Relaxed), 1);
    assert_eq!(store.current_undo_offer().unwrap().id, "next");
}

#[test]
fn restoring_a_hidden_offer_does_not_notify() {
    let (fixture, _) = committed_fixture();
    let store = Store::new(Some(&fixture.dir), fixture.root.path());
    let notifications = Arc::new(AtomicUsize::new(0));
    let callback_notifications = notifications.clone();
    store.on_undo_change(move || {
        callback_notifications.fetch_add(1, Ordering::Relaxed);
    });
    let transaction = store.current_undo_offer().unwrap();
    let mut tracked_transaction = transaction.clone();
    let tracked = store
        .track_undo_transaction(&mut tracked_transaction)
        .unwrap();
    let pending = TransactionManifest {
        state: TransactionState::Committing,
        ..TransactionManifest::default()
    };

    store.restore_undo_slot(Some(transaction), vec![pending]);

    assert!(store.current_undo_offer().is_none());
    assert_eq!(notifications.load(Ordering::Relaxed), 0);
    drop(tracked);
}

#[test]
fn invalidation_is_durable_and_survives_reload() {
    let (fixture, result) = committed_fixture();
    fixture.store.invalidate_undo().unwrap();

    assert!(fixture.store.current_undo_offer().is_none());
    assert_eq!(
        super::transaction_test_support::single_manifest(&fixture.dir)
            .0
            .state,
        TransactionState::Invalidated
    );
    let reloaded = Store::new(Some(&fixture.dir), fixture.root.path());
    assert!(reloaded.current_undo_offer().is_none());
    assert_eq!(
        super::transaction_test_support::single_manifest(&fixture.dir)
            .0
            .id,
        result.transaction_id
    );
}

#[test]
fn persistence_failure_keeps_the_in_memory_offer_and_does_not_notify() {
    let (fixture, result) = committed_fixture();
    let path = manifest_path(&fixture.dir, &result.transaction_id);
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let notifications = Arc::new(AtomicUsize::new(0));
    let callback_notifications = notifications.clone();
    fixture.store.on_undo_change(move || {
        callback_notifications.fetch_add(1, Ordering::Relaxed);
    });

    let error = fixture
        .store
        .invalidate_undo()
        .expect_err("persist failure");
    assert!(matches!(error, StoreError::UndoInvalidation { .. }));
    assert_eq!(
        fixture.store.current_undo_offer().unwrap().id,
        result.transaction_id
    );
    assert_eq!(notifications.load(Ordering::Relaxed), 0);

    fs::remove_dir(&path).unwrap();
    fs::write(&path, original).unwrap();
    fixture.store.invalidate_undo().unwrap();
    assert_eq!(notifications.load(Ordering::Relaxed), 1);
    assert!(fixture.store.current_undo_offer().is_none());
}

#[test]
fn beginning_a_turn_retires_the_offer_durably() {
    let (fixture, result) = committed_fixture();
    let notifications = Arc::new(AtomicUsize::new(0));
    let callback_notifications = notifications.clone();
    fixture.store.on_undo_change(move || {
        callback_notifications.fetch_add(1, Ordering::Relaxed);
    });

    fixture.store.begin(8, "next turn", 8).unwrap();

    assert!(fixture.store.current_undo_offer().is_none());
    assert_eq!(notifications.load(Ordering::Relaxed), 1);
    assert_eq!(
        super::transaction_test_support::single_manifest(&fixture.dir)
            .0
            .state,
        TransactionState::Invalidated
    );
    assert!(
        Store::new(Some(&fixture.dir), fixture.root.path())
            .current_undo_offer()
            .is_none()
    );
    assert_eq!(
        result.transaction_id,
        super::transaction_test_support::single_manifest(&fixture.dir)
            .0
            .id
    );
}

#[test]
fn committed_backup_cleanup_failure_does_not_fail_rewind_or_hide_the_offer() {
    struct BlockBackupCleanup {
        root: std::path::PathBuf,
    }

    impl ConversationApplier for BlockBackupCleanup {
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

        fn truncate_checkpoints(&self, _from_turn: i64) -> Result<(), ConversationError> {
            let mut blocked = 0;
            for entry in fs::read_dir(&self.root).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|extension| extension == "bak") {
                    fs::remove_file(&path).unwrap();
                    fs::create_dir(&path).unwrap();
                    fs::write(path.join("blocker"), b"keep backup cleanup failing").unwrap();
                    blocked += 1;
                }
            }
            assert!(blocked > 0, "rewind produced no backup to block");
            Ok(())
        }

        fn restore_checkpoints(&self, _backup: &[u8]) -> Result<(), ConversationError> {
            Ok(())
        }
    }

    let fixture = super::transaction_test_support::modify_add_delete_fixture();
    let rewind = super::transaction_test_support::plan(&fixture.store, RewindScope::Both);
    let applier = BlockBackupCleanup {
        root: fixture.root.path().to_path_buf(),
    };
    let result = fixture
        .store
        .commit_rewind(&rewind.plan_id, 7, b"forward", Some(&applier), None)
        .expect("committed rewind survives best-effort backup cleanup");

    assert!(result.ok);
    assert!(result.undo_available);
    assert_eq!(
        fixture.store.current_undo_offer().unwrap().id,
        result.transaction_id
    );
}

#[test]
fn startup_restores_the_latest_committed_offer() {
    let fixture = fixture();
    save_manifest(
        &fixture.dir,
        &terminal("older", TransactionState::Committed, "2026-01-01T00:00:00Z"),
    );
    save_manifest(
        &fixture.dir,
        &terminal("newer", TransactionState::Committed, "2026-01-02T00:00:00Z"),
    );

    let store = Store::new(Some(&fixture.dir), fixture.root.path());
    assert_eq!(store.current_undo_offer().unwrap().id, "newer");
}

#[test]
fn a_committed_undo_manifest_suppresses_its_parent_offer() {
    let fixture = fixture();
    save_manifest(
        &fixture.dir,
        &terminal(
            "parent",
            TransactionState::Committed,
            "2026-01-01T00:00:00Z",
        ),
    );
    let mut undo = terminal("undo", TransactionState::Committed, "2026-01-02T00:00:00Z");
    undo.kind = "undo".into();
    undo.parent_transaction = "parent".into();
    save_manifest(&fixture.dir, &undo);

    let store = Store::new(Some(&fixture.dir), fixture.root.path());
    assert!(store.current_undo_offer().is_none());
}

#[test]
fn a_newer_invalidated_manifest_suppresses_an_older_committed_offer() {
    let fixture = fixture();
    save_manifest(
        &fixture.dir,
        &terminal("older", TransactionState::Committed, "2026-01-01T00:00:00Z"),
    );
    save_manifest(
        &fixture.dir,
        &terminal(
            "newer",
            TransactionState::Invalidated,
            "2026-01-02T00:00:00Z",
        ),
    );

    let store = Store::new(Some(&fixture.dir), fixture.root.path());
    assert!(store.current_undo_offer().is_none());
}
