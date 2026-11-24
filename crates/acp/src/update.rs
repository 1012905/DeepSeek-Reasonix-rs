//! `session/update` notification variants (protocol.go:576-664; toolcall_wire.go:
//! 1-36; usage_update.go:5-28).
//!
//! Eleven variants ride one envelope: `{sessionId, update}` where the
//! discriminator is `update.sessionUpdate`. The list here follows what the
//! reference actually emits; the contract index omits `usage_update` and the
//! replay-only `user_message_chunk` (matrix B §2.2, conflict C6).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::wire::{ContentBlock, SessionConfigOption};

/// `session/update` notification params (protocol.go:583-586).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionUpdateParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub update: SessionUpdate,
}

/// One tool-call location pointer (toolcall_wire.go:19-22).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolCallLocation {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i32>,
}

/// Wrapper around a content block carried in tool results (toolcall_wire.go:
/// 33-36).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub block_type: String,
    pub content: ContentBlock,
}

/// `metadata.error` on a failed chunk (protocol.go:613-617).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdateError {
    pub name: String,
    pub message: String,
}

/// Coded notice identity on a warning chunk; the payload rides only when the
/// code defines one (notice_meta.go:27-37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdateNotice {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
}

/// Optional chunk metadata; omitted when it carries neither an error nor a
/// coded notice (protocol.go:610-612; notice_meta.go:22-38).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdateMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<UpdateError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<UpdateNotice>,
}

/// One command offered through the slash surface (protocol.go:628-633).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AvailableCommandInput {
    pub hint: String,
}

/// One offered slash command (protocol.go:628-633).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<AvailableCommandInput>,
}

/// One plan row; each `plan` update carries the full plan and replaces the
/// previous one (protocol.go:645-651, 654-658).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

/// Accumulated usage cost, carried only when both fields exist
/// (usage_update.go:26-28).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UsageCost {
    pub amount: f64,
    pub currency: String,
}

/// Every `session/update` discriminator the reference emits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "sessionUpdate")]
pub enum SessionUpdate {
    #[allow(clippy::large_enum_variant)]
    /// Live assistant text delta.
    #[serde(rename = "agent_message_chunk")]
    AgentMessageChunk {
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        content: ContentBlock,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<UpdateMetadata>,
    },
    /// Live reasoning delta.
    #[serde(rename = "agent_thought_chunk")]
    AgentThoughtChunk {
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        content: ContentBlock,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<UpdateMetadata>,
    },
    /// Replay-only re-emission of an earlier user turn.
    #[serde(rename = "user_message_chunk")]
    UserMessageChunk {
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        content: ContentBlock,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<UpdateMetadata>,
    },
    /// Structured extension-UI surface (negotiated; text fallback still sent).
    #[serde(rename = "_reasonix.io/extension_surface")]
    ExtensionSurface {
        #[serde(rename = "_meta")]
        meta: BTreeMap<String, Value>,
    },
    /// Slash command list refresh.
    #[serde(rename = "available_commands_update")]
    AvailableCommands {
        #[serde(rename = "availableCommands")]
        available_commands: Vec<AvailableCommand>,
    },
    /// Full config-option list refresh.
    #[serde(rename = "config_option_update")]
    ConfigOptions {
        #[serde(rename = "configOptions")]
        config_options: Vec<SessionConfigOption>,
    },
    /// Full plan replacement.
    #[serde(rename = "plan")]
    Plan { entries: Vec<PlanEntry> },
    /// Current collaboration mode changed.
    #[serde(rename = "current_mode_update")]
    CurrentMode {
        #[serde(rename = "currentModeId")]
        current_mode_id: String,
    },
    /// Pending (or replay-completed) tool call card.
    #[serde(rename = "tool_call")]
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(rename = "rawInput", skip_serializing_if = "Option::is_none")]
        raw_input: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<ToolCallLocation>>,
    },
    /// Tool call result / failure update.
    #[serde(rename = "tool_call_update")]
    ToolCallUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ToolContent>>,
    },
    /// Token/cost snapshot synthesized after a turn, not by a Usage event.
    #[serde(rename = "usage_update")]
    Usage {
        used: i64,
        size: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost: Option<UsageCost>,
    },
}

