//! TOOL-046 start-time validation, finish state classification and the
//! lifecycle text a running job surfaces (Go `internal/tools/jobs/jobs.go`
//! §316-332, `run.go` §42-52, `jobs.go` §597-666, §856-894).
//!
//! Done here: the path-segment and trusted-session-path validation decision
//! table with the verbatim E1–E6 messages, the finish status classifier, the
//! preserve-published-status guard, stall/started notice wording and the
//! reservation capacity decision. The goroutine launch, wait-group accounting,
//! stall timer and real cancellation propagation are host runtime WIP.

#![forbid(unsafe_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use super::Status;

/// Validation and lifecycle rejection, typed so the wrapper layer can name
/// the branch instead of parsing prose. Messages mirror the Go errors verbatim
/// (`jobs.go:316-332, 966-976`).
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum JobsValidationError {
    /// E1: the `kind` field is empty (the job id is built from it).
    #[error("jobs: {field} must not be empty")]
    Empty { field: String },
    /// E2: a field carries a control character (NUL, newline, TAB, DEL, …).
    #[error("jobs: {field} contains control character 0x{value:02x} at index {index}")]
    Control {
        field: String,
        value: u32,
        index: usize,
    },
    /// E3: a field carries a path separator the containment join would honor.
    #[error("jobs: {field} contains path separator {sep:?} at index {index}")]
    Separator {
        field: String,
        sep: char,
        index: usize,
    },
    /// E4: the field is a reserved segment name (`.` / `..`).
    #[error("jobs: {field} is reserved ({name:?})")]
    Reserved { field: String, name: String },
    /// E5: a trusted session path was supplied empty.
    #[error("jobs: sessionPath must not be empty")]
    SessionPathEmpty,
    /// E6: the trusted session path carries a control character.
    #[error("jobs: sessionPath contains control character 0x{value:02x} at index {index}")]
    SessionPathControl { value: u32, index: usize },
}

fn is_control(r: char) -> bool {
    (r as u32) < 0x20 || r == '\u{7f}'
}

