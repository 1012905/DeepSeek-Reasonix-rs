//! `session/request_permission` round-trip, option kinds, and the posture
//! ceiling helpers (protocol.go:747-793; dispatch.go:412-632;
//! tool_approval_ceiling.go:35-97).
//!
//! The ceiling never filters the permission option list itself: whether a
//! prompt appears is the controller's posture decision. It only filters the
//! `tool_approval` config-option choices and rejects switching above the
//! ceiling (matrix B §3.7, conflict C7).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{ErrorData, RpcError, RpcErrorCode};
use crate::update::ToolCallLocation;

/// The four v1 permission option kinds (protocol.go:750-755). `reject_always`
/// is retained by the enum even though the reference never produces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// One option offered in a permission prompt (protocol.go:758-762).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    #[serde(rename = "optionId")]
    pub option_id: String,
    pub name: String,
    pub kind: PermissionOptionKind,
}

/// The tool call being put to the user (protocol.go:772-782).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PermissionToolCall {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<crate::update::ToolContent>>,
    #[serde(rename = "rawInput", skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<ToolCallLocation>>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, Value>>,
}

/// `session/request_permission` params (protocol.go:765-769).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequestParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "toolCall")]
    pub tool_call: PermissionToolCall,
    pub options: Vec<PermissionOption>,
}

/// The selected option, or an explicit cancellation (protocol.go:785-790).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PermissionOutcomeBody {
    pub outcome: String,
    #[serde(rename = "optionId", skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
}

/// `session/request_permission` result (protocol.go:785-793).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PermissionRequestResult {
    pub outcome: PermissionOutcomeBody,
}

/// Granted flags decoded from a selected option (dispatch.go:436-450).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PermissionGrant {
    pub allow: bool,
    pub session: bool,
    pub persist: bool,
}

/// Decision reached after resolving the client reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Option mapped to a grant.
    Grant(PermissionGrant),
    /// Transport failure, cancelled reply, unknown option, or malformed body:
    /// deny by default.
    Deny,
}

/// Resolves a permission reply to a grant. Every failure mode — transport
/// error, unparseable body, non-selected outcome, unknown optionId — resolves
/// to deny (dispatch.go:437-450).
pub fn resolve_permission(result: &PermissionRequestResult) -> PermissionDecision {
    if result.outcome.outcome != "selected" {
        return PermissionDecision::Deny;
    }
    match result.outcome.option_id.as_deref() {
        Some("allow_once") => PermissionDecision::Grant(PermissionGrant {
            allow: true,
            session: false,
            persist: false,
        }),
        Some("allow_always") => PermissionDecision::Grant(PermissionGrant {
            allow: true,
            session: true,
            persist: false,
        }),
        // Vendor persist option keeps the v1 `allow_always` kind; only the
        // optionId carries the persist flag (matrix B §3.2, conflict C5).
        Some("allow_persist") => PermissionDecision::Grant(PermissionGrant {
            allow: true,
            session: true,
            persist: true,
        }),
        _ => PermissionDecision::Deny,
    }
}

// ---- Approval option builders ---------------------------------------------

const ALLOW_ONCE_NAME: &str = "Allow";
const REJECT_NAME: &str = "Reject";

/// Standard three-option set: allow once, session grant, reject once
/// (dispatch.go:625-631).
pub fn standard_options(session_label: &str) -> Vec<PermissionOption> {
    vec![
        PermissionOption {
            option_id: "allow_once".to_owned(),
            name: ALLOW_ONCE_NAME.to_owned(),
            kind: PermissionOptionKind::AllowOnce,
        },
        PermissionOption {
            option_id: "allow_always".to_owned(),
            name: session_label.to_owned(),
            kind: PermissionOptionKind::AllowAlways,
        },
        PermissionOption {
            option_id: "reject_once".to_owned(),
            name: REJECT_NAME.to_owned(),
            kind: PermissionOptionKind::RejectOnce,
        },
    ]
}