/// A raw plan row before ACP normalization (dispatch.go:768-800).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TodoDraft {
    pub content: String,
    pub status: String,
    /// Todo level; `0` maps to high priority, anything else to medium.
    pub level: i32,
}

/// Normalizes tool todos into plan entries: priority by level, status whitelist,
/// empty content dropped. An empty result means no plan update is sent.
pub fn map_plan_entries(todos: &[TodoDraft]) -> Vec<PlanEntry> {
    todos
        .iter()
        .filter(|todo| !todo.content.trim().is_empty())
        .map(|todo| {
            let priority = if todo.level == 0 { "high" } else { "medium" };
            let status = match todo.status.as_str() {
                "pending" | "in_progress" | "completed" => todo.status.as_str(),
                _ => "pending",
            };
            PlanEntry {
                content: todo.content.clone(),
                priority: priority.to_owned(),
                status: status.to_owned(),
            }
        })
        .collect()
}

/// Prefix for a warning chunk text. The two leading newlines are part of the
/// user-visible contract (notice_meta.go:25, matrix B §6.4, conflict C9).
pub const WARNING_PREFIX: &str = "\n\n[warning] ";

/// Builds the warning chunk text with the fixed leading prefix.
pub fn warning_text(text: &str) -> String {
    format!("{WARNING_PREFIX}{text}")
}

/// Compaction notice text, emitted only when a summary exists
/// (dispatch.go:225-233).
pub fn compaction_text(messages: i64) -> String {
    format!("\n\n[compacted {messages} earlier messages to save context]")
}

/// Builds a usage update, honoring the emit gates: `size <= 0` suppresses the
/// update, `used` floors at zero, and cost rides only when a priced amount and a
/// non-empty currency both exist (usage_update.go:21-28).
pub fn build_usage_update(used: i64, size: i64, cost: Option<UsageCost>) -> Option<SessionUpdate> {
    if size <= 0 {
        return None;
    }
    let cost = cost.filter(|c| !c.currency.is_empty());
    Some(SessionUpdate::Usage {
        used: used.max(0),
        size,
        cost,
    })
}

