use std::cell::{Cell, RefCell};
use std::fs;
use std::sync::mpsc;
use std::time::Duration;

use super::disk_tests::support;
use super::*;
use crate::store::naming::session_inbox_dir;

fn open_store() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    (temp, store)
}

fn enqueue(store: &Store, text: &str) -> String {
    store
        .enqueue(EnqueueRequest {
            intent: INTENT_FOLLOWUP.clone(),
            envelope: PromptEnvelope {
                submit_text: text.to_owned(),
                ..PromptEnvelope::default()
            },
            source: "test".to_owned(),
            ..EnqueueRequest::default()
        })
        .unwrap()
        .item_id
}

fn item(store: &Store, id: &str) -> InboxItemMeta {
    store
        .snapshot()
        .items
        .unwrap()
        .into_iter()
        .find(|item| item.id == id)
        .unwrap()
}

fn break_transaction_lock(store: &Store) {
    let path = store.dir().join("transaction.lock");
    fs::remove_file(path).unwrap();
    fs::create_dir(store.dir().join("transaction.lock")).unwrap();
}

#[test]
fn nil_owner_recovers_only_active_states_and_preserves_pending_items() {
    let (_temp, store) = open_store();
    let states = [
        STATE_RUNNING,
        STATE_STEER_ACCEPTED,
        STATE_STEER_CONSUMED,
        STATE_QUEUED,
        STATE_BLOCKED,
        STATE_UNCERTAIN,
    ];
    let mut ids = Vec::new();
    for (index, state) in states.iter().enumerate() {
        let id = enqueue(&store, &format!("item-{index}"));
        store
            .set_state_coded(
                &id,
                state.clone(),
                BLOCK_SNAPSHOT_FAILED.clone(),
                "before recovery",
            )
            .unwrap();
        ids.push(id);
    }
    let before = store.snapshot();
    let before_items = before.items.clone().unwrap();

    assert_eq!(store.recover_orphaned_in_flight_owned_by(None).unwrap(), 3);

    let after = store.snapshot();
    assert_eq!(after.revision, before.revision + 1);
    assert!(after.paused);
    assert!(after.recovered);
    assert_eq!(after.recovered_n, 3);
    assert_eq!(after.pause_code, PauseCode::default());
    let after_items = after.items.unwrap();
    for (index, original) in before_items.iter().enumerate() {
        let recovered = &after_items[index];
        assert_eq!(recovered.id, original.id);
        assert_eq!(recovered.created_at, original.created_at);
        if index < 3 {
            assert_eq!(recovered.state, STATE_UNCERTAIN);
            assert_eq!(recovered.block_code, BLOCK_OWNER_INACTIVE);
            assert_eq!(
                recovered.block_reason,
                "in-flight owner is no longer active"
            );
            assert!(recovered.updated_at.get() >= original.updated_at.get());
        } else {
            assert_eq!(recovered, original);
        }
    }
    let disk = decode_manifest(&fs::read(store.dir().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(disk.items[0].state, STATE_UNCERTAIN);
    assert_eq!(disk.items[1].state, STATE_UNCERTAIN);
    assert_eq!(disk.items[2].state, STATE_UNCERTAIN);
    assert_eq!(disk.items[3].state, STATE_QUEUED);
    assert_eq!(disk.items[4].state, STATE_BLOCKED);
    assert_eq!(disk.items[5].state, STATE_UNCERTAIN);
}

#[test]
fn owned_ids_are_exact_and_an_empty_slice_owns_nothing() {
    let (_temp, store) = open_store();
    let running = enqueue(&store, "running");
    let accepted = enqueue(&store, "accepted");
    let consumed = enqueue(&store, "consumed");
    let queued = enqueue(&store, "queued");
    store
        .set_state(&running, STATE_RUNNING.clone(), "running")
        .unwrap();
    store
        .set_state(&accepted, STATE_STEER_ACCEPTED.clone(), "accepted")
        .unwrap();
    store
        .set_state(&consumed, STATE_STEER_CONSUMED.clone(), "consumed")
        .unwrap();
    let owned = vec![
        running.clone(),
        String::new(),
        format!(" {accepted} "),
        running.clone(),
    ];

    assert_eq!(store.recover_orphaned_in_flight(&owned).unwrap(), 2);
    assert_eq!(item(&store, &running).state, STATE_RUNNING);
    assert_eq!(item(&store, &accepted).state, STATE_UNCERTAIN);
    assert_eq!(item(&store, &consumed).state, STATE_UNCERTAIN);
    assert_eq!(item(&store, &queued).state, STATE_QUEUED);

    store
        .set_state(&running, STATE_RUNNING.clone(), "running again")
        .unwrap();
    store
        .set_state(&accepted, STATE_STEER_ACCEPTED.clone(), "accepted again")
        .unwrap();
    store
        .set_state(&consumed, STATE_STEER_CONSUMED.clone(), "consumed again")
        .unwrap();
    assert_eq!(store.recover_orphaned_in_flight(&[]).unwrap(), 3);
    assert_eq!(item(&store, &running).state, STATE_UNCERTAIN);
    assert_eq!(item(&store, &accepted).state, STATE_UNCERTAIN);
    assert_eq!(item(&store, &consumed).state, STATE_UNCERTAIN);
    assert_eq!(item(&store, &queued).state, STATE_QUEUED);
}

#[test]
fn recovery_accumulates_below_the_live_item_cap() {
    let (_temp, store) = open_store();
    let first = enqueue(&store, "first");
    let second = enqueue(&store, "second");
    store
        .set_state(&first, STATE_RUNNING.clone(), "running")
        .unwrap();
    store
        .set_state(&second, STATE_RUNNING.clone(), "running")
        .unwrap();

    assert_eq!(
        store
            .recover_orphaned_in_flight(std::slice::from_ref(&first))
            .unwrap(),
        1
    );
    let after_first = store.snapshot();
    assert_eq!(after_first.recovered_n, 1);
    assert_eq!(after_first.capacity.items, 2);

    store
        .set_state(&first, STATE_RUNNING.clone(), "running again")
        .unwrap();
    assert_eq!(store.recover_orphaned_in_flight(&[]).unwrap(), 1);
    let after_second = store.snapshot();
    assert_eq!(after_second.recovered_n, 2);
    assert_eq!(after_second.capacity.items, 2);
    let disk = decode_manifest(&fs::read(store.dir().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(disk.recovered_n, 2);
}

#[test]
fn recovery_refreshes_the_manifest_and_re_resolves_ownership_for_every_item() {
    let (temp, store) = open_store();
    let running = enqueue(&store, "running");
    let pending = enqueue(&store, "pending");
    store
        .set_state(&running, STATE_RUNNING.clone(), "running")
        .unwrap();
    let second = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let added = enqueue(&second, "added");
    second
        .set_state(&added, STATE_STEER_ACCEPTED.clone(), "accepted")
        .unwrap();

    let calls = RefCell::new(Vec::new());
    let running_scans = Cell::new(0usize);
    let owned_by = |id: &str| {
        assert!(store.state.try_lock().is_err());
        calls.borrow_mut().push(id.to_owned());
        if id == running {
            let scans = running_scans.get() + 1;
            running_scans.set(scans);
            scans > 1
        } else {
            false
        }
    };

    assert_eq!(
        store
            .recover_orphaned_in_flight_owned_by(Some(&owned_by))
            .unwrap(),
        1
    );
    assert_eq!(
        calls.into_inner(),
        vec![
            running.clone(),
            pending.clone(),
            running.clone(),
            pending.clone(),
            added.clone()
        ]
    );
    assert_eq!(item(&store, &running).state, STATE_RUNNING);
    assert_eq!(item(&store, &pending).state, STATE_QUEUED);
    assert_eq!(item(&store, &added).state, STATE_UNCERTAIN);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.recovered_n, 1);
    assert!(snapshot.paused);
}

#[test]
fn recovery_with_no_candidates_after_refresh_does_not_commit() {
    let (temp, store) = open_store();
    let active = enqueue(&store, "active");
    store
        .set_state(&active, STATE_RUNNING.clone(), "running")
        .unwrap();
    let second = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    second
        .set_state(&active, STATE_QUEUED.clone(), "pending again")
        .unwrap();
    let manifest = store.dir().join("manifest.json");
    let before = fs::read(&manifest).unwrap();
    let calls = RefCell::new(Vec::new());
    let owned_by = |id: &str| {
        calls.borrow_mut().push(id.to_owned());
        false
    };

    assert_eq!(
        store
            .recover_orphaned_in_flight_owned_by(Some(&owned_by))
            .unwrap(),
        0
    );
    assert_eq!(calls.into_inner(), vec![active.clone(), active.clone()]);
    let snapshot = store.snapshot();
    assert_eq!(item(&store, &active).state, STATE_QUEUED);
    assert!(!snapshot.paused);
    assert!(!snapshot.recovered);
    assert_eq!(snapshot.recovered_n, 0);
    assert_eq!(fs::read(manifest).unwrap(), before);
}

#[test]
fn no_initial_candidate_returns_before_the_transaction_and_mutability_check() {
    let (_temp, store) = open_store();
    let queued = enqueue(&store, "queued");
    store.close();
    break_transaction_lock(&store);
    let calls = RefCell::new(Vec::new());
    let owned_by = |id: &str| {
        calls.borrow_mut().push(id.to_owned());
        true
    };

    assert_eq!(
        store
            .recover_orphaned_in_flight_owned_by(Some(&owned_by))
            .unwrap(),
        0
    );
    assert_eq!(calls.into_inner(), vec![queued]);
}

#[test]
fn an_initial_candidate_still_checks_store_mutability() {
    let (_temp, store) = open_store();
    let running = enqueue(&store, "running");
    store
        .set_state(&running, STATE_RUNNING.clone(), "running")
        .unwrap();
    store.close();

    assert!(matches!(
        store.recover_orphaned_in_flight(&[]),
        Err(Error::Closed)
    ));
}

#[test]
fn a_newer_schema_active_candidate_is_readonly_and_never_rewritten() {
    let temp = tempfile::tempdir().unwrap();
    let mut manifest = empty_manifest(process_run_id());
    manifest.schema_version = SCHEMA_VERSION + 1;
    manifest.revision = 12;
    manifest
        .items
        .push(support::item("future-running", STATE_RUNNING.as_str()));
    support::write_manifest(temp.path(), "session", &manifest);
    let path = session_inbox_dir(&temp.path().join("session.jsonl")).join("manifest.json");
    let before = fs::read(&path).unwrap();

    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let snapshot = store.snapshot();
    assert!(snapshot.readonly);
    assert!(snapshot.paused);
    assert!(!snapshot.recovered);
    assert_eq!(snapshot.recovered_n, 0);
    assert_eq!(snapshot.items.as_ref().unwrap()[0].state, STATE_RUNNING);

    assert!(matches!(
        store.recover_orphaned_in_flight(&[]),
        Err(Error::SchemaReadonly)
    ));
    assert_eq!(store.snapshot(), snapshot);
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn recovery_caps_the_cumulative_count_and_preserves_the_pause_code() {
    let (_temp, store) = open_store();
    let kept = enqueue(&store, "kept");
    let removed = enqueue(&store, "removed");
    store
        .set_state(&kept, STATE_RUNNING.clone(), "running")
        .unwrap();
    store
        .set_state(&removed, STATE_STEER_ACCEPTED.clone(), "accepted")
        .unwrap();
    assert_eq!(store.recover_orphaned_in_flight(&[]).unwrap(), 2);
    store.pause(PAUSE_USER).unwrap();
    store
        .set_state(&kept, STATE_RUNNING.clone(), "running again")
        .unwrap();
    store
        .set_state(&removed, STATE_QUEUED.clone(), "pending")
        .unwrap();
    store.delete_item(&removed).unwrap();

    assert_eq!(store.recover_orphaned_in_flight(&[]).unwrap(), 1);
    let snapshot = store.snapshot();
    assert!(snapshot.paused);
    assert!(snapshot.recovered);
    assert_eq!(snapshot.recovered_n, 1);
    assert_eq!(snapshot.pause_code, PAUSE_USER);
    let disk = decode_manifest(&fs::read(store.dir().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(disk.recovered_n, 1);
    assert_eq!(disk.pause_code, PAUSE_USER);
}

#[test]
fn repeated_recovery_is_a_noop_and_notifies_only_for_a_real_recovery() {
    let (_temp, store) = open_store();
    let running = enqueue(&store, "running");
    store
        .set_state(&running, STATE_RUNNING.clone(), "running")
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    store.on_change(move |snapshot| {
        let _ = sender.send(snapshot);
    });

    assert_eq!(store.recover_orphaned_in_flight(&[]).unwrap(), 1);
    let notified = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(notified.recovered);
    assert_eq!(notified.recovered_n, 1);
    let before = store.snapshot();

    assert_eq!(store.recover_orphaned_in_flight(&[]).unwrap(), 0);
    assert_eq!(store.snapshot(), before);
    assert!(matches!(
        receiver.recv_timeout(Duration::from_millis(200)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
}

#[test]
fn pending_metadata_survives_a_recovery_commit() {
    let (_temp, store) = open_store();
    let active = enqueue(&store, "active");
    let blocked = enqueue(&store, "blocked");
    store
        .set_state(&active, STATE_RUNNING.clone(), "running")
        .unwrap();
    store
        .set_state_coded(
            &blocked,
            STATE_BLOCKED.clone(),
            BLOCK_STEER_UNAPPLIED.clone(),
            "held for retry",
        )
        .unwrap();
    let before = item(&store, &blocked);

    store.recover_orphaned_in_flight(&[]).unwrap();

    assert_eq!(item(&store, &blocked), before);
    assert_eq!(item(&store, &active).block_code, BLOCK_OWNER_INACTIVE);
}
