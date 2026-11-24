//! ACP-013: tool call / update wire shapes and message id rules.

use reasonix_acp::dispatch::{raw_input, tool_locations};
use reasonix_acp::message_ids::MessageIds;
use reasonix_acp::update::{SessionUpdate, ToolCallLocation, ToolContent};
use reasonix_acp::wire::ContentBlock;

#[test]
fn message_ids_match_live_and_replay_shapes() {
    let mut ids = MessageIds::with_prefix("a1b2c3d4e5f6").unwrap();
    assert_eq!(ids.live(1, 1).unwrap(), "a1b2c3d4e5f6-sa-1-1");
    assert_eq!(ids.live(2, 3).unwrap(), "a1b2c3d4e5f6-sa-2-3");
    assert_eq!(ids.replayed(), "a1b2c3d4e5f6-r1");
    assert_eq!(ids.replayed(), "a1b2c3d4e5f6-r2");
    assert_eq!(ids.live(0, 1), None);
}

#[test]
fn tool_call_card_serializes_declared_field_order() {
    let update = SessionUpdate::ToolCall {
        tool_call_id: "tc-1".to_owned(),
        name: Some("read_file".to_owned()),
        title: Some("read_file".to_owned()),
        kind: Some("read".to_owned()),
        status: Some("pending".to_owned()),
        raw_input: raw_input(r#"{"path":"src/lib.rs"}"#),
        locations: Some(vec![ToolCallLocation {
            path: "/proj/src/lib.rs".to_owned(),
            line: Some(1),
        }]),
    };
    let text = serde_json::to_string(&update).unwrap();
    let expected = "{\"sessionUpdate\":\"tool_call\",\"toolCallId\":\"tc-1\",\"name\":\"read_file\",\"title\":\"read_file\",\"kind\":\"read\",\"status\":\"pending\",\"rawInput\":{\"path\":\"src/lib.rs\"},\"locations\":[{\"path\":\"/proj/src/lib.rs\",\"line\":1}]}";
    assert_eq!(text, expected);
}

#[test]
fn tool_call_card_omits_absent_raw_input_and_locations() {
    let update = SessionUpdate::ToolCall {
        tool_call_id: "tc-2".to_owned(),
        name: None,
        title: None,
        kind: None,
        status: None,
        raw_input: None,
        locations: None,
    };
    let text = serde_json::to_string(&update).unwrap();
    assert_eq!(
        text,
        "{\"sessionUpdate\":\"tool_call\",\"toolCallId\":\"tc-2\"}"
    );
}

#[test]
fn tool_call_update_wraps_each_content_block_kind() {
    let text = ContentBlock {
        block_type: "text".to_owned(),
        text: Some("result".to_owned()),
        ..Default::default()
    };
    let resource = ContentBlock {
        block_type: "resource".to_owned(),
        resource: Some(reasonix_acp::wire::ResourceContents {
            uri: "file:///proj/a.go".to_owned(),
            mime_type: Some("text/x-go".to_owned()),
            text: Some("package main".to_owned()),
            blob: None,
        }),
        ..Default::default()
    };
    let update = SessionUpdate::ToolCallUpdate {
        tool_call_id: "tc-1".to_owned(),
        status: Some("completed".to_owned()),
        content: Some(vec![
            ToolContent {
                block_type: "content".to_owned(),
                content: text,
            },
            ToolContent {
                block_type: "content".to_owned(),
                content: resource,
            },
        ]),
    };
    let value = serde_json::to_value(&update).unwrap();
    assert_eq!(value["sessionUpdate"], "tool_call_update");
    assert_eq!(value["content"][0]["content"]["type"], "text");
    assert_eq!(value["content"][1]["content"]["type"], "resource");
    assert_eq!(
        value["content"][1]["content"]["resource"]["uri"],
        "file:///proj/a.go"
    );
}

#[test]
fn image_and_audio_blocks_serialize_mime_and_data() {
    let image = ContentBlock {
        block_type: "image".to_owned(),
        mime_type: Some("image/png".to_owned()),
        data: Some("iVBORw0KGgo=".to_owned()),
        ..Default::default()
    };
    let audio = ContentBlock {
        block_type: "audio".to_owned(),
        mime_type: Some("audio/wav".to_owned()),
        data: Some("UklGRg==".to_owned()),
        ..Default::default()
    };
    let iv = serde_json::to_value(&image).unwrap();
    assert_eq!(iv["type"], "image");
    assert_eq!(iv["mimeType"], "image/png");
    assert!(iv.get("text").is_none());
    let av = serde_json::to_value(&audio).unwrap();
    assert_eq!(av["type"], "audio");
    assert_eq!(av["mimeType"], "audio/wav");
}

#[test]
fn locations_derive_from_args_and_follow_whitelist() {
    let locs = tool_locations("read_file", r#"{"path":"a.go","offset":0}"#, "/proj").unwrap();
    assert_eq!(locs[0].path, "/proj/a.go");
    assert!(locs[0].line.is_none());
    assert!(tool_locations("grep", r#"{"path":"src"}"#, "/proj").is_none());
}
