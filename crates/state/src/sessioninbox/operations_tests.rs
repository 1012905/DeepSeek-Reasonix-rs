use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};

use super::disk_tests::support;
use super::*;
use crate::sessionstore::eventlog::record::CreatedAt;

fn open_store(limits: Limits) -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), limits).unwrap();
    (temp, store)
}

fn envelope(text: &str) -> PromptEnvelope {
    PromptEnvelope {
        submit_text: text.to_owned(),
        ..PromptEnvelope::default()
    }
}

fn request(text: &str) -> EnqueueRequest {
    EnqueueRequest {
        intent: INTENT_FOLLOWUP.clone(),
        envelope: envelope(text),
        source: "test".to_owned(),
        ..EnqueueRequest::default()
    }
}

fn enqueue(store: &Store, text: &str) -> InboxReceipt {
    store.enqueue(request(text)).unwrap()
}

fn state(store: &Store, id: &str) -> InboxItemMeta {
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

fn blob_names(store: &Store) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(store.dir().join("blobs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn held_duplicate_skips_nonwaiting_copies_and_reuses_the_next_queued_copy() {
    let (_temp, store) = open_store(Limits::default());
    let running = enqueue(&store, "same words");
    store.claim_item(&running.item_id).unwrap();
    let waiting = enqueue(&store, "same words");

    let replay = enqueue(&store, "same words");
    assert_eq!(replay.item_id, waiting.item_id);
    assert!(replay.idempotent);
    assert_eq!(store.snapshot().capacity.items, 2);
}

#[test]
fn alias_receipt_for_another_item_is_a_typed_conflict() {
    let (_temp, store) = open_store(Limits::default());
    let mut original = request("original");
    original.idempotency = "receipt-key".to_owned();
    let first = store.enqueue(original).unwrap();
    store.claim_item(&first.item_id).unwrap();
    store.ack_dequeue(&first.item_id).unwrap();
    let second = enqueue(&store, "second");

    let error = store
        .update_item_with_idempotency(
            &second.item_id,
            envelope("changed"),
            "receipt-key",
            envelope("original"),
        )
        .unwrap_err();
    assert!(matches!(error, Error::IdempotencyConflict));
    assert_eq!(
        store.read_item(&second.item_id).unwrap().1.submit_text,
        "second"
    );
}

#[test]
fn requeue_unapplied_steer_does_not_trim_its_id() {
    let (_temp, store) = open_store(Limits::default());
    let item = enqueue(&store, "steer");
    store
        .set_state(&item.item_id, STATE_STEER_ACCEPTED.clone(), "accepted")
        .unwrap();

    assert!(matches!(
        store.requeue_unapplied_steer(&format!(" {} ", item.item_id)),
        Err(Error::NotFound)
    ));
    assert_eq!(state(&store, &item.item_id).state, STATE_STEER_ACCEPTED);
}

#[test]
fn durable_enqueue_completes_body_orders_and_reopens() {
    let (temp, store) = open_store(Limits::default());
    fs::write(temp.path().join("session.jsonl"), "{}\n").unwrap();
    let mut invocation = request("/init");
    invocation.envelope.display_text = " /init ".to_owned();
    invocation.envelope.invocation = Some(StructuredInvocation {
        name: "init".to_owned(),
        ..StructuredInvocation::default()
    });
    invocation.intent = InboxIntent::new("unknown");
    invocation.idempotency = " first ".to_owned();
    invocation.session_id = " branch ".to_owned();

    let first = store.enqueue(invocation).unwrap();
    let second = enqueue(&store, "second");
    assert_eq!(first.position, 1);
    assert_eq!(second.position, 2);
    assert_eq!(first.disposition, DISPOSITION_QUEUED_FOLLOWUP);
    assert!(!first.idempotent);
    assert_eq!(first.capacity.items, 1);
    assert_eq!(second.capacity.items, 2);
    assert_eq!(state(&store, &first.item_id).session_id, "branch");
    assert_eq!(state(&store, &first.item_id).intent, INTENT_FOLLOWUP);

    let (meta, body) = store.read_item(&format!(" {} ", first.item_id)).unwrap();
    assert_eq!(meta.id, first.item_id);
    assert_eq!(meta.blob_name, first.item_id);
    assert_eq!(meta.preview, "/init");
    assert_eq!(body.display_text, "/init");
    assert_eq!(body.submit_text, "/init");
    assert_eq!(body.raw_text, "");
    assert!(body.idempotency.is_empty());
    assert_eq!(meta.idempotency, "first");

    drop(store);
    let reopened = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    assert_eq!(reopened.snapshot().items.unwrap()[0].id, first.item_id);
    assert_eq!(reopened.snapshot().items.unwrap()[1].id, second.item_id);
    assert_eq!(reopened.read_item(&first.item_id).unwrap().1, body);
    assert_eq!(reopened.snapshot().revision, 2);
}

#[test]
fn enqueue_rolls_back_a_failed_blob_write() {
    let (_temp, store) = open_store(Limits::default());
    fs::write(store.dir().join("blobs"), b"not a directory").unwrap();
    let before = store.snapshot();

    let err = store.enqueue(request("cannot persist")).unwrap_err();
    assert!(matches!(err, Error::NotRegularFile { .. }));
    assert_eq!(store.snapshot(), before);
    assert!(!store.dir().join("manifest.json").exists());
}

#[cfg(windows)]
#[test]
fn enqueue_rolls_back_a_failed_manifest_publication() {
    let (_temp, store) = open_store(Limits::default());
    enqueue(&store, "published");
    let manifest = store.dir().join("manifest.json");
    let before_bytes = fs::read(&manifest).unwrap();
    let before_blobs = blob_names(&store);
    let before = store.snapshot();
    let permissions = fs::metadata(&manifest).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&manifest, readonly).unwrap();

    let result = store.enqueue(request("must roll back"));
    fs::set_permissions(&manifest, permissions).unwrap();

    assert!(matches!(result, Err(Error::Io { .. })));
    assert_eq!(fs::read(&manifest).unwrap(), before_bytes);
    assert_eq!(blob_names(&store), before_blobs);
    assert_eq!(store.snapshot(), before);
}

#[test]
fn update_uses_a_new_blob_and_clears_only_a_blocked_state() {
    let (_temp, store) = open_store(Limits::default());
    let receipt = enqueue(&store, "old");
    let old = state(&store, &receipt.item_id);
    store
        .set_state_coded(
            &receipt.item_id,
            STATE_BLOCKED.clone(),
            BLOCK_SNAPSHOT_FAILED.clone(),
            "held",
        )
        .unwrap();
    let mut body = envelope("new");
    body.display_text = " new ".to_owned();
    body.raw_text = " raw ".to_owned();
    body.format = " text ".to_owned();
    body.refs.push(RefSnapshot {
        kind: "frozen".to_owned(),
        display_path: " shown.txt ".to_owned(),
        path: "actual.txt".to_owned(),
        commit: "abc".to_owned(),
        content: b"reference body".to_vec(),
        ..RefSnapshot::default()
    });
    body.frozen_images
        .push("data:image/png;base64,AA==".to_owned());

    let updated = store.update_item(&receipt.item_id, body.clone()).unwrap();
    assert_ne!(updated.blob_name, old.blob_name);
    assert_eq!(updated.state, STATE_QUEUED);
    assert!(updated.block_reason.is_empty());
    assert!(updated.block_code.is_empty());
    assert!(updated.images);
    assert_eq!(updated.preview, "new");
    assert_eq!(updated.refs[0].path, "shown.txt");
    assert_eq!(updated.refs[0].bytes, 14);
    assert_eq!(updated.revision, store.snapshot().revision);
    assert!(
        !store
            .dir()
            .join("blobs")
            .join(format!("{}.json", old.blob_name))
            .exists()
    );
    let (_, stored) = store.read_item(&receipt.item_id).unwrap();
    assert_eq!(stored.submit_text, "new");
    assert_eq!(stored.display_text, "new");
    assert_eq!(stored.raw_text, "raw");
    assert_eq!(stored.format, "text");
}

#[cfg(windows)]
#[test]
fn update_rolls_back_a_failed_manifest_publication() {
    let (_temp, store) = open_store(Limits::default());
    let receipt = enqueue(&store, "old body");
    let old = state(&store, &receipt.item_id);
    let manifest = store.dir().join("manifest.json");
    let before_bytes = fs::read(&manifest).unwrap();
    let before_blobs = blob_names(&store);
    let permissions = fs::metadata(&manifest).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&manifest, readonly).unwrap();

    let result = store.update_item(&receipt.item_id, envelope("new body"));
    fs::set_permissions(&manifest, permissions).unwrap();

    assert!(matches!(result, Err(Error::Io { .. })));
    assert_eq!(fs::read(&manifest).unwrap(), before_bytes);
    assert_eq!(blob_names(&store), before_blobs);
    assert_eq!(state(&store, &receipt.item_id), old);
    assert_eq!(
        store.read_item(&receipt.item_id).unwrap().1.submit_text,
        "old body"
    );
}

