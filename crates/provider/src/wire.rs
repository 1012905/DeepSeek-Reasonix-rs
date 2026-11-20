//! The OpenAI-compatible chat wire protocol (Go: `openai/openai.go` wire
//! section). Field order and omitempty behavior are a parity surface: the
//! request body must stay byte-identical to the Go client's so the vendor's
//! prompt cache sees the same prefix.

use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::usage::WireUsage;

/// `reasoning_effort` / `thinking.type` carrier (Go: `thinkingMode`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThinkingMode {
    #[serde(rename = "type")]
    pub kind: String,
}

/// `stream_options` (Go: `streamOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamOptions {
    #[serde(rename = "include_usage")]
    pub include_usage: bool,
}

/// One wire message (Go: `chatMessage`). Field order matches the Go struct:
/// role, content, prefix, reasoning_content, tool_calls, tool_call_id, name.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChatMessage {
    #[serde(rename = "role")]
    pub role: String,
    /// Always present (DeepSeek's strict deserializer rejects a missing
    /// field): `null` for a pure tool_calls assistant turn, a string
    /// otherwise, an array for a vision user turn.
    #[serde(rename = "content")]
    pub content: Option<ChatContent>,
    /// Wire-only marker on an automatic DeepSeek assistant tail; omitempty
    /// keeps every ordinary request byte-stable.
    #[serde(rename = "prefix", skip_serializing_if = "crate::wire::is_false")]
    pub prefix: bool,
    /// `Option` mirrors Go's pointer: `None` omits the key; `Some("")`
    /// serializes an empty string (DeepSeek thinking requires the key present
    /// on assistant tool_calls turns).
    #[serde(rename = "reasoning_content", skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(rename = "tool_calls", skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ChatToolCall>,
    #[serde(rename = "tool_call_id", skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,
    /// Tool messages keep the key even empty for strict backends (MiMo);
    /// ordinary messages omit it (byte-stable prefix).
    #[serde(rename = "name", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `content` shapes (Go: `Content any`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "text", skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(rename = "image_url", skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ChatImageUrl>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatImageUrl {
    #[serde(rename = "url")]
    pub url: String,
    #[serde(rename = "detail", skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// One tool call on the wire (Go: `chatToolCall`). `index` is response-only:
/// requests omit it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatToolCall {
    #[serde(
        rename = "index",
        default,
        skip_serializing_if = "crate::wire::is_zero"
    )]
    pub index: i64,
    #[serde(rename = "id", default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(rename = "type", default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(
        rename = "extra_content",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub extra_content: Option<ChatToolCallExtraContent>,
    #[serde(rename = "function", default)]
    pub function: ChatFunctionPayload,
}

/// Function payload of a tool call; `name`/`arguments` always serialize.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatFunctionPayload {
    #[serde(rename = "name", default)]
    pub name: String,
    #[serde(rename = "arguments", default)]
    pub arguments: String,
    /// Legacy Gemini OpenAI shape; decode compatibility only.
    #[serde(
        rename = "thought_signature",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub thought_signature: String,
}

/// Gemini's current signature carrier (Go: `chatToolCallExtraContent`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatToolCallExtraContent {
    #[serde(rename = "google")]
    pub google: GoogleThoughtSignature,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoogleThoughtSignature {
    #[serde(
        rename = "thought_signature",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub thought_signature: String,
}

/// One tool definition (Go: `chatTool`/`chatFunction`).
#[derive(Debug, Clone, Serialize)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "function")]
    pub function: ChatFunction,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatFunction {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "description", skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Raw JSON Schema bytes riding the wire unparsed (Go:
    /// `json.RawMessage`); None is the omitempty case.
    #[serde(rename = "parameters", skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Box<serde_json::value::RawValue>>,
}

impl ChatFunction {
    /// Wraps raw schema bytes (Go: `json.RawMessage` construction).
    pub fn raw_parameters(raw: &str) -> Option<Box<serde_json::value::RawValue>> {
        if raw.trim().is_empty() {
            return None;
        }
        Some(serde_json::value::RawValue::from_string(raw.to_owned()).expect("valid JSON schema"))
    }
}

/// The chat-completions request body (Go: `chatRequest`, field order
/// preserved). Encode with [`ChatRequest::encode_body`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChatRequest {
    #[serde(rename = "model")]
    pub model: String,
    #[serde(rename = "messages")]
    pub messages: Vec<ChatMessage>,
    #[serde(rename = "tools", skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ChatTool>,
    #[serde(rename = "tool_choice", skip_serializing_if = "String::is_empty")]
    pub tool_choice: String,
    #[serde(rename = "stream")]
    pub stream: bool,
    #[serde(rename = "stream_options", skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    #[serde(rename = "temperature", skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(rename = "max_tokens", skip_serializing_if = "crate::wire::is_zero")]
    pub max_tokens: i64,
    #[serde(
        rename = "max_completion_tokens",
        skip_serializing_if = "crate::wire::is_zero"
    )]
    pub max_completion_tokens: i64,
    #[serde(rename = "reasoning_effort", skip_serializing_if = "String::is_empty")]
    pub reasoning_effort: String,
    #[serde(rename = "thinking", skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingMode>,
    /// Extra top-level fields from config; merged last (Go: `ExtraBody`
    /// `json:"-"` + `MarshalJSON` override).
    #[serde(skip)]
    pub extra_body: serde_json::Map<String, serde_json::Value>,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

