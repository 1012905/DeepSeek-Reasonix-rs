//! TOOL-046 jobs Manager/Job lifecycle, validation and recovery gates.
//!
//! Covers the pure decision tables this slice locks: E1–E6 validation prose,
//! status/cause/process-class mapping, finish classification, reservation
//! capacity, the invalid-job placeholder, kill flip, evidence
//! lease/pending/commit gates and the finalizer once semantics. Real process
//! spawn, artifact files and the stall timer are WIP and exercised only where
//! the in-memory table makes the decision observable.

use std::sync::{Arc, Mutex};

use reasonix_tools::builtin::jobs::*;

struct MockRecorder {
    starts: Mutex<Vec<String>>,
    done: Mutex<Vec<(String, Status, Option<String>)>>,
}

impl TaskRecorder for MockRecorder {
    fn record_start(&self, id: &str, _kind: &str, _label: &str) {
        self.starts.lock().expect("starts lock").push(id.to_owned());
    }

    fn record_done(&self, id: &str, status: Status, err: Option<&str>) {
        self.done
            .lock()
            .expect("done lock")
            .push((id.to_owned(), status, err.map(str::to_owned)));
    }
}

// ---------------------------------------------------------------------------
// E1–E6 validation
// ---------------------------------------------------------------------------

#[test]
fn e1_empty_kind_is_rejected_with_the_verbatim_message() {
    let err = validate_path_segment("", "kind").expect_err("empty kind");
    assert_eq!(err.to_string(), "jobs: kind must not be empty");
}

#[test]
fn unscoped_parent_session_is_allowed_empty() {
    validate_path_segment("", "parentSession").expect("unscoped parent valid");
}

#[test]
fn e2_control_characters_carry_value_and_index() {
    let err = validate_path_segment("\0ab", "kind").expect_err("NUL");
    assert_eq!(
        err.to_string(),
        "jobs: kind contains control character 0x00 at index 0"
    );
    let err = validate_path_segment("a\nb", "kind").expect_err("LF");
    assert_eq!(
        err.to_string(),
        "jobs: kind contains control character 0x0a at index 1"
    );
    let err = validate_path_segment("x\ty", "parentSession").expect_err("TAB");
    assert_eq!(
        err.to_string(),
        "jobs: parentSession contains control character 0x09 at index 1"
    );
    let err = validate_path_segment("x\u{7f}", "kind").expect_err("DEL");
    assert_eq!(
        err.to_string(),
        "jobs: kind contains control character 0x7f at index 1"
    );
}

#[test]
fn e3_path_separators_are_rejected_quoted() {
    let err = validate_path_segment("a/b", "kind").expect_err("slash");
    assert_eq!(
        err.to_string(),
        "jobs: kind contains path separator '/' at index 1"
    );
    let err = validate_path_segment("a\\b", "parentSession").expect_err("backslash");
    assert_eq!(
        err.to_string(),
        "jobs: parentSession contains path separator '\\\\' at index 1"
    );
}

#[test]
fn e4_reserved_segments_are_rejected() {
    for name in [".", ".."] {
        let err = validate_path_segment(name, "kind").expect_err("reserved");
        assert_eq!(
            err.to_string(),
            format!("jobs: kind is reserved ({name:?})")
        );
    }
}

#[test]
fn e5_trusted_session_path_rejects_empty() {
    let err = validate_trusted_session_path("").expect_err("empty");
    assert_eq!(err.to_string(), "jobs: sessionPath must not be empty");
}

#[test]
fn e6_trusted_session_path_rejects_control_but_keeps_separators_and_dotdot() {
    let err = validate_trusted_session_path("a\tb").expect_err("TAB");
    assert_eq!(
        err.to_string(),
        "jobs: sessionPath contains control character 0x09 at index 1"
    );
    validate_trusted_session_path("sessions/../other").expect("separators kept");
}

