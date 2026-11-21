//! Interception DTOs and decision guards (contract §4, §12.6).
//!
//! The host blocks on `extension/intercept`; `extension/event` observes the
//! same 17 points fire-and-forget. Intercept payloads and replacements are
//! externalizable and carry a monotonic sequence for drop detection.

use crate::content::ExternalizedField;
use crate::enums::{InterceptDecision, InterceptEvent};
use crate::rawjson::RawJson;
use serde::{Deserialize, Serialize};

/// One blocking interception request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterceptParams {
    pub event: InterceptEvent,
    /// Host-monotonic intercept sequence for the session (>=1).
    pub seq: u64,
    /// Inline payload, or null when externalized.
    pub payload: Option<RawJson>,
    #[serde(rename = "timeoutMillis")]
    pub timeout_millis: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub externalized: Vec<ExternalizedField>,
}

/// The extension's ruling on an intercepted event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterceptResult {
    pub decision: InterceptDecision,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<RawJson>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub externalized: Vec<ExternalizedField>,
}

/// Why an interception result violates the frozen decision contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InterceptValidationError {
    #[error("intercept seq must be >= 1")]
    BadSeq,
    #[error("timeoutMillis must be non-negative")]
    BadTimeout,
    #[error("answer is only legal with block at tool.before")]
    IllegalAnswer,
    #[error("replacement is only legal with a replace decision")]
    IllegalReplacement,
    #[error("allow, deny, and ask are only legal at permission.decision")]
    IllegalPermissionDecision,
}

/// Validates an intercept request's wire invariants.
pub fn validate_intercept_params(params: &InterceptParams) -> Result<(), InterceptValidationError> {
    if params.seq < 1 {
        return Err(InterceptValidationError::BadSeq);
    }
    if params.timeout_millis < 0 {
        return Err(InterceptValidationError::BadTimeout);
    }
    Ok(())
}

/// Validates an intercept result against the frozen per-point rules.
///
/// `answer` is legal only with `block` at `tool.before`; `allow`/`deny`/`ask`
/// are legal only at `permission.decision`.
pub fn validate_intercept_result(
    event: InterceptEvent,
    result: &InterceptResult,
) -> Result<(), InterceptValidationError> {
    if !result.answer.is_empty()
        && !(result.decision == InterceptDecision::Block && event == InterceptEvent::ToolBefore)
    {
        return Err(InterceptValidationError::IllegalAnswer);
    }
    if result.replacement.is_some() && result.decision != InterceptDecision::Replace {
        return Err(InterceptValidationError::IllegalReplacement);
    }
    if matches!(
        result.decision,
        InterceptDecision::Allow | InterceptDecision::Deny | InterceptDecision::Ask
    ) && event != InterceptEvent::PermissionDecision
    {
        return Err(InterceptValidationError::IllegalPermissionDecision);
    }
    Ok(())
}

/// A fire-and-forget observation of one hook point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventParams {
    pub event: InterceptEvent,
    pub payload: Option<RawJson>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub externalized: Vec<ExternalizedField>,
}

/// Default timeout families from contract §4.
pub mod timeouts {
    /// input/tool/permission points (milliseconds).
    pub const INTERACTIVE_MS: u64 = 5_000;
    /// session/context/compaction/system-prompt family (milliseconds).
    pub const FAMILY_MS: u64 = 30_000;
    /// Manifest ceiling per runtime (milliseconds).
    pub const CEILING_MS: u64 = 60_000;
}

/// Returns the default timeout for a hook point in milliseconds.
pub fn default_timeout_ms(event: InterceptEvent) -> u64 {
    match event {
        InterceptEvent::InputReceive
        | InterceptEvent::ToolBefore
        | InterceptEvent::ToolAfter
        | InterceptEvent::PermissionDecision => timeouts::INTERACTIVE_MS,
        _ => timeouts::FAMILY_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(s: &str) -> RawJson {
        RawJson::parse(s).unwrap()
    }

    #[test]
    fn intercept_params_round_trip_and_seq_guard() {
        let params = InterceptParams {
            event: InterceptEvent::ToolBefore,
            seq: 7,
            payload: Some(raw("{\"tool\":\"bash\"}")),
            timeout_millis: 250,
            externalized: vec![],
        };
        let json = serde_json::to_string(&params).unwrap();
        assert!(json.contains("\"seq\":7"));
        assert!(json.contains("\"event\":\"tool.before\""));
        let decoded: InterceptParams = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, params);
        assert!(validate_intercept_params(&params).is_ok());
        let mut bad = params;
        bad.seq = 0;
        assert_eq!(
            validate_intercept_params(&bad),
            Err(InterceptValidationError::BadSeq)
        );
    }

    #[test]
    fn replace_decision_round_trips() {
        let result = InterceptResult {
            decision: InterceptDecision::Replace,
            reason: "rewritten".into(),
            answer: String::new(),
            replacement: Some(raw("{\"tool\":\"read\"}")),
            externalized: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(
            json,
            "{\"decision\":\"replace\",\"reason\":\"rewritten\",\"replacement\":{\"tool\":\"read\"}}"
        );
        assert!(validate_intercept_result(InterceptEvent::ToolBefore, &result).is_ok());
    }

    #[test]
    fn answer_only_at_tool_before_block() {
        let answer = InterceptResult {
            decision: InterceptDecision::Block,
            reason: "handled".into(),
            answer: "here is the answer".into(),
            replacement: None,
            externalized: vec![],
        };
        assert!(validate_intercept_result(InterceptEvent::ToolBefore, &answer).is_ok());
        assert_eq!(
            validate_intercept_result(InterceptEvent::InputReceive, &answer),
            Err(InterceptValidationError::IllegalAnswer)
        );
        let continue_with_answer = InterceptResult {
            decision: InterceptDecision::Continue,
            answer: "x".into(),
            ..answer
        };
        assert_eq!(
            validate_intercept_result(InterceptEvent::ToolBefore, &continue_with_answer),
            Err(InterceptValidationError::IllegalAnswer)
        );
    }

    #[test]
    fn permission_decisions_are_point_scoped() {
        let allow = InterceptResult {
            decision: InterceptDecision::Allow,
            reason: String::new(),
            answer: String::new(),
            replacement: None,
            externalized: vec![],
        };
        assert!(validate_intercept_result(InterceptEvent::PermissionDecision, &allow).is_ok());
        assert_eq!(
            validate_intercept_result(InterceptEvent::SessionStart, &allow),
            Err(InterceptValidationError::IllegalPermissionDecision)
        );
    }

    #[test]
    fn default_timeout_families_match_contract() {
        assert_eq!(default_timeout_ms(InterceptEvent::InputReceive), 5_000);
        assert_eq!(
            default_timeout_ms(InterceptEvent::PermissionDecision),
            5_000
        );
        assert_eq!(default_timeout_ms(InterceptEvent::ContextPrepare), 30_000);
        assert_eq!(default_timeout_ms(InterceptEvent::SessionStart), 30_000);
        assert_eq!(timeouts::CEILING_MS, 60_000);
    }
}
