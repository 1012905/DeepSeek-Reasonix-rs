//! ACP-023: acpSession runtime state machine transitions, refusal identities,
//! and the retired-lease tracker.

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_state::{BeginRefusal, RetiredLeases, RuntimeMachine, RuntimeState};

#[test]
fn begin_finish_round_trip_through_running() {
    let mut machine = RuntimeMachine::new();
    assert_eq!(machine.state(), RuntimeState::Idle);
    machine.begin(false).unwrap();
    assert_eq!(machine.state(), RuntimeState::Running);
    machine.finish();
    assert_eq!(machine.state(), RuntimeState::Idle);
}

#[test]
fn begin_refuses_every_non_idle_and_queued_condition() {
    let mut machine = RuntimeMachine::new();
    machine.begin(false).unwrap();
    assert_eq!(
        machine.begin(false).unwrap_err(),
        BeginRefusal::BusyActivePrompt
    );
    machine.finish();

    machine.queue_config();
    assert_eq!(
        machine.begin(false).unwrap_err(),
        BeginRefusal::QueuedConfig
    );
    machine.drain_config();

    machine.start_maintenance().unwrap();
    assert_eq!(
        machine.begin(false).unwrap_err(),
        BeginRefusal::MaintenanceInFlight
    );
    machine.finish_maintenance();

    machine.delete_and_wait();
    assert_eq!(machine.state(), RuntimeState::Deleted);
    assert_eq!(machine.begin(false).unwrap_err(), BeginRefusal::Deleted);
}

#[test]
fn maintenance_and_delete_transitions_are_exclusive() {
    let mut machine = RuntimeMachine::new();
    assert!(machine.start_maintenance().is_ok());
    assert!(machine.start_maintenance().is_err());
    machine.finish_maintenance();
    assert_eq!(machine.state(), RuntimeState::Idle);

    machine.begin(false).unwrap();
    machine.abort();
    assert_eq!(machine.state(), RuntimeState::Idle);

    machine.begin(false).unwrap();
    machine.abort_and_wait();
    assert_eq!(machine.state(), RuntimeState::Idle);
}

#[test]
fn busy_active_prompt_refusal_keeps_in_process_concurrent_code() {
    let err = RuntimeMachine::refusal_error("session/prompt", BeginRefusal::BusyActivePrompt);
    assert_eq!(err.code, RpcErrorCode::InvalidRequest);
    assert_eq!(err.code.code(), -32600);
    assert_eq!(
        err.message,
        "session/prompt: session already has an active prompt"
    );

    let maintenance =
        RuntimeMachine::refusal_error("session/prompt", BeginRefusal::MaintenanceInFlight);
    assert_eq!(maintenance.code, RpcErrorCode::InvalidRequest);
    assert!(maintenance.message.contains("maintenance in flight"));
}

#[test]
fn retired_lease_tracker_counts_deferred_releases() {
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