#[test]
fn capacity_checks_preserve_go_order_and_no_item_limit_still_bounds_bytes() {
    let (_temp, store) = open_store(Limits {
        max_items: 2,
        max_item_bytes: 100,
        max_total_bytes: 1000,
    });
    assert!(matches!(
        store.enqueue(request(&"x".repeat(500))),
        Err(Error::ItemTooLarge)
    ));
    enqueue(&store, "one");
    enqueue(&store, "two");
    assert!(matches!(
        store.enqueue(request("three")),
        Err(Error::CapacityItems)
    ));
    assert_eq!(store.snapshot().capacity.items, 2);

    let (_temp, unlimited) = open_store(Limits {
        max_items: NO_ITEM_LIMIT,
        max_total_bytes: 100,
        ..Limits::default()
    });
    let mut last = Ok(InboxReceipt::default());
    for index in 0..50 {
        last = unlimited.enqueue(request(&format!("padding line number {index}")));
        if last.is_err() {
            break;
        }
    }
    assert!(matches!(last, Err(Error::CapacityBytes)));
    assert_eq!(unlimited.snapshot().capacity.max_items, 0);
}

#[test]
fn idempotency_replay_precedes_capacity_and_conflicts_are_typed() {
    let (_temp, store) = open_store(Limits {
        max_items: 1,
        ..Limits::default()
    });
    let mut original = request("same");
    original.idempotency = "msg-1".to_owned();
    let first = store.enqueue(original.clone()).unwrap();
    let replay = store.enqueue(original.clone()).unwrap();
    assert_eq!(replay.item_id, first.item_id);
    assert_eq!(replay.position, 1);
    assert_eq!(replay.disposition, DISPOSITION_IDEMPOTENT_HIT);
    assert!(replay.idempotent);
    assert_eq!(store.snapshot().revision, 1);

    let mut conflict = original;
    conflict.envelope.submit_text = "different".to_owned();
    assert!(matches!(
        store.enqueue(conflict),
        Err(Error::IdempotencyConflict)
    ));
    assert_eq!(store.snapshot().capacity.items, 1);
}

