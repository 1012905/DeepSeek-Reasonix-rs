//! Streaming provider DTOs and guards (contract §5, §12.8).
//!
//! The extension holds credentials and runs streams; the host only sees
//! credential-free descriptors and ordered chunks. Raw provider errors never
//! cross the wire — only redacted codes and generic messages.

use crate::enums::{ProviderChunkType, ProviderErrorCode, ProviderRole};
use crate::rawjson::RawJson;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A credential-free provider descriptor.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDescriptor {
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(
        rename = "displayName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(rename = "contextWindow", default, skip_serializing_if = "is_zero_i64")]
    pub context_window: i64,
    #[serde(
        rename = "pricingCurrency",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub pricing_currency: String,
    #[serde(
        rename = "cacheHitPerMillion",
        default,
        skip_serializing_if = "is_zero_f64"
    )]
    pub cache_hit_per_million: f64,
    #[serde(
        rename = "inputPerMillion",
        default,
        skip_serializing_if = "is_zero_f64"
    )]
    pub input_per_million: f64,
    #[serde(
        rename = "outputPerMillion",
        default,
        skip_serializing_if = "is_zero_f64"
    )]
    pub output_per_million: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vision: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tools: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    #[serde(
        rename = "defaultEffort",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub default_effort: String,
    #[serde(
        rename = "toolCallReasoning",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub tool_call_reasoning: bool,
    #[serde(
        rename = "reasoningRoundTrip",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub reasoning_round_trip: bool,
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}
fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

/// A provider-visible chat message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMessage {
    #[serde(default, skip_serializing_if = "is_default_role")]
    pub role: ProviderRole,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(
        rename = "reasoning_content",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub reasoning_content: String,
    #[serde(
        rename = "reasoning_signature",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub reasoning_signature: String,
    #[serde(rename = "tool_calls", default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(
        rename = "tool_call_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

fn is_default_role(role: &ProviderRole) -> bool {
    matches!(role, ProviderRole::System)
}

/// A provider-visible tool call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(
        rename = "thought_signature",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub thought_signature: String,
}

/// A provider tool schema whose parameters are a JSON object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderToolSchema {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub parameters: RawJson,
}

/// Optional output-shape constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponseFormat {
    #[serde(rename = "type")]
    pub type_: String,
}

/// A credential-free completion request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    pub messages: Vec<ProviderMessage>,
    pub tools: Vec<ProviderToolSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(rename = "maxTokens")]
    pub max_tokens: i64,
    #[serde(
        rename = "responseFormat",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub response_format: Option<ProviderResponseFormat>,
}

/// Why a provider DTO violated its wire invariants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderValidationError {
    #[error("messages and tools must be arrays, not null")]
    NullArrays,
    #[error("maxTokens must be non-negative")]
    NegativeMaxTokens,
    #[error("responseFormat.type must be non-empty")]
    EmptyResponseFormat,
    #[error("tool parameters must be a JSON object")]
    BadToolParameters,
    #[error("argChars must be non-negative")]
    NegativeArgChars,
    #[error("error chunks require error")]
    MissingError,
    #[error("non-error chunks forbid error")]
    UnexpectedError,
    #[error("usage chunks require usage")]
    MissingUsage,
    #[error("streamId and providerRef are required")]
    MissingStreamIdentity,
    #[error("seq must be >= 1")]
    BadSeq,
    #[error("streamId is required")]
    MissingStreamId,
}

impl ProviderRequest {
    /// Enforces the deterministic wire shape.
    pub fn validate(&self) -> Result<(), ProviderValidationError> {
        // Serde's Vec fields reject explicit null before this runs; the guard
        // documents the Go invariant for callers building values in Rust.
        if self.max_tokens < 0 {
            return Err(ProviderValidationError::NegativeMaxTokens);
        }
        if self
            .response_format
            .as_ref()
            .is_some_and(|f| f.type_.trim().is_empty())
        {
            return Err(ProviderValidationError::EmptyResponseFormat);
        }
        for tool in &self.tools {
            let text = tool.parameters.get().trim();
            if !text.starts_with('{') || serde_json::from_str::<serde_json::Value>(text).is_err() {
                return Err(ProviderValidationError::BadToolParameters);
            }
        }
        Ok(())
    }
}

