//! ACP-019: goal/resume and goal/pause transitions, refusal identities, and
//! verbatim messages.

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::goal::{GoalAction, GoalFailure, GoalStatus, apply_action, goal_error};

#[test]
fn resume_accepts_blocked_and_stopped_only() {
    for resumable in [GoalStatus::Blocked, GoalStatus::Stopped] {
        assert_eq!(
            apply_action(resumable, GoalAction::Resume).unwrap(),
            GoalStatus::Running
        );
    }
    for refused in [GoalStatus::Running, GoalStatus::Complete, GoalStatus::None] {
        assert_eq!(
            apply_action(refused, GoalAction::Resume).unwrap_err(),
            GoalFailure::NotResumable
        );
    }
}

#[test]
fn pause_accepts_running_only() {
    assert_eq!(
        apply_action(GoalStatus::Running, GoalAction::Pause).unwrap(),
        GoalStatus::Blocked
    );
    for refused in [
        GoalStatus::Complete,
        GoalStatus::Blocked,
        GoalStatus::Stopped,
        GoalStatus::None,
    ] {
        assert_eq!(
            apply_action(refused, GoalAction::Pause).unwrap_err(),
            GoalFailure::NotRunning
        );
    }
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
fn goal_status_wire_vocabulary_round_trips() {
    for status in [
        GoalStatus::Running,
        GoalStatus::Complete,
        GoalStatus::Blocked,
        GoalStatus::Stopped,
        GoalStatus::None,
    ] {
        assert_eq!(GoalStatus::from_wire(status.wire()), status);
    }
    assert_eq!(GoalStatus::from_wire("done"), GoalStatus::None);
}