#[test]
fn receipts_survive_ack_and_reopen_but_expired_receipts_do_not_replay() {
    let (temp, store) = open_store(Limits::default());
    let mut original = request("write once");
    original.idempotency = "msg-1".to_owned();
    let first = store.enqueue(original.clone()).unwrap();
    store.claim_item(&first.item_id).unwrap();
    store.ack_dequeue(&first.item_id).unwrap();
    assert_eq!(
        store.receipt_disposition("msg-1").unwrap(),
        (Disposition::new("acknowledged"), true)
    );
    drop(store);

    let reopened = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let replay = reopened.enqueue(original.clone()).unwrap();
    assert_eq!(replay.item_id, first.item_id);
    assert_eq!(replay.position, 0);
    assert!(replay.idempotent);
    assert!(reopened.snapshot().items.is_none());
    let mut conflict = original.clone();
    conflict.envelope.submit_text = "different".to_owned();
    assert!(matches!(
        reopened.enqueue(conflict),
        Err(Error::IdempotencyConflict)
    ));
    drop(reopened);

    let manifest_path = support::inbox(temp.path(), "session").join("manifest.json");
    let mut manifest = decode_manifest(&fs::read(&manifest_path).unwrap()).unwrap();
    let receipt = manifest.receipts.get_mut("msg-1").unwrap();
    receipt.completed_at =
        CreatedAt::from_datetime((Utc::now() - ChronoDuration::days(8)).fixed_offset());
    support::write_manifest(temp.path(), "session", &manifest);
    let reopened = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let fresh = reopened.enqueue(original).unwrap();
    assert_ne!(fresh.item_id, first.item_id);
    assert!(!fresh.idempotent);
    assert_eq!(fresh.position, 1);
    assert_eq!(
        reopened.receipt_disposition("msg-1").unwrap(),
        (Disposition::new("acknowledged"), true)
    );
}

#[test]
fn alias_replay_hashes_the_original_request_and_survives_ack() {
    let (_temp, store) = open_store(Limits::default());
    let mut first_request = request("first");
    first_request.idempotency = "msg-1".to_owned();
    let first = store.enqueue(first_request).unwrap();
    let original = PromptEnvelope {
        submit_text: "second".to_owned(),
        source: "bot".to_owned(),
        extra: BTreeMap::from([("route".to_owned(), "chat-1".to_owned())]),
        ..PromptEnvelope::default()
    };
    let merged = envelope("first\nsecond");
    let updated = store
        .update_item_with_idempotency(&first.item_id, merged.clone(), "msg-2", original.clone())
        .unwrap();
    let manifest_bytes = fs::read(store.dir().join("manifest.json")).unwrap();
    let replayed = store
        .update_item_with_idempotency(
            &first.item_id,
            envelope("a different merged body"),
            "msg-2",
            original.clone(),
        )
        .unwrap();
    assert_eq!(replayed, updated);
    assert_eq!(
        fs::read(store.dir().join("manifest.json")).unwrap(),
        manifest_bytes
    );
    assert_eq!(store.read_item(&first.item_id).unwrap().1, merged);

    let mut conflicting = original.clone();
    conflicting.submit_text = "not second".to_owned();
    assert!(matches!(
        store.update_item_with_idempotency(
            &first.item_id,
            envelope("ignored"),
            "msg-2",
            conflicting,
        ),
        Err(Error::IdempotencyConflict)
    ));
    let other = enqueue(&store, "other");
    assert!(matches!(
        store.update_item_with_idempotency(
            &other.item_id,
            envelope("ignored"),
            "msg-2",
            original.clone(),
        ),
        Err(Error::IdempotencyConflict)
    ));

    store.claim_item(&first.item_id).unwrap();
    store.ack_dequeue(&first.item_id).unwrap();
    let mut retry = request("placeholder");
    retry.envelope = original;
    retry.idempotency = "msg-2".to_owned();
    let receipt = store.enqueue(retry).unwrap();
    assert_eq!(receipt.item_id, first.item_id);
    assert!(receipt.idempotent);
    assert!(store.snapshot().items.unwrap().len() == 1);
}

