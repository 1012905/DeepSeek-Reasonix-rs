//! TOOL-048 jobs completion/finalization/exit_error behavior.
//!
//! Covers the deterministic completion event id, the per-status Notice
//! switch and payload, the active-session emit gate, observer ack semantics
//! (success only), the drain queue, the completion ordering invariants, and
//! the `ErrFinalization` once wrapper with typed exit extraction. Persistent
//! observer storage and cross-turn drain fold are WIP.

use std::error::Error;
use std::sync::{Arc, Mutex};

use reasonix_contract::eventwire::notice_payload::decode_job_notice;
use reasonix_tools::builtin::jobs::completion;
use reasonix_tools::builtin::jobs::*;

struct MockObserver {
    events: Mutex<Vec<CompletionEvent>>,
    fail: Mutex<bool>,
}

impl MockObserver {
    fn new(fail: bool) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            fail: Mutex::new(fail),
        }
    }
}

impl CompletionObserver for MockObserver {
    fn on_completion(&self, event: &CompletionEvent) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.events.lock().expect("events lock").push(event.clone());
        if *self.fail.lock().expect("fail lock") {
            Err(Box::new(std::io::Error::other("observer down")))
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic event identity and note
// ---------------------------------------------------------------------------

#[test]
fn completion_event_id_is_deterministic_completion_jobid_terminal_v1() {
    assert_eq!(
        completion_event_id("task-12"),
        "completion/task-12/terminal-v1"
    );
}

#[test]
fn note_is_tag_plus_status_with_or_without_label() {
    assert_eq!(
        completion_note("bash-1", "build", Status::Done),
        "bash-1 (build) — done"
    );
    assert_eq!(
        completion_note("bash-1", "", Status::Killed),
        "bash-1 — killed"
    );
}

// ---------------------------------------------------------------------------
// Notice dispatch
// ---------------------------------------------------------------------------

#[test]
fn finished_notice_is_info_with_the_baseline_payload() {
    let input = CompletionInput {
        parent_session: "s1".to_owned(),
        id: "bash-1".to_owned(),
        kind: "bash".to_owned(),
        label: "build".to_owned(),
        status: Status::Done,
        finished_at_ms: 1_000,
        ..Default::default()
    };
    let (event, notice) = record_completion(&input);
    assert_eq!(event.id, "completion/bash-1/terminal-v1");
    assert_eq!(event.note, "bash-1 (build) — done");
    assert_eq!(notice.code, "job_finished");
    assert_eq!(notice.level, "info");
    assert_eq!(notice.text, "background bash finished: bash-1");
    let payload = decode_job_notice(&notice.payload.encode()).expect("payload");
    assert_eq!(payload.kind, "bash");
    assert_eq!(payload.id, "bash-1");
    assert_eq!(payload.label, "build");
    assert!(payload.error.is_empty());
    assert!(payload.exit_code.is_none());
}

#[test]
fn failed_notice_is_warn_and_carries_the_typed_exit() {
    let input = CompletionInput {
        id: "bash-2".to_owned(),
        kind: "bash".to_owned(),
        status: Status::Failed,
        err: Some("signal: killed".to_owned()),
        exit: Some(ExitInfo {
            code: -1,
            signal: 9,
        }),
        finished_at_ms: 2_000,
        ..Default::default()
    };
    let (_event, notice) = record_completion(&input);
    assert_eq!(notice.code, "job_failed");
    assert_eq!(notice.level, "warn");
    assert_eq!(
        notice.text,
        "background bash failed: bash-2 — signal: killed"
    );
    let payload = decode_job_notice(&notice.payload.encode()).expect("payload");
    assert_eq!(payload.error, "signal: killed");
    assert_eq!(payload.exit_code, Some(-1));
    assert_eq!(payload.signal, 9);
}

#[test]
fn failed_notice_without_a_typed_exit_omits_code_and_signal() {
    let input = CompletionInput {
        id: "task-3".to_owned(),
        kind: "task".to_owned(),
        status: Status::Failed,
        err: Some("boom".to_owned()),
        ..Default::default()
    };
    let (_event, notice) = record_completion(&input);
    let payload = decode_job_notice(&notice.payload.encode()).expect("payload");
    assert_eq!(payload.error, "boom");
    assert!(payload.exit_code.is_none());
    assert_eq!(payload.signal, 0);
}

#[test]
fn killed_notice_uses_the_killed_code_and_baseline_payload() {
    let input = CompletionInput {
        id: "bash-4".to_owned(),
        kind: "bash".to_owned(),
        status: Status::Killed,
        ..Default::default()
    };
    let (_event, notice) = record_completion(&input);
    assert_eq!(notice.code, "job_killed");
    assert_eq!(notice.level, "info");
    assert_eq!(notice.text, "background bash killed: bash-4");
}

#[test]
fn interrupted_rides_the_finished_baseline_branch() {
    let input = CompletionInput {
        id: "bash-5".to_owned(),
        kind: "bash".to_owned(),
        status: Status::Interrupted,
        ..Default::default()
    };
    let (_event, notice) = record_completion(&input);
    assert_eq!(notice.code, "job_finished");
}

// ---------------------------------------------------------------------------
// Emit gate and ack decision
// ---------------------------------------------------------------------------

#[test]
fn emit_gate_follows_active_and_session_match() {
    assert!(should_emit_public("", "s1"));
    assert!(should_emit_public("s1", ""));
    assert!(should_emit_public("s1", "s1"));
    assert!(!should_emit_public("s1", "s2"));
}

fn should_emit_public(active: &str, session: &str) -> bool {
    completion::should_emit(active, session)
}

#[test]
fn publish_keeps_the_note_without_an_observer_or_on_observer_failure() {
    let no_observer = publish(false, false, false);
    assert!(!no_observer.delivered && !no_observer.ack);
    let failed = publish(false, true, true);
    assert!(failed.delivered && !failed.ack);
    let ok = publish(false, true, false);
    assert!(ok.delivered && ok.ack);
}

fn publish(event_id_empty: bool, observer: bool, failed: bool) -> completion::AckDecision {
    completion::publish_decision(event_id_empty, observer, failed)
}

#[test]
fn empty_event_id_never_publishes_even_with_an_observer() {
    let decision = publish(true, true, false);
    assert!(!decision.delivered && !decision.ack);
}

#[test]
fn resumes_on_completion_needs_observer_and_active_session_match() {
    assert!(resumes_on_completion(true, "s1", "s1"));
    assert!(!resumes_on_completion(true, "s1", "s2"));
    assert!(!resumes_on_completion(true, "s1", ""));
    assert!(!resumes_on_completion(false, "s1", "s1"));
}

#[test]
fn ack_removes_only_the_matching_event_id() {
    let mut items = vec![
        CompletionQueueItem {
            session_id: "s1".to_owned(),
            text: "a".to_owned(),
            event_id: "completion/a/terminal-v1".to_owned(),
        },
        CompletionQueueItem {
            session_id: "s1".to_owned(),
            text: "b".to_owned(),
            event_id: String::new(),
        },
    ];
    ack_item(&mut items, "completion/a/terminal-v1");
    assert_eq!(items.len(), 1);
    assert!(items[0].event_id.is_empty());
}

// ---------------------------------------------------------------------------
// Manager completion orchestration
// ---------------------------------------------------------------------------

#[test]
fn success_observer_receives_the_event_and_acks_the_queued_note() {
    let observer = Arc::new(MockObserver::new(false));
    let manager = Arc::new(Manager::new(None));
    manager.set_completion_observer(Some(observer.clone()));
    let job = manager
        .start("", "bash", "build", Presentation::default(), 1_000)
        .unwrap();

    manager.complete_job(&job, Status::Done, None, None, String::new(), 2_000);

    assert_eq!(observer.events.lock().expect("events lock").len(), 1);
    let event = &observer.events.lock().expect("events lock")[0];
    assert_eq!(event.id, completion_event_id(&job.id));
    assert_eq!(event.status, Status::Done);
    assert!(job.is_done());
    assert!(
        manager.drain_completions("").is_empty(),
        "acked note drained"
    );
}

#[test]
fn observer_failure_keeps_the_queued_note_for_the_next_drain() {
    let observer = Arc::new(MockObserver::new(true));
    let manager = Arc::new(Manager::new(None));
    manager.set_completion_observer(Some(observer.clone()));
    let job = manager
        .start("", "bash", "", Presentation::default(), 1_000)
        .unwrap();

    manager.complete_job(
        &job,
        Status::Failed,
        Some("boom"),
        None,
        String::new(),
        2_000,
    );

    assert_eq!(observer.events.lock().expect("events lock").len(), 1);
    let drained = manager.drain_completions("");
    assert_eq!(drained.len(), 1, "observer failure keeps the note");
    assert_eq!(drained[0].event_id, completion_event_id(&job.id));
}

#[test]
fn completion_queues_before_the_status_flip_and_closes_done_before_publish() {
    let manager = Arc::new(Manager::new(None));
    let job = manager
        .start("", "bash", "", Presentation::default(), 1_000)
        .unwrap();

    manager.complete_job(&job, Status::Done, None, None, String::new(), 2_000);

    assert_eq!(job.status(), Status::Done);
    assert!(job.is_done());
    let drained = manager.drain_completions("");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].text, format!("{} — done", job.id));
}