/// `validatePathSegment`: reject anything a temp-root join could read as a
/// traversal or a split (`jobs.go:316-332`). An empty `parentSession` is
/// allowed (unscoped); an empty `kind` is not.
pub fn validate_path_segment(name: &str, field: &str) -> Result<(), JobsValidationError> {
    if field == "kind" && name.is_empty() {
        return Err(JobsValidationError::Empty {
            field: field.to_owned(),
        });
    }
    for (index, r) in name.char_indices() {
        if is_control(r) {
            return Err(JobsValidationError::Control {
                field: field.to_owned(),
                value: r as u32,
                index,
            });
        }
        if r == '/' || r == '\\' {
            return Err(JobsValidationError::Separator {
                field: field.to_owned(),
                sep: r,
                index,
            });
        }
    }
    if name == "." || name == ".." {
        return Err(JobsValidationError::Reserved {
            field: field.to_owned(),
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// `validateTrustedSessionPath`: a session transcript path is trusted syntax,
/// not a containment boundary, so separators and `..` are deliberately kept;
/// only empty and control characters are rejected (`jobs.go:966-976`).
pub fn validate_trusted_session_path(path: &str) -> Result<(), JobsValidationError> {
    if path.is_empty() {
        return Err(JobsValidationError::SessionPathEmpty);
    }
    for (index, r) in path.char_indices() {
        if is_control(r) {
            return Err(JobsValidationError::SessionPathControl {
                value: r as u32,
                index,
            });
        }
    }
    Ok(())
}

/// `cancelledStatus`: a cancel published by the manager root is an
/// `Interrupted`; a cancel of the job alone is `Killed` (`status.go:25-30`).
pub fn cancelled_status(root_cancelled: bool) -> Status {
    if root_cancelled {
        Status::Interrupted
    } else {
        Status::Killed
    }
}

/// Finish classifier mirroring the run goroutine's status decision
/// (`run.go:42-52`): a cancelled context wins over the run error; an error
/// makes `Failed`; a clean return is `Done`.
pub fn classify_finish(ctx_cancelled: bool, root_cancelled: bool, failed: bool) -> Status {
    if ctx_cancelled {
        cancelled_status(root_cancelled)
    } else if failed {
        Status::Failed
    } else {
        Status::Done
    }
}

/// A status published on the cancel path stands: the run goroutine does not
/// flip a `Killed`/`Interrupted` job back (`run.go:94`).
pub fn preserve_published_status(status: Status) -> bool {
    matches!(status, Status::Killed | Status::Interrupted)
}

/// `tag`: label-qualified job identity for drain notes and notices.
pub fn completion_tag(id: &str, label: &str) -> String {
    if label.is_empty() {
        id.to_owned()
    } else {
        format!("{id} ({label})")
    }
}

/// `started` notice text: label-qualified when one was supplied.
pub fn started_notice_text(kind: &str, id: &str, label: &str) -> String {
    if label.is_empty() {
        format!("background {kind} started: {id}")
    } else {
        format!("background {kind} started: {id} ({label})")
    }
}

/// Stalled work sentence: whether any step was reported before it went quiet
/// (`jobs.go:633-666`).
pub fn stall_work_text(steps: i64) -> String {
    if steps == 0 {
        "no progress has been reported, so it may be writing elsewhere rather than stuck".to_owned()
    } else {
        format!(
            "{steps} step(s) were reported before it went quiet, so it may be waiting on something"
        )
    }
}

/// The stalled drain single-line note queued for the next turn.
pub fn stalled_drain_text(tag: &str, quiet: &str, work: &str) -> String {
    format!(
        "{tag} has been inactive for {quiet} — {work}. It may be waiting on something, or it may be stuck. Check with wait or bash_output, or stop it with kill_shell."
    )
}

/// The stalled Notice text (`NoticeCodeJobStalled`, LevelWarn).
pub fn stalled_notice_text(kind: &str, id: &str, quiet: &str, work: &str) -> String {
    format!(
        "background {kind} {id} has been inactive for {quiet} — {work}; check with wait/bash_output or stop with kill_shell"
    )
}

/// How long the stall threshold renders: rounded to the whole second, the
/// way Go `time.Duration.Round(time.Second)` reads it back.
pub fn quiet_seconds(warning_ms: i64) -> i64 {
    let secs = (warning_ms.max(0) + 500) / 1000;
    secs.max(1)
}

/// Reservation capacity decision: running jobs and in-flight reservations
/// both count against the limit; `limit <= 0` imposes no bound
/// (`jobs.go:856-894`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReserveDecision {
    /// Running jobs plus already-held reservations.
    pub used: u32,
    pub allowed: bool,
}

pub fn reserve_decision(running: u32, reserved: u32, limit: i64) -> ReserveDecision {
    if limit <= 0 {
        return ReserveDecision {
            used: running,
            allowed: true,
        };
    }
    let used = running + reserved;
    ReserveDecision {
        used,
        allowed: used < limit as u32,
    }
}

/// Current unix milliseconds. Real-clock seam; tests pass explicit
/// timestamps into the pure decision functions instead.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_kind_is_rejected_with_the_e1_message() {
        let err = validate_path_segment("", "kind").expect_err("empty kind");
        assert_eq!(err.to_string(), "jobs: kind must not be empty");
    }

    #[test]
    fn empty_parent_session_is_allowed_as_unscoped() {
        validate_path_segment("", "parentSession").expect("unscoped parent is valid");
    }

    #[test]
    fn control_characters_are_rejected_with_value_and_index() {
        let err = validate_path_segment("a\nb", "kind").expect_err("newline");
        assert_eq!(
            err.to_string(),
            "jobs: kind contains control character 0x0a at index 1"
        );
        let err = validate_path_segment("x\u{7f}y", "parentSession").expect_err("DEL");
        assert_eq!(
            err.to_string(),
            "jobs: parentSession contains control character 0x7f at index 1"
        );
        let err = validate_path_segment("\0ab", "kind").expect_err("NUL");
        assert_eq!(
            err.to_string(),
            "jobs: kind contains control character 0x00 at index 0"
        );
    }

    #[test]
    fn separators_are_rejected_with_the_literal_quote() {
        let err = validate_path_segment("a/b", "kind").expect_err("slash");
        assert_eq!(
            err.to_string(),
            "jobs: kind contains path separator '/' at index 1"
        );
        let err = validate_path_segment("a\\b", "kind").expect_err("backslash");
        assert_eq!(
            err.to_string(),
            "jobs: kind contains path separator '\\\\' at index 1"
        );
    }

    #[test]
    fn reserved_segments_are_rejected() {
        for name in [".", ".."] {
            let err = validate_path_segment(name, "kind").expect_err("reserved");
            assert_eq!(
                err.to_string(),
                format!("jobs: kind is reserved ({name:?})")
            );
        }
    }

    #[test]
    fn trusted_session_path_keeps_separators_and_dotdot() {
        validate_trusted_session_path("sessions/../other").expect("separators kept");
        let err = validate_trusted_session_path("").expect_err("empty");
        assert_eq!(err.to_string(), "jobs: sessionPath must not be empty");
        let err = validate_trusted_session_path("a\tb").expect_err("control");
        assert_eq!(
            err.to_string(),
            "jobs: sessionPath contains control character 0x09 at index 1"
        );
    }

    #[test]
    fn finish_classifier_prefers_cancellation_over_failure() {
        assert_eq!(classify_finish(true, false, true), Status::Killed);
        assert_eq!(classify_finish(true, true, true), Status::Interrupted);
        assert_eq!(classify_finish(false, true, true), Status::Failed);
        assert_eq!(classify_finish(false, false, false), Status::Done);
    }

    #[test]
    fn cancel_published_statuses_survive_the_goroutine_writeback() {
        assert!(preserve_published_status(Status::Killed));
        assert!(preserve_published_status(Status::Interrupted));
        assert!(!preserve_published_status(Status::Failed));
        assert!(!preserve_published_status(Status::Done));
        assert!(!preserve_published_status(Status::Running));
    }

    #[test]
    fn reservation_counts_running_and_held_slots_together() {
        let unlimited = reserve_decision(5, 2, 0);
        assert!(unlimited.allowed);
        assert_eq!(unlimited.used, 5);

        let under = reserve_decision(3, 1, 5);
        assert!(under.allowed);
        assert_eq!(under.used, 4);

        let at_limit = reserve_decision(4, 1, 5);
        assert!(!at_limit.allowed, "4+1 reaches the limit of 5");
        let over = reserve_decision(5, 0, 5);
        assert!(!over.allowed);
    }

    #[test]
    fn stall_text_names_steps_or_the_no_step_alternative() {
        assert_eq!(
            stall_work_text(0),
            "no progress has been reported, so it may be writing elsewhere rather than stuck"
        );
        assert_eq!(
            stall_work_text(3),
            "3 step(s) were reported before it went quiet, so it may be waiting on something"
        );
        assert_eq!(
            stalled_drain_text("bash-1 (build)", "15s", &stall_work_text(0)),
            "bash-1 (build) has been inactive for 15s — no progress has been reported, so it may be writing elsewhere rather than stuck. It may be waiting on something, or it may be stuck. Check with wait or bash_output, or stop it with kill_shell."
        );
        assert_eq!(
            stalled_notice_text("bash", "bash-1", "15s", &stall_work_text(2)),
            "background bash bash-1 has been inactive for 15s — 2 step(s) were reported before it went quiet, so it may be waiting on something; check with wait/bash_output or stop with kill_shell"
        );
    }

    #[test]
    fn started_notice_is_label_qualified_when_present() {
        assert_eq!(
            started_notice_text("bash", "bash-1", ""),
            "background bash started: bash-1"
        );
        assert_eq!(
            started_notice_text("bash", "bash-1", "build"),
            "background bash started: bash-1 (build)"
        );
    }

    #[test]
    fn quiet_seconds_rounds_up_and_never_reports_zero() {
        assert_eq!(quiet_seconds(15_000), 15);
        assert_eq!(quiet_seconds(500), 1);
        assert_eq!(quiet_seconds(1_400), 1);
        assert_eq!(quiet_seconds(1_600), 2);
    }
}