#[test]
fn held_duplicates_require_the_original_checksum_and_origin() {
    let (_temp, store) = open_store(Limits::default());
    let first = enqueue(&store, "same words");
    let mut steer = request("same words");
    steer.intent = INTENT_STEER.clone();
    let second = store.enqueue(steer).unwrap();
    assert_eq!(second.item_id, first.item_id);
    assert!(second.idempotent);
    assert_eq!(second.position, 1);

    let mut host = request("same words");
    host.origin = ORIGIN_HOST.clone();
    store.enqueue(host.clone()).unwrap();
    store.enqueue(host).unwrap();
    let mut other_source = request("same words");
    other_source.source = "other".to_owned();
    store.enqueue(other_source).unwrap();
    let mut other_display = request("same words");
    other_display.envelope.display_text = "different display".to_owned();
    store.enqueue(other_display).unwrap();
    assert_eq!(store.snapshot().capacity.items, 4);
}

#[test]
fn resending_an_unapplied_steer_requeues_the_original_as_followup() {
    let (_temp, store) = open_store(Limits::default());
    let mut steer = request("stop and write the handoff");
    steer.intent = INTENT_STEER.clone();
    let first = store.enqueue(steer.clone()).unwrap();
    store
        .set_state_coded(
            &first.item_id,
            STATE_UNCERTAIN.clone(),
            BLOCK_STEER_UNAPPLIED.clone(),
            "unapplied",
        )
        .unwrap();
    let replay = store.enqueue(steer).unwrap();
    assert_eq!(replay.item_id, first.item_id);
    assert!(replay.idempotent);
    let item = state(&store, &first.item_id);
    assert_eq!(item.state, STATE_QUEUED);
    assert_eq!(item.intent, INTENT_FOLLOWUP);
    assert!(item.block_code.is_empty());
    assert!(item.block_reason.is_empty());
    assert_eq!(store.snapshot().capacity.items, 1);
}

#[test]
fn delete_and_cancel_enforce_state_scopes_and_remember_receipts() {
    for (target, allowed) in [
        (STATE_QUEUED.clone(), true),
        (STATE_BLOCKED.clone(), true),
        (STATE_UNCERTAIN.clone(), true),
        (STATE_RUNNING.clone(), false),
        (STATE_STEER_ACCEPTED.clone(), false),
        (STATE_STEER_CONSUMED.clone(), false),
    ] {
        let (_temp, store) = open_store(Limits::default());
        let mut req = request("delete me");
        req.idempotency = "delete-key".to_owned();
        let receipt = store.enqueue(req).unwrap();
        store
            .set_state(&receipt.item_id, target.clone(), "")
            .unwrap();
        let before = store.snapshot();
        let result = store.delete_item(&receipt.item_id);
        if allowed {
            result.unwrap();
            assert!(store.snapshot().items.is_none());
            assert!(
                !store
                    .dir()
                    .join("blobs")
                    .join(format!("{}.json", receipt.item_id))
                    .exists()
            );
            assert_eq!(
                store.receipt_disposition("delete-key").unwrap(),
                (DISPOSITION_DELETED.clone(), true)
            );
        } else {
            assert!(matches!(result, Err(Error::InvalidState)));
            assert_eq!(store.snapshot(), before);
        }
    }

    let (_temp, store) = open_store(Limits::default());
    let receipt = enqueue(&store, "cancel me");
    assert!(matches!(
        store.cancel_accepted_steer(&receipt.item_id),
        Err(Error::InvalidState)
    ));
    store
        .set_state(&receipt.item_id, STATE_STEER_ACCEPTED.clone(), "")
        .unwrap();
    store.cancel_accepted_steer(&receipt.item_id).unwrap();
    assert!(store.snapshot().items.is_none());
}

#[test]
fn discard_is_atomic_and_source_scoped() {
    let (_temp, store) = open_store(Limits::default());
    let mut desktop_req = request("desktop");
    desktop_req.source = "desktop".to_owned();
    desktop_req.idempotency = "desktop-key".to_owned();
    let desktop = store.enqueue(desktop_req).unwrap();
    let mut bot_req = request("bot");
    bot_req.source = "bot".to_owned();
    bot_req.idempotency = "bot-key".to_owned();
    let bot = store.enqueue(bot_req).unwrap();
    store
        .set_state(&bot.item_id, STATE_RUNNING.clone(), "")
        .unwrap();
    let before = store.snapshot();

    assert!(matches!(
        store.discard_pending_items(&[desktop.item_id.clone(), bot.item_id.clone()]),
        Err(Error::InvalidState)
    ));
    assert_eq!(store.snapshot(), before);
    store
        .discard_pending_items_owned(
            &[
                desktop.item_id.clone(),
                bot.item_id.clone(),
                "missing".to_owned(),
            ],
            "desktop",
        )
        .unwrap();
    assert_eq!(store.snapshot().items.unwrap()[0].id, bot.item_id);
    assert_eq!(
        store.receipt_disposition("desktop-key").unwrap(),
        (Disposition::new("discarded"), true)
    );
    assert_eq!(
        store.receipt_disposition("bot-key").unwrap(),
        (Disposition::default(), false)
    );
    let revision = store.snapshot().revision;
    store.discard_pending_items(&[]).unwrap();
    store
        .discard_pending_items(&["missing".to_owned()])
        .unwrap();
    assert_eq!(store.snapshot().revision, revision);
}

