//! TOOL-048 completion events, the observer contract and Notice dispatch
//! (Go `internal/tools/jobs/completion.go`).
//!
//! Done here: the deterministic event ID, the event and legacy drain-note
//! shapes, the per-status Notice code/level/text/payload decision, the
//! active-session emit gate, and the ack decision (observer success only).
//! Observer persistence and the drain fold across turns are host WIP.

#![forbid(unsafe_code)]

use std::error::Error;
use std::sync::Arc;

use reasonix_contract::eventwire::notice::code::{JOB_FAILED, JOB_FINISHED, JOB_KILLED};
use reasonix_contract::eventwire::notice::{LEVEL_INFO, LEVEL_WARN};
use reasonix_contract::eventwire::notice_payload::JobNotice;

use super::Status;
use super::finalization::ExitInfo;
use super::lifecycle::completion_tag;

/// `completionEventVersion` (`completion.go:16`).
pub const COMPLETION_EVENT_VERSION: &str = "terminal-v1";

/// One terminal job record handed to the completion observer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEvent {
    /// Deterministic id `completion/<job-id>/terminal-v1`.
    pub id: String,
    pub session_id: String,
    pub job_id: String,
    pub kind: String,
    pub label: String,
    pub status: Status,
    /// Log path when the artifact survived; empty otherwise.
    pub result_ref: String,
    /// Legacy drain single-line note.
    pub note: String,
    pub finished_at_ms: i64,
}

/// `CompletionEventID`: the deterministic event identity
/// (`completion.go:41-43`).
pub fn completion_event_id(job_id: &str) -> String {
    format!("completion/{job_id}/{COMPLETION_EVENT_VERSION}")
}

/// The completion observer installed after construction; nil receiver safe.
/// A nil return only means the event reached persistent storage, not that the
/// model consumed it.
pub trait CompletionObserver: Send + Sync {
    fn on_completion(&self, event: &CompletionEvent) -> Result<(), Box<dyn Error + Send + Sync>>;
}

/// A queued legacy single-line summary (`completed` queue item). Stalled
/// warnings carry an empty event id: nothing terminal happened, so nothing
/// can acknowledge them away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionQueueItem {
    pub session_id: String,
    pub text: String,
    pub event_id: String,
}

/// The coded Notice assembled for one terminal transition.
#[derive(Debug, Clone, PartialEq)]
pub struct NoticeDescriptor {
    pub code: String,
    pub level: String,
    pub text: String,
    pub payload: JobNotice,
}

/// Inputs to one `recordCompletion` call.
#[derive(Debug, Clone, Default)]
pub struct CompletionInput {
    pub parent_session: String,
    pub id: String,
    pub kind: String,
    pub label: String,
    pub result_ref: String,
    pub status: Status,
    /// Run error text; `None` on a clean finish.
    pub err: Option<String>,
    /// Typed exit extracted from the run error, when present.
    pub exit: Option<ExitInfo>,
    pub finished_at_ms: i64,
}

/// `Note`: `<id> (<label>) — <status>` (no label: `<id> — <status>`).
pub fn completion_note(id: &str, label: &str, status: Status) -> String {
    format!("{} — {}", completion_tag(id, label), status.as_str())
}

/// Whether a Notice may leave the manager: active session, unscoped job, or
/// no active session at all (`completion.go:165-166`).
pub fn should_emit(active: &str, session: &str) -> bool {
    active.is_empty() || session.is_empty() || active == session
}

/// Builds the event and its Notice descriptor the way `recordCompletion`
/// does before the status flip (`completion.go:143-196`).
pub fn record_completion(input: &CompletionInput) -> (CompletionEvent, NoticeDescriptor) {
    let note = completion_note(&input.id, &input.label, input.status);
    let event = CompletionEvent {
        id: completion_event_id(&input.id),
        session_id: input.parent_session.trim().to_owned(),
        job_id: input.id.clone(),
        kind: input.kind.clone(),
        label: input.label.clone(),
        status: input.status,
        result_ref: input.result_ref.clone(),
        note,
        finished_at_ms: input.finished_at_ms,
    };

    let descriptor = notice_for(input);
    (event, descriptor)
}