// ---------------------------------------------------------------------------
// Status vocabulary
// ---------------------------------------------------------------------------

#[test]
fn status_roundtrips_through_its_text() {
    for status in [
        Status::Running,
        Status::Done,
        Status::Failed,
        Status::Killed,
        Status::Interrupted,
    ] {
        assert_eq!(Status::from_str(status.as_str()), Some(status));
    }
    assert!(Status::from_str("nope").is_none());
}

#[test]
fn interrupted_carries_the_session_ended_cause_only() {
    assert_eq!(
        Status::Interrupted.cause(),
        " — the session that ran it ended before it finished (a restart or close), not a kill; its output up to then is kept"
    );
    for status in [
        Status::Running,
        Status::Done,
        Status::Failed,
        Status::Killed,
    ] {
        assert_eq!(status.cause(), "");
    }
}

#[test]
fn process_class_projection_maps_each_status() {
    assert_eq!(Status::Running.process_class(), ("running", true));
    assert_eq!(Status::Done.process_class(), ("completed", true));
    assert_eq!(Status::Failed.process_class(), ("failed", true));
    assert_eq!(Status::Killed.process_class(), ("stopped", true));
    assert_eq!(Status::Interrupted.process_class(), ("interrupted", true));
}

#[test]
fn finish_classification_prefers_cancellation_then_failure_then_done() {
    assert_eq!(classify_finish(true, false, true), Status::Killed);
    assert_eq!(classify_finish(true, true, true), Status::Interrupted);
    assert_eq!(classify_finish(false, true, true), Status::Failed);
    assert_eq!(classify_finish(false, false, false), Status::Done);
    assert_eq!(cancelled_status(false), Status::Killed);
    assert_eq!(cancelled_status(true), Status::Interrupted);
}

#[test]
fn cancel_published_statuses_survive_the_goroutine_writeback() {
    assert!(preserve_published_status(Status::Killed));
    assert!(preserve_published_status(Status::Interrupted));
    assert!(!preserve_published_status(Status::Failed));
    assert!(!preserve_published_status(Status::Done));
}

// ---------------------------------------------------------------------------
// Tail ring boundaries
// ---------------------------------------------------------------------------

#[test]
fn tail_ring_keeps_the_final_limit_bytes_with_one_byte_boundary() {
    let mut buf = Vec::new();
    append_tail(&mut buf, b"abcdef", 4);
    assert_eq!(buf, b"cdef");
    append_tail(&mut buf, b"g", 4);
    assert_eq!(buf, b"defg");
    append_tail(&mut buf, b"", 4);
    assert_eq!(buf, b"defg");
    append_tail(&mut buf, b"hi", 4);
    assert_eq!(buf, b"fghi");
}

#[test]
fn tail_ring_clears_on_zero_limit() {
    let mut buf = vec![1, 2, 3];
    append_tail(&mut buf, &[4, 5], 0);
    assert!(buf.is_empty());
}

// ---------------------------------------------------------------------------
// Reservation capacity
// ---------------------------------------------------------------------------

#[test]
fn reservation_counts_running_and_held_slots_together_at_the_limit() {
    assert!(reserve_decision(5, 2, 0).allowed);
    let under = reserve_decision(3, 1, 5);
    assert!(under.allowed);
    assert_eq!(under.used, 4);
    let at = reserve_decision(4, 1, 5);
    assert!(!at.allowed, "4 running + 1 reservation reaches limit 5");
    assert!(!reserve_decision(5, 0, 5).allowed);
}

#[test]
fn reservation_guard_releases_its_slot_on_drop() {
    let manager = Arc::new(Manager::new(None));
    let (_guard, used, allowed) = manager.reserve_start("s1", "bash", 5);
    assert!(allowed);
    assert_eq!(used, 1);
    drop(_guard);
    let (_guard2, used2, _) = manager.reserve_start("s1", "bash", 5);
    assert_eq!(used2, 1, "the reservation slot was held again");
    drop(_guard2);
    let (_guard3, used3, _) = manager.reserve_start("s1", "bash", 5);
    assert_eq!(used3, 1);
}

