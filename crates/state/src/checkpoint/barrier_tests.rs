use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Waker};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::timeout;

use super::barrier::{BarrierError, MutationBarrier};

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);

fn assert_pending<F: Future>(mut future: Pin<&mut F>) {
    let mut context = Context::from_waker(Waker::noop());
    assert!(future.as_mut().poll(&mut context).is_pending());
}

async fn announce<F>(started: oneshot::Sender<()>, future: F) -> F::Output
where
    F: Future,
{
    let _ = started.send(());
    future.await
}

#[test]
fn writers_share_admission_and_count_each_release() {
    let barrier = MutationBarrier::new();
    assert_eq!(barrier.generation(), 0);
    assert!(!barrier.has_writers());
    assert!(!barrier.busy());

    let first = barrier.try_enter_write().expect("first writer");
    let second = barrier.clone().try_enter_write().expect("second writer");
    assert!(barrier.has_writers());
    assert!(barrier.busy());
    assert!(barrier.try_enter_exclusive().is_none());
    assert_eq!(barrier.generation(), 0);

    drop(first);
    assert_eq!(barrier.generation(), 1);
    assert!(barrier.has_writers());
    assert!(barrier.busy());
    assert!(barrier.try_enter_exclusive().is_none());

    drop(second);
    assert_eq!(barrier.generation(), 2);
    assert!(!barrier.has_writers());
    assert!(!barrier.busy());

    let exclusive = barrier.try_enter_exclusive().expect("exclusive");
    assert!(barrier.busy());
    assert!(barrier.try_enter_write().is_none());
    assert!(barrier.try_enter_exclusive().is_none());
    assert_eq!(barrier.generation(), 2);
    drop(exclusive);
    assert_eq!(barrier.generation(), 3);
    assert!(!barrier.busy());
}

#[test]
fn exclusive_excludes_try_entrants() {
    let barrier = MutationBarrier::new();
    let exclusive = barrier.try_enter_exclusive().expect("exclusive");
    assert!(barrier.try_enter_write().is_none());
    assert!(barrier.try_enter_exclusive().is_none());
    assert!(!barrier.has_writers());
    assert!(barrier.busy());
    drop(exclusive);
    assert!(!barrier.busy());
}

#[tokio::test]
async fn closed_barrier_rejects_try_and_async_entrants() {
    let barrier = MutationBarrier::new();
    barrier.close();
    barrier.close();

    assert!(barrier.try_enter_write().is_none());
    assert!(barrier.try_enter_exclusive().is_none());
    assert!(!barrier.busy());
    assert_eq!(barrier.generation(), 0);
    assert_eq!(
        timeout(WAIT_TIMEOUT, barrier.enter_write())
            .await
            .expect("closed writer returned promptly")
            .expect_err("closed writer"),
        BarrierError::Closed
    );
    assert_eq!(
        timeout(WAIT_TIMEOUT, barrier.enter_exclusive())
            .await
            .expect("closed exclusive returned promptly")
            .expect_err("closed exclusive"),
        BarrierError::Closed
    );
}

#[tokio::test]
async fn async_exclusive_waits_for_last_writer() {
    let barrier = MutationBarrier::new();
    let first = barrier.try_enter_write().expect("first writer");
    let second = barrier.try_enter_write().expect("second writer");
    let (started_tx, started_rx) = oneshot::channel();
    let waiting_barrier = barrier.clone();
    let waiter = tokio::spawn(announce(started_tx, async move {
        waiting_barrier.enter_exclusive().await
    }));

    started_rx.await.expect("waiter started");
    assert!(!waiter.is_finished());
    drop(first);
    assert_eq!(barrier.generation(), 1);
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());
    assert!(barrier.has_writers());

    drop(second);
    assert_eq!(barrier.generation(), 2);
    let exclusive = timeout(WAIT_TIMEOUT, waiter)
        .await
        .expect("exclusive waiter woke")
        .expect("waiter task")
        .expect("exclusive admission");
    assert!(barrier.busy());
    assert!(!barrier.has_writers());
    assert!(barrier.try_enter_write().is_none());
    drop(exclusive);
    assert_eq!(barrier.generation(), 3);
    assert!(!barrier.busy());
}

#[tokio::test]
async fn async_writer_waits_for_exclusive_release() {
    let barrier = MutationBarrier::new();
    let exclusive = barrier.try_enter_exclusive().expect("exclusive");
    let (started_tx, started_rx) = oneshot::channel();
    let waiting_barrier = barrier.clone();
    let waiter = tokio::spawn(announce(started_tx, async move {
        waiting_barrier.enter_write().await
    }));

    started_rx.await.expect("waiter started");
    assert!(!waiter.is_finished());
    assert!(barrier.try_enter_write().is_none());
    drop(exclusive);
    assert_eq!(barrier.generation(), 1);

    let writer = timeout(WAIT_TIMEOUT, waiter)
        .await
        .expect("writer waiter woke")
        .expect("waiter task")
        .expect("writer admission");
    assert!(barrier.has_writers());
    assert!(barrier.try_enter_exclusive().is_none());
    drop(writer);
    assert_eq!(barrier.generation(), 2);
    assert!(!barrier.busy());
}