/// The per-status Notice switch. `Interrupted` rides the finished baseline;
/// only Failed and Killed get their own codes (`completion.go:173-191`).
fn notice_for(input: &CompletionInput) -> NoticeDescriptor {
    let mut payload = JobNotice {
        kind: input.kind.clone(),
        id: input.id.clone(),
        label: input.label.clone(),
        ..JobNotice::default()
    };

    match input.status {
        Status::Failed => {
            let err_text = input.err.clone().unwrap_or_default();
            payload.error = err_text.clone();
            if let Some(exit) = input.exit {
                payload.exit_code = Some(exit.code as i64);
                payload.signal = exit.signal as i64;
            }
            NoticeDescriptor {
                code: JOB_FAILED.to_owned(),
                level: LEVEL_WARN.to_owned(),
                text: format!(
                    "background {} failed: {} — {err_text}",
                    input.kind, input.id
                ),
                payload,
            }
        }
        Status::Killed => NoticeDescriptor {
            code: JOB_KILLED.to_owned(),
            level: LEVEL_INFO.to_owned(),
            text: format!("background {} killed: {}", input.kind, input.id),
            payload,
        },
        Status::Done | Status::Interrupted | Status::Running => NoticeDescriptor {
            code: JOB_FINISHED.to_owned(),
            level: LEVEL_INFO.to_owned(),
            text: format!("background {} finished: {}", input.kind, input.id),
            payload,
        },
    }
}

/// Outcome of `publishCompletion`: whether the observer was reached and
/// whether the queued note may be acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AckDecision {
    /// An observer existed and was called.
    pub delivered: bool,
    /// The queued note may be removed; false keeps it for the next drain.
    pub ack: bool,
}

/// `publishCompletion` decision (`completion.go:97-111`): an empty event id
/// returns straight away; a nil observer keeps the note; an observer error
/// keeps the note too.
pub fn publish_decision(
    event_id_empty: bool,
    observer_present: bool,
    observer_failed: bool,
) -> AckDecision {
    if event_id_empty || !observer_present {
        return AckDecision {
            delivered: false,
            ack: false,
        };
    }
    AckDecision {
        delivered: true,
        ack: !observer_failed,
    }
}

/// `ResumesOnCompletion`: an observer is installed and the session matches
/// the active one (`completion.go:66-74`).
pub fn resumes_on_completion(observer_present: bool, active: &str, session: &str) -> bool {
    observer_present && !session.trim().is_empty() && active == session.trim()
}

/// Erase a queued item by its event id; an already-drained id is not an
/// error (`ackCompletion`).
pub fn ack_item(items: &mut Vec<CompletionQueueItem>, event_id: &str) {
    items.retain(|item| item.event_id != event_id);
}

/// Holds the installed observer for the manager's publish path.
#[derive(Default)]
pub struct ObserverSlot {
    observer: Option<Arc<dyn CompletionObserver>>,
}

impl ObserverSlot {
    pub fn set(&mut self, observer: Option<Arc<dyn CompletionObserver>>) {
        self.observer = observer;
    }

    pub fn get(&self) -> Option<Arc<dyn CompletionObserver>> {
        self.observer.clone()
    }

    pub fn present(&self) -> bool {
        self.observer.is_some()
    }

    /// `ObservedBy`: whether this observer instance is the installed one.
    pub fn observed_by(&self, other: &Arc<dyn CompletionObserver>) -> bool {
        self.observer
            .as_ref()
            .is_some_and(|installed| Arc::ptr_eq(installed, other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::eventwire::notice_payload::decode_job_notice;

    #[test]
    fn event_id_is_deterministic_completion_jobid_terminal_v1() {
        assert_eq!(
            completion_event_id("task-12"),
            "completion/task-12/terminal-v1"
        );
    }

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
        assert_eq!(notice.code, JOB_FINISHED);
        assert_eq!(notice.level, LEVEL_INFO);
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
            parent_session: String::new(),
            id: "bash-2".to_owned(),
            kind: "bash".to_owned(),
            label: String::new(),
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
        assert_eq!(notice.code, JOB_FAILED);
        assert_eq!(notice.level, LEVEL_WARN);
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
        assert_eq!(notice.code, JOB_KILLED);
        assert_eq!(notice.level, LEVEL_INFO);
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
        assert_eq!(notice.code, JOB_FINISHED);
    }

    #[test]
    fn emit_gate_follows_active_and_session_match() {
        assert!(should_emit("", "s1"));
        assert!(should_emit("s1", ""));
        assert!(should_emit("s1", "s1"));
        assert!(!should_emit("s1", "s2"));
    }

    #[test]
    fn publish_keeps_the_note_without_an_observer_or_on_failure() {
        assert_eq!(
            publish_decision(false, false, false),
            AckDecision {
                delivered: false,
                ack: false
            }
        );
        assert_eq!(
            publish_decision(false, true, true),
            AckDecision {
                delivered: true,
                ack: false
            }
        );
        assert_eq!(
            publish_decision(false, true, false),
            AckDecision {
                delivered: true,
                ack: true
            }
        );
    }

    #[test]
    fn empty_event_id_never_publishes() {
        let decision = publish_decision(true, true, false);
        assert!(!decision.delivered);
        assert!(!decision.ack);
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
}
