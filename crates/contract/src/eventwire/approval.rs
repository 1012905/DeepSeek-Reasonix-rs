//! The approval frame and its recovery payload, mirroring `eventwire/approval.go`.

use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// A decision the model may not make alone. `scope` is what approving
/// authorizes beyond the call; `input`, the call's arguments, rides only with
/// a scope, since they are what it applies to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(
        rename = "reasonCode",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub reason_code: String,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub fresh: bool,
    #[serde(
        rename = "allowsSession",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub allows_session: bool,
    #[serde(
        rename = "allowsPersist",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub allows_persist: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryApproval>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
}

/// Why a run is stopped mid-flight and what resuming needs; mirrors
/// `eventwire.RecoveryApproval`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryApproval {
    #[serde(
        rename = "source_agent",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub source_agent: String,
    #[serde(
        rename = "failed_tool",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub failed_tool: String,
    #[serde(
        rename = "failed_summary",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub failed_summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub diagnosis: String,
    #[serde(
        rename = "next_tool",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub next_tool: String,
    #[serde(
        rename = "next_action",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub next_action: String,
    #[serde(
        rename = "change_kind",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub change_kind: String,
    #[serde(
        rename = "change_rationale",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub change_rationale: String,
    #[serde(
        rename = "review_rationale",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub review_rationale: String,
    #[serde(
        rename = "plan_before",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub plan_before: String,
    #[serde(
        rename = "plan_after",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub plan_after: String,
    #[serde(
        rename = "can_grant_task",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub can_grant_task: bool,
    #[serde(
        rename = "task_grant_scope",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub task_grant_scope: String,
}