/// The discriminator string on the wire, for dispatch-side switches.
impl SessionUpdate {
    pub fn discriminant(&self) -> &'static str {
        match self {
            Self::AgentMessageChunk { .. } => "agent_message_chunk",
            Self::AgentThoughtChunk { .. } => "agent_thought_chunk",
            Self::UserMessageChunk { .. } => "user_message_chunk",
            Self::ExtensionSurface { .. } => "_reasonix.io/extension_surface",
            Self::AvailableCommands { .. } => "available_commands_update",
            Self::ConfigOptions { .. } => "config_option_update",
            Self::Plan { .. } => "plan",
            Self::CurrentMode { .. } => "current_mode_update",
            Self::ToolCall { .. } => "tool_call",
            Self::ToolCallUpdate { .. } => "tool_call_update",
            Self::Usage { .. } => "usage_update",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_block(text: &str) -> ContentBlock {
        ContentBlock {
            block_type: "text".to_owned(),
            text: Some(text.to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn message_chunk_variants_omit_optional_fields() {
        let update = SessionUpdate::AgentThoughtChunk {
            message_id: None,
            content: text_block("thinking"),
            metadata: None,
        };
        let value = serde_json::to_value(&update).unwrap();
        assert_eq!(value["sessionUpdate"], "agent_thought_chunk");
        assert!(value.get("messageId").is_none());
        assert!(value.get("metadata").is_none());
        assert_eq!(value["content"]["text"], "thinking");

        let with_meta = SessionUpdate::AgentMessageChunk {
            message_id: Some("a1b2c3d4e5f6-1".to_owned()),
            content: text_block("hello"),
            metadata: Some(UpdateMetadata {
                error: Some(UpdateError {
                    name: "UpstreamError".to_owned(),
                    message: "boom".to_owned(),
                }),
                notice: None,
            }),
        };
        let value = serde_json::to_value(&with_meta).unwrap();
        assert_eq!(value["messageId"], "a1b2c3d4e5f6-1");
        assert_eq!(value["metadata"]["error"]["name"], "UpstreamError");
    }

    #[test]
    fn plan_entries_normalize_priority_and_status() {
        let entries = map_plan_entries(&[
            TodoDraft {
                content: "Phase one".to_owned(),
                status: "in_progress".to_owned(),
                level: 0,
            },
            TodoDraft {
                content: "".to_owned(),
                status: "pending".to_owned(),
                level: 1,
            },
            TodoDraft {
                content: "Phase two".to_owned(),
                status: "weird".to_owned(),
                level: 2,
            },
        ]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].priority, "high");
        assert_eq!(entries[0].status, "in_progress");
        assert_eq!(entries[1].priority, "medium");
        assert_eq!(entries[1].status, "pending");
    }

    #[test]
    fn usage_update_gates_size_used_floor_and_cost() {
        let gated = build_usage_update(-5, 0, None);
        assert!(gated.is_none());

        let plain = build_usage_update(1200, 8000, None).unwrap();
        let SessionUpdate::Usage { used, size, cost } = plain else {
            panic!()
        };
        assert_eq!(used, 1200);
        assert_eq!(size, 8000);
        assert!(cost.is_none());

        let floored = build_usage_update(-9, 100, None).unwrap();
        let SessionUpdate::Usage { used, .. } = floored else {
            panic!()
        };
        assert_eq!(used, 0);

        let no_currency = build_usage_update(
            1,
            100,
            Some(UsageCost {
                amount: 0.03,
                currency: String::new(),
            }),
        )
        .unwrap();
        let SessionUpdate::Usage { cost, .. } = no_currency else {
            panic!()
        };
        assert!(cost.is_none());

        let zero_amount = build_usage_update(
            1,
            100,
            Some(UsageCost {
                amount: 0.0,
                currency: "USD".to_owned(),
            }),
        )
        .unwrap();
        let SessionUpdate::Usage { cost, .. } = zero_amount else {
            panic!()
        };
        assert!(cost.is_some());
    }

    #[test]
    fn notice_metadata_carries_code_and_optional_payload() {
        let meta = UpdateMetadata {
            error: None,
            notice: Some(UpdateNotice {
                code: "rate_limited".to_owned(),
                payload: Some("42".to_owned()),
            }),
        };
        let value = serde_json::to_value(&meta).unwrap();
        assert_eq!(value["notice"]["code"], "rate_limited");
        assert_eq!(value["notice"]["payload"], "42");

        let bare = UpdateNotice {
            code: "warning".to_owned(),
            payload: None,
        };
        let value = serde_json::to_value(&bare).unwrap();
        assert!(value.get("payload").is_none());
    }

    #[test]
    fn compaction_text_matches_fixed_sentence() {
        assert_eq!(
            compaction_text(12),
            "\n\n[compacted 12 earlier messages to save context]"
        );
    }

    #[test]
    fn tool_call_update_content_wraps_single_content() {
        let update = SessionUpdate::ToolCallUpdate {
            tool_call_id: "tc-1".to_owned(),
            status: Some("completed".to_owned()),
            content: Some(vec![ToolContent {
                block_type: "content".to_owned(),
                content: text_block("<result>"),
            }]),
        };
        let value = serde_json::to_value(&update).unwrap();
        assert_eq!(value["sessionUpdate"], "tool_call_update");
        assert_eq!(value["content"][0]["type"], "content");
        assert_eq!(value["content"][0]["content"]["text"], "<result>");
    }

    #[test]
    fn envelope_carries_session_id_and_discriminant() {
        let params = SessionUpdateParams {
            session_id: "s1".to_owned(),
            update: SessionUpdate::CurrentMode {
                current_mode_id: "plan".to_owned(),
            },
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["sessionId"], "s1");
        assert_eq!(value["update"]["sessionUpdate"], "current_mode_update");
        assert_eq!(value["update"]["currentModeId"], "plan");
    }
}
