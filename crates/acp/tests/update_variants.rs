//! ACP-008: all eleven `session/update` variants, byte-level envelope shape.

use reasonix_acp::update::{
    AvailableCommand, AvailableCommandInput, PlanEntry, SessionUpdate, SessionUpdateParams,
    TodoDraft, ToolCallLocation, ToolContent, UpdateError, UpdateMetadata, UsageCost,
    build_usage_update, map_plan_entries, warning_text,
};
use reasonix_acp::wire::ContentBlock;

fn text_block(text: &str) -> ContentBlock {
    ContentBlock {
        block_type: "text".to_owned(),
        text: Some(text.to_owned()),
        ..Default::default()
    }
}

fn envelope(update: SessionUpdate) -> serde_json::Value {
    let params = SessionUpdateParams {
        session_id: "s1".to_owned(),
        update,
    };
    serde_json::to_value(&params).unwrap()
}

#[test]
fn three_message_chunk_variants_round_trip() {
    for (discriminant, variant) in [
        (
            "agent_message_chunk",
            SessionUpdate::AgentMessageChunk {
                message_id: Some("a1b2c3d4e5f6-1".to_owned()),
                content: text_block("hello"),
                metadata: None,
            },
        ),
        (
            "agent_thought_chunk",
            SessionUpdate::AgentThoughtChunk {
                message_id: None,
                content: text_block("thinking"),
                metadata: None,
            },
        ),
        (
            "user_message_chunk",
            SessionUpdate::UserMessageChunk {
                message_id: Some("a1b2c3d4e5f6-r1".to_owned()),
                content: text_block("replayed user"),
                metadata: None,
            },
        ),
    ] {
        let v = envelope(variant);
        assert_eq!(v["update"]["sessionUpdate"], discriminant);
        assert_eq!(v["update"]["content"]["type"], "text");
        let back: SessionUpdateParams = serde_json::from_value(v).unwrap();
        assert_eq!(back.update.discriminant(), discriminant);
    }
}

#[test]
fn chunk_metadata_error_serializes_when_present() {
    let update = SessionUpdate::AgentMessageChunk {
        message_id: None,
        content: text_block("..."),
        metadata: Some(UpdateMetadata {
            error: Some(UpdateError {
                name: "UpstreamError".to_owned(),
                message: "boom".to_owned(),
            }),
            notice: None,
        }),
    };
    let v = envelope(update);
    assert_eq!(v["update"]["metadata"]["error"]["name"], "UpstreamError");
    assert_eq!(v["update"]["metadata"]["error"]["message"], "boom");
}

#[test]
fn extension_surface_variant_lands_meta_path() {
    let mut meta = std::collections::BTreeMap::new();
    meta.insert(
        "reasonix.io".to_owned(),
        serde_json::json!({"extensionSurface": {"kind": "card", "pluginId": "alpha"}}),
    );
    let update = SessionUpdate::ExtensionSurface { meta };
    let v = envelope(update);
    assert_eq!(
        v["update"]["sessionUpdate"],
        "_reasonix.io/extension_surface"
    );
    assert_eq!(
        v["update"]["_meta"]["reasonix.io"]["extensionSurface"]["pluginId"],
        "alpha"
    );
}

#[test]
fn available_commands_update_shape() {
    let update = SessionUpdate::AvailableCommands {
        available_commands: vec![AvailableCommand {
            name: "/review".to_owned(),
            description: "Review diff".to_owned(),
            input: Some(AvailableCommandInput {
                hint: "diff".to_owned(),
            }),
        }],
    };
    let v = envelope(update);
    assert_eq!(v["update"]["sessionUpdate"], "available_commands_update");
    assert_eq!(v["update"]["availableCommands"][0]["name"], "/review");
    assert_eq!(v["update"]["availableCommands"][0]["input"]["hint"], "diff");
}

#[test]
fn config_option_update_is_full_refresh() {
    let update = SessionUpdate::ConfigOptions {
        config_options: vec![reasonix_acp::wire::SessionConfigOption {
            id: "tool_approval".to_owned(),
            name: "Tool Approval".to_owned(),
            description: None,
            category: Some("tool_approval".to_owned()),
            r#type: "select".to_owned(),
            current_value: "ask".to_owned(),
            options: vec![],
        }],
    };
    let v = envelope(update);
    assert_eq!(v["update"]["sessionUpdate"], "config_option_update");
    assert_eq!(v["update"]["configOptions"][0]["id"], "tool_approval");
}

