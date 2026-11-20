//! Tool call shapes (Go: `provider.go`, `argument_normalization.go`).

use crate::gojson::{go_struct, is_zero_i64, null_default};

/// Reason code of a redundant arguments-envelope removal (Go:
/// `ArgumentEnvelopeUnwrapped`).
pub const ARGUMENT_ENVELOPE_UNWRAPPED: &str = "redundant_arguments_envelope";

go_struct! {
    /// Evidence of a tool-call argument rewrite (Go: `ArgumentNormalization`).
    #[derive(PartialEq, Eq)]
    pub struct ArgumentNormalization : ArgumentNormalizationWire {
        #[serde(rename = "reason", default, deserialize_with = "null_default")]
        pub reason: String,
        #[serde(rename = "original", default, deserialize_with = "null_default")]
        pub original: String,
        #[serde(rename = "effective", default, deserialize_with = "null_default")]
        pub effective: String,
    }
}

go_struct! {
    /// A tool invocation requested by the model (Go: `ToolCall`).
    #[derive(PartialEq, Eq)]
    pub struct ToolCall : ToolCallWire {
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        pub id: String,
        /// May be empty in old sessions; normalization backfills from results.
        #[serde(rename = "name", default, deserialize_with = "null_default")]
        pub name: String,
        /// Raw JSON arguments; may be truncated from a cut stream.
        #[serde(rename = "arguments", default, deserialize_with = "null_default")]
        pub arguments: String,
        #[serde(
            rename = "argument_normalization",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub argument_normalization: Option<ArgumentNormalization>,
        /// Opaque Gemini-issued proof.
        #[serde(
            rename = "thought_signature",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub thought_signature: String,
        #[serde(
            rename = "diff",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub diff: String,
        #[serde(
            rename = "added",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub added: i64,
        #[serde(
            rename = "removed",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub removed: i64,
        /// Display-only use_capability proxy metadata.
        #[serde(
            rename = "resolved_name",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub resolved_name: String,
        /// Display-only use_capability proxy metadata.
        #[serde(
            rename = "capability_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub capability_id: String,
        /// Display-only use_capability proxy metadata.
        #[serde(
            rename = "resolved_read_only",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub resolved_read_only: Option<bool>,
    }
}

impl ToolCall {
    /// The arguments actually executed: the envelope-unwrap rewrite when it
    /// applies to this call's arguments, else the recorded arguments (Go:
    /// `ExecutionArguments`).
    pub fn execution_arguments(&self) -> &str {
        match &self.argument_normalization {
            Some(normalization)
                if normalization.reason == ARGUMENT_ENVELOPE_UNWRAPPED
                    && normalization.original == self.arguments =>
            {
                &normalization.effective
            }
            _ => &self.arguments,
        }
    }
}
