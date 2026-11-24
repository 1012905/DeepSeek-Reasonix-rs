//! ACP-040 warning notices wire shape (`notice_meta.go:7-38`).
//!
//! A coded notice rides as an `agent_message_chunk` whose `metadata.notice`
//! names the stable code and, only for codes whose `Detail` is the typed
//! payload its sentence is worded from, the payload itself. A notice without
//! a code carries no metadata at all. The two leading newlines on the text
//! block are part of the user-visible contract (`docs/contracts/acp.md §13.1`).
//!
//! The event source that classifies kernel notices into text/code/detail is
//! runtime wiring; this module owns the chunk builder, the payload-code set,
//! and the omission rules.

use crate::update::{SessionUpdate, UpdateMetadata, UpdateNotice};
use crate::wire::ContentBlock;

/// Notice codes whose `Detail` is the typed payload (`event.DetailIsPayload`,
/// `internal/contract/event/notice_payload.go:7-11`). For any other code the
/// detail rides only into the rendered sentence, never onto `notice.payload`.
pub const PAYLOAD_CODES: [&str; 15] = [
    "unapplied_steer",
    "extension_skipped",
    "inbox_recovered",
    "inbox_unrecovered",
    "permission_rules_dormant",
    "subagent_waiting",
    "context_composition",
    "compaction_telemetry",
    "user_turns_dropped",
    "kept_turns_folded",
    "job_started",
    "job_finished",
    "job_killed",
    "job_failed",
    "job_stalled",
];

/// Whether the code's detail is the typed payload its sentence is worded from.
pub fn detail_is_payload(code: &str) -> bool {
    PAYLOAD_CODES.contains(&code)
}

/// A classified kernel notice ready to cross as a warning chunk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WarningEvent {
    /// Rendered notice sentence; the fixed prefix is added by the chunk builder.
    pub text: String,
    /// Stable notice code; `None` when the event carried no code.
    pub code: Option<String>,
    /// Typed payload body; rides only when [`detail_is_payload`] matches.
    pub detail: String,
}

/// Prefix kept verbatim from the reference (`notice_meta.go:25`).
pub const WARNING_PREFIX: &str = "\n\n[warning] ";

fn text_block(text: &str) -> ContentBlock {
    ContentBlock {
        block_type: "text".to_owned(),
        text: Some(text.to_owned()),
        ..Default::default()
    }
}

/// Builds the warning chunk the ACP client receives (`warningChunk`,
/// `notice_meta.go:22-38`).
///
/// * no code → `metadata` is omitted entirely;
/// * coded notice → `notice.code` rides; `notice.payload` rides only when the
///   code defines a payload, and the omitted field is dropped from the wire.
pub fn warning_chunk(event: &WarningEvent) -> SessionUpdate {
    let content = text_block(&format!("{WARNING_PREFIX}{}", event.text));
    let Some(code) = event.code.as_deref() else {
        return SessionUpdate::AgentMessageChunk {
            message_id: None,
            content,
            metadata: None,
        };
    };

    let notice = UpdateNotice {
        code: code.to_owned(),
        payload: detail_is_payload(code).then(|| event.detail.clone()),
    };
    SessionUpdate::AgentMessageChunk {
        message_id: None,
        content,
        metadata: Some(UpdateMetadata {
            error: None,
            notice: Some(notice),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncode_notice_carries_no_metadata() {
        let event = WarningEvent {
            text: "something looks off".to_owned(),
            code: None,
            detail: String::new(),
        };
        let value = serde_json::to_value(warning_chunk(&event)).unwrap();
        assert_eq!(value["sessionUpdate"], "agent_message_chunk");
        assert_eq!(
            value["content"]["text"],
            "\n\n[warning] something looks off"
        );
        assert!(value.get("metadata").is_none());
    }

    #[test]
    fn coded_non_payload_notice_omits_payload_field() {
        let event = WarningEvent {
            text: "plan approved".to_owned(),
            code: Some("plan_auto_approved".to_owned()),
            detail: "ignored-on-wire".to_owned(),
        };
        let value = serde_json::to_value(warning_chunk(&event)).unwrap();
        let metadata = value["metadata"].as_object().unwrap();
        assert!(metadata.get("error").is_none());
        let notice = &metadata["notice"];
        assert_eq!(notice["code"], "plan_auto_approved");
        assert!(notice.get("payload").is_none());
    }

    #[test]
    fn payload_code_rides_detail_as_payload() {
        let event = WarningEvent {
            text: "one extension was skipped".to_owned(),
            code: Some("extension_skipped".to_owned()),
            detail: r#"{"extension":"git","point":"prompt","reason":"no_live_sidecar"}"#.to_owned(),
        };
        let value = serde_json::to_value(warning_chunk(&event)).unwrap();
        let notice = &value["metadata"]["notice"];
        assert_eq!(notice["code"], "extension_skipped");
        assert_eq!(
            notice["payload"],
            r#"{"extension":"git","point":"prompt","reason":"no_live_sidecar"}"#
        );
    }

    #[test]
    fn payload_code_set_matches_reference_fifteen() {
        assert_eq!(PAYLOAD_CODES.len(), 15);
        assert!(detail_is_payload("job_stalled"));
        assert!(!detail_is_payload("compacted"));
    }
}
