//! Non-streaming chat-completions response decoding.
//!
//! The Go adapter always streams (`stream: true` is unconditional), so this
//! decoder is the one extension beyond the reference: the same tolerant frame
//! semantics and the same usage normalization, applied to the standard
//! one-shot `/chat/completions` JSON body the endpoints also serve.

use reasonix_contract::provider::ToolCall;
use serde::Deserialize;

use crate::usage::{Usage, normalize_usage};
use crate::wire::{ChatToolCall, StreamWireError};

/// One choice of a non-streaming completion (Go analog: the accumulated
/// stream's final frame).
#[derive(Debug, Clone, Default, Deserialize)]
struct CompletionChoice {
    #[serde(default)]
    message: CompletionMessage,
    #[serde(default, rename = "finish_reason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CompletionMessage {
    #[serde(default)]
    content: String,
    #[serde(default, rename = "reasoning_content")]
    reasoning_content: String,
    #[serde(default, rename = "reasoning")]
    reasoning: String,
    #[serde(default, rename = "tool_calls")]
    tool_calls: Vec<ChatToolCall>,
}

/// The one-shot body shape (Go analog: `streamResponse`).
#[derive(Debug, Clone, Default, Deserialize)]
struct CompletionBody {
    #[serde(default)]
    choices: Vec<CompletionChoice>,
    #[serde(default)]
    usage: Option<crate::usage::WireUsage>,
    #[serde(default)]
    error: Option<StreamWireError>,
}

/// A complete non-streaming response (Go analog: the accumulated chunks of
/// one stream).
#[derive(Debug, Clone, Default)]
pub struct CompletionResponse {
    /// The visible answer text.
    pub text: String,
    /// Chain-of-thought text (`reasoning_content`, falling back to
    /// `reasoning`).
    pub reasoning: String,
    /// Complete tool calls, in the order the response carried them.
    pub tool_calls: Vec<ToolCall>,
    /// The terminal finish reason.
    pub finish_reason: String,
    /// Normalized token accounting.
    pub usage: Usage,
    /// A gateway error envelope inside a 200 body, if it sent one (Go:
    /// `StreamPayloadError` on the streaming path).
    pub error: Option<StreamWireError>,
}

/// Decodes a one-shot completion body. Unknown fields are ignored; missing
/// fields are zero (Go: `json.Unmarshal` tolerance).
pub fn decode_completion(body: &str) -> Result<CompletionResponse, serde_json::Error> {
    let parsed: CompletionBody = serde_json::from_str(body)?;
    let mut out = CompletionResponse {
        error: parsed.error,
        ..CompletionResponse::default()
    };
    if let Some(choice) = parsed.choices.first() {
        out.finish_reason = choice.finish_reason.clone().unwrap_or_default();
        let message = &choice.message;
        out.reasoning = if message.reasoning_content.is_empty() {
            message.reasoning.clone()
        } else {
            message.reasoning_content.clone()
        };
        out.text = message.content.clone();
        out.tool_calls = message
            .tool_calls
            .iter()
            .map(|tc| ToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments: tc.function.arguments.clone(),
                thought_signature: tc.extra_content.as_ref().map_or_else(
                    || tc.function.thought_signature.clone(),
                    |extra| extra.google.thought_signature.clone(),
                ),
                ..ToolCall::default()
            })
            .collect();
    }
    if let Some(usage) = &parsed.usage {
        let mut u = normalize_usage(usage);
        u.finish_reason = out.finish_reason.clone();
        out.usage = u;
    }
    Ok(out)
}