/// Token accounting for one completed chunk or stream.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUsage {
    #[serde(rename = "promptTokens")]
    pub prompt_tokens: i64,
    #[serde(rename = "completionTokens")]
    pub completion_tokens: i64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    #[serde(rename = "cacheHitTokens")]
    pub cache_hit_tokens: i64,
    #[serde(rename = "cacheMissTokens")]
    pub cache_miss_tokens: i64,
    #[serde(rename = "reasoningTokens")]
    pub reasoning_tokens: i64,
    #[serde(
        rename = "finishReason",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub finish_reason: String,
}

/// A deliberately generic, redacted provider failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderError {
    pub code: ProviderErrorCode,
    pub message: String,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

/// One chunk of an extension-hosted provider stream.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderChunk {
    #[serde(rename = "type")]
    pub type_: ProviderChunkType,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    #[serde(rename = "toolCall", default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ProviderToolCall>,
    #[serde(rename = "argChars", default, skip_serializing_if = "is_zero_i64")]
    pub arg_chars: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ProviderUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProviderError>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub epoch: String,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

impl ProviderChunk {
    /// Enforces chunk invariants the tags cannot express.
    pub fn validate(&self) -> Result<(), ProviderValidationError> {
        if self.arg_chars < 0 {
            return Err(ProviderValidationError::NegativeArgChars);
        }
        if self.type_ == ProviderChunkType::Error && self.error.is_none() {
            return Err(ProviderValidationError::MissingError);
        }
        if self.type_ != ProviderChunkType::Error && self.error.is_some() {
            return Err(ProviderValidationError::UnexpectedError);
        }
        if self.type_ == ProviderChunkType::Usage && self.usage.is_none() {
            return Err(ProviderValidationError::MissingUsage);
        }
        Ok(())
    }
}

/// Request the extension's full non-secret provider catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCatalogResult {
    pub providers: Vec<ProviderDescriptor>,
}

/// Opens one provider stream; chunks are numbered from `seq_base`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamOpenParams {
    #[serde(rename = "streamId")]
    pub stream_id: String,
    #[serde(rename = "providerRef")]
    pub provider_ref: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    pub request: ProviderRequest,
    #[serde(rename = "seqBase")]
    pub seq_base: i64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub epoch: String,
}

