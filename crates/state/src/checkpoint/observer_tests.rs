use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::transaction_test_support::{committed_fixture, fixture, write};
use super::*;

const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[test]
fn children_share_store_registry_and_sequence_with_their_own_identity() {
    let fixture = fixture();
    fixture.store.begin(10, "ownership", 0).unwrap();
    write(fixture.root.path(), "a.txt", "before");
    let sequence = Arc::new(AtomicI64::new(40));
    let parent = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 10,
        writer_id: "parent".into(),
        background: false,
        seq: Some(sequence.clone()),
    });
    let foreground = parent.clone_for_subagent("foreground", parent.ownership_turn(), false);
    let background = parent.clone_for_subagent("background", 7, true);

    assert!(std::ptr::eq(parent.store(), foreground.store()));
    assert_eq!(foreground.ownership_turn(), 10);
    assert_eq!(background.ownership_turn(), 7);
    assert_eq!(foreground.writer_id(), "foreground");
    assert_eq!(background.writer_id(), "background");
    assert!(!foreground.background());
    assert!(background.background());

    foreground.before_mutation("a.txt", "edit", CaptureSource::BeforeMutation);
    write(fixture.root.path(), "a.txt", "after");
    background
        .after_mutation("a.txt", "edit")
        .expect("after capture");
    assert_eq!(sequence.load(Ordering::Relaxed), 41);

    let mut writer = background
        .try_register_writer("background", "background", 7)
        .expect("register through child");
    assert_eq!(parent.active_writers()[0].id, "background");
    assert!(parent.has_other_active_writers());
    writer.unregister();
    assert!(parent.active_writers().is_empty());
}

#[test]
fn captures_forward_source_and_fingerprints_and_count_failed_after_attempts() {
    let fixture = fixture();
    fixture.store.begin(4, "capture", 0).unwrap();
    write(fixture.root.path(), "a.txt", "before");
    let sequence = Arc::new(AtomicI64::new(8));
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 4,
        writer_id: "writer".into(),
        background: false,
        seq: Some(sequence.clone()),
    });

    observer.before_mutation("a.txt", "edit", CaptureSource::BeforeMutation);
    write(fixture.root.path(), "a.txt", "after");
    observer.after_mutation("a.txt", "edit").unwrap();
    let state = fixture.store.state();
    let current = state.current.as_ref().unwrap();
    let file = &current.files[0];
    assert_eq!(file.content.as_deref(), Some("before"));
    assert_eq!(file.capture_source, Some(CaptureSource::BeforeMutation));
    assert_eq!(file.after_existed, Some(true));
    assert_eq!(file.after_sha256, digest(b"after"));
    assert_eq!(current.last_mutation_seq, 9);
    drop(state);

    let error = observer
        .after_mutation(fixture.root.path(), "edit")
        .expect_err("directory capture fails");
    assert!(matches!(error, StoreError::Capture(_)));
    assert_eq!(sequence.load(Ordering::Relaxed), 10);
}

#[test]
fn preview_capture_uses_the_previewer_source() {
    let fixture = fixture();
    fixture.store.begin(2, "preview", 0).unwrap();
    write(fixture.root.path(), "a.txt", "disk");
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 2,
        writer_id: "writer".into(),
        background: false,
        seq: None,
    });
    observer.before_mutation_from_change(
        PreviewChange {
            path: "a.txt".into(),
            old_text: "preview".into(),
            ..PreviewChange::default()
        },
        "edit",
    );

    let state = fixture.store.state();
    let file = &state.current.as_ref().unwrap().files[0];
    assert_eq!(file.content.as_deref(), Some("preview"));
    assert_eq!(file.capture_source, Some(CaptureSource::Previewer));
}

#[tokio::test]
async fn background_context_registration_returns_an_owned_permit() {
    let fixture = fixture();
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 1,
        writer_id: "owner".into(),
        background: false,
        seq: None,
    });

    let mut writer = observer
        .register_writer("background", "shell", 1)
        .await
        .expect("background-context registration");
    assert_eq!(observer.active_writers()[0].id, "background");
    assert!(fixture.store.barrier().has_writers());
    writer.unregister();
    assert!(!fixture.store.barrier().busy());
}

