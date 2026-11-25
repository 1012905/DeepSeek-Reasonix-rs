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
fn committed_rewind_records_an_internal_offer_without_advertising_undo() {
    let (fixture, result) = committed_fixture();
    assert!(result.ok);
    assert!(!result.undo_available);
    let offer = fixture.store.current_undo_offer().unwrap();
    assert_eq!(offer.id, result.transaction_id);
    assert_eq!(offer.state, TransactionState::Committed);
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
