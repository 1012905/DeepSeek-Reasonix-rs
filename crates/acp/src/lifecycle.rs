//! Session lifecycle state machine, session-id validation, lease refusal code
//! table, and control-axis normalization (service.go:1422-1447; session.go:
//! 250-275; session_config.go:559-572; tool_approval_ceiling.go:19-67).
//!
//! Pure protocol layer: state transitions, id validation, code tables and
//! string normalization are implemented here. Real lease acquisition, the
//! transcript store, controller wiring, and replay bursts depend on the
//! session runtime and are marked WIP at the boundary; this module never
//! touches a filesystem or spawns a process.

use crate::errors::{ErrorData, RpcError, RpcErrorCode};

// ---- Session id validation --------------------------------------------------

/// Validates a session id the way the service accepts one (service.go:
/// 1422-1447): trimmed non-empty, never `.`/`..`, charset
/// `[a-zA-Z0-9._-]`. The `method` labels the two `-32602` messages.
pub fn validate_session_id(method: &str, session_id: &str) -> Result<(), RpcError> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(RpcError::invalid_params(format!(
            "{method}: missing sessionId"
        )));
    }
    if trimmed != session_id || trimmed == "." || trimmed == ".." {
        return Err(RpcError::invalid_params(format!(
            "{method}: invalid sessionId"
        )));
    }
    let valid = trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !valid {
        return Err(RpcError::invalid_params(format!(
            "{method}: invalid sessionId"
        )));
    }
    Ok(())
}

// ---- Lifecycle state machine -------------------------------------------------

/// Visible session state. Real persistence is owned by the session runtime;
/// this enum only encodes the legal transitions the protocol surface implies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// Opened, no turn run yet.
    New,
    /// Turn active or idle after a turn.
    Active,
    /// Closed; history retained.
    Closed,
    /// Deleted; transcript removed.
    Deleted,
}

/// Lifecycle events the state machine reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    Open,
    Close,
    Delete,
}

/// A transition the current state does not permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid transition {event:?} from {state:?}")]
pub struct InvalidTransition {
    pub state: SessionState,
    pub event: LifecycleEvent,
}

/// Applies one lifecycle event. `close` and `delete` are idempotent against an
/// unknown session at the service layer; the state machine itself only tracks
/// sessions it has seen.
pub fn next_state(
    state: SessionState,
    event: LifecycleEvent,
) -> Result<SessionState, InvalidTransition> {
    match (state, event) {
        (SessionState::New, LifecycleEvent::Open) => Ok(SessionState::Active),
        (SessionState::Active, LifecycleEvent::Close) => Ok(SessionState::Closed),
        (
            SessionState::New | SessionState::Active | SessionState::Closed,
            LifecycleEvent::Delete,
        ) => Ok(SessionState::Deleted),
        (state, event) => Err(InvalidTransition { state, event }),
    }
}

// ---- Lease refusal code table -------------------------------------------------

/// Stable data.code for a held lease (tool_approval_ceiling.go sibling;
/// session.go:49-50).
pub const CODE_SESSION_IN_USE: &str = "busy.session_in_use";
/// Stable data.code for an unprovable lease ownership (session.go:51-55).
pub const CODE_LEASE_UNPROVABLE: &str = "session.lease_unprovable";

/// Fallback holder description when the store reports no detail
/// (session_lease_keeper.go:268-276).
pub const SESSION_IN_USE_FALLBACK: &str =
    "this session is in use by another Reasonix window or process";

/// Classified lease refusal, ready to be mapped onto a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseRefusal {
    /// Another holder owns the transcript lease.
    Held { holder: String },
    /// Ownership is unprovable; the client must not be told to close a window.
    Unprovable { detail: String },
    /// Any other lease failure, surfaced as an internal error.
    Other { detail: String },
}

/// Maps a classified refusal to its frozen RPC code, data.code, and message
/// template (session.go:28-57).
pub fn lease_error(method: &str, refusal: LeaseRefusal) -> RpcError {
    match refusal {
        LeaseRefusal::Held { holder } => {
            let holder = if holder.trim().is_empty() {
                SESSION_IN_USE_FALLBACK
            } else {
                holder.as_str()
            };
            let data = ErrorData {
                code: CODE_SESSION_IN_USE.to_owned(),
                ceiling: None,
            };
            RpcError::new(
                RpcErrorCode::SessionLeaseHeld,
                format!("{method}: {holder}; close the other Reasonix window or process first"),
            )
            .with_data(serde_json::to_value(&data).expect("lease data serializes"))
        }
        LeaseRefusal::Unprovable { detail } => {
            let data = ErrorData {
                code: CODE_LEASE_UNPROVABLE.to_owned(),
                ceiling: None,
            };
            RpcError::new(
                RpcErrorCode::SessionLeaseUnprovable,
                format!("{method}: {detail}"),
            )
            .with_data(serde_json::to_value(&data).expect("lease data serializes"))
        }
        LeaseRefusal::Other { detail } => RpcError::new(
            RpcErrorCode::Internal,
            format!("{method}: session lease: {detail}"),
        ),
    }
}

