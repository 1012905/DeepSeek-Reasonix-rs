//! Notice Detail payloads, mirroring `event/notice_payload.go`. For a coded
//! notice in `detail_is_payload`, Detail is the typed JSON its sentence is
//! worded from, so a sink printing Text must not append it again.

use serde::de::DeserializeOwned;

use crate::gojson::marshal;

/// Typed identity for refusing a Detail that is not a payload. Callers keep
/// the kernel's English when either variant is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DetailDecodeError {
    #[error("notice detail is not a JSON object")]
    NotPayload,
    #[error("notice detail failed its payload guard")]
    GuardFailed,
}

impl DetailDecodeError {
    pub fn is_not_payload(&self) -> bool {
        matches!(
            self,
            DetailDecodeError::NotPayload | DetailDecodeError::GuardFailed
        )
    }
}

/// Reports that a coded notice's Detail is the typed payload its sentence is
/// worded from. `unapplied_steer` carries raw steer text, not a JSON object.
pub fn detail_is_payload(code: &str) -> bool {
    matches!(
        code,
        super::notice::code::UNAPPLIED_STEER
            | super::notice::code::EXTENSION_SKIPPED
            | super::notice::code::INBOX_RECOVERED
            | super::notice::code::INBOX_UNRECOVERED
            | super::notice::code::PERMISSION_RULES_DORMANT
            | super::notice::code::SUBAGENT_WAITING
            | super::notice::code::CONTEXT_COMPOSITION
            | super::notice::code::COMPACTION_TELEMETRY
            | super::notice::code::USER_TURNS_DROPPED
            | super::notice::code::KEPT_TURNS_FOLDED
            | super::notice::code::JOB_STARTED
            | super::notice::code::JOB_FINISHED
            | super::notice::code::JOB_KILLED
            | super::notice::code::JOB_FAILED
            | super::notice::code::JOB_STALLED
    )
}

/// Decode a payload from a Detail string. JSON `null` and missing keys become
/// zero values, as in Go; unknown fields are ignored. A JSON value that is not
/// an object, or a type mismatch, is refused as `NotPayload`.
fn decode_payload<T: Default + DeserializeOwned>(detail: &str) -> Result<T, DetailDecodeError> {
    let value: Option<T> =
        serde_json::from_str(detail).map_err(|_| DetailDecodeError::NotPayload)?;
    Ok(value.unwrap_or_default())
}

pub const EXTENSION_SKIP_REASON_NO_LIVE_SIDECAR: &str = "no_live_sidecar";

/// Which extension was left out, where, and the stable reason why.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExtensionSkipped {
    pub extension: String,
    pub point: String,
    pub reason: String,
}

impl ExtensionSkipped {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_extension_skipped(detail: &str) -> Result<ExtensionSkipped, DetailDecodeError> {
    let p = decode_payload::<ExtensionSkipped>(detail)?;
    if p.extension.is_empty() {
        return Err(DetailDecodeError::GuardFailed);
    }
    Ok(p)
}

/// How many unfinished instructions a reopened inbox holds while paused.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InboxRecovered {
    pub count: i64,
}

impl InboxRecovered {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_inbox_recovered(detail: &str) -> Result<InboxRecovered, DetailDecodeError> {
    let p = decode_payload::<InboxRecovered>(detail)?;
    if p.count <= 0 {
        return Err(DetailDecodeError::GuardFailed);
    }
    Ok(p)
}

/// The queue directory whose bodies could not be read.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InboxUnrecovered {
    pub dir: String,
}

impl InboxUnrecovered {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_inbox_unrecovered(detail: &str) -> Result<InboxUnrecovered, DetailDecodeError> {
    let p = decode_payload::<InboxUnrecovered>(detail)?;
    if p.dir.is_empty() {
        return Err(DetailDecodeError::GuardFailed);
    }
    Ok(p)
}

/// Which background job ended, by kind, id and its started label.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct JobNotice {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, rename = "exitCode", skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "crate::eventwire::util::is_zero")]
    pub signal: i64,
    #[serde(
        default,
        rename = "quietSeconds",
        skip_serializing_if = "crate::eventwire::util::is_zero"
    )]
    pub quiet_seconds: i64,
    #[serde(default, skip_serializing_if = "crate::eventwire::util::is_zero")]
    pub steps: i64,
}