#[test]
fn active_writers_are_sorted_and_other_excludes_the_observers_own_id() {
    let fixture = fixture();
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 3,
        writer_id: "a".into(),
        background: true,
        seq: None,
    });
    let mut z = observer
        .try_register_writer("z", "shell", 3)
        .expect("register z");
    let mut a = observer
        .try_register_writer("a", "mcp", 3)
        .expect("register a");

    assert_eq!(
        observer
            .active_writers()
            .iter()
            .map(|writer| writer.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "z"]
    );
    assert!(observer.has_other_active_writers());
    z.unregister();
    assert!(!observer.has_other_active_writers());
    a.unregister();
    assert!(!observer.has_active_writers());
}

#[test]
fn cross_turn_background_writers_record_the_go_gap_detail() {
    let fixture = fixture();
    fixture.store.begin(5, "new turn", 0).unwrap();
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 5,
        writer_id: "new".into(),
        background: false,
        seq: None,
    });
    let mut old = observer
        .try_register_writer("old", "background", 2)
        .expect("register old writer");
    let mut current = observer
        .try_register_writer("current", "background", 5)
        .expect("register current writer");

    observer.note_cross_turn_background_writer(5);
    let state = fixture.store.state();
    let gaps = &state.current.as_ref().unwrap().coverage_gaps;
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].reason, GAP_BACKGROUND_WRITER);
    assert_eq!(
        gaps[0].detail,
        "background writer from earlier turn still active"
    );
    assert_eq!(gaps[0].tool, "background");
    drop(state);
    current.unregister();
    old.unregister();
}

#[tokio::test]
async fn cancellation_before_admission_preserves_the_owner_and_undo_offer() {
    let (fixture, result) = committed_fixture();
    let exclusive = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("exclusive owner");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 1,
        writer_id: "writer".into(),
        background: false,
        seq: None,
    });

    let error = timeout(WAIT_TIMEOUT, observer.begin_write_context(&cancellation))
        .await
        .expect("cancelled admission returned")
        .expect_err("cancelled admission");
    assert!(matches!(error, BeginWriteError::Cancelled));
    assert!(fixture.store.barrier().busy());
    assert_eq!(
        fixture.store.current_undo_offer().unwrap().id,
        result.transaction_id
    );
    drop(exclusive);
}

#[tokio::test]
async fn cancellation_after_durable_invalidation_releases_without_reviving_undo() {
    let (fixture, _) = committed_fixture();
    let cancellation = CancellationToken::new();
    let callback_cancellation = cancellation.clone();
    fixture
        .store
        .on_undo_change(move || callback_cancellation.cancel());
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 1,
        writer_id: "writer".into(),
        background: false,
        seq: None,
    });

    let error = observer
        .begin_write_context(&cancellation)
        .await
        .expect_err("post-invalidation cancellation");
    assert!(matches!(error, BeginWriteError::Cancelled));
    assert!(!fixture.store.barrier().busy());
    assert!(fixture.store.current_undo_offer().is_none());
    assert!(
        Store::new(Some(&fixture.dir), fixture.root.path())
            .current_undo_offer()
            .is_none()
    );
}

#[test]
fn write_permit_release_is_idempotent_and_owned() {
    let fixture = fixture();
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 1,
        writer_id: "writer".into(),
        background: false,
        seq: None,
    });
    let cancellation = CancellationToken::new();
    let mut first = observer
        .try_begin_write(&cancellation)
        .expect("first writer");
    let mut second = observer
        .try_begin_write(&cancellation)
        .expect("second writer");
    first.release();
    let generation = fixture.store.barrier().generation();
    first.release();

    assert_eq!(fixture.store.barrier().generation(), generation);
    assert!(fixture.store.barrier().has_writers());
    assert!(fixture.store.barrier().try_enter_exclusive().is_none());
    second.release();
    second.release();
    assert!(!fixture.store.barrier().busy());
    assert_eq!(fixture.store.barrier().generation(), generation + 1);
}

#[tokio::test]
async fn dropping_a_waiting_admission_future_does_not_leak_a_permit() {
    let fixture = fixture();
    let exclusive = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("exclusive owner");
    let observer = MutationObserver::new(ObserverOptions {
        store: &fixture.store,
        ownership_turn: 1,
        writer_id: "writer".into(),
        background: false,
        seq: None,
    });
    let cancellation = CancellationToken::new();
    let mut waiting = Box::pin(observer.begin_write_context(&cancellation));
    tokio::select! {
        biased;
        _ = &mut waiting => panic!("blocked admission completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    drop(waiting);
    drop(exclusive);

    let successor = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("dropped waiter leaked admission");
    drop(successor);
    assert!(!fixture.store.barrier().busy());
}