// ---- Control-axis normalization ---------------------------------------------

/// Legacy mode ids mapped onto the current collaboration mode plus the
/// implied tool-approval posture (session.go:258-273).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeResolution {
    pub mode_id: String,
    pub implied_posture: Option<&'static str>,
}

/// Resolves a `session/set_mode` id. Unknown ids are reported by the handler
/// with the verbatim unknown-modeId message; this function returns `None`.
pub fn resolve_mode_id(mode_id: &str) -> Option<ModeResolution> {
    match mode_id {
        "normal" => Some(ModeResolution {
            mode_id: "normal".to_owned(),
            implied_posture: None,
        }),
        "plan" => Some(ModeResolution {
            mode_id: "plan".to_owned(),
            implied_posture: None,
        }),
        "goal" => Some(ModeResolution {
            mode_id: "goal".to_owned(),
            implied_posture: None,
        }),
        "default" => Some(ModeResolution {
            mode_id: "normal".to_owned(),
            implied_posture: Some("ask"),
        }),
        "auto" => Some(ModeResolution {
            mode_id: "normal".to_owned(),
            implied_posture: Some("yolo"),
        }),
        _ => None,
    }
}

/// `-32602` empty prompt body (session.go:366-368).
pub fn empty_prompt(method: &str) -> RpcError {
    RpcError::invalid_params(format!("{method}: empty prompt"))
}

/// `-32600` a prompt is already active for the session (session.go:372-375).
pub fn active_prompt_conflict(method: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidRequest,
        format!("{method}: session already has an active prompt"),
    )
}

/// `-32600` steer arrived with no active prompt to redirect (session.go:471).
pub fn no_active_prompt(method: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidRequest,
        format!("{method}: session has no active prompt"),
    )
}

/// `-32602` list carries a cursor, which the in-process list does not support
/// (session.go:601-603).
pub fn unsupported_cursor(method: &str) -> RpcError {
    RpcError::invalid_params(format!("{method}: unsupported cursor"))
}

/// `-32602` unknown collaboration mode (session.go:275).
pub fn unknown_mode_id(mode_id: &str) -> RpcError {
    RpcError::invalid_params(format!("session/set_mode: unknown modeId {mode_id}"))
}

/// `-32602` unknown config-option axis (session.go:512).
pub fn unknown_config_option(config_id: &str) -> RpcError {
    RpcError::invalid_params(format!(
        "session/set_config_option: unknown config option {config_id}"
    ))
}

/// `-32602` value not among the option's advertised choices (session.go:520).
pub fn invalid_config_value(value: &str, config_id: &str) -> RpcError {
    RpcError::invalid_params(format!(
        "session/set_config_option: invalid value {value} for {config_id}"
    ))
}

/// `-32602` config axis this session does not support (session.go:539).
pub fn unsupported_config_option(config_id: &str) -> RpcError {
    RpcError::invalid_params(format!(
        "session/set_config_option: unsupported config option {config_id}"
    ))
}

