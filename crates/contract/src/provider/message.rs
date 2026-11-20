//! The stored provider message (Go: `provider.go` `Message`).

use serde_json::value::RawValue;

use crate::agentcomposition;
use crate::gojson::{go_raw_vec, go_struct, go_vec, is_zero_bool, is_zero_i64, null_default};
use crate::provider::Role;
use crate::provider::local_metadata::{
    DecisionReceipt, InterruptedTurnRecovery, MemoryCitation, ToolExecution, ToolFailure,
    UnavailableImage, Via,
};
use crate::provider::tool_call::ToolCall;
use crate::provider::video::VideoRef;

/// Placeholder a repaired unanswered tool call receives (Go:
/// `interruptedToolResult`).
pub const INTERRUPTED_TOOL_RESULT: &str =
    "[no result: the previous turn was interrupted before this tool call completed]";

go_struct! {
    /// One stored message (Go: `Message`); field order matches the Go
    /// struct so serialized keys stay byte-compatible.
    pub struct Message : MessageWire {
        /// Live composition binding attached when the agent composed this turn.
        #[serde(
            rename = "agent_composition",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub agent_composition: Option<agentcomposition::Binding>,
        #[serde(rename = "role", default, deserialize_with = "null_default")]
        pub role: Role,
        /// Legacy text field (Go: `Content`).
        #[serde(
            rename = "content",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub content: String,
        /// User text before host-injected context (Go: `RawContent`).
        #[serde(
            rename = "raw_content",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub raw_content: String,
        /// Transitional Context Engine v2 field (Go: `ProviderContent`).
        #[serde(
            rename = "provider_content",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub provider_content: String,
        /// Image data URLs.
        #[serde(
            rename = "images",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub images: Vec<String>,
        /// Video file references.
        #[serde(
            rename = "videos",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub videos: Vec<VideoRef>,
        /// Images whose stored bytes could not be read back.
        #[serde(
            rename = "unavailable_images",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub unavailable_images: Vec<UnavailableImage>,
        /// Thinking-mode chain-of-thought (Go: `ReasoningContent`).
        #[serde(
            rename = "reasoning_content",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub reasoning_content: String,
        /// Provider-issued reasoning item id (Go: `ReasoningID`).
        #[serde(
            rename = "reasoning_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub reasoning_id: String,
        /// `"in_progress" | "completed"` from the server's done event.
        #[serde(
            rename = "reasoning_status",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub reasoning_status: String,
        /// Opaque provider-issued reasoning proof (Go: `ReasoningSignature`).
        #[serde(
            rename = "reasoning_signature",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub reasoning_signature: String,
        /// Tool calls requested by the assistant.
        #[serde(
            rename = "tool_calls",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub tool_calls: Vec<ToolCall>,
        /// Opaque provider Responses API items replayed on stateless follow-up.
        #[serde(
            rename = "responses_items",
            default,
            deserialize_with = "go_raw_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub responses_items: Vec<Box<RawValue>>,
        /// Links a tool result to its call (Go: `ToolCallID`).
        #[serde(
            rename = "tool_call_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub tool_call_id: String,
        /// Tool name on a tool message.
        #[serde(
            rename = "name",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub name: String,
        /// Local memory-citation metadata.
        #[serde(
            rename = "memoryCitations",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub memory_citations: Vec<MemoryCitation>,
        #[serde(
            rename = "workDurationMs",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub work_duration_ms: i64,
        /// Host-measured thinking time.
        #[serde(
            rename = "thoughtMs",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub thought_ms: i64,
        /// Creation time, unix milliseconds.
        #[serde(
            rename = "createdAt",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub created_at: i64,
        /// Whether the user prompt was edited inline.
        #[serde(
            rename = "edited",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub edited: bool,
        /// User prompt before an inline edit.
        #[serde(
            rename = "original",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub original: String,
        /// Marks a host-composed user-role message.
        #[serde(
            rename = "host_authored",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub host_authored: bool,
        /// Model that wrote this assistant turn.
        #[serde(
            rename = "modelRef",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub model_ref: String,
        /// Paired device a user message was sent from.
        #[serde(
            rename = "via",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub via: Option<Via>,
        /// Durable transcript content never sent to a provider.
        #[serde(
            rename = "local_only",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub local_only: bool,
        /// Durable, provider-excluded approval evidence (singular form).
        #[serde(
            rename = "decision_receipt",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub decision_receipt: Option<DecisionReceipt>,
        /// Receipts attached to a provider-visible message (Go:
        /// `DecisionReceipts []*DecisionReceipt`; null elements decode to
        /// `None`).
        #[serde(
            rename = "decision_receipts",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub decision_receipts: Vec<Option<DecisionReceipt>>,
        /// Provider-excluded interrupted-turn handoff.
        #[serde(
            rename = "interrupted_turn",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub interrupted_turn: Option<InterruptedTurnRecovery>,
        /// Local shell UI metadata on tool-result messages.
        #[serde(
            rename = "tool_execution",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub tool_execution: Option<ToolExecution>,
        /// The host's own account of a failed result.
        #[serde(
            rename = "tool_failure",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub tool_failure: Option<ToolFailure>,
    }
}
