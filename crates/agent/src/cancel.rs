//! Turn and per-call cancellation (Go: `call_stop.go` + the Run context).
//!
//! The turn token ends the whole run; the per-call stops end one running
//! call without ending the turn. Tools in this build execute synchronously,
//! so a mid-execution stop lands between calls: a cancelled turn keeps its
//! completed tool pairs, and a stopped call's slot is closed with the
//! cancelled outcome before the batch moves on. When async tools arrive,
//! the same token reaches into execution.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

/// Cooperative turn cancellation (Go: the Run context's Done channel).
#[derive(Default)]
pub struct CancelToken {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Cancels the turn and wakes every await point parked on
    /// [`CancelToken::wait`].
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
    }

    /// Resolves when the token is cancelled; immediate when it already is.
    pub async fn wait(&self) {
        while !self.is_cancelled() {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

/// One tool call a user may stop by id (Go: `callStops`).
#[derive(Default)]
pub struct CallStops {
    stopped: Mutex<HashMap<String, ()>>,
}

impl CallStops {
    /// Marks one call stopped; reports whether it was registered as
    /// startable (Go: `CancelToolCall`'s bool).
    pub fn cancel(&self, id: &str) -> bool {
        self.stopped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_owned(), ())
            .is_none()
    }

    /// Whether the user stopped this call before it ran.
    pub fn is_stopped(&self, id: &str) -> bool {
        self.stopped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    /// Clears the registration at the end of the batch (Go: the begin
    /// closure's unregister).
    pub fn finish(&self, id: &str) {
        self.stopped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }
}

/// The model-facing outcome of a stopped call (Go: `markCancelledFrom`).
pub const STOPPED_CALL_OUTPUT: &str = "cancelled: context cancelled before execution";
