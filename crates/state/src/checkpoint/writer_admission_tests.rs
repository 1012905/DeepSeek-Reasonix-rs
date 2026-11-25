use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::transaction_test_support::{committed_fixture, fixture};
use super::writer_admission::WriterRegistry;
use super::*;

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);

fn observer(store: &Store) -> MutationObserver<'_> {
    MutationObserver::new(ObserverOptions {
        store,
        ownership_turn: 1,
        writer_id: "owner".into(),
        background: false,
        seq: None,
    })
}

async fn wait_for_registration(observer: &MutationObserver<'_>, id: &str) {
    timeout(WAIT_TIMEOUT, async {
        while !observer.has_writer_registration_for_test(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer registration was not inserted");
}

#[tokio::test]
async fn same_id_follower_cancellation_does_not_cancel_the_leader_or_extra_permit() {
    let fixture = fixture();
    let observer = observer(&fixture.store);
    let exclusive = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("exclusive owner");
    let leader_cancellation = CancellationToken::new();
    let mut leader =
        Box::pin(observer.register_writer_context(&leader_cancellation, "shared", "leader", 1));
    tokio::select! {
        biased;
        _ = &mut leader => panic!("blocked leader completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    wait_for_registration(&observer, "shared").await;

    let cancellation = CancellationToken::new();
    let mut follower =
        Box::pin(observer.register_writer_context(&cancellation, "shared", "follower", 2));
    tokio::select! {
        biased;
        _ = &mut follower => panic!("pending follower completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    cancellation.cancel();
    let error = timeout(WAIT_TIMEOUT, follower)
        .await
        .expect("follower cancellation returned")
        .expect_err("cancelled follower");
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);

    drop(exclusive);
    let mut leader = timeout(WAIT_TIMEOUT, leader)
        .await
        .expect("leader admission completed")
        .expect("leader admission");
    assert_eq!(observer.active_writers()[0].kind, "leader");
    assert!(fixture.store.barrier().has_writers());

    let mut follower = observer
        .register_writer_context(&CancellationToken::new(), "shared", "follower", 2)
        .await
        .expect("ready follower");
    assert_eq!(observer.active_writers().len(), 1);
    follower.unregister();
    assert!(!fixture.store.barrier().busy());
    let generation = fixture.store.barrier().generation();
    leader.unregister();
    assert_eq!(fixture.store.barrier().generation(), generation);
}

#[tokio::test]
async fn unregister_of_a_pending_writer_cancels_without_waiting_for_admission() {
    let fixture = fixture();
    let observer = observer(&fixture.store);
    let exclusive = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("exclusive owner");
    let registration_cancellation = CancellationToken::new();
    let mut registering = Box::pin(observer.register_writer_context(
        &registration_cancellation,
        "pending",
        "background",
        1,
    ));
    tokio::select! {
        biased;
        _ = &mut registering => panic!("blocked registration completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    wait_for_registration(&observer, "pending").await;

    observer.unregister_writer("pending");
    let error = timeout(WAIT_TIMEOUT, registering)
        .await
        .expect("pending unregister returned")
        .expect_err("retired registration");
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);
    assert!(observer.active_writers().is_empty());
    assert!(fixture.store.barrier().busy());
    assert!(!fixture.store.barrier().has_writers());
    drop(exclusive);
}

#[tokio::test]
async fn dropping_a_pending_registration_retires_its_placeholder() {
    let fixture = fixture();
    let observer = observer(&fixture.store);
    let exclusive = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("exclusive owner");
    let cancellation = CancellationToken::new();
    let mut registering =
        Box::pin(observer.register_writer_context(&cancellation, "dropped", "background", 1));
    tokio::select! {
        biased;
        _ = &mut registering => panic!("blocked registration completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    wait_for_registration(&observer, "dropped").await;

    drop(registering);
    drop(exclusive);
    let mut replacement = observer
        .try_register_writer("dropped", "replacement", 2)
        .expect("dropped registration retired its placeholder");
    assert_eq!(observer.active_writers()[0].kind, "replacement");
    replacement.unregister();
    assert!(!fixture.store.barrier().busy());
}

#[tokio::test]
async fn dropping_a_pending_leader_closes_its_followers() {
    let fixture = fixture();
    let observer = observer(&fixture.store);
    let exclusive = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("exclusive owner");
    let leader_cancellation = CancellationToken::new();
    let follower_cancellation = CancellationToken::new();
    let mut leader =
        Box::pin(observer.register_writer_context(&leader_cancellation, "shared", "leader", 1));
    tokio::select! {
        biased;
        _ = &mut leader => panic!("blocked leader completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    wait_for_registration(&observer, "shared").await;
    let mut follower =
        Box::pin(observer.register_writer_context(&follower_cancellation, "shared", "follower", 2));
    tokio::select! {
        biased;
        _ = &mut follower => panic!("blocked follower completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while observer.writer_followers_for_test("shared") == 0 {
        assert!(Instant::now() < deadline, "follower did not subscribe");
        tokio::task::yield_now().await;
    }

    drop(leader);
    let error = timeout(WAIT_TIMEOUT, follower)
        .await
        .expect("dropped leader closed its follower")
        .expect_err("abandoned leader");
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);
    assert!(observer.active_writers().is_empty());
    drop(exclusive);
}

#[tokio::test]
async fn dropping_an_explicitly_retired_pending_leader_still_closes_followers() {
    let fixture = fixture();
    let observer = observer(&fixture.store);
    let exclusive = fixture
        .store
        .barrier()
        .try_enter_exclusive()
        .expect("exclusive owner");
    let leader_cancellation = CancellationToken::new();
    let follower_cancellation = CancellationToken::new();
    let mut leader =
        Box::pin(observer.register_writer_context(&leader_cancellation, "shared", "leader", 1));
    tokio::select! {
        biased;
        _ = &mut leader => panic!("blocked leader completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    wait_for_registration(&observer, "shared").await;
    let mut follower =
        Box::pin(observer.register_writer_context(&follower_cancellation, "shared", "follower", 2));
    tokio::select! {
        biased;
        _ = &mut follower => panic!("blocked follower completed unexpectedly"),
        _ = tokio::task::yield_now() => {}
    }
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while observer.writer_followers_for_test("shared") == 0 {
        assert!(Instant::now() < deadline, "follower did not subscribe");
        tokio::task::yield_now().await;
    }

    observer.unregister_writer("shared");
    drop(leader);
    let error = timeout(WAIT_TIMEOUT, follower)
        .await
        .expect("retired leader drop closed its follower")
        .expect_err("retired leader");
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);
    drop(exclusive);
}

#[tokio::test]
async fn empty_writer_ids_are_successful_noops() {
    let fixture = fixture();
    let observer = observer(&fixture.store);
    let cancellation = CancellationToken::new();
    let mut asynchronous = observer
        .register_writer_context(&cancellation, "", "background", 1)
        .await
        .expect("empty asynchronous registration");
    asynchronous.unregister();
    let mut synchronous = observer
        .try_register_writer("", "background", 1)
        .expect("empty synchronous registration");
    synchronous.unregister();

    assert!(observer.active_writers().is_empty());
    assert!(!fixture.store.barrier().busy());
}

#[test]
fn old_cleanup_cannot_remove_or_release_a_replacement_generation() {
    let fixture = fixture();
    let observer = observer(&fixture.store);
    let mut first = observer
        .try_register_writer("reused", "old", 1)
        .expect("first generation");
    first.unregister();
    let mut second = observer
        .try_register_writer("reused", "new", 2)
        .expect("replacement generation");

    first.unregister();
    let writers = observer.active_writers();
    assert_eq!(writers.len(), 1);
    assert_eq!(writers[0].turn, 2);
    assert_eq!(writers[0].kind, "new");
    assert!(fixture.store.barrier().has_writers());
    second.unregister();
    assert!(!fixture.store.barrier().busy());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retired_preparation_cannot_delete_a_replacement() {
    let (fixture, _) = committed_fixture();
    let observer = observer(&fixture.store);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let finish_rx = Arc::new(Mutex::new(finish_rx));
    fixture.store.on_undo_change(move || {
        let _ = entered_tx.send(());
        let _ = finish_rx.lock().unwrap().recv();
    });
    let runtime = tokio::runtime::Handle::current();
    let (result_tx, result_rx) = mpsc::channel();

    let (error, mut replacement) = std::thread::scope(|scope| {
        let thread_runtime = runtime.clone();
        let preparing_observer = &observer;
        scope.spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                thread_runtime.block_on(preparing_observer.register_writer_context(
                    &CancellationToken::new(),
                    "reused",
                    "old",
                    1,
                ))
            }));
            let _ = result_tx.send(result);
        });
        entered_rx
            .recv_timeout(WAIT_TIMEOUT)
            .expect("undo callback");

        observer.unregister_writer("reused");
        let replacement = observer
            .try_register_writer("reused", "new", 2)
            .expect("replacement registration");
        let _ = finish_tx.send(());
        let error = result_rx
            .recv_timeout(WAIT_TIMEOUT)
            .expect("retired preparation returned")
            .expect("preparation did not panic")
            .expect_err("retired preparation");
        (error, replacement)
    });
    assert_eq!(error.kind(), WriterErrorKind::Cancelled);
    let writers = observer.active_writers();
    assert_eq!(writers.len(), 1);
    assert_eq!(writers[0].kind, "new");
    assert!(fixture.store.barrier().has_writers());
    replacement.unregister();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panic_during_preparation_closes_followers_releases_and_repanics() {
    let (fixture, _) = committed_fixture();
    let observer = observer(&fixture.store);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (fail_tx, fail_rx) = mpsc::channel();
    let fail_rx = Arc::new(Mutex::new(fail_rx));
    fixture.store.on_undo_change(move || {
        let _ = entered_tx.send(());
        let _ = fail_rx.lock().unwrap().recv();
        panic!("preparation callback panic");
    });
    let runtime = tokio::runtime::Handle::current();
    let (result_tx, result_rx) = mpsc::channel();
    let (follower_tx, follower_rx) = mpsc::channel();
    let (follower_started_tx, follower_started_rx) = mpsc::channel();

    let (panic, error) = std::thread::scope(|scope| {
        let thread_runtime = runtime.clone();
        let preparing_observer = &observer;
        scope.spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                thread_runtime.block_on(preparing_observer.register_writer_context(
                    &CancellationToken::new(),
                    "panic",
                    "background",
                    1,
                ))
            }));
            let _ = result_tx.send(result);
        });
        entered_rx
            .recv_timeout(WAIT_TIMEOUT)
            .expect("undo callback");
        let follower_runtime = runtime.clone();
        let follower_observer = &observer;
        scope.spawn(move || {
            let _ = follower_started_tx.send(());
            let result = follower_runtime.block_on(follower_observer.register_writer_context(
                &CancellationToken::new(),
                "panic",
                "background",
                1,
            ));
            let _ = follower_tx.send(result);
        });
        follower_started_rx
            .recv_timeout(WAIT_TIMEOUT)
            .expect("follower started");
        let deadline = Instant::now() + WAIT_TIMEOUT;
        while observer.writer_followers_for_test("panic") == 0 {
            assert!(
                Instant::now() < deadline,
                "follower did not subscribe to the pending registration"
            );
            std::thread::yield_now();
        }

        let _ = fail_tx.send(());
        let panic = result_rx
            .recv_timeout(WAIT_TIMEOUT)
            .expect("panicked preparation returned")
            .expect_err("preparation repanicked");
        let error = follower_rx
            .recv_timeout(WAIT_TIMEOUT)
            .expect("follower was closed")
            .expect_err("panicked leader");
        (panic, error)
    });
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"preparation callback panic")
    );
    assert_eq!(error.kind(), WriterErrorKind::Panicked);
    assert!(observer.active_writers().is_empty());
    assert!(!fixture.store.barrier().busy());
    assert!(fixture.store.current_undo_offer().is_none());
}

#[tokio::test]
async fn registry_mutex_is_not_held_across_undo_callbacks() {
    let (fixture, _) = committed_fixture();
    let registry = Arc::new(WriterRegistry::default());
    let locked = Arc::new(AtomicBool::new(false));
    let callback_locked = locked.clone();
    let callback_registry = registry.clone();
    fixture.store.on_undo_change(move || {
        callback_locked.store(
            callback_registry.state.try_lock().is_err(),
            Ordering::Relaxed,
        );
    });
    let observer = MutationObserver::with_registry(
        ObserverOptions {
            store: &fixture.store,
            ownership_turn: 1,
            writer_id: "owner".into(),
            background: false,
            seq: None,
        },
        registry,
    );

    let mut writer = observer
        .register_writer_context(&CancellationToken::new(), "writer", "background", 1)
        .await
        .expect("writer registration");
    assert!(!locked.load(Ordering::Relaxed));
    writer.unregister();
}

#[test]
fn publication_is_serialized_so_an_older_snapshot_cannot_restore_a_removed_writer() {
    let fixture = fixture();
    let observer = Arc::new(observer(&fixture.store));
    let mut first = observer
        .try_register_writer("first", "background", 1)
        .expect("first writer");
    let mut second = observer
        .try_register_writer("second", "background", 1)
        .expect("second writer");
    let store_state = fixture.store.state();
    let published_observer = observer.clone();

    let wait_for = |count| {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        while published_observer.active_writers().len() != count {
            assert!(
                Instant::now() < deadline,
                "registry update did not complete"
            );
            std::thread::yield_now();
        }
    };
    std::thread::scope(|scope| {
        scope.spawn(|| first.unregister());
        wait_for(1);
        scope.spawn(|| second.unregister());
        wait_for(0);
        drop(store_state);
    });

    assert!(fixture.store.state().active_writers.is_empty());
    assert!(!fixture.store.barrier().busy());
}
