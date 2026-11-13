//! The `Event` envelope and its scalar payloads, mirroring `eventwire/wire.go`.

use crate::eventwire::guardian::Guardian;
use crate::eventwire::lease::WorkspaceLease;
use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// The JSON-friendly form shared by event frontends. Field names, omitempty
/// sets and kind strings must match the Go producer exactly; the parity gates
/// hold both directions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    #[serde(default)]
    pub kind: String,
    #[serde(
        rename = "nonPersistable",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub non_persistable: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning: String,
    #[serde(rename = "thoughtMs", default, skip_serializing_if = "util::is_zero")]
    pub thought_ms: i64,
    #[serde(
        rename = "memoryCitations",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub memory_citations: Vec<MemoryCitation>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub audience: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<crate::eventwire::tool::Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::eventwire::usage::Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<crate::eventwire::approval::Approval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<crate::eventwire::ask::Ask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<Compaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<ContextMaintenance>,
    #[serde(
        rename = "todoProgress",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub todo_progress: Option<TodoProgress>,
    #[serde(
        rename = "progressWatch",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub progress_watch: Option<ProgressWatch>,
    #[serde(
        rename = "workspaceLease",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_lease: Option<WorkspaceLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardian: Option<Guardian>,
    #[serde(
        rename = "decisionReceipt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub decision_receipt: Option<DecisionReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<crate::eventwire::extension::ExtensionSurface>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub err: String,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<crate::eventwire::receipt::CompletionReceipt>,
    #[serde(
        rename = "checkpointTurn",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub checkpoint_turn: Option<i64>,
    #[serde(
        rename = "workDurationMs",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub work_duration_ms: i64,
    #[serde(
        rename = "workStartedAt",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub work_started_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(
        rename = "authoredTurn",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub authored_turn: Option<i64>,
    #[serde(rename = "msgIndex", default, skip_serializing_if = "Option::is_none")]
    pub msg_index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Via>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::eventwire::lease::Attachment>,
    #[serde(
        rename = "hostAuthored",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub host_authored: bool,
    #[serde(rename = "modelRef", default, skip_serializing_if = "String::is_empty")]
    pub model_ref: String,
    #[serde(
        rename = "retryAttempt",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub retry_attempt: i64,
    #[serde(rename = "retryMax", default, skip_serializing_if = "util::is_zero")]
    pub retry_max: i64,
    #[serde(
        rename = "retryScope",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub retry_scope: String,
    #[serde(
        rename = "retryCause",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub retry_cause: String,
    #[serde(rename = "retryStatus", default, skip_serializing_if = "util::is_zero")]
    pub retry_status: i64,
    #[serde(
        rename = "retryDelayMs",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub retry_delay_ms: i64,
    #[serde(
        rename = "retryTimeoutSecs",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub retry_timeout_secs: i64,
    #[serde(
        rename = "streamAttempt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub stream_attempt: Option<StreamAttempt>,
    #[serde(rename = "itemId", default, skip_serializing_if = "String::is_empty")]
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceChanged>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<crate::agentgraph::Delta>,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub seq: i64,
}

/// Content-free quality summary of a finished turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionSummary {
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub mutations: i64,
    #[serde(rename = "checks_passed", default)]
    pub checks_passed: i64,
    #[serde(rename = "checks_failed", default)]
    pub checks_failed: i64,
    #[serde(rename = "checks_suppressed", default)]
    pub checks_suppressed: i64,
    #[serde(default)]
    pub review: String,
    #[serde(rename = "gap_kinds", default, skip_serializing_if = "Vec::is_empty")]
    pub gap_kinds: Vec<String>,
    #[serde(
        rename = "criteria_rewritten",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub criteria_rewritten: Vec<String>,
}

