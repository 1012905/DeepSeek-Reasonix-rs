//! Writer goroutine, bounded notification pool, and the write-stall watchdog.

use std::io;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::error::{ConnError, Direction, OutboundQueueFullError, WriteStallError};
use crate::framing::{FrameWriter, WriteError};
use crate::rawjson::RawJson;

use super::NotificationHandlerFn;
use super::Wire;

pub(crate) struct WriteJob {
    pub(crate) frame: Vec<u8>,
    pub(crate) aborted: Arc<AtomicBool>,
    pub(crate) release: Option<Arc<dyn Fn() + Send + Sync>>,
    pub(crate) result: Sender<Result<(), WriteError>>,
}

pub(crate) struct NotificationCall {
    pub(crate) handler: Arc<NotificationHandlerFn>,
    pub(crate) params: Option<RawJson>,
}

/// Marks physical write activity for the stall watchdog.
#[derive(Default)]
pub(crate) struct WriteProgress {
    active: AtomicBool,
    last_nanos: AtomicU64,
}

impl WriteProgress {
    pub(crate) fn begin(&self, started: Instant) {
        self.last_nanos
            .store(started.elapsed().as_nanos() as u64, Ordering::SeqCst);
        self.active.store(true, Ordering::SeqCst);
    }

    pub(crate) fn end(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    pub(crate) fn idle_for(&self, started: Instant) -> Duration {
        let last = Duration::from_nanos(self.last_nanos.load(Ordering::SeqCst));
        started.elapsed().saturating_sub(last)
    }
}

pub(crate) fn writer_loop<W: Write>(
    wire: &Wire,
    writer: FrameWriter<W>,
    receiver: Receiver<WriteJob>,
) {
    while let Ok(job) = receiver.recv() {
        wire.write_slots.release();
        if job.aborted.load(Ordering::SeqCst) {
            complete(
                job,
                Err(WriteError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "caller gave up before the write started",
                ))),
            );
            continue;
        }
        if wire.closed.load(Ordering::SeqCst) {
            complete(job, Err(WriteError::Io(closed_error())));
            continue;
        }
        wire.progress.begin(wire.started);
        let written = writer.write_encoded(&job.frame).map(|_| ());
        wire.progress.end();
        let failed = written.is_err();
        complete(job, written);
        if failed {
            return;
        }
    }
}

pub(crate) fn complete(job: WriteJob, outcome: Result<(), WriteError>) {
    let _ = job.result.send(outcome);
    if let Some(release) = job.release {
        release();
    }
}

/// Fails the connection when a physical write makes no progress for `bound`.
///
/// Deliberately independent of any caller deadline: a short per-call timeout
/// cannot preempt it, so a wedged peer is still torn down.
pub(crate) fn stall_watchdog(wire: Weak<Wire>, bound: Duration) {
    let interval = (bound / 2).max(Duration::from_millis(1));
    loop {
        std::thread::sleep(interval);
        let Some(wire) = wire.upgrade() else {
            return;
        };
        if wire.closed.load(Ordering::SeqCst) {
            return;
        }
        if wire.progress.active.load(Ordering::SeqCst)
            && wire.progress.idle_for(wire.started) > bound
        {
            wire.shutdown(Some(ConnError::WriteStall(WriteStallError {
                direction: Direction::Outbound,
                stall_ms: bound.as_millis() as u64,
            })));
            return;
        }
    }
}

pub(crate) fn closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "connection closed")
}

pub(crate) fn queue_full(limit: usize) -> ConnError {
    ConnError::QueueFull(OutboundQueueFullError { limit })
}

pub(crate) fn is_caller_abort(err: &ConnError) -> bool {
    matches!(
        err,
        ConnError::FrameTooLarge(_) | ConnError::TimedOut | ConnError::QueueFull(_)
    )
}

pub(crate) fn write_error_to_conn(err: WriteError) -> ConnError {
    match err {
        WriteError::FrameTooLarge(too_large) => ConnError::FrameTooLarge(too_large),
        WriteError::QueueFull(full) => ConnError::QueueFull(full),
        WriteError::Io(io) => ConnError::Io(io.to_string()),
    }
}

pub(crate) fn conn_to_write_error(err: &ConnError) -> WriteError {
    match err {
        ConnError::FrameTooLarge(too_large) => WriteError::FrameTooLarge(*too_large),
        ConnError::QueueFull(full) => WriteError::QueueFull(*full),
        other => WriteError::Io(io::Error::other(other.to_string())),
    }
}