/// Normalizes legacy config-option aliases onto the current axis id
/// (session_config.go:559-572). The lookup and the fallback both trim
/// surrounding whitespace first.
pub fn normalize_config_id(raw: &str) -> String {
    match raw.trim() {
        "models" => "model".to_owned(),
        "reasoning_effort" | "thought_level" => "effort".to_owned(),
        "profile" | "runtime_profile" | "token_mode" => "work_mode".to_owned(),
        "approval" | "approval_mode" | "tool_approval_mode" => "tool_approval".to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_validation_rejects_blank_dotdot_and_bad_chars() {
        let ok = validate_session_id("session/new", "abc-DEF_123.x");
        assert!(ok.is_ok());

        let blank = validate_session_id("session/new", "   ").unwrap_err();
        assert_eq!(blank.code, RpcErrorCode::InvalidParams);
        assert_eq!(blank.message, "session/new: missing sessionId");

        let dot = validate_session_id("session/close", "..").unwrap_err();
        assert_eq!(dot.message, "session/close: invalid sessionId");

        let bad = validate_session_id("session/load", "has space").unwrap_err();
        assert_eq!(bad.message, "session/load: invalid sessionId");

        let padded = validate_session_id("session/close", " s1 ").unwrap_err();
        assert_eq!(padded.message, "session/close: invalid sessionId");
    }

    #[test]
    fn state_machine_allows_open_close_delete_only() {
        use LifecycleEvent::*;
        assert_eq!(
            next_state(SessionState::New, Open).unwrap(),
            SessionState::Active
        );
        assert_eq!(
            next_state(SessionState::Active, Close).unwrap(),
            SessionState::Closed
        );
        assert_eq!(
            next_state(SessionState::Closed, Delete).unwrap(),
            SessionState::Deleted
        );
        assert!(next_state(SessionState::Closed, Close).is_err());
        assert!(next_state(SessionState::Deleted, Delete).is_err());
    }

    #[test]
    fn lease_refusals_map_to_frozen_codes_and_data() {
        let held = lease_error(
            "session/load",
            LeaseRefusal::Held {
                holder: String::new(),
            },
        );
        assert_eq!(held.code, RpcErrorCode::SessionLeaseHeld);
        assert_eq!(held.code.code(), -32022);
        assert_eq!(held.data.as_ref().unwrap()["code"], CODE_SESSION_IN_USE);
        assert!(held.message.contains(SESSION_IN_USE_FALLBACK));

        let unprovable = lease_error(
            "session/resume",
            LeaseRefusal::Unprovable {
                detail: "owner probe timed out".to_owned(),
            },
        );
        assert_eq!(unprovable.code, RpcErrorCode::SessionLeaseUnprovable);
        assert_eq!(unprovable.code.code(), -32023);
        assert_eq!(
            unprovable.data.as_ref().unwrap()["code"],
            CODE_LEASE_UNPROVABLE
        );
        assert_eq!(unprovable.message, "session/resume: owner probe timed out");

        let other = lease_error(
            "session/new",
            LeaseRefusal::Other {
                detail: "io failure".to_owned(),
            },
        );
        assert_eq!(other.code, RpcErrorCode::Internal);
        assert_eq!(other.message, "session/new: session lease: io failure");
        assert!(other.data.is_none());
    }

    #[test]
    fn prompt_and_cursor_error_messages_match_templates() {
        assert_eq!(
            empty_prompt("session/prompt").message,
            "session/prompt: empty prompt"
        );
        let conflict = active_prompt_conflict("session/prompt");
        assert_eq!(conflict.code, RpcErrorCode::InvalidRequest);
        assert_eq!(
            conflict.message,
            "session/prompt: session already has an active prompt"
        );
        let steer = no_active_prompt("_reasonix.io/session/steer");
        assert_eq!(steer.code, RpcErrorCode::InvalidRequest);
        assert_eq!(
            steer.message,
            "_reasonix.io/session/steer: session has no active prompt"
        );
        assert_eq!(
            unsupported_cursor("session/list").message,
            "session/list: unsupported cursor"
        );
    }

    #[test]
    fn config_and_mode_error_messages_match_templates() {
        assert_eq!(
            unknown_mode_id("bogus").message,
            "session/set_mode: unknown modeId bogus"
        );
        assert_eq!(
            unknown_config_option("profile").message,
            "session/set_config_option: unknown config option profile"
        );
        assert_eq!(
            invalid_config_value("nope", "tool_approval").message,
            "session/set_config_option: invalid value nope for tool_approval"
        );
        assert_eq!(
            unsupported_config_option("agent_composition").message,
            "session/set_config_option: unsupported config option agent_composition"
        );
    }

    #[test]
    fn legacy_mode_and_config_aliases_normalize() {
        let default = resolve_mode_id("default").unwrap();
        assert_eq!(default.mode_id, "normal");
        assert_eq!(default.implied_posture, Some("ask"));
        let auto = resolve_mode_id("auto").unwrap();
        assert_eq!(auto.implied_posture, Some("yolo"));
        assert!(resolve_mode_id("bogus").is_none());

        assert_eq!(normalize_config_id("models"), "model");
        assert_eq!(normalize_config_id("thought_level"), "effort");
        assert_eq!(normalize_config_id("token_mode"), "work_mode");
        assert_eq!(normalize_config_id("approval_mode"), "tool_approval");
        assert_eq!(normalize_config_id("model"), "model");
    }
}
