//! The `recall` tool (Go `internal/tools/builtin/recall.go`): brings a folded
//! part of this conversation back, either by the `#n` addresses a compaction
//! index lists or by query over the folded region. It reads this session's
//! own canonical transcript through the host-bound recaller; the transcript
//! never leaves the agent.

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::abi::{CallContext, CapabilitySet, PlanModeClassifier, Tool, ToolResult};

/// The most addresses one call may read (Go `maxRecallPositions`).
pub const MAX_RECALL_POSITIONS: usize = 8;

/// `recall`, the tool name the model calls.
pub const RECALL_TOOL_NAME: &str = "recall";

/// Go `recallContext.Description()` for the search-off arm, verbatim.
pub const RECALL_DESCRIPTION_SEARCH_OFF: &str = "Bring a folded part of this conversation back, addressed by the #n positions listed under \"Folded work index\" in a compaction summary. Prefer re-reading a file over recalling it — a current copy beats a folded one. Each compaction grants a fresh budget; a request past it is refused whole, not truncated.";

/// Go `recallContext.Description()` for the full arm, verbatim.
pub const RECALL_DESCRIPTION_FULL: &str = "Bring folded conversation back: read the #n positions listed under \"Folded work index\" in a compaction summary, or search the whole folded region by query when no index line names what you need. Prefer re-reading a file over recalling it — a current copy beats a folded one. One budget per compaction covers both, and a request past it is refused whole, not truncated.";

const GO_SCHEMA_SEARCH_OFF: &str = r#"{
"type":"object",
"additionalProperties":false,
"properties":{
  "positions":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"integer","minimum":0},"description":"The #n addresses from a folded-work index line, without the '#'."}
},
"required":["positions"]
}"#;

const GO_SCHEMA_FULL: &str = r#"{
"type":"object",
"additionalProperties":false,
"properties":{
  "positions":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"integer","minimum":0},"description":"Read these #n addresses, from an index line or a search hit, without the '#'."},
  "query":{"type":"string","description":"Search for these words instead. A tool result matches on its output and is addressed by its call."},
  "limit":{"type":"integer","minimum":1,"maximum":20,"description":"Search hits (default 8)."}
}
}"#;

/// Names folded transcript positions to bring back, or a query to search
/// them by (Go `tool.RecallRequest`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecallRequest {
    pub positions: Vec<i64>,
    pub query: String,
    /// Passed through to the recaller as sent: the tool layer neither fills
    /// the schema's default 8 nor clamps the 1–20 range; that normalization
    /// lives in the host (`normalizeRecallLimit`).
    pub limit: i64,
}

/// One search result: what matched, and the canonical position to read it at
/// (Go `tool.RecallHit`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecallHit {
    pub position: i64,
    pub kind: String,
    pub tool: String,
    pub snippet: String,
}

/// One recall's content plus what it cost (Go `tool.RecallResult`). `text`
/// is what the model reads; the counters let it see the budget draining.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecallResult {
    pub text: String,
    pub recalled: Vec<i64>,
    pub missing: Vec<i64>,
    pub hits: Vec<RecallHit>,
    pub searched: i64,
    pub tokens: i64,
    pub budget_left: i64,
}

/// Supplied by the agent owning the active session: the canonical transcript
/// never leaves it, so the tool asks it for a position rather than reading a
/// session file (Go `tool.ContextRecaller`).
pub trait ContextRecaller: Send + Sync {
    fn recall_context(
        &self,
        request: RecallRequest,
    ) -> Result<RecallResult, Box<dyn std::error::Error + Send + Sync>>;
}

/// The recaller bound to this call, when the active session installed one
/// (Go `ContextRecallerFromContext`).
pub fn recaller_from(ctx: &CallContext) -> Option<Arc<dyn ContextRecaller>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn ContextRecaller>>()
        .cloned()
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

fn invalid(message: impl Into<String>) -> ToolResult {
    Err(invalid_err(message))
}

/// The `recall` tool. `search_off` is the pre-search arm: it reads an
/// address it was given and cannot look one up (Go `recallContext`).
#[derive(Debug, Clone, Copy)]
pub struct RecallTool {
    search_off: bool,
}