impl ChatRequest {
    /// Serializes the request body the way Go's `json.NewEncoder().Encode`
    /// does: compact, HTML-escaped, `extra_body` merged over the base object
    /// (a Go map re-marshal, so keys sort alphabetically), one trailing
    /// newline. Marshalling fails only on absurd extra-body values.
    pub fn encode_body(&self) -> Result<Vec<u8>, serde_json::Error> {
        let json = if self.extra_body.is_empty() {
            reasonix_contract::gojson::marshal(self)?
        } else {
            let mut object = match serde_json::to_value(self)? {
                serde_json::Value::Object(map) => map,
                _ => serde_json::Map::new(),
            };
            for (key, value) in &self.extra_body {
                object.insert(key.clone(), go_number_shape(value));
            }
            reasonix_contract::gojson::marshal(&serde_json::Value::Object(object))?
        };
        let mut body = json.into_bytes();
        body.push(b'\n');
        Ok(body)
    }
}

/// Go re-decodes `extra_body` through `any`, so every number becomes a
/// float64 and integral values re-marshal without a decimal point. Keep that
/// shape so the merged bytes match.
fn go_number_shape(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(go_number_shape).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), go_number_shape(v)))
                .collect(),
        ),
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && f.fract() == 0.0
                && f.abs() < 9.007_199_254_740_992e15
                && let Some(i) = serde_json::Number::from_i128(f as i128)
            {
                return serde_json::Value::Number(i);
            }
            value.clone()
        }
        other => other.clone(),
    }
}

/// A gateway error envelope on a `data:` frame (Go: `streamWireError`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StreamWireError {
    #[serde(default)]
    pub message: String,
    /// A string as itself, any other scalar as its text (Go: `flexstr.String`).
    #[serde(default, deserialize_with = "crate::wire::flex_string")]
    pub code: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub param: String,
}

fn flex_string<'de, D>(de: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(de)?;
    Ok(match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text,
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    })
}

/// One streamed chat-completions frame (Go: `streamResponse`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StreamResponse {
    #[serde(default)]
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<WireUsage>,
    #[serde(default)]
    pub error: Option<StreamWireError>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StreamChoice {
    #[serde(default)]
    pub delta: StreamDelta,
    #[serde(default, rename = "finish_reason")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StreamDelta {
    #[serde(default)]
    pub content: String,
    #[serde(default, rename = "reasoning_content")]
    pub reasoning_content: String,
    #[serde(default, rename = "reasoning")]
    pub reasoning: String,
    #[serde(default, rename = "tool_calls")]
    pub tool_calls: Vec<ChatToolCall>,
}

/// Decodes one `data:` frame; unknown fields are ignored and missing fields
/// are zero (Go: `json.Unmarshal` into the stream structs). A type mismatch
/// fails the whole frame.
pub fn decode_stream_frame(data: &str) -> Result<StreamResponse, serde_json::Error> {
    serde_json::from_str(data)
}

/// Builds the typed failure for a non-OK status (Go: `SendWithRetry` status
/// handling, minus the retry ladder).
pub(crate) fn status_error(
    provider: &str,
    status: u16,
    body: &str,
    key_env: &str,
    key_source: &str,
    has_key: bool,
) -> ProviderError {
    let body = body.trim().to_owned();
    if status == 401 || status == 403 {
        return ProviderError::Auth {
            provider: provider.to_owned(),
            key_env: key_env.to_owned(),
            key_source: key_source.to_owned(),
            status,
            has_key,
            body,
        };
    }
    ProviderError::Api {
        provider: provider.to_owned(),
        status,
        body,
        trace_id: String::new(),
        tool_context: String::new(),
    }
}
