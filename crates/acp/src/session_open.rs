//! ACP-025 `session/load` + `session/resume` open decisions and replay flags
//! (`session.go:297-347`; `service.go:486-659`).
//!
//! The two handlers share one `openExistingSession`; their only difference on
//! the decision surface is the `replay` boolean: load replays the stored
//! conversation as a burst of `session/update` notifications before the result
//! returns, resume does not. Live reconnect, persisted-meta recovery, lease
//! acquisition, and the replay event sequence are runtime wiring outside this
//! protocol crate; this module owns the branch selection, the refusal table,
//! the after-response notification order, and the mode-state vocabulary.

use crate::errors::RpcError;
use crate::extensions::unknown_session;
use crate::lifecycle::validate_session_id;
use crate::wire::{SessionMode, SessionModeState};

/// Load replays history; resume does not (`service.go:302`, `service.go:334`).
pub const REPLAY_LOAD: bool = true;
pub const REPLAY_RESUME: bool = false;

/// afterResponse notification order, identical for load and resume
/// (`session.go:308-313`, `session.go:341-345`). The result frame is written
/// before any of these go out.
pub const OPEN_AFTER_RESPONSE: &[&str] = &[
    "send_available_commands",
    "send_usage_update",
    "publish_mcp_status",
];

/// Stable `data.code` for the additional-directories mismatch refusal
/// (`additional_dirs.go:18`).
pub const CODE_ADDITIONAL_DIRS_CHANGED: &str = "acp.additional_directories.session_open";

/// Normal collaboration mode ids (`service.go:438-442`).
pub const MODE_NORMAL: &str = "normal";
pub const MODE_PLAN: &str = "plan";
pub const MODE_GOAL: &str = "goal";

/// Live session snapshot available to the open decision. The real registry
/// lookup and controller live behind the runtime seam this crate does not wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveOpen {
    /// Transcript is marked cleanup-pending; the session must not reopen.
    pub cleanup_pending: bool,
    /// Additional directories the live session was opened with.
    pub additional_directories: Vec<String>,
    /// Live session's current mode id, kept so a reconnecting client's picker
    /// is not reset to normal (`session.go:320-325`).
    pub current_mode_id: String,
}

/// Persisted sidecar snapshot available to the open decision.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PersistedOpen {
    /// Recovered transcript path (or id-keyed redirect) is cleanup-pending.
    pub cleanup_pending: bool,
}

/// Which open branch the request takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenBranch {
    /// Already held in this process; no controller rebuild.
    Live,
    /// Restored from the on-disk transcript.
    Persisted,
    /// No live and no persisted entry: an unknown session.
    Unknown,
}

/// Decision reached before the runtime opens anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDecision {
    pub branch: OpenBranch,
    /// Whether history must be replayed to the client for this open.
    pub replay: bool,
    /// Live session's mode id, when the live branch was selected.
    pub live_mode_id: Option<String>,
}

