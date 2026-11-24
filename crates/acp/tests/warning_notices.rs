//! ACP-040 behavior coverage: warning chunk wire shape, payload-code gating,
//! and metadata omission rules (`notice_meta.go:7-38`).

use reasonix_acp::update::SessionUpdate;
use reasonix_acp::warnings::{WarningEvent, detail_is_payload, warning_chunk};

fn serialized(event: &WarningEvent) -> serde_json::Value {
    serde_json::to_value(warning_chunk(event)).unwrap()
}

#[test]
fn uncoded_warning_chunk_has_no_metadata_field() {
    let value = serialized(&WarningEvent {
        text: "one thing needs attention".to_owned(),
        code: None,
        detail: String::new(),
    });
    assert_eq!(value["sessionUpdate"], "agent_message_chunk");
    assert_eq!(value["content"]["type"], "text");
    assert_eq!(
        value["content"]["text"],
        "\n\n[warning] one thing needs attention"
    );
    assert!(value.get("messageId").is_none());
    assert!(value.get("metadata").is_none());
}

#[test]
fn coded_non_payload_notice_drops_payload_field_wire_side() {
    let value = serialized(&WarningEvent {
        text: "plan was approved automatically".to_owned(),
        code: Some("plan_auto_approved".to_owned()),
        detail: "must not ride".to_owned(),
    });
    let metadata = value["metadata"].as_object().expect("metadata rides");
    assert!(metadata.get("error").is_none());
    let notice = &metadata["notice"];
    assert_eq!(notice["code"], "plan_auto_approved");
    assert!(notice.get("payload").is_none());
}

#[test]
fn payload_code_carries_detail_into_notice_payload() {
    let detail = r#"{"extension":"git","point":"prompt","reason":"no_live_sidecar"}"#;
    let value = serialized(&WarningEvent {
        text: "one extension was skipped".to_owned(),
        code: Some("extension_skipped".to_owned()),
        detail: detail.to_owned(),
    });
    let notice = &value["metadata"]["notice"];
    assert_eq!(notice["code"], "extension_skipped");
    assert_eq!(notice["payload"], detail);
}

#[test]
fn every_payload_code_gates_detail_and_round_trips() {
    for code in [
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
    ] {
        assert!(detail_is_payload(code));
        let value = serialized(&WarningEvent {
            text: "notice".to_owned(),
            code: Some(code.to_owned()),
            detail: "payload-body".to_owned(),
        });
        assert_eq!(value["metadata"]["notice"]["payload"], "payload-body");
        let back: SessionUpdate = serde_json::from_value(value).unwrap();
        let SessionUpdate::AgentMessageChunk { metadata, .. } = back else {
            panic!("warning chunk stays agent_message_chunk");
        };
        assert_eq!(
            metadata.unwrap().notice.unwrap().payload,
            Some("payload-body".to_owned())
        );
    }
    assert!(!detail_is_payload("compacted"));
    assert!(!detail_is_payload(""));
}

#[test]
fn two_leading_newlines_are_preserved_verbatim() {
    let value = serialized(&WarningEvent {
        text: "body".to_owned(),
        code: Some("x".to_owned()),
        detail: String::new(),
    });
    let text = value["content"]["text"].as_str().unwrap();
    assert!(text.starts_with("\n\n[warning] "));
    assert_eq!(text, "\n\n[warning] body");
}