#[tokio::test]
async fn close_wakes_all_waiters_without_releasing_owner() {
    let barrier = MutationBarrier::new();
    let owner = barrier.try_enter_exclusive().expect("exclusive owner");
    let (writer_started_tx, writer_started_rx) = oneshot::channel();
    let writer_barrier = barrier.clone();
    let writer = tokio::spawn(announce(writer_started_tx, async move {
        writer_barrier.enter_write().await
    }));
    let (exclusive_started_tx, exclusive_started_rx) = oneshot::channel();
    let exclusive_barrier = barrier.clone();
    let exclusive = tokio::spawn(announce(exclusive_started_tx, async move {
        exclusive_barrier.enter_exclusive().await
    }));

    writer_started_rx.await.expect("writer started");
    exclusive_started_rx.await.expect("exclusive started");
    assert!(!writer.is_finished());
    assert!(!exclusive.is_finished());
    barrier.clone().close();

    assert_eq!(
        timeout(WAIT_TIMEOUT, writer)
            .await
            .expect("writer woke")
            .expect("writer task")
            .expect_err("closed writer"),
        BarrierError::Closed
    );
    assert_eq!(
        timeout(WAIT_TIMEOUT, exclusive)
            .await
            .expect("exclusive woke")
            .expect("exclusive task")
            .expect_err("closed exclusive"),
        BarrierError::Closed
    );
    assert!(barrier.busy());
    assert!(!barrier.has_writers());
    assert_eq!(barrier.generation(), 0);
    assert!(barrier.try_enter_write().is_none());
    assert!(barrier.try_enter_exclusive().is_none());
    drop(owner);
    assert_eq!(barrier.generation(), 1);
    assert!(!barrier.busy());
}

#[test]
fn permit_release_is_idempotent() {
    let barrier = MutationBarrier::new();
    let mut writer = barrier.try_enter_write().expect("writer");
    writer.release();
    writer.release();
    assert_eq!(barrier.generation(), 1);
    assert!(!barrier.busy());
    drop(writer);
    assert_eq!(barrier.generation(), 1);

    let mut exclusive = barrier.try_enter_exclusive().expect("exclusive");
    exclusive.release();
    exclusive.release();
    assert_eq!(barrier.generation(), 2);
    assert!(!barrier.busy());
    drop(exclusive);
    assert_eq!(barrier.generation(), 2);
}

#[tokio::test]
async fn cancelled_write_wait_cannot_leak_admission() {
    let barrier = MutationBarrier::new();
    let owner = barrier.try_enter_exclusive().expect("exclusive owner");
    let mut enter = Box::pin(barrier.enter_write());
    assert_pending(enter.as_mut());
    let outcome = tokio::select! {
        biased;
        result = &mut enter => panic!("blocked writer completed: {result:?}"),
        _ = tokio::task::yield_now() => BarrierError::Cancelled,
    };
    assert_eq!(outcome, BarrierError::Cancelled);
    drop(enter);

    assert!(barrier.busy());
    assert!(!barrier.has_writers());
    assert_eq!(barrier.generation(), 0);
    assert!(barrier.try_enter_write().is_none());
    assert!(barrier.try_enter_exclusive().is_none());
    drop(owner);
    assert_eq!(barrier.generation(), 1);

    let successor = barrier.try_enter_exclusive().expect("no leaked writer");
    drop(successor);
    assert_eq!(barrier.generation(), 2);
    assert!(!barrier.busy());
}

#[tokio::test]
async fn cancelled_exclusive_wait_cannot_leak_admission() {
    let barrier = MutationBarrier::new();
    let owner = barrier.try_enter_write().expect("writer owner");
    let mut enter = Box::pin(barrier.enter_exclusive());
    assert_pending(enter.as_mut());
    let outcome = tokio::select! {
        biased;
        result = &mut enter => panic!("blocked exclusive completed: {result:?}"),
        _ = tokio::task::yield_now() => BarrierError::Cancelled,
    };
    assert_eq!(outcome, BarrierError::Cancelled);
    drop(enter);

    assert!(barrier.busy());
    assert!(barrier.has_writers());
    assert_eq!(barrier.generation(), 0);
    assert!(barrier.try_enter_exclusive().is_none());
    drop(owner);
    assert_eq!(barrier.generation(), 1);

    let successor = barrier.try_enter_exclusive().expect("no leaked exclusive");
    assert!(barrier.busy());
    drop(successor);
    assert_eq!(barrier.generation(), 2);
    assert!(!barrier.busy());
}

#[tokio::test]
async fn lost_wakeup_between_failed_admission_and_wait_is_not_lost() {
    let barrier = MutationBarrier::new();
    let mut owner = Some(barrier.try_enter_exclusive().expect("exclusive owner"));
    let mut first_attempt = true;
    let exclusive = timeout(
        WAIT_TIMEOUT,
        barrier.wait_for_admission(|| {
            let admitted = barrier.try_enter_exclusive();
            if first_attempt {
                first_attempt = false;
                owner.take();
            }
            Ok(admitted)
        }),
    )
    .await
    .expect("release between admission check and wait woke the waiter")
    .expect("exclusive admission");

    assert!(!first_attempt);
    assert_eq!(barrier.generation(), 1);
    assert!(barrier.busy());
    assert!(barrier.try_enter_write().is_none());
    drop(exclusive);
    assert_eq!(barrier.generation(), 2);
    assert!(!barrier.busy());
}
