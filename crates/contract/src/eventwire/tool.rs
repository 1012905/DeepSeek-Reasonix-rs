//! The tool-call frame and its payloads, mirroring `eventwire/tool.go` and the
//! `provider.ArgumentNormalization` wire type it carries.

use crate::eventwire::lease::WorkspaceLease;
use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// The tool frame itself. Who issued a call is the one thing separating the
/// model's work from the host's own bookkeeping.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<crate::delivery::File>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    #[serde(
        rename = "argumentNormalization",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub argument_normalization: Option<ArgumentNormalization>,
    #[serde(
        rename = "resolvedName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub resolved_name: String,
    #[serde(
        rename = "capabilityId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub capability_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub videos: Vec<VideoShown>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub err: String,
    #[serde(
        rename = "refusalCode",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub refusal_code: String,
    #[serde(
        rename = "workspaceLease",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_lease: Option<WorkspaceLease>,
    #[serde(rename = "outputDiff", default, skip_serializing_if = "util::is_false")]
    pub output_diff: bool,
    #[serde(rename = "readOnly", default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound: Option<Bound>,
    #[serde(rename = "durationMs", default, skip_serializing_if = "util::is_zero")]
    pub duration_ms: i64,
    #[serde(rename = "rate", default, skip_serializing_if = "util::is_zero")]
    pub rate_tokens_per_sec: i64,
    #[serde(
        rename = "contextTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub context_tokens: i64,
    #[serde(rename = "startedAt", default, skip_serializing_if = "util::is_zero")]
    pub started_at: i64,
    #[serde(rename = "endedAt", default, skip_serializing_if = "util::is_zero")]
    pub ended_at: i64,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub partial: bool,
    #[serde(rename = "argChars", default, skip_serializing_if = "util::is_zero")]
    pub arg_chars: i64,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub refreshed: bool,
    #[serde(rename = "parentId", default, skip_serializing_if = "String::is_empty")]
    pub parent_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub issuer: String,
    #[serde(
        rename = "attemptId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub attempt_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub diff: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub added: i64,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub removed: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ShellExecution>,
}

/// The subagent model/effort resolved for a tool call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub count: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// How a result was fitted into the model's context, absent when it arrived
/// whole. "spilled" keeps `path`; "truncated" keeps `kept_bytes`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bound {
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub lines: i64,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub bytes: i64,
    #[serde(rename = "keptBytes", default, skip_serializing_if = "util::is_zero")]
    pub kept_bytes: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

/// A video file the call handed to the model to watch natively: name and size,
/// never the bytes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoShown {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: i64,
    #[serde(rename = "durationMs", default, skip_serializing_if = "util::is_zero")]
    pub duration_ms: i64,
    #[serde(default)]
    pub model: String,
}

/// How the command ran, which its output alone cannot say.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellExecution {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shell: String,
    #[serde(
        rename = "shellVersion",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub shell_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub platform: String,
    #[serde(rename = "supportsAndAnd", default)]
    pub supports_and_and: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    #[serde(
        rename = "failurePhase",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub failure_phase: String,
    #[serde(rename = "exitCode", default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(
        rename = "outputTail",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub output_tail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    #[serde(
        rename = "mutationRisk",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub mutation_risk: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verification: String,
    #[serde(rename = "durationMs", default, skip_serializing_if = "util::is_zero")]
    pub duration_ms: i64,
    #[serde(rename = "jobId", default, skip_serializing_if = "String::is_empty")]
    pub job_id: String,
}

/// Where the host corrected the model's arguments before the call ran.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentNormalization {
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub original: String,
    #[serde(default)]
    pub effective: String,
}
