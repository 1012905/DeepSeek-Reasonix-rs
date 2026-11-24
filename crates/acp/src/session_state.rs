//! In-session runtime state machine and the retired-lease tracker
//! (service.go:213-301, 341-373, 397-432; matrix B §5).
//!
//! The Go oracle derives its state from `running`, `deleted`, `maintenanceDone`,
//! and `pendingConfig` booleans; this port makes the four states explicit. The
//! machine owns admission and transition rules only. Real cancellation
//! channels, lease release, and the connection close-all sweep are runtime
//! wiring; the lease refusal code table lives in [`crate::lifecycle`].

use crate::errors::{RpcError, RpcErrorCode};
use crate::lifecycle::active_prompt_conflict;

/// Derived runtime state of one session (matrix B §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RuntimeState {
    /// Idle: no turn, no maintenance, no queued config.
    #[default]
    Idle,
    /// A turn is in flight.
    Running,
    /// Idle background maintenance (e.g. a config rebuild) is in flight.
    Maintenance,
    /// Retired: deleteAndWait has marked the session deleted.
    Deleted,
}

/// Why `begin` refused to admit a new turn (service.go:287-295).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BeginRefusal {
    /// A turn is already active in this process.
    #[error("busy active prompt")]
    BusyActivePrompt,
    /// The session has been deleted.
    #[error("session deleted")]
    Deleted,
    /// Background maintenance is in flight.
    #[error("maintenance in flight")]
    MaintenanceInFlight,
    /// A queued config delta blocks new turns.
    #[error("queued config")]
    QueuedConfig,
}

/// Pure runtime machine. `pending_config` mirrors the queued config deltas; a
/// non-empty queue blocks a new turn so a prompt never runs on the outgoing
/// config.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeMachine {
    state: RuntimeState,
    pending_config: bool,
}

impl RuntimeMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> RuntimeState {
        self.state
    }

    /// Admits a new turn. Refuses when a turn or maintenance is in flight, the
    /// session is deleted, or a config delta is queued (service.go:287-295).
    pub fn begin(&mut self, pending_config: bool) -> Result<(), BeginRefusal> {
        match self.state {
            RuntimeState::Running => Err(BeginRefusal::BusyActivePrompt),
            RuntimeState::Deleted => Err(BeginRefusal::Deleted),
            RuntimeState::Maintenance => Err(BeginRefusal::MaintenanceInFlight),
            RuntimeState::Idle if self.pending_config => Err(BeginRefusal::QueuedConfig),
            RuntimeState::Idle => {
                self.state = RuntimeState::Running;
                self.pending_config = pending_config;
                Ok(())
            }
        }
    }

    /// `finish()`: the turn ended; clear the cancel/done slots and return idle.
    pub fn finish(&mut self) {
        if self.state == RuntimeState::Running {
            self.state = RuntimeState::Idle;
        }
    }

    /// `abort()`: cancel the in-flight turn without waiting for completion.
    pub fn abort(&mut self) {
        self.finish();
    }

    /// `abortAndWait()`: cancel, then wait for the turn and maintenance to
    /// drain; the pure machine lands back idle.
    pub fn abort_and_wait(&mut self) {
        self.finish();
    }

    /// `deleteAndWait()`: mark deleted first, then cancel and drain.
    pub fn delete_and_wait(&mut self) {
        self.state = RuntimeState::Deleted;
    }

    /// Idle maintenance starts; only an idle session can enter maintenance.
    pub fn start_maintenance(&mut self) -> Result<(), BeginRefusal> {
        match self.state {
            RuntimeState::Idle => {
                self.state = RuntimeState::Maintenance;
                Ok(())
            }
            RuntimeState::Running => Err(BeginRefusal::BusyActivePrompt),
            RuntimeState::Maintenance => Err(BeginRefusal::MaintenanceInFlight),
            RuntimeState::Deleted => Err(BeginRefusal::Deleted),
        }
    }

    /// `finishMaintenance(done)`: clear the maintenance slot and return idle.
    /// A stale channel must not close a newer one; the caller matches the
    /// channel before calling this (service.go:359-373).
    pub fn finish_maintenance(&mut self) {
        if self.state == RuntimeState::Maintenance {
            self.state = RuntimeState::Idle;
        }
    }

    /// Queues a config delta; a non-empty queue blocks the next `begin`.
    pub fn queue_config(&mut self) {
        self.pending_config = true;
    }

    /// Drains the queued config after the applying turn/maintenance finishes.
    pub fn drain_config(&mut self) {
        self.pending_config = false;
    }

    /// Maps a begin refusal onto its RPC identity. The in-process concurrent
    /// prompt keeps the `-32600` active-prompt message; the other refusals are
    /// admission failures the prompt path reports as invalid request.
    pub fn refusal_error(method: &str, refusal: BeginRefusal) -> RpcError {
        match refusal {
            BeginRefusal::BusyActivePrompt => active_prompt_conflict(method),
            BeginRefusal::Deleted => RpcError::invalid_params(format!("{method}: session deleted")),
            BeginRefusal::MaintenanceInFlight => RpcError::new(
                RpcErrorCode::InvalidRequest,
                format!("{method}: maintenance in flight"),
            ),
            BeginRefusal::QueuedConfig => RpcError::new(
                RpcErrorCode::InvalidRequest,
                format!("{method}: queued config blocks a new turn"),
            ),
        }
    }
}