#[test]
fn move_clamps_positions_and_a_same_position_is_a_noop() {
    let (_temp, store) = open_store(Limits::default());
    let a = enqueue(&store, "a");
    let b = enqueue(&store, "b");
    let c = enqueue(&store, "c");
    store.move_item(&c.item_id, -10).unwrap();
    assert_eq!(store.snapshot().items.unwrap()[0].id, c.item_id);
    store.move_item(&c.item_id, 100).unwrap();
    let ids: Vec<String> = store
        .snapshot()
        .items
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, [a.item_id.clone(), b.item_id, c.item_id]);
    let before = store.snapshot();
    store.move_item(&a.item_id, 0).unwrap();
    assert_eq!(store.snapshot(), before);
    store
        .set_state(&a.item_id, STATE_RUNNING.clone(), "")
        .unwrap();
    assert!(matches!(
        store.move_item(&a.item_id, 2),
        Err(Error::InvalidState)
    ));
}

#[test]
fn pause_precedence_resume_and_force_pause_match_go() {
    let (_temp, store) = open_store(Limits::default());
    store.pause(PAUSE_REFERENCE_BLOCKED.clone()).unwrap();
    store.pause(PAUSE_SESSION_CHANGED.clone()).unwrap();
    assert_eq!(store.snapshot().pause_code, PAUSE_REFERENCE_BLOCKED);
    let revision = store.snapshot().revision;
    store.pause(PAUSE_SESSION_CHANGED.clone()).unwrap();
    assert_eq!(store.snapshot().revision, revision);
    store.pause(PAUSE_USER.clone()).unwrap();
    store.pause(PAUSE_SESSION_CHANGED.clone()).unwrap();
    assert_eq!(store.snapshot().pause_code, PAUSE_USER);

    store.force_pause(true, 3).unwrap();
    let snap = store.snapshot();
    assert!(snap.paused);
    assert_eq!(snap.pause_code, PAUSE_RECOVERED);
    assert!(snap.recovered);
    assert_eq!(snap.recovered_n, 3);
    store.force_pause(false, 0).unwrap();
    assert_eq!(store.snapshot().recovered_n, 3);
    store.resume().unwrap();
    let snap = store.snapshot();
    assert!(!snap.paused);
    assert!(snap.pause_code.is_empty());
    assert!(!snap.recovered);
    assert_eq!(snap.recovered_n, 0);
    let revision = snap.revision;
    store.resume().unwrap();
    assert_eq!(store.snapshot().revision, revision);
}