impl JobNotice {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_job_notice(detail: &str) -> Result<JobNotice, DetailDecodeError> {
    let p = decode_payload::<JobNotice>(detail)?;
    if p.id.is_empty() {
        return Err(DetailDecodeError::GuardFailed);
    }
    Ok(p)
}

/// One saved rule no tool can match.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DormantPermissionRule {
    pub list: String,
    pub rule: String,
    pub tool: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PermissionRulesDormant {
    pub rules: Vec<DormantPermissionRule>,
}

impl PermissionRulesDormant {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_permission_rules_dormant(
    detail: &str,
) -> Result<PermissionRulesDormant, DetailDecodeError> {
    decode_payload(detail)
}

/// One class of what the prompt holds and the tokens it costs.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ContextPart {
    pub name: String,
    pub tokens: i64,
}

/// One of the largest messages the prompt holds.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ContextItem {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    pub tokens: i64,
    #[serde(default, skip_serializing_if = "crate::eventwire::util::is_false")]
    pub active: bool,
}

/// What holds the tokens of a prompt over its compaction threshold.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ContextComposition {
    pub total: i64,
    pub at: i64,
    pub active: i64,
    pub parts: Vec<ContextPart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub largest: Vec<ContextItem>,
}

impl ContextComposition {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_context_composition(detail: &str) -> Result<ContextComposition, DetailDecodeError> {
    decode_payload(detail)
}

/// Which sub-agent is held back, by whose write claim, and whether the claim
/// covered a whole workspace.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SubagentWaiting {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub task: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
    #[serde(default, skip_serializing_if = "crate::eventwire::util::is_false")]
    pub whole: bool,
}

impl SubagentWaiting {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_subagent_waiting(detail: &str) -> Result<SubagentWaiting, DetailDecodeError> {
    decode_payload(detail)
}

/// How a finished fold went, as numbers a reader can word in their own
/// language. `mode` and `trigger` are identifiers, never sentences.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CompactionNotice {
    pub trigger: String,
    pub mode: String,
    #[serde(
        default,
        rename = "cache_state",
        skip_serializing_if = "String::is_empty"
    )]
    pub cache_state: String,
    #[serde(rename = "source_tokens")]
    pub source_tokens: i64,
    #[serde(rename = "fold_tokens")]
    pub fold_tokens: i64,
    pub spans: i64,
    #[serde(rename = "projection_tokens")]
    pub projection_tokens: i64,
    #[serde(rename = "input_tokens")]
    pub input_tokens: i64,
    #[serde(rename = "output_tokens")]
    pub output_tokens: i64,
    #[serde(rename = "cache_hit_tokens")]
    pub cache_hit_tokens: i64,
    #[serde(rename = "cache_miss_tokens")]
    pub cache_miss_tokens: i64,
    #[serde(rename = "cache_write_tokens")]
    pub cache_write_tokens: i64,
    #[serde(rename = "request_count")]
    pub request_count: i64,
    #[serde(rename = "user_turns_kept")]
    pub user_turns_kept: i64,
    #[serde(rename = "user_turns_dropped")]
    pub user_turns_dropped: i64,
    #[serde(
        default,
        rename = "provider_request_id",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider_request_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl CompactionNotice {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_compaction_notice(detail: &str) -> Result<CompactionNotice, DetailDecodeError> {
    let p = decode_payload::<CompactionNotice>(detail)?;
    if p.mode.is_empty() {
        return Err(DetailDecodeError::GuardFailed);
    }
    Ok(p)
}

/// How many user turns a fold left to the summary, their tokens, and the
/// budget they exceeded.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TurnsDropped {
    pub count: i64,
    pub tokens: i64,
    pub budget: i64,
}

impl TurnsDropped {
    pub fn encode(&self) -> String {
        marshal(self).unwrap_or_default()
    }
}

pub fn decode_turns_dropped(detail: &str) -> Result<TurnsDropped, DetailDecodeError> {
    decode_payload(detail)
}