/// Deferred-release leases from a recovery handoff. A lease retired inside the
/// save callback cannot release synchronously (it would deadlock the save it
/// is running in), so its release completes on a side channel; any operation
/// that exposes a completed snapshot waits for every retired lease first
/// (service.go:400-432).
#[derive(Debug, Clone, Default)]
pub struct RetiredLeases {
    pending: usize,
}

impl RetiredLeases {
    /// Registers one retired lease whose release is still pending.
    pub fn retire(&mut self) {
        self.pending += 1;
    }

    /// Marks one retired lease's release as complete.
    pub fn complete(&mut self) {
        self.pending = self.pending.saturating_sub(1);
    }

    /// True when every retired lease has released.
    pub fn all_done(&self) -> bool {
        self.pending == 0
    }

    /// Number of releases still pending, for diagnostics and tests.
    pub fn pending(&self) -> usize {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_refuses_while_running_deleted_maintenance_or_queued() {
        let mut m = RuntimeMachine::new();
        assert!(m.begin(false).is_ok());
        assert!(matches!(
            m.begin(false).unwrap_err(),
            BeginRefusal::BusyActivePrompt
        ));
        m.finish();

        m.queue_config();
        assert!(matches!(
            m.begin(false).unwrap_err(),
            BeginRefusal::QueuedConfig
        ));
        m.drain_config();
        assert!(m.begin(false).is_ok());
        m.finish();

        assert!(m.start_maintenance().is_ok());
        assert!(matches!(
            m.begin(false).unwrap_err(),
            BeginRefusal::MaintenanceInFlight
        ));
        m.finish_maintenance();

        m.delete_and_wait();
        assert!(matches!(m.begin(false).unwrap_err(), BeginRefusal::Deleted));
    }

    #[test]
    fn finish_abort_delete_transitions_land_on_expected_state() {
        let mut m = RuntimeMachine::new();
        m.begin(false).unwrap();
        m.finish();
        assert_eq!(m.state(), RuntimeState::Idle);

        m.begin(false).unwrap();
        m.abort();
        assert_eq!(m.state(), RuntimeState::Idle);

        m.begin(false).unwrap();
        m.abort_and_wait();
        assert_eq!(m.state(), RuntimeState::Idle);

        m.delete_and_wait();
        assert_eq!(m.state(), RuntimeState::Deleted);
    }

    #[test]
    fn refusal_error_keeps_active_prompt_code() {
        let err = RuntimeMachine::refusal_error("session/prompt", BeginRefusal::BusyActivePrompt);
        assert_eq!(err.code, RpcErrorCode::InvalidRequest);
        assert_eq!(
            err.message,
            "session/prompt: session already has an active prompt"
        );
    }

    #[test]
    fn retired_lease_tracker_counts_pending_releases() {
        let mut leases = RetiredLeases::default();
        assert!(leases.all_done());
        leases.retire();
        leases.retire();
        assert_eq!(leases.pending(), 2);
        assert!(!leases.all_done());
        leases.complete();
        assert!(!leases.all_done());
        leases.complete();
        assert!(leases.all_done());
    }
}