#[test]
fn set_state_coded_preserves_image_fields_and_has_no_transition_whitelist() {
    let (_temp, store) = open_store(Limits::default());
    let mut req = request("look");
    req.envelope.fingerprint_version = 1;
    req.envelope.request_fingerprint = "a".repeat(64);
    req.envelope.image_inputs =
        serde_json::value::RawValue::from_string(r#"[{"kind":"attachment"}]"#.to_owned()).ok();
    let receipt = store.enqueue(req).unwrap();
    assert!(state(&store, &receipt.item_id).images);
    store
        .set_state_coded(
            &receipt.item_id,
            STATE_STEER_CONSUMED.clone(),
            BLOCK_ACK_FAILED.clone(),
            "coded reason",
        )
        .unwrap();
    let item = state(&store, &receipt.item_id);
    assert_eq!(item.state, STATE_STEER_CONSUMED);
    assert_eq!(item.block_code, BLOCK_ACK_FAILED);
    assert_eq!(item.block_reason, "coded reason");
    let (_, body) = store.read_item(&receipt.item_id).unwrap();
    assert_eq!(body.fingerprint_version, 1);
    assert_eq!(body.request_fingerprint, "a".repeat(64));
    assert!(body.has_foreign_images());
}

#[test]
fn pending_operations_share_the_same_state_scope() {
    for (target, pending) in [
        (STATE_QUEUED.clone(), true),
        (STATE_BLOCKED.clone(), true),
        (STATE_UNCERTAIN.clone(), true),
        (STATE_RUNNING.clone(), false),
        (STATE_STEER_ACCEPTED.clone(), false),
        (STATE_STEER_CONSUMED.clone(), false),
    ] {
        let (_temp, store) = open_store(Limits::default());
        let receipt = enqueue(&store, "scoped");
        store.set_state(&receipt.item_id, target, "").unwrap();
        let before = store.snapshot();
        let update = store.update_item(&receipt.item_id, envelope("updated"));
        let convert = store.convert_intent(&receipt.item_id, INTENT_STEER.clone());
        assert_eq!(update.is_ok(), pending, "update scope for {before:?}");
        assert_eq!(convert.is_ok(), pending, "convert scope for {before:?}");
        if !pending {
            assert!(matches!(update, Err(Error::InvalidState)));
            assert!(matches!(convert, Err(Error::InvalidState)));
            assert_eq!(store.snapshot(), before);
        }
    }
}

#[test]
fn claim_over_pause_keeps_pause_and_checks_pause_before_missing_id() {
    let (_temp, store) = open_store(Limits::default());
    let older = enqueue(&store, "older");
    let newer = enqueue(&store, "newer");
    store.pause(PAUSE_USER.clone()).unwrap();
    assert!(matches!(
        store.claim_item(&older.item_id),
        Err(Error::Paused)
    ));
    assert!(matches!(store.claim_item("missing"), Err(Error::Paused)));
    store.claim_item_over_pause(&newer.item_id).unwrap();
    assert!(store.snapshot().paused);
    assert_eq!(state(&store, &older.item_id).state, STATE_QUEUED);
    assert_eq!(state(&store, &newer.item_id).state, STATE_RUNNING);
    assert!(matches!(
        store.claim_item_over_pause(&newer.item_id),
        Err(Error::InvalidState)
    ));
}

#[test]
fn convert_intent_normalizes_unknown_values_without_changing_state() {
    let (_temp, store) = open_store(Limits::default());
    let receipt = enqueue(&store, "convert");
    store
        .convert_intent(&receipt.item_id, InboxIntent::new("other"))
        .unwrap();
    let item = state(&store, &receipt.item_id);
    assert_eq!(item.intent, INTENT_FOLLOWUP);
    assert_eq!(item.state, STATE_QUEUED);
    store
        .convert_intent(&receipt.item_id, INTENT_STEER.clone())
        .unwrap();
    assert_eq!(state(&store, &receipt.item_id).intent, INTENT_STEER);
}

#[test]
fn ack_accepts_only_in_flight_states_and_records_each_key() {
    for (target, allowed) in [
        (STATE_RUNNING.clone(), true),
        (STATE_STEER_ACCEPTED.clone(), true),
        (STATE_STEER_CONSUMED.clone(), true),
        (STATE_QUEUED.clone(), false),
        (STATE_BLOCKED.clone(), false),
        (STATE_UNCERTAIN.clone(), false),
    ] {
        let (_temp, store) = open_store(Limits::default());
        let mut req = request("ack");
        req.idempotency = "primary".to_owned();
        let receipt = store.enqueue(req).unwrap();
        store
            .update_item_with_idempotency(
                &receipt.item_id,
                envelope("ack"),
                "alias",
                envelope("ack"),
            )
            .unwrap();
        store.set_state(&receipt.item_id, target, "").unwrap();
        let before = store.snapshot();
        let result = store.ack_dequeue(&receipt.item_id);
        assert_eq!(result.is_ok(), allowed);
        if allowed {
            assert!(store.snapshot().items.is_none());
            for key in ["primary", "alias"] {
                assert_eq!(
                    store.receipt_disposition(key).unwrap(),
                    (Disposition::new("acknowledged"), true)
                );
            }
        } else {
            assert!(matches!(result, Err(Error::InvalidState)));
            assert_eq!(store.snapshot(), before);
        }
    }
}

#[test]
fn retry_refuses_unsafe_uncertainty_and_clears_safe_blocks() {
    for code in [
        BLOCK_SNAPSHOT_FAILED.clone(),
        BLOCK_ACK_FAILED.clone(),
        BLOCK_OWNER_INACTIVE.clone(),
        BLOCK_MANIFEST_SALVAGED.clone(),
        BlockCode::default(),
    ] {
        let (_temp, store) = open_store(Limits::default());
        let receipt = enqueue(&store, "retry");
        store
            .set_state_coded(
                &receipt.item_id,
                STATE_UNCERTAIN.clone(),
                code.clone(),
                "held",
            )
            .unwrap();
        let before = store.snapshot();
        assert!(matches!(
            store.retry_item(&receipt.item_id),
            Err(Error::RetryMayRepeat)
        ));
        assert_eq!(store.snapshot(), before);
    }
    for (target, code) in [
        (STATE_UNCERTAIN.clone(), BLOCK_STEER_UNAPPLIED.clone()),
        (STATE_BLOCKED.clone(), BLOCK_SNAPSHOT_FAILED.clone()),
    ] {
        let (_temp, store) = open_store(Limits::default());
        let receipt = enqueue(&store, "retry");
        store
            .set_state_coded(&receipt.item_id, target, code, "held")
            .unwrap();
        store.retry_item(&receipt.item_id).unwrap();
        let item = state(&store, &receipt.item_id);
        assert_eq!(item.state, STATE_QUEUED);
        assert!(item.block_code.is_empty());
        assert!(item.block_reason.is_empty());
    }
}

#[test]
fn next_queued_prefers_fifo_followups_then_admits_queued_steers() {
    let (_temp, store) = open_store(Limits::default());
    let mut first_steer = request("first steer");
    first_steer.intent = INTENT_STEER.clone();
    let steer = store.enqueue(first_steer).unwrap();
    let blocked = enqueue(&store, "blocked");
    store
        .set_state(&blocked.item_id, STATE_BLOCKED.clone(), "")
        .unwrap();
    let followup = enqueue(&store, "followup");
    assert_eq!(
        store.next_queued().unwrap(),
        (state(&store, &followup.item_id), true)
    );
    store.delete_item(&followup.item_id).unwrap();
    assert_eq!(
        store.next_queued().unwrap(),
        (state(&store, &steer.item_id), true)
    );
    store.claim_item(&steer.item_id).unwrap();
    assert_eq!(
        store.next_queued().unwrap(),
        (InboxItemMeta::default(), false)
    );
    store.pause(PAUSE_USER.clone()).unwrap();
    let waiting = enqueue(&store, "waiting");
    assert_eq!(
        store.next_queued().unwrap(),
        (InboxItemMeta::default(), false)
    );
    store.resume().unwrap();
    assert_eq!(
        store.next_queued().unwrap(),
        (state(&store, &waiting.item_id), true)
    );
}

#[test]
fn next_queued_lock_free_fallback_honors_empty_and_paused_queues() {
    let (_temp, empty) = open_store(Limits::default());
    break_transaction_lock(&empty);
    assert_eq!(
        empty.next_queued().unwrap(),
        (InboxItemMeta::default(), false)
    );

    let (_temp, paused) = open_store(Limits::default());
    enqueue(&paused, "held");
    paused.pause(PAUSE_USER.clone()).unwrap();
    break_transaction_lock(&paused);
    assert_eq!(
        paused.next_queued().unwrap(),
        (InboxItemMeta::default(), false)
    );
}

#[test]
fn next_queued_double_failure_preserves_both_typed_causes() {
    let (_temp, store) = open_store(Limits::default());
    enqueue(&store, "queued");
    let manifest = store.dir().join("manifest.json");
    fs::remove_file(&manifest).unwrap();
    fs::create_dir(&manifest).unwrap();
    break_transaction_lock(&store);

    match store.next_queued().unwrap_err() {
        Error::QueueRead { read, transaction } => {
            assert!(matches!(*read, Error::NotRegularFile { .. }));
            assert!(matches!(
                *transaction,
                Error::Io { op, ref path, ref source }
                    if op == "open transaction lock"
                        && path.ends_with("transaction.lock")
                        && source.raw_os_error().is_some()
            ));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn next_queued_refuses_to_adopt_a_foreign_run_without_the_transaction() {
    let (_temp, store) = open_store(Limits::default());
    let mut foreign = empty_manifest("foreign-run");
    foreign.items.push(InboxItemMeta {
        id: "foreign-item".to_owned(),
        blob_name: "foreign-item".to_owned(),
        intent: INTENT_FOLLOWUP.clone(),
        state: STATE_QUEUED.clone(),
        ..InboxItemMeta::default()
    });
    support::write_manifest(_temp.path(), "session", &foreign);
    break_transaction_lock(&store);

    match store.next_queued().unwrap_err() {
        Error::QueueRead { read, .. } => assert!(matches!(*read, Error::ForeignRun)),
        other => panic!("unexpected error: {other:?}"),
    }
    assert!(store.snapshot().items.is_none());
}

#[test]
fn requeue_unapplied_steer_moves_only_accepted_items_and_only_once() {
    let (_temp, store) = open_store(Limits::default());
    let receipt = enqueue(&store, "steer");
    assert!(!store.requeue_unapplied_steer(&receipt.item_id).unwrap());
    store
        .set_state_coded(
            &receipt.item_id,
            STATE_STEER_ACCEPTED.clone(),
            BLOCK_STEER_UNAPPLIED.clone(),
            "accepted",
        )
        .unwrap();
    assert!(store.requeue_unapplied_steer(&receipt.item_id).unwrap());
    let item = state(&store, &receipt.item_id);
    assert_eq!(item.state, STATE_QUEUED);
    assert_eq!(item.intent, INTENT_FOLLOWUP);
    assert!(item.block_code.is_empty());
    let before = store.snapshot();
    assert!(!store.requeue_unapplied_steer(&receipt.item_id).unwrap());
    assert_eq!(store.snapshot(), before);
}

#[test]
fn missing_ids_and_validation_keep_go_error_ordering() {
    let (_temp, store) = open_store(Limits::default());
    assert!(matches!(
        store.read_item("missing"),
        Err(error) if matches!(error.source, Error::NotFound) && *error.meta == InboxItemMeta::default()
    ));
    assert!(matches!(store.delete_item("missing"), Err(Error::NotFound)));
    assert!(matches!(
        store.cancel_accepted_steer("missing"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.move_item("missing", 0),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.set_state("missing", STATE_QUEUED.clone(), ""),
        Err(Error::NotFound)
    ));
    assert!(matches!(store.claim_item("missing"), Err(Error::NotFound)));
    assert!(matches!(
        store.convert_intent("missing", INTENT_STEER.clone()),
        Err(Error::NotFound)
    ));
    assert!(matches!(store.ack_dequeue("missing"), Err(Error::NotFound)));
    assert!(matches!(
        store.retry_item("missing"),
        Err(Error::RetryMayRepeat | Error::NotFound)
    ));
    assert!(matches!(
        store.requeue_unapplied_steer("missing"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.update_item("missing", envelope("x")),
        Err(Error::NotFound)
    ));
    assert_eq!(
        store.receipt_disposition("missing").unwrap(),
        (Disposition::default(), false)
    );

    let mut empty = request("   ");
    empty.idempotency = "bad\0key".to_owned();
    assert!(matches!(store.enqueue(empty), Err(Error::Empty)));
    assert!(matches!(
        store.update_item("missing", envelope(" ")),
        Err(Error::Empty)
    ));
    assert!(matches!(
        store.update_item_with_idempotency("missing", envelope("x"), "bad\0key", envelope("x")),
        Err(Error::InvalidIdempotencyKey)
    ));
    store.close();
    assert!(matches!(store.enqueue(request(" ")), Err(Error::Empty)));
    assert!(matches!(store.delete_item("missing"), Err(Error::Closed)));
    store.discard_pending_items(&[]).unwrap();
}

#[test]
fn listeners_receive_commits_asynchronously_but_not_noops() {
    let (_temp, store) = open_store(Limits::default());
    let (tx, rx) = mpsc::channel();
    let revisions = Arc::new(Mutex::new(Vec::new()));
    let callback_revisions = Arc::clone(&revisions);
    store.on_change(move |snapshot| {
        callback_revisions.lock().unwrap().push(snapshot.revision);
        tx.send(()).unwrap();
    });
    enqueue(&store, "notify");
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(*revisions.lock().unwrap(), [1]);
    let before = store.snapshot();
    store.move_item(&before.items.unwrap()[0].id, 0).unwrap();
    store.pause(PAUSE_USER.clone()).unwrap();
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(*revisions.lock().unwrap(), [1, 2]);
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn readonly_schema_refuses_mutations_and_force_pause_is_a_silent_noop() {
    let (temp, store) = open_store(Limits::default());
    drop(store);
    let mut future = empty_manifest(process_run_id());
    future.schema_version = SCHEMA_VERSION + 1;
    support::write_manifest(temp.path(), "session", &future);
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let before = store.snapshot();
    assert!(before.readonly);
    assert!(before.paused);
    store.force_pause(true, 9).unwrap();
    assert_eq!(store.snapshot(), before);
    assert!(matches!(
        store.enqueue(request("no")),
        Err(Error::SchemaReadonly)
    ));
    assert!(matches!(
        store.delete_item("missing"),
        Err(Error::SchemaReadonly)
    ));
}

#[test]
fn update_capacity_uses_byte_delta_and_replay_precedes_limits() {
    let (_temp, store) = open_store(Limits {
        max_item_bytes: 200,
        max_total_bytes: 220,
        ..Limits::default()
    });
    let receipt = enqueue(&store, "old");
    let used = store.snapshot().capacity.bytes;
    assert!(matches!(
        store.update_item(&receipt.item_id, envelope(&"x".repeat(500))),
        Err(Error::ItemTooLarge)
    ));
    let max_total = used + 10;
    let (_temp, store) = open_store(Limits {
        max_total_bytes: max_total,
        ..Limits::default()
    });
    let receipt = enqueue(&store, "old");
    let original = envelope("old");
    store
        .update_item_with_idempotency(
            &receipt.item_id,
            envelope("a much larger body"),
            "alias",
            original.clone(),
        )
        .unwrap();
    let before = store.snapshot();
    let replayed = store
        .update_item_with_idempotency(
            &receipt.item_id,
            envelope(&"z".repeat(500)),
            "alias",
            original,
        )
        .unwrap();
    assert_eq!(state(&store, &receipt.item_id), replayed);
    assert_eq!(store.snapshot(), before);
}

#[test]
fn multiple_stores_refresh_before_append_and_keep_fifo_order() {
    let (temp, _) = open_store(Limits::default());
    let session = temp.path().join("session.jsonl");
    let first = Store::open(&session, Limits::default()).unwrap();
    let second = Store::open(&session, Limits::default()).unwrap();
    let a = enqueue(&first, "from first");
    let b = enqueue(&second, "from second");
    let items = first.snapshot().items.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, a.item_id);
    assert_eq!(items[1].id, b.item_id);
}

#[test]
fn read_item_detects_body_checksum_corruption() {
    let (_temp, store) = open_store(Limits::default());
    let receipt = enqueue(&store, "body");
    let path = store
        .dir()
        .join("blobs")
        .join(format!("{}.json", receipt.item_id));
    fs::write(path, br#"{"submitText":"tampered"}"#).unwrap();
    let error = store.read_item(&receipt.item_id).unwrap_err();
    assert_eq!(error.meta.id, receipt.item_id);
    assert!(matches!(error.source, Error::ChecksumMismatch));
}