impl StreamOpenParams {
    pub fn validate(&self) -> Result<(), ProviderValidationError> {
        if self.stream_id.trim().is_empty() || self.provider_ref.trim().is_empty() {
            return Err(ProviderValidationError::MissingStreamIdentity);
        }
        self.request.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamOpenResult {
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamCancelParams {
    #[serde(rename = "streamId")]
    pub stream_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamCancelResult {
    pub cancelled: bool,
}

/// One numbered chunk notification, Extension to Host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamChunkParams {
    #[serde(rename = "streamId")]
    pub stream_id: String,
    pub seq: i64,
    pub chunk: ProviderChunk,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub epoch: String,
}

impl StreamChunkParams {
    pub fn validate(&self) -> Result<(), ProviderValidationError> {
        if self.stream_id.trim().is_empty() {
            return Err(ProviderValidationError::MissingStreamId);
        }
        if self.seq < 1 {
            return Err(ProviderValidationError::BadSeq);
        }
        self.chunk.validate()
    }
}

/// Ends a stream. `last_seq` freezes the terminal ordering boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamEndParams {
    #[serde(rename = "streamId")]
    pub stream_id: String,
    #[serde(rename = "lastSeq")]
    pub last_seq: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interrupted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(s: &str) -> RawJson {
        RawJson::parse(s).unwrap()
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            messages: vec![ProviderMessage {
                role: ProviderRole::Assistant,
                content: "hi".into(),
                ..Default::default()
            }],
            tools: vec![ProviderToolSchema {
                name: "bash".into(),
                description: "run".into(),
                parameters: raw("{\"type\":\"object\"}"),
            }],
            temperature: Some(0.5),
            max_tokens: 1024,
            response_format: None,
        }
    }

    #[test]
    fn descriptor_never_carries_credentials() {
        let desc = ProviderDescriptor {
            ref_: "plugin/acme/model".into(),
            display_name: "Acme".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&desc).unwrap();
        assert!(json.contains("\"ref\":\"plugin/acme/model\""));
        assert!(!json.contains("apiKey"));
        assert!(!json.contains("endpoint"));
    }

    #[test]
    fn strict_null_arrays_rejected() {
        assert!(
            serde_json::from_str::<ProviderRequest>(
                "{\"messages\":null,\"tools\":[],\"maxTokens\":0}"
            )
            .is_err(),
            "explicit null arrays must be rejected"
        );
        let req: ProviderRequest =
            serde_json::from_str("{\"messages\":[],\"tools\":[],\"maxTokens\":0}").unwrap();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn tool_schema_parameters_must_be_object() {
        let mut req = request();
        req.tools[0].parameters = raw("[]");
        assert_eq!(
            req.validate(),
            Err(ProviderValidationError::BadToolParameters)
        );
        assert!(request().validate().is_ok());
    }

    #[test]
    fn chunk_invariants_match_go() {
        let mut usage = ProviderChunk {
            type_: ProviderChunkType::Usage,
            ..Default::default()
        };
        assert_eq!(usage.validate(), Err(ProviderValidationError::MissingUsage));
        usage.usage = Some(ProviderUsage::default());
        assert!(usage.validate().is_ok());

        let mut error = ProviderChunk {
            type_: ProviderChunkType::Error,
            ..Default::default()
        };
        assert_eq!(error.validate(), Err(ProviderValidationError::MissingError));
        error.error = Some(ProviderError {
            code: ProviderErrorCode::ProviderFailed,
            message: "model unavailable".into(),
        });
        assert!(error.validate().is_ok());

        let text = ProviderChunk {
            type_: ProviderChunkType::Text,
            error: error.error.clone(),
            ..Default::default()
        };
        assert_eq!(
            text.validate(),
            Err(ProviderValidationError::UnexpectedError)
        );
    }

    #[test]
    fn stream_sequence_guards() {
        let open = StreamOpenParams {
            stream_id: "st-1".into(),
            provider_ref: "plugin/acme/model".into(),
            model: "acme-1".into(),
            effort: "high".into(),
            request: request(),
            seq_base: 1,
            generation: 0,
            epoch: String::new(),
        };
        assert!(open.validate().is_ok());
        let mut bad = open;
        bad.stream_id.clear();
        assert_eq!(
            bad.validate(),
            Err(ProviderValidationError::MissingStreamIdentity)
        );
    }

    #[test]
    fn stream_chunk_round_trip_and_seq_guard() {
        let chunk = StreamChunkParams {
            stream_id: "st-1".into(),
            seq: 2,
            chunk: ProviderChunk {
                type_: ProviderChunkType::Usage,
                usage: Some(ProviderUsage {
                    prompt_tokens: 1,
                    completion_tokens: 2,
                    total_tokens: 3,
                    cache_hit_tokens: 4,
                    cache_miss_tokens: 5,
                    reasoning_tokens: 6,
                    finish_reason: "stop".into(),
                }),
                ..Default::default()
            },
            generation: 0,
            epoch: String::new(),
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("\"seq\":2"));
        let decoded: StreamChunkParams = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, chunk);
        assert!(chunk.validate().is_ok());
        let mut bad = chunk;
        bad.seq = 0;
        assert_eq!(bad.validate(), Err(ProviderValidationError::BadSeq));
    }

    #[test]
    fn stream_end_freezes_terminal_boundary() {
        let end = StreamEndParams {
            stream_id: "st-1".into(),
            last_seq: 9,
            error: String::new(),
            interrupted: true,
        };
        let json = serde_json::to_string(&end).unwrap();
        assert_eq!(
            json,
            "{\"streamId\":\"st-1\",\"lastSeq\":9,\"interrupted\":true}"
        );
    }
}
