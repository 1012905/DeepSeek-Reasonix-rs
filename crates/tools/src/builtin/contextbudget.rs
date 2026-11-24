//! The `context_budget` tool and its reporter seam (tools.md §1.36; Go
//! `internal/tools/builtin/contextbudget.go`,
//! `internal/contract/tool/context_budget.go`).
//!
//! The figure that drives behavior is the distance to the compaction
//! trigger, not the raw window: the fold is the event the model can still
//! act ahead of, and it fires well before the window is physically full.
//! The budget is pull-only: the host never appends it to a turn or raises a
//! notice about it.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::abi::{CallContext, Tool, ToolResult};

/// `context_budget`, the tool name the model calls.
pub const CONTEXT_BUDGET_TOOL_NAME: &str = "context_budget";

/// Budget measured against a live window and trigger.
pub const STATUS_OK: &str = "ok";
/// No window could be measured: every figure is absent and [`ContextBudget::known`]
/// is false.
pub const STATUS_UNMEASURED: &str = "unmeasured";

/// What the model is told about its remaining room (Go `tool.ContextBudget`).
/// Optional figures serialize only when present; the Go side omits both
/// absent and zero values, and the reporter populates them only on a measured
/// budget.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContextBudget {
    /// `ok` or `unmeasured`.
    pub status: String,
    #[serde(rename = "tokens_remaining", skip_serializing_if = "Option::is_none")]
    pub tokens_remaining: Option<i64>,
    #[serde(rename = "tokens_used", skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<i64>,
    #[serde(rename = "compact_at", skip_serializing_if = "Option::is_none")]
    pub compact_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ContextBudget {
    /// Whether the host could measure the budget at all (Go
    /// `ContextBudget.Known`). A provider declaring no context window leaves
    /// every figure zero, and a reported zero remaining would read as "out of
    /// room" rather than "never measured".
    pub fn known(&self) -> bool {
        self.window.is_some_and(|w| w > 0) && self.compact_at.is_some_and(|c| c > 0)
    }

    /// The unmeasured shape: status only, plus why it could not be measured.
    pub fn unmeasured(reason: impl Into<String>) -> Self {
        Self {
            status: STATUS_UNMEASURED.to_owned(),
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
}

/// Supplies the active session's budget, bound per call by the owning agent
/// (Go `tool.ContextBudgetReporter`). Parent and child agents share one tool
/// schema without sharing session state.
pub trait ContextBudgetReporter: Send + Sync {
    fn context_budget(&self) -> ContextBudget;

    /// Type-erased view for host-side recognition.
    fn as_any(&self) -> &dyn Any;
}

/// The reporter bound to one call, when the owning agent installed one (Go
/// `tool.ContextBudgetReporterFromContext`).
pub fn reporter_from(ctx: &CallContext) -> Option<Arc<dyn ContextBudgetReporter>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn ContextBudgetReporter>>()
        .cloned()
}

/// Go `contextBudget` tool.
#[derive(Debug, Default, Clone, Copy)]
pub struct ContextBudgetTool;

/// The one instance a composition root registers.
pub fn context_budget_tool() -> Arc<dyn Tool> {
    Arc::new(ContextBudgetTool)
}

/// Go `contextBudget.Description()`, verbatim.
pub const CONTEXT_BUDGET_DESCRIPTION: &str = "Report how much context room is left before this conversation is automatically compacted. Nothing reports it unprompted: check it at the start of a long task, before a large change, and before output you may not finish reading (a broad search, a large file, a long build log), so you can narrow the command. `tokens_remaining` counts down to the compaction trigger, not the physical window.";

/// Go `contextBudget.Schema()`, verbatim.
const GO_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{}}"#;

impl Tool for ContextBudgetTool {
    fn name(&self) -> &str {
        CONTEXT_BUDGET_TOOL_NAME
    }