/// Fresh-approval two-option set: allow once, reject once
/// (dispatch.go:620-623).
pub fn fresh_options() -> Vec<PermissionOption> {
    vec![
        PermissionOption {
            option_id: "allow_once".to_owned(),
            name: ALLOW_ONCE_NAME.to_owned(),
            kind: PermissionOptionKind::AllowOnce,
        },
        PermissionOption {
            option_id: "reject_once".to_owned(),
            name: REJECT_NAME.to_owned(),
            kind: PermissionOptionKind::RejectOnce,
        },
    ]
}

/// Persist entry appended after the session grant when the host allows it.
/// Its kind stays `allow_always`; only the optionId is vendor-specific
/// (dispatch.go:596,608).
pub fn persist_option(subject: &str) -> PermissionOption {
    PermissionOption {
        option_id: "allow_persist".to_owned(),
        name: format!("Always allow {subject} (charged per request; saved to your settings)"),
        kind: PermissionOptionKind::AllowAlways,
    }
}

/// Per-question options: numbered `<qid>:<n>` allow entries plus the fixed
/// `<qid>:cancel` reject entry (dispatch.go:536-544).
pub fn ask_options(
    question_id: &str,
    labels: &[(String, Option<String>)],
) -> Vec<PermissionOption> {
    let mut options = Vec::new();
    for (index, (label, description)) in labels.iter().enumerate() {
        let name = match description {
            Some(desc) if !desc.is_empty() => format!("{label} - {desc}"),
            _ => label.clone(),
        };
        options.push(PermissionOption {
            option_id: format!("{question_id}:{}", index + 1),
            name,
            kind: PermissionOptionKind::AllowOnce,
        });
    }
    options.push(PermissionOption {
        option_id: format!("{question_id}:cancel"),
        name: "Cancel".to_owned(),
        kind: PermissionOptionKind::RejectOnce,
    });
    options
}

// ---- Posture ceiling --------------------------------------------------------

/// Posture ranks low → high (tool_approval_ceiling.go:35-38).
pub const POSTURE_ORDER: [&str; 5] = ["readOnly", "dontAsk", "ask", "auto", "yolo"];

/// Rank of a posture; unknown values rank lowest so an unset mode cannot
/// climb above a ceiling by accident.
pub fn posture_rank(mode: &str) -> usize {
    POSTURE_ORDER
        .iter()
        .position(|&name| name == mode)
        .unwrap_or(0)
}

/// True when `mode` is at or below the ceiling. An empty ceiling means
/// unrestricted (tool_approval_ceiling.go:79-81).
pub fn within_ceiling(mode: &str, ceiling: &str) -> bool {
    ceiling.is_empty() || posture_rank(mode) <= posture_rank(ceiling)
}

/// Clamps a requested mode down to the ceiling when it would climb above it.
pub fn clamp_to_ceiling(mode: &str, ceiling: &str) -> String {
    if within_ceiling(mode, ceiling) {
        mode.to_owned()
    } else {
        ceiling.to_owned()
    }
}

/// Filters posture choices for the config-option list. No ceiling hides the
/// two most restrictive choices; a set ceiling keeps only at-or-below entries
/// (session_config.go:532-537).
pub fn filter_posture_choices(choices: &[String], ceiling: &str) -> Vec<String> {
    choices
        .iter()
        .filter(|choice| {
            if ceiling.is_empty() {
                posture_rank(choice) >= posture_rank("ask")
            } else {
                within_ceiling(choice, ceiling)
            }
        })
        .cloned()
        .collect()
}

