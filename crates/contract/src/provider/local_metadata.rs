//! Local metadata types (Go: `local_metadata.go`, `decision.go`,
//! `provider.go`).

use crate::gojson::{go_struct, go_vec, is_zero_bool, is_zero_i64, null_default};
use crate::provider::stream_failure::StreamFailureCause;

go_struct! {
    /// An image whose stored bytes the store could not read back (Go:
    /// `UnavailableImage`).
    #[derive(PartialEq, Eq)]
    pub struct UnavailableImage : UnavailableImageWire {
        #[serde(rename = "index", default, deserialize_with = "null_default")]
        pub index: i64,
        /// `sha256:<hex64>` blob key of the image content.
        #[serde(rename = "ref", default, deserialize_with = "null_default")]
        pub reference: String,
    }
}

go_struct! {
    /// Local display metadata for memories that influenced a turn (Go:
    /// `MemoryCitation`).
    #[derive(PartialEq, Eq)]
    pub struct MemoryCitation : MemoryCitationWire {
        #[serde(
            rename = "id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub id: String,
        #[serde(rename = "source", default, deserialize_with = "null_default")]
        pub source: String,
        #[serde(
            rename = "lineStart",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub line_start: i64,
        #[serde(
            rename = "lineEnd",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub line_end: i64,
        #[serde(
            rename = "note",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub note: String,
        #[serde(
            rename = "kind",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub kind: String,
    }
}

go_struct! {
    /// The paired device a person acted from (Go: `Via`).
    #[derive(PartialEq, Eq)]
    pub struct Via : ViaWire {
        /// Pairing id, stable while paired.
        #[serde(rename = "device", default, deserialize_with = "null_default")]
        pub device: String,
        /// Number the window listed the device under when it acted.
        #[serde(rename = "ordinal", default, deserialize_with = "null_default")]
        pub ordinal: i64,
    }
}

go_struct! {
    /// Durable, provider-excluded evidence of a user-owned approval decision
    /// (Go: `DecisionReceipt`).
    #[derive(PartialEq, Eq)]
    pub struct DecisionReceipt : DecisionReceiptWire {
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        pub id: String,
        #[serde(rename = "kind", default, deserialize_with = "null_default")]
        pub kind: String,
        #[serde(
            rename = "tool",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub tool: String,
        #[serde(
            rename = "subject",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub subject: String,
        #[serde(rename = "outcome", default, deserialize_with = "null_default")]
        pub outcome: String,
        #[serde(
            rename = "via",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub via: Option<Via>,
    }
}

go_struct! {
    /// A completed, fully paired tool call of an interrupted turn (Go:
    /// `InterruptedToolSummary`).
    #[derive(PartialEq, Eq)]
    pub struct InterruptedToolSummary : InterruptedToolSummaryWire {
        #[serde(
            rename = "id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub id: String,
        #[serde(rename = "name", default, deserialize_with = "null_default")]
        pub name: String,
        #[serde(
            rename = "files",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub files: Vec<String>,
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
    }
}

go_struct! {
    /// Durable, provider-excluded handoff for a turn that stopped before a
    /// clean final answer (Go: `InterruptedTurnRecovery`).
    #[derive(PartialEq, Eq)]
    pub struct InterruptedTurnRecovery : InterruptedTurnRecoveryWire {
        #[serde(
            rename = "stream_failure",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "StreamFailureCause::is_empty"
        )]
        pub stream_failure: StreamFailureCause,
        #[serde(
            rename = "pending",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub pending: bool,
        #[serde(
            rename = "completed_tools",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub completed_tools: Vec<InterruptedToolSummary>,
        #[serde(
            rename = "interrupted_tools",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub interrupted_tools: Vec<String>,
        #[serde(
            rename = "dropped_partial_text",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub dropped_partial_text: bool,
        #[serde(
            rename = "dropped_partial_reasoning",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub dropped_partial_reasoning: bool,
    }
}

go_struct! {
    /// Host-local shell metadata (Go: `ToolExecution`).
    #[derive(PartialEq, Eq)]
    pub struct ToolExecution : ToolExecutionWire {
        #[serde(
            rename = "kind",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub kind: String,
        #[serde(
            rename = "shell",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub shell: String,
        #[serde(
            rename = "shellVersion",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub shell_version: String,
        #[serde(
            rename = "platform",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub platform: String,
        #[serde(rename = "supportsAndAnd", default, deserialize_with = "null_default")]
        pub supports_and_and: bool,
        #[serde(
            rename = "state",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub state: String,
        #[serde(
            rename = "failurePhase",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub failure_phase: String,
        #[serde(
            rename = "exitCode",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub exit_code: Option<i64>,
        #[serde(
            rename = "outputTail",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub output_tail: String,
        #[serde(
            rename = "mutationRisk",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub mutation_risk: String,
        #[serde(
            rename = "verification",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub verification: String,
        #[serde(
            rename = "durationMs",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub duration_ms: i64,
    }
}

go_struct! {
    /// The host's own account of a result that did not succeed (Go:
    /// `ToolFailure`).
    #[derive(PartialEq, Eq)]
    pub struct ToolFailure : ToolFailureWire {
        #[serde(
            rename = "refusalCode",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub refusal_code: String,
        #[serde(
            rename = "blocked",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub blocked: bool,
    }
}