/// The full tool: read addresses or search by query.
pub fn recall_tool() -> Arc<dyn Tool> {
    Arc::new(RecallTool { search_off: false })
}

/// The recall tool with its search half removed, for the no-recall-search
/// ablation arm (Go `RecallWithoutSearch`).
pub fn recall_without_search() -> Arc<dyn Tool> {
    Arc::new(RecallTool { search_off: true })
}

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default)]
    positions: Vec<i64>,
    #[serde(default)]
    query: String,
    #[serde(default)]
    limit: i64,
}

impl Tool for RecallTool {
    fn name(&self) -> &str {
        RECALL_TOOL_NAME
    }

    fn description(&self) -> &str {
        if self.search_off {
            RECALL_DESCRIPTION_SEARCH_OFF
        } else {
            RECALL_DESCRIPTION_FULL
        }
    }

    fn schema(&self) -> Value {
        let raw = if self.search_off {
            GO_SCHEMA_SEARCH_OFF
        } else {
            GO_SCHEMA_FULL
        };
        serde_json::from_str(raw).expect("the recall schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let args = match args.as_object() {
            Some(_) => args.clone(),
            None => Value::Object(Default::default()),
        };
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| invalid_err(format!("invalid recall args: {e}")))?;

        let query = parsed.query.trim().to_owned();
        if self.search_off {
            if !query.is_empty() {
                return invalid("recall: this configuration reads addresses only");
            }
            if parsed.positions.is_empty() {
                return invalid("recall: positions must name at least one #n address");
            }
        }
        if query.is_empty() && parsed.positions.is_empty() {
            return invalid("recall: give positions to read, or a query to search for them");
        }
        if parsed.positions.len() > MAX_RECALL_POSITIONS {
            return invalid(format!(
                "recall: at most {MAX_RECALL_POSITIONS} positions per call"
            ));
        }

        let Some(recaller) = recaller_from(ctx) else {
            return invalid("recall is unavailable outside an active agent session");
        };
        let result = recaller.recall_context(RecallRequest {
            positions: parsed.positions,
            query: query.clone(),
            limit: parsed.limit,
        })?;

        if !query.is_empty() {
            Ok(render_recall_search(result))
        } else {
            Ok(render_recall(result))
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "plan_mode_safe",
            Arc::new(PlanSafe) as Arc<dyn PlanModeClassifier>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct PlanSafe;
impl crate::abi::Capability for PlanSafe {}
impl PlanModeClassifier for PlanSafe {
    fn plan_mode_safe(&self) -> bool {
        true
    }
}

/// Search path footer: the same accounting-line shape as a read, whichever
/// operation the model ran (Go `renderRecallSearch`).
pub fn render_recall_search(res: RecallResult) -> String {
    let mut b = String::from(res.text.as_str());
    b.push_str(&format!(
        "\n\n[searched {} folded messages; {} tokens; {} left before the next compaction",
        res.searched, res.tokens, res.budget_left
    ));
    if !res.hits.is_empty() {
        b.push_str("; read them with positions");
    }
    b.push(']');
    b
}

/// Read path: the accounting line last, so the content the model came to read
/// leads the message (Go `renderRecall`).
pub fn render_recall(res: RecallResult) -> String {
    let mut b = String::from(res.text.as_str());
    if !b.ends_with('\n') {
        b.push('\n');
    }
    b.push_str(&format!(
        "\n[recalled {}; {} tokens; {} left before the next compaction",
        position_list(&res.recalled),
        res.tokens,
        res.budget_left
    ));
    if !res.missing.is_empty() {
        b.push_str(&format!(
            "; no longer addressable: {}",
            position_list(&res.missing)
        ));
    }
    b.push(']');
    b
}

/// Renders addresses as `#n`, comma-separated; an empty list reads as
/// `nothing` (Go `positionList`).
pub fn position_list(positions: &[i64]) -> String {
    if positions.is_empty() {
        return "nothing".to_owned();
    }
    positions
        .iter()
        .map(|p| format!("#{p}"))
        .collect::<Vec<_>>()
        .join(", ")
}
