//! ACP-020: `.acp.json` sidecar schema, path derivation, load backfill, list
//! beat rule, and decode error identity.

use reasonix_acp::sidecar::{
    AcpSessionMeta, backfill_loaded, decode_error, list_meta_beats, meta_path, preview_title,
};
use serde_json::json;

#[test]
fn populated_sidecar_serializes_twelve_field_order() {
    let meta = AcpSessionMeta {
        session_id: "s1".to_owned(),
        cwd: "/proj".to_owned(),
        model: Some("anthropic/claude".to_owned()),
        effort_override: Some(String::new()),
        runtime_profile: Some("economy".to_owned()),
        tool_approval_mode: Some("ask".to_owned()),
        collaboration_mode: Some("normal".to_owned()),
        title: Some("first prompt".to_owned()),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: "2026-01-02T00:00:00Z".to_owned(),
        status: Some(json!({"sequence": 1})),
        active_transcript: Some("branch.jsonl".to_owned()),
    };
    let text = serde_json::to_string(&meta).unwrap();
    let expected = "{\"sessionId\":\"s1\",\"cwd\":\"/proj\",\"model\":\"anthropic/claude\",\"effortOverride\":\"\",\"runtimeProfile\":\"economy\",\"toolApprovalMode\":\"ask\",\"collaborationMode\":\"normal\",\"title\":\"first prompt\",\"createdAt\":\"2026-01-01T00:00:00Z\",\"updatedAt\":\"2026-01-02T00:00:00Z\",\"status\":{\"sequence\":1},\"activeTranscript\":\"branch.jsonl\"}";
    assert_eq!(text, expected);
}

#[test]
fn empty_sidecar_omits_optional_fields_and_has_no_version() {
    let text = serde_json::to_string(&AcpSessionMeta::default()).unwrap();
    assert_eq!(
        text,
        "{\"sessionId\":\"\",\"cwd\":\"\",\"createdAt\":\"\",\"updatedAt\":\"\"}"
    );
    assert!(!text.contains("schemaVersion"));
}

#[test]
fn meta_path_strips_transcript_extension() {
    assert_eq!(meta_path("/sessions/abc.jsonl"), "/sessions/abc.acp.json");
    assert_eq!(meta_path("plain"), "plain.acp.json");
}

#[test]
fn title_preview_normalizes_space_and_caps_runes() {
    assert_eq!(preview_title("  one   two  "), "one two");
    assert_eq!(preview_title("short"), "short");
    let long = "汉".repeat(100);
    let preview = preview_title(&long);
    assert!(preview.ends_with("..."));
    assert_eq!(preview.chars().count(), 80);
}

#[test]
fn backfill_fills_loaded_session_fields() {
    let filled = backfill_loaded(
        AcpSessionMeta::default(),
        "s1",
        "/proj",
        "history title",
        "2026-01-01T00:00:00Z",
    );
    assert_eq!(filled.session_id, "s1");
    assert_eq!(filled.cwd, "/proj");
    assert_eq!(filled.title.as_deref(), Some("history title"));
    assert_eq!(filled.created_at, "2026-01-01T00:00:00Z");
    assert_eq!(filled.updated_at, "2026-01-01T00:00:00Z");
}

#[test]
fn live_sidecar_beats_redirect_recency_breaks_same_kind_tie() {
    let live = AcpSessionMeta {
        active_transcript: None,
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
        ..Default::default()
    };
    let redirect = AcpSessionMeta {
        active_transcript: Some("branch.jsonl".to_owned()),
        updated_at: "2026-02-01T00:00:00Z".to_owned(),
        ..Default::default()
    };
    assert!(list_meta_beats(&live, &redirect));
    assert!(!list_meta_beats(&redirect, &live));

    let older = AcpSessionMeta {
        updated_at: "2026-01-01T00:00:00Z".to_owned(),
        ..Default::default()
    };
    let newer = AcpSessionMeta {
        updated_at: "2026-01-02T00:00:00Z".to_owned(),
        ..Default::default()
    };
    assert!(list_meta_beats(&newer, &older));
}

#[test]
fn decode_error_is_verbatim() {
    assert_eq!(
        decode_error("/sessions/s1.acp.json", "invalid character 'x'"),
        "decode ACP session metadata /sessions/s1.acp.json: invalid character 'x'"
    );
}
