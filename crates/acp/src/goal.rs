//! Goal resume/pause semantics (session_goal.go:15-118).
//!
//! Resume re-enters a retained Goal with its objective, counters, todos, and
//! delivery checkpoint; it does **not** start a turn — the next `session/prompt`
//! continues it. Pause takes effect at the next Goal continuation boundary and
//! leaves the current turn to `session/cancel`. The controller owns the actual
//! resume/pause call; this module freezes the admissible state transitions and
//! the typed refusal identities.

use serde::{Deserialize, Serialize};

use crate::errors::RpcError;
use crate::extensions::{goal_not_resumable, goal_not_running};
use crate::methods::{SESSION_GOAL_PAUSE, SESSION_GOAL_RESUME};

/// Goal lifecycle status (control/input.go:51-54; projection `none` rides the
/// status snapshot as the empty-goal state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Running,
    Complete,
    Blocked,
    Stopped,
    None,
}

impl GoalStatus {
    /// Wire vocabulary string.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Blocked => "blocked",
            Self::Stopped => "stopped",
            Self::None => "none",
        }
    }

    /// Parses the wire status; anything outside the whitelist is `none`.
    pub fn from_wire(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "complete" => Self::Complete,
            "blocked" => Self::Blocked,
            "stopped" => Self::Stopped,
            _ => Self::None,
        }
    }

    /// A retained goal can be re-entered from `blocked` or `stopped`. The
    /// controller rejects an empty goal, a completed goal, and a running goal
    /// (goal.go:344-367; matrix B §1.3.5): a live continuation must not have
    /// its epoch advanced underneath it.
    pub fn resumable(self) -> bool {
        matches!(self, Self::Blocked | Self::Stopped)
    }

    /// Only a running goal can be paused (goal_control.go:81-84).
    pub fn pausable(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// The two explicit Goal control verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalAction {
    Resume,
    Pause,
}

impl GoalAction {
    /// The vendor method name this action rides on.
    pub fn method(self) -> &'static str {
        match self {
            Self::Resume => SESSION_GOAL_RESUME,
            Self::Pause => SESSION_GOAL_PAUSE,
        }
    }
}

/// Why a Goal control verb was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GoalFailure {
    /// No retained Goal to re-enter.
    #[error("no resumable goal")]
    NotResumable,
    /// No running Goal to pause.
    #[error("no running goal")]
    NotRunning,
}

/// Applies the pure transition. Resume lands in `running`; pause leaves the
/// status to the next continuation boundary, so the reported status becomes
/// `blocked` only when the controller confirms — the pure refusal is what
/// this returns.
pub fn apply_action(status: GoalStatus, action: GoalAction) -> Result<GoalStatus, GoalFailure> {
    match action {
        GoalAction::Resume => {
            if status.resumable() {
                Ok(GoalStatus::Running)
            } else {
                Err(GoalFailure::NotResumable)
            }
        }
        GoalAction::Pause => {
            if status.pausable() {
                Ok(GoalStatus::Blocked)
            } else {
                Err(GoalFailure::NotRunning)
            }
        }
    }
}

/// Maps a refusal onto the frozen RPC error with the verbatim message
/// (session_goal.go:76, 90; extensions.rs:113-126).
pub fn goal_error(action: GoalAction, failure: GoalFailure) -> RpcError {
    match failure {
        GoalFailure::NotResumable => goal_not_resumable(action.method()),
        GoalFailure::NotRunning => goal_not_running(action.method()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    #[test]
    fn resumable_and_pausable_tables_match_reference() {
        assert!(GoalStatus::Blocked.resumable());
        assert!(GoalStatus::Stopped.resumable());
        assert!(!GoalStatus::Complete.resumable());
        assert!(!GoalStatus::Running.resumable());
        assert!(!GoalStatus::None.resumable());

        assert!(GoalStatus::Running.pausable());
        assert!(!GoalStatus::Complete.pausable());
        assert!(!GoalStatus::Blocked.pausable());
        assert!(!GoalStatus::Stopped.pausable());
        assert!(!GoalStatus::None.pausable());
    }

    #[test]
    fn resume_pause_transitions_and_refusals() {
        assert_eq!(
            apply_action(GoalStatus::Blocked, GoalAction::Resume).unwrap(),
            GoalStatus::Running
        );
        assert_eq!(
            apply_action(GoalStatus::Running, GoalAction::Pause).unwrap(),
            GoalStatus::Blocked
        );

        let not_resumable = apply_action(GoalStatus::None, GoalAction::Resume).unwrap_err();
        assert_eq!(not_resumable, GoalFailure::NotResumable);
        let not_running = apply_action(GoalStatus::Complete, GoalAction::Pause).unwrap_err();
        assert_eq!(not_running, GoalFailure::NotRunning);
    }

    #[test]
    fn refusal_errors_carry_frozen_codes_and_verbatim_messages() {
        let resume = goal_error(GoalAction::Resume, GoalFailure::NotResumable);
        assert_eq!(resume.code, RpcErrorCode::GoalNotResumable);
        assert_eq!(resume.code.code(), -32010);
        assert_eq!(
            resume.message,
            "_reasonix.io/session/goal/resume: session has no resumable goal"
        );

        let pause = goal_error(GoalAction::Pause, GoalFailure::NotRunning);
        assert_eq!(pause.code, RpcErrorCode::GoalNotRunning);
        assert_eq!(pause.code.code(), -32011);
        assert_eq!(
            pause.message,
            "_reasonix.io/session/goal/pause: session has no running goal"
        );
    }

    #[test]
    fn wire_status_round_trips() {
        for status in [
            GoalStatus::Running,
            GoalStatus::Complete,
            GoalStatus::Blocked,
            GoalStatus::None,
        ] {
            assert_eq!(GoalStatus::from_wire(status.wire()), status);
        }
        assert_eq!(GoalStatus::from_wire("weird"), GoalStatus::None);
    }
}