#[test]
fn failed_completion_records_the_error_and_typed_exit_on_the_event() {
    let manager = Arc::new(Manager::new(None));
    let job = manager
        .start("", "bash", "", Presentation::default(), 1_000)
        .unwrap();
    let exit = ExitInfo { code: 2, signal: 0 };

    manager.complete_job(
        &job,
        Status::Failed,
        Some("exit status 2"),
        Some(exit),
        String::new(),
        2_000,
    );

    assert_eq!(job.status(), Status::Failed);
    assert_eq!(job.exit_code(), Some(2));
    let drained = manager.drain_completions("");
    assert_eq!(drained[0].text, format!("{} — failed", job.id));
}

#[test]
fn invalid_placeholder_completes_as_failed_with_the_validation_message() {
    let manager = Arc::new(Manager::new(None));
    let err = validate_path_segment("", "kind").expect_err("empty kind");
    let job = manager.register_invalid("", "", &err, 1_000);
    assert_eq!(job.status(), Status::Failed);
    assert!(job.is_done());
    let drained = manager.drain_completions("");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].text, format!("{} — failed", job.id));
}

// ---------------------------------------------------------------------------
// Finalization wrapper and exit extraction
// ---------------------------------------------------------------------------

#[test]
fn errfinalization_wraps_a_finalizer_error_at_one_level() {
    let inner: Box<dyn Error + Send + Sync> =
        Box::new(std::io::Error::other("lock release failed"));
    let err = FinalizationError::wrap(inner);
    assert_eq!(
        err.to_string(),
        "job resource finalization failed: lock release failed"
    );
    assert!(is_finalization(&err));
}

#[test]
fn exit_of_walks_the_wrapped_chain_for_the_typed_exit() {
    let exit_err = ExitError::new(137, 0, Box::new(std::io::Error::other("exit 137")));
    let wrapped: Box<dyn Error + Send + Sync + 'static> =
        Box::new(FinalizationError::wrap(Box::new(exit_err)));
    let info = exit_of(wrapped.as_ref()).expect("typed exit found");
    assert_eq!(info.code, 137);
    assert_eq!(info.signal, 0);
}

#[test]
fn finalizers_do_not_rerun_after_the_first_pass() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let finalizers = JobFinalizers::new();
    let calls_a = calls.clone();
    finalizers.push(Box::new(move |_: &Finalization| {
        calls_a.lock().expect("calls lock").push("once");
        Err(Box::new(std::io::Error::other("cleanup failed")))
    }));
    let state = Finalization::default();
    finalizers.finalize(&state).expect_err("failure recorded");
    finalizers
        .finalize(&state)
        .expect_err("stored error repeats");
    assert_eq!(*calls.lock().expect("calls lock"), vec!["once"]);
}