/// Above-ceiling rejection carried on `session/set_config_option` / legacy
/// mode switches (tool_approval_ceiling.go:91-97).
pub fn above_ceiling_error(method: &str, mode: &str, ceiling: &str) -> RpcError {
    let data = ErrorData {
        code: "acp.tool_approval.above_ceiling".to_owned(),
        ceiling: Some(ceiling.to_owned()),
    };
    RpcError::new(
        RpcErrorCode::InvalidParams,
        format!(
            "{method}: tool approval {mode} is above the ceiling this agent was started with ({ceiling})"
        ),
    )
    .with_data(serde_json::to_value(&data).expect("ceiling data serializes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_kinds_round_trip_wire_values() {
        let cases = [
            (PermissionOptionKind::AllowOnce, "allow_once"),
            (PermissionOptionKind::AllowAlways, "allow_always"),
            (PermissionOptionKind::RejectOnce, "reject_once"),
            (PermissionOptionKind::RejectAlways, "reject_always"),
        ];
        for (kind, wire) in cases {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(wire));
        }
    }

    #[test]
    fn persist_option_keeps_allow_always_kind() {
        let option = persist_option("network write");
        assert_eq!(option.option_id, "allow_persist");
        assert_eq!(option.kind, PermissionOptionKind::AllowAlways);
    }

    #[test]
    fn selected_options_map_to_grants_and_everything_else_denies() {
        let grant = |option_id: &str| PermissionRequestResult {
            outcome: PermissionOutcomeBody {
                outcome: "selected".to_owned(),
                option_id: Some(option_id.to_owned()),
            },
        };
        assert_eq!(
            resolve_permission(&grant("allow_once")),
            PermissionDecision::Grant(PermissionGrant {
                allow: true,
                session: false,
                persist: false,
            })
        );
        assert_eq!(
            resolve_permission(&grant("allow_always")),
            PermissionDecision::Grant(PermissionGrant {
                allow: true,
                session: true,
                persist: false,
            })
        );
        assert_eq!(
            resolve_permission(&grant("allow_persist")),
            PermissionDecision::Grant(PermissionGrant {
                allow: true,
                session: true,
                persist: true,
            })
        );

        let cancelled = PermissionRequestResult {
            outcome: PermissionOutcomeBody {
                outcome: "cancelled".to_owned(),
                option_id: None,
            },
        };
        assert_eq!(resolve_permission(&cancelled), PermissionDecision::Deny);
        assert_eq!(
            resolve_permission(&grant("bogus")),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn ask_options_number_entries_and_append_cancel() {
        let options = ask_options(
            "ask-1",
            &[
                ("First".to_owned(), None),
                ("Second".to_owned(), Some("desc".to_owned())),
            ],
        );
        assert_eq!(options[0].option_id, "ask-1:1");
        assert_eq!(options[0].name, "First");
        assert_eq!(options[1].name, "Second - desc");
        assert_eq!(options[2].option_id, "ask-1:cancel");
        assert_eq!(options[2].kind, PermissionOptionKind::RejectOnce);
    }

    #[test]
    fn ceiling_filters_config_choices_but_not_permission_options() {
        let choices: Vec<String> = POSTURE_ORDER.iter().map(|s| s.to_string()).collect();
        let unrestricted = filter_posture_choices(&choices, "");
        assert_eq!(unrestricted, vec!["ask", "auto", "yolo"]);

        let capped = filter_posture_choices(&choices, "ask");
        assert_eq!(capped, vec!["readOnly", "dontAsk", "ask"]);

        assert!(within_ceiling("auto", "yolo"));
        assert!(!within_ceiling("yolo", "ask"));
        assert_eq!(clamp_to_ceiling("yolo", "ask"), "ask");
        assert_eq!(clamp_to_ceiling("ask", ""), "ask");
    }

    #[test]
    fn above_ceiling_error_carries_code_and_ceiling() {
        let err = above_ceiling_error("session/set_mode", "yolo", "ask");
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(
            err.message,
            "session/set_mode: tool approval yolo is above the ceiling this agent was started with (ask)"
        );
        let data = err.data.expect("data present");
        assert_eq!(data["code"], "acp.tool_approval.above_ceiling");
        assert_eq!(data["ceiling"], "ask");
    }
}