    fn description(&self) -> &str {
        CONTEXT_BUDGET_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(GO_SCHEMA).expect("the context_budget schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, _args: &Value) -> ToolResult {
        let Some(reporter) = reporter_from(ctx) else {
            return Err(Box::new(std::io::Error::other(
                "context_budget is unavailable outside an active agent session",
            )));
        };
        let budget = reporter.context_budget();
        let out = serde_json::to_string(&budget).map_err(|e| {
            Box::new(std::io::Error::other(format!("encode context budget: {e}")))
                as Box<dyn std::error::Error + Send + Sync>
        })?;
        Ok(out)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct StubReporter {
        budget: ContextBudget,
    }

    impl ContextBudgetReporter for StubReporter {
        fn context_budget(&self) -> ContextBudget {
            self.budget.clone()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn schema_name_and_description_match_the_go_contract() {
        let t = ContextBudgetTool;
        assert_eq!(t.name(), CONTEXT_BUDGET_TOOL_NAME);
        assert_eq!(t.name(), "context_budget");
        assert_eq!(t.description(), CONTEXT_BUDGET_DESCRIPTION);
        assert!(t.read_only());
        let s = t.schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["properties"], json!({}));
    }

    #[test]
    fn known_requires_both_window_and_compact_at_positive() {
        let measured = ContextBudget {
            status: STATUS_OK.to_owned(),
            tokens_remaining: Some(300),
            tokens_used: Some(700),
            compact_at: Some(1000),
            window: Some(128_000),
            reason: None,
        };
        assert!(measured.known());

        let no_window = ContextBudget {
            compact_at: Some(1000),
            window: None,
            ..measured.clone()
        };
        assert!(!no_window.known());

        let no_trigger = ContextBudget {
            window: Some(128_000),
            compact_at: None,
            ..measured.clone()
        };
        assert!(!no_trigger.known());

        let zero_window = ContextBudget {
            window: Some(0),
            compact_at: Some(0),
            ..ContextBudget::default()
        };
        assert!(!zero_window.known(), "zero figures read as never measured");
    }

    #[test]
    fn the_measured_budget_serializes_the_go_field_names() {
        let budget = ContextBudget {
            status: STATUS_OK.to_owned(),
            tokens_remaining: Some(300),
            tokens_used: Some(700),
            compact_at: Some(1000),
            window: Some(128_000),
            reason: None,
        };
        let wire = serde_json::to_value(&budget).expect("serialize");
        assert_eq!(wire["status"], "ok");
        assert_eq!(wire["tokens_remaining"], 300);
        assert_eq!(wire["tokens_used"], 700);
        assert_eq!(wire["compact_at"], 1000);
        assert_eq!(wire["window"], 128_000);
        assert!(wire.get("reason").is_none());
    }

    #[test]
    fn the_unmeasured_budget_omits_every_figure() {
        let budget = ContextBudget::unmeasured(
            "the active provider declares no context window, so compaction is disabled",
        );
        assert!(!budget.known());
        let wire = serde_json::to_value(&budget).expect("serialize");
        assert_eq!(wire["status"], "unmeasured");
        assert!(wire.get("tokens_remaining").is_none());
        assert!(wire.get("tokens_used").is_none());
        assert!(wire.get("compact_at").is_none());
        assert!(wire.get("window").is_none());
        assert!(wire["reason"].is_string());
    }

    #[test]
    fn execute_routes_to_the_reporter_and_reports_unavailable_without_one() {
        let reporter = Arc::new(StubReporter {
            budget: ContextBudget {
                status: STATUS_OK.to_owned(),
                tokens_remaining: Some(300),
                tokens_used: Some(700),
                compact_at: Some(1000),
                window: Some(128_000),
                reason: None,
            },
        });
        let port: Arc<dyn ContextBudgetReporter> = reporter;
        let ctx = CallContext::new().with_value(Arc::new(port));
        let out = ContextBudgetTool
            .execute(&ctx, &json!({}))
            .expect("budget reports");
        let wire: Value = serde_json::from_str(&out).expect("wire json");
        assert_eq!(wire["status"], "ok");
        assert_eq!(wire["tokens_remaining"], 300);

        let err = ContextBudgetTool
            .execute(&CallContext::new(), &json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("active agent session"), "{err}");
    }
}