/// Order-insensitive additional-directory equality (`additional_dirs.go:73-79`).
pub fn same_dirs(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut a: Vec<&str> = a.iter().map(String::as_str).collect();
    let mut b: Vec<&str> = b.iter().map(String::as_str).collect();
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// `-32602` additional-directories mismatch, carrying the stable `data.code`
/// (`service.go:508-510`).
pub fn additional_dirs_changed_error(method: &str) -> RpcError {
    RpcError::invalid_params(format!(
        "{method}: the session is open with other additional directories; close it first"
    ))
    .with_data(serde_json::json!({"code": CODE_ADDITIONAL_DIRS_CHANGED}))
}

/// Branch selection for load/resume. `replay` is the handler's load/resume
/// flag; both branches honor it, the persisted branch only registers the
/// replay step after the session is restored (`service.go:511-516`,
/// `service.go:655-657`).
pub fn decide_open(
    method: &str,
    id: &str,
    live: Option<&LiveOpen>,
    persisted: Option<&PersistedOpen>,
    request_dirs: &[String],
    replay: bool,
) -> Result<OpenDecision, RpcError> {
    validate_session_id(method, id)?;

    if let Some(live) = live {
        if live.cleanup_pending {
            return Err(unknown_session(method, id));
        }
        if !same_dirs(&live.additional_directories, request_dirs) {
            return Err(additional_dirs_changed_error(method));
        }
        return Ok(OpenDecision {
            branch: OpenBranch::Live,
            replay,
            live_mode_id: Some(live.current_mode_id.clone()),
        });
    }

    match persisted {
        Some(persisted) if persisted.cleanup_pending => Err(unknown_session(method, id)),
        Some(_) => Ok(OpenDecision {
            branch: OpenBranch::Persisted,
            replay,
            live_mode_id: None,
        }),
        None => Ok(OpenDecision {
            branch: OpenBranch::Unknown,
            replay: false,
            live_mode_id: None,
        }),
    }
}

/// Available mode catalog with the reference display names and descriptions
/// (`session.go:222-231`). A current id outside the three known modes falls
/// back to normal.
pub fn session_modes_state(current: Option<&str>) -> SessionModeState {
    let current = match current {
        Some("plan") => MODE_PLAN,
        Some("goal") => MODE_GOAL,
        _ => MODE_NORMAL,
    };
    SessionModeState {
        current_mode_id: current.to_owned(),
        available_modes: vec![
            SessionMode {
                id: MODE_NORMAL.to_owned(),
                name: "Normal".to_owned(),
                description: Some("Work directly and pause when user input is required".to_owned()),
            },
            SessionMode {
                id: MODE_PLAN.to_owned(),
                name: "Plan".to_owned(),
                description: Some("Research and propose a plan before making changes".to_owned()),
            },
            SessionMode {
                id: MODE_GOAL.to_owned(),
                name: "Goal".to_owned(),
                description: Some(
                    "Keep advancing the next prompt as a goal until complete or blocked".to_owned(),
                ),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    #[test]
    fn replay_flag_is_the_only_load_resume_difference() {
        const _LOAD: () = assert!(REPLAY_LOAD);
        const _RESUME: () = assert!(!REPLAY_RESUME);
        assert_eq!(
            OPEN_AFTER_RESPONSE,
            [
                "send_available_commands",
                "send_usage_update",
                "publish_mcp_status"
            ]
        );
    }

    #[test]
    fn live_branch_keeps_current_mode_and_honors_replay() {
        let live = LiveOpen {
            cleanup_pending: false,
            additional_directories: vec!["/a".to_owned(), "/b".to_owned()],
            current_mode_id: "plan".to_owned(),
        };
        let decision = decide_open(
            "session/load",
            "s1",
            Some(&live),
            None,
            &["/b".to_owned(), "/a".to_owned()],
            REPLAY_LOAD,
        )
        .unwrap();
        assert_eq!(decision.branch, OpenBranch::Live);
        assert!(decision.replay);
        assert_eq!(decision.live_mode_id.as_deref(), Some("plan"));

        let resume = decide_open(
            "session/resume",
            "s1",
            Some(&live),
            None,
            &["/a".to_owned(), "/b".to_owned()],
            REPLAY_RESUME,
        )
        .unwrap();
        assert!(!resume.replay);
    }

    #[test]
    fn live_cleanup_pending_and_dir_mismatch_refuse_verbatim() {
        let pending = LiveOpen {
            cleanup_pending: true,
            additional_directories: vec![],
            current_mode_id: "normal".to_owned(),
        };
        let err = decide_open("session/load", "s1", Some(&pending), None, &[], true).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(err.message, "session/load: unknown session s1");

        let live = LiveOpen {
            cleanup_pending: false,
            additional_directories: vec!["/a".to_owned()],
            current_mode_id: "normal".to_owned(),
        };
        let err = decide_open(
            "session/resume",
            "s1",
            Some(&live),
            None,
            &["/z".to_owned()],
            false,
        )
        .unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(
            err.message,
            "session/resume: the session is open with other additional directories; close it first"
        );
        assert_eq!(
            err.data.as_ref().unwrap()["code"],
            CODE_ADDITIONAL_DIRS_CHANGED
        );
    }

    #[test]
    fn persisted_and_unknown_branches_resolve_replay() {
        let persisted = PersistedOpen::default();
        let open = decide_open("session/load", "s1", None, Some(&persisted), &[], true).unwrap();
        assert_eq!(open.branch, OpenBranch::Persisted);
        assert!(open.replay);

        let missing = decide_open("session/resume", "s1", None, None, &[], false).unwrap();
        assert_eq!(missing.branch, OpenBranch::Unknown);
        assert!(!missing.replay);
    }

    #[test]
    fn invalid_session_id_is_rejected_before_branching() {
        let err = decide_open("session/load", "has space", None, None, &[], true).unwrap_err();
        assert_eq!(err.message, "session/load: invalid sessionId");
    }

    #[test]
    fn mode_state_falls_back_to_normal_and_keeps_live_picker() {
        let with_goal = session_modes_state(Some("goal"));
        assert_eq!(with_goal.current_mode_id, "goal");
        assert_eq!(with_goal.available_modes.len(), 3);
        let unknown = session_modes_state(Some("bogus"));
        assert_eq!(unknown.current_mode_id, "normal");
        let absent = session_modes_state(None);
        assert_eq!(absent.current_mode_id, "normal");
    }
}