// ---------------------------------------------------------------------------
// Manager table, kill and invalid placeholder
// ---------------------------------------------------------------------------

#[test]
fn start_registers_a_running_job_with_recorder_routing() {
    let manager = Arc::new(Manager::new(None));
    let recorder = Arc::new(MockRecorder {
        starts: Mutex::new(Vec::new()),
        done: Mutex::new(Vec::new()),
    });
    manager.set_task_recorder(Some(recorder.clone()));
    let job = manager
        .start("s1", "bash", "build", Presentation::default(), 1_000)
        .expect("valid start");
    assert_eq!(job.id, "bash-1");
    assert_eq!(job.kind, "bash");
    assert_eq!(job.label, "build");
    assert_eq!(job.session_id, "s1");
    assert_eq!(job.status(), Status::Running);
    assert!(!job.is_done());
    assert_eq!(recorder.starts.lock().expect("starts lock").len(), 1);
}

#[test]
fn invalid_kind_registers_a_visible_failed_placeholder_with_the_error_text() {
    let manager = Arc::new(Manager::new(None));
    let err = validate_path_segment("", "kind").expect_err("empty kind");
    let job = manager.register_invalid("s1", "", &err, 1_000);
    assert!(job.id.starts_with("invalid-"));
    assert_eq!(job.status(), Status::Failed);
    assert!(job.is_done());
    let (text, status) = job.take_output();
    assert_eq!(status, Status::Failed);
    assert!(text.contains("job artifact incomplete: jobs: kind must not be empty"));
}

#[test]
fn kill_flips_only_a_running_job_and_only_once() {
    let manager = Arc::new(Manager::new(None));
    let job = manager
        .start("s1", "bash", "", Presentation::default(), 1_000)
        .unwrap();
    assert!(manager.kill_for_session("s1", &job.id));
    assert_eq!(job.status(), Status::Killed);
    assert!(
        !manager.kill_for_session("s1", &job.id),
        "terminal job stays"
    );
}

#[test]
fn jobs_are_scoped_by_session_with_unscoped_lookup_fallback() {
    let manager = Arc::new(Manager::new(None));
    let a = manager
        .start("s1", "bash", "", Presentation::default(), 1)
        .unwrap();
    let b = manager
        .start("s2", "bash", "", Presentation::default(), 2)
        .unwrap();
    assert!(manager.job("s1", &a.id).is_some());
    assert!(manager.job("s2", &a.id).is_none());
    assert!(manager.job("", &b.id).is_some());
    assert!(manager.has_unfinished_for_session("s1"));
    assert!(manager.has_unfinished_for_session("s2"));
}

#[test]
fn job_key_joins_trimmed_parts_with_nul() {
    assert_eq!(job_key(" s1 ", " j1 "), "s1\x00j1");
    assert!(session_matches("", "s1"));
    assert!(session_matches("s1", ""));
    assert!(session_matches("s1", "s1"));
    assert!(!session_matches("s1", "s2"));
}

// ---------------------------------------------------------------------------
// Evidence lease / pending / commit gates
// ---------------------------------------------------------------------------

#[test]
fn evidence_lease_gates_on_job_presence_done_and_committed() {
    let evidence = ChildEvidenceSummary::default();
    assert!(!try_lease(false, false, false, Some(&evidence)).ready);
    assert!(!try_lease(true, true, false, Some(&evidence)).ready);
    let committed = try_lease(true, false, true, Some(&evidence));
    assert!(committed.ready);
    assert!(committed.already_committed);
    let ready = try_lease(true, false, false, Some(&evidence));
    assert!(ready.ready);
    assert!(!ready.already_committed);
    assert!(ready.summary.is_some());
}