/// A workspace mutation. Revisions and watch state always arrive; a nil
/// payload still renders on the Go side, so readers never split "no changes"
/// from "the event arrived without one".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceChanged {
    #[serde(default)]
    pub revisions: WorkspaceRevision,
    #[serde(default)]
    pub changes: Vec<WorkspacePathChange>,
    #[serde(rename = "allPaths", default)]
    pub all_paths: bool,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "watchState", default)]
    pub watch_state: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRevision {
    #[serde(default)]
    pub content: u64,
    #[serde(default)]
    pub tree: u64,
    #[serde(rename = "workingTree", default)]
    pub working_tree: u64,
    #[serde(rename = "gitMeta", default)]
    pub git_meta: u64,
    #[serde(default)]
    pub session: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePathChange {
    #[serde(default)]
    pub path: String,
    #[serde(rename = "oldPath", default, skip_serializing_if = "String::is_empty")]
    pub old_path: String,
    #[serde(default)]
    pub op: String,
}

/// One speculative stream attempt over one provider response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamAttempt {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub action: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub attempt: i64,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub max: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(
        rename = "firstTokenMs",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub first_token_ms: i64,
}

/// What the kernel recorded when a prompt was settled.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionReceipt {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Via>,
}

/// The paired device a person acted from; absent is the window's own.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Via {
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub ordinal: i64,
}

/// One local memory reference a turn drew on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCitation {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "lineStart", default, skip_serializing_if = "util::is_zero")]
    pub line_start: i64,
    #[serde(rename = "lineEnd", default, skip_serializing_if = "util::is_zero")]
    pub line_end: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

/// What a fold did, what it cost, and what the prompt still holds after it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compaction {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub messages: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(
        rename = "sourceTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub source_tokens: i64,
    #[serde(
        rename = "projectionTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub projection_tokens: i64,
    #[serde(
        rename = "coverageRequired",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub coverage_required: i64,
    #[serde(
        rename = "coverageMissing",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub coverage_missing: i64,
    #[serde(
        rename = "coverageBackstopped",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub coverage_backstopped: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub boundary: String,
    #[serde(
        rename = "triggerTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub trigger_tokens: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remains: Option<ContextComposition>,
}

/// One class of what the prompt holds and the tokens it costs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextPart {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tokens: i64,
}
/// One message among the largest the prompt holds; `active` is the running
/// turn, which a fold cannot take.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextItem {
    #[serde(default)]
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    #[serde(default)]
    pub tokens: i64,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub active: bool,
}

/// What holds the tokens of a prompt: the classes by size, largest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextComposition {
    #[serde(default)]
    pub total: i64,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub active: i64,
    #[serde(default)]
    pub parts: Vec<ContextPart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub largest: Vec<ContextItem>,
}

/// One context-maintenance transaction. status "noop" carries `code`: the
/// attempt ran and freed nothing, and code is what tells "already folded this
/// turn" from "nothing left to fold".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMaintenance {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub action: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger: String,
    #[serde(
        rename = "operationId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub operation_id: String,
    #[serde(rename = "inputTokens", default, skip_serializing_if = "util::is_zero")]
    pub input_tokens: i64,
    #[serde(
        rename = "resultTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub result_tokens: i64,
    #[serde(rename = "savedTokens", default, skip_serializing_if = "util::is_zero")]
    pub saved_tokens: i64,
    #[serde(
        rename = "affectedToolResults",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub affected_tool_results: i64,
    #[serde(
        rename = "projectionVersion",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub projection_version: u64,
    #[serde(rename = "cacheBreak", default, skip_serializing_if = "util::is_false")]
    pub cache_break: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub boundary: String,
    #[serde(
        rename = "triggerTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub trigger_tokens: i64,
}

/// One task-list transition. kind is the structural verdict; the three
/// revisions count writes, plan changes, and execution movement separately.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoProgress {
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub steps: i64,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub completed: i64,
    #[serde(
        rename = "contentRevision",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub content_revision: i64,
    #[serde(
        rename = "planRevision",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub plan_revision: i64,
    #[serde(
        rename = "progressRevision",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub progress_revision: i64,
}

/// The host's reading of whether the run still moves: rounds since the last
/// effect it could observe, and input spent against the backstop.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressWatch {
    #[serde(default)]
    pub stalled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cause: String,
    #[serde(rename = "idleRounds", default)]
    pub idle_rounds: i64,
    #[serde(rename = "roundLimit", default, skip_serializing_if = "util::is_zero")]
    pub round_limit: i64,
    #[serde(
        rename = "promptTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub prompt_tokens: i64,
    #[serde(rename = "tokenLimit", default, skip_serializing_if = "util::is_zero")]
    pub token_limit: i64,
    #[serde(
        rename = "tokenMultiple",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub token_multiple: i64,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub pausing: bool,
}