#[test]
fn plan_variant_carries_full_entries() {
    let entries = map_plan_entries(&[TodoDraft {
        content: "Phase one".to_owned(),
        status: "in_progress".to_owned(),
        level: 0,
    }]);
    let update = SessionUpdate::Plan { entries };
    let v = envelope(update);
    assert_eq!(v["update"]["sessionUpdate"], "plan");
    assert_eq!(v["update"]["entries"][0]["priority"], "high");
    assert_eq!(v["update"]["entries"][0]["status"], "in_progress");
}

#[test]
fn current_mode_update_shape() {
    let update = SessionUpdate::CurrentMode {
        current_mode_id: "plan".to_owned(),
    };
    let v = envelope(update);
    assert_eq!(v["update"]["sessionUpdate"], "current_mode_update");
    assert_eq!(v["update"]["currentModeId"], "plan");
}

#[test]
fn tool_call_pending_shape_and_locations() {
    let update = SessionUpdate::ToolCall {
        tool_call_id: "tc-1".to_owned(),
        name: Some("read_file".to_owned()),
        title: Some("read_file".to_owned()),
        kind: Some("read".to_owned()),
        status: Some("pending".to_owned()),
        raw_input: Some(serde_json::json!({"path": "pkg/a.go"})),
        locations: Some(vec![ToolCallLocation {
            path: "/proj/pkg/a.go".to_owned(),
            line: Some(42),
        }]),
    };
    let v = envelope(update);
    assert_eq!(v["update"]["sessionUpdate"], "tool_call");
    assert_eq!(v["update"]["toolCallId"], "tc-1");
    assert_eq!(v["update"]["locations"][0]["line"], 42);
}

#[test]
fn tool_call_update_wraps_single_content_block() {
    let update = SessionUpdate::ToolCallUpdate {
        tool_call_id: "tc-1".to_owned(),
        status: Some("completed".to_owned()),
        content: Some(vec![ToolContent {
            block_type: "content".to_owned(),
            content: text_block("<result>"),
        }]),
    };
    let v = envelope(update);
    assert_eq!(v["update"]["sessionUpdate"], "tool_call_update");
    assert_eq!(v["update"]["content"][0]["type"], "content");
    assert_eq!(v["update"]["content"][0]["content"]["text"], "<result>");
}

#[test]
fn usage_update_gates_and_cost_shape() {
    let gated = build_usage_update(1200, 0, None);
    assert!(gated.is_none());

    let update = build_usage_update(
        1200,
        8000,
        Some(UsageCost {
            amount: 0.03,
            currency: "USD".to_owned(),
        }),
    )
    .unwrap();
    let v = envelope(update);
    assert_eq!(v["update"]["sessionUpdate"], "usage_update");
    assert_eq!(v["update"]["used"], 1200);
    assert_eq!(v["update"]["size"], 8000);
    assert_eq!(v["update"]["cost"]["amount"], 0.03);
    assert_eq!(v["update"]["cost"]["currency"], "USD");
}

#[test]
fn warning_text_keeps_two_leading_newlines() {
    let text = warning_text("disk nearly full");
    assert!(text.starts_with("\n\n[warning] "));
    assert_eq!(text, "\n\n[warning] disk nearly full");
}

#[test]
fn plan_entry_normalization_rules() {
    let entries = map_plan_entries(&[
        TodoDraft {
            content: "A".to_owned(),
            status: "completed".to_owned(),
            level: 0,
        },
        TodoDraft {
            content: "  ".to_owned(),
            status: "pending".to_owned(),
            level: 3,
        },
        TodoDraft {
            content: "B".to_owned(),
            status: "weird".to_owned(),
            level: 1,
        },
    ]);
    assert_eq!(
        entries,
        vec![
            PlanEntry {
                content: "A".to_owned(),
                priority: "high".to_owned(),
                status: "completed".to_owned(),
            },
            PlanEntry {
                content: "B".to_owned(),
                priority: "medium".to_owned(),
                status: "pending".to_owned(),
            },
        ]
    );
}