#[test]
fn pending_evidence_requires_terminal_uncommitted_receipts() {
    assert!(pending_evidence(true, false, true));
    assert!(!pending_evidence(false, false, true));
    assert!(!pending_evidence(true, true, true));
    assert!(!pending_evidence(true, false, false));
}

#[test]
fn evidence_commit_is_a_one_way_gate() {
    assert!(!commit_decision(false, false, false).committed);
    assert!(!commit_decision(true, true, false).committed);
    assert!(!commit_decision(true, false, true).committed);
    assert!(commit_decision(true, false, false).committed);
}

#[test]
fn recovered_receipt_keeps_the_recovery_tool_and_proven_evidence() {
    let receipt = recovered_task_mutation_receipt(vec!["/x".to_string()]);
    assert_eq!(receipt.tool_name, RECOVERED_BACKGROUND_TOOL_NAME);
    assert!(receipt.success && receipt.write && receipt.mutation);
    assert_eq!(receipt.mutation_evidence, "proven");
}

// ---------------------------------------------------------------------------
// Finalization once semantics and typed exit
// ---------------------------------------------------------------------------

#[test]
fn finalizers_run_once_and_join_errors_in_order() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let finalizers = JobFinalizers::new();
    let calls_a = calls.clone();
    finalizers.push(Box::new(move |_: &Finalization| {
        calls_a.lock().expect("calls lock").push("a");
        Err(Box::new(std::io::Error::other("first cleanup failed")))
    }));
    let calls_b = calls.clone();
    finalizers.push(Box::new(move |_: &Finalization| {
        calls_b.lock().expect("calls lock").push("b");
        Err(Box::new(std::io::Error::other("second cleanup failed")))
    }));

    let state = Finalization::default();
    let first = finalizers.finalize(&state).expect_err("two failures");
    assert!(is_finalization(&first));
    assert!(first.to_string().contains("first cleanup failed"));
    assert!(first.to_string().contains("second cleanup failed"));

    let second = finalizers.finalize(&state).expect_err("stored error");
    assert!(is_finalization(&second));
    assert_eq!(*calls.lock().expect("calls lock"), vec!["a", "b"]);
}

#[test]
fn clean_finalize_stays_ok_on_rerun() {
    let finalizers = JobFinalizers::new();
    finalizers.push(Box::new(|_: &Finalization| Ok(())));
    finalizers
        .finalize(&Finalization::default())
        .expect("clean");
    finalizers
        .finalize(&Finalization::default())
        .expect("rerun does not fail");
}

#[test]
fn exit_error_extracts_typed_exit_from_a_wrapped_chain() {
    let inner: Box<dyn std::error::Error + Send + Sync> =
        Box::new(std::io::Error::other("signal: killed"));
    let ex = ExitError::new(-1, 9, inner);
    let wrapped: Box<dyn std::error::Error + Send + Sync + 'static> =
        Box::new(FinalizationError::wrap(Box::new(ex)));
    let info = exit_of(wrapped.as_ref()).expect("exit in chain");
    assert_eq!(info.code, -1);
    assert_eq!(info.signal, 9);
    assert!(exit_of(Box::new(std::io::Error::other("boom")).as_ref()).is_none());
}

#[test]
fn finalizer_panic_wraps_at_the_errfinalization_level() {
    let err = FinalizationError::from_panic("boom");
    assert_eq!(
        err.to_string(),
        "job resource finalization failed: panic: boom"
    );
    assert!(is_finalization(&err));
    assert_eq!(
        panic_cause(INTERNAL_PANIC_PREFIX, "boom"),
        "internal error: panic: boom"
    );
}

#[test]
fn persisted_artifact_needs_both_completion_and_path() {
    assert!(!persisted_artifact_complete(false, "/x/job.log"));
    assert!(!persisted_artifact_complete(true, ""));
    assert!(persisted_artifact_complete(true, "/x/job.log"));
}
