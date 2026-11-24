//! The `update_goal` tool (Go `internal/tools/builtin/updategoal.go`): records
//! the model's structured per-turn goal disposition for the active goal turn.
//! It has no host side effects of its own — the call only records candidate
//! state, and the real FSM transition happens after the turn ends once
//! Delivery readiness and budget checks pass. Outside an active goal turn it
//! fails closed without changing any state.

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::abi::{
    CallContext, Capability, CapabilitySet, ContextualReasoner, ContextualTool, PlanModeClassifier,
    Tool, ToolResult,
};
use crate::refusal::Refusal;
use crate::registry::{FEATURE_GOAL_MODE, HostRequired};

/// `update_goal`, the tool name the model calls.
pub const UPDATE_GOAL_TOOL_NAME: &str = "update_goal";

/// `goal.no_active_turn` — the turn carries no goal recorder, so no goal
/// state may change.
pub const CODE_GOAL_NO_ACTIVE_TURN: &str = "goal.no_active_turn";

/// Go `updateGoal.Description()`, verbatim.
pub const UPDATE_GOAL_DESCRIPTION: &str = "Report this turn's disposition for the active goal. Call it at the end of every goal turn instead of using prose markers: `continue` (work is ongoing — give a concrete next_action), `complete` (the request is fully done, output format and constraints satisfied, and verification was attempted or reported unavailable), or `blocked` (only the user can unblock: missing user-only information, an irreversible/externally visible operation, or changed scope). The host validates your claim against Delivery acceptance criteria and decides whether to continue automatically.";

/// Go `updateGoal.Schema()`, verbatim.
const GO_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "status":{"type":"string","enum":["continue","complete","blocked"],"description":"continue = keep working autonomously; complete = the goal is fully done and verified; blocked = only the user can unblock."},
  "reason":{"type":"string","description":"Short explanation. REQUIRED for continue and blocked; optional for complete."},
  "next_action":{"type":"string","description":"Optional concrete next step. Recommended for continue so the host can guide the next turn."},
  "completion":{
    "type":"object",
    "description":"Your own account of the finished work.",
    "properties":{
      "verified":{"type":"array","items":{"type":"string"},"description":"Commands you ran as proof, as they actually ran. One that never ran, failed, or predates your latest change is recorded as an unbacked claim."},
      "unverified":{"type":"array","items":{"type":"string"},"description":"What you did NOT verify. The host cannot infer what you skipped, so stating it is the only way it is known — and it never blocks completion."},
      "risks":{"type":"array","items":{"type":"string"},"description":"Known risks to carry forward."}
    }
  }
},
"required":["status"]
}"#;

/// The model's structured per-turn goal disposition recorded via the tool
/// (Go `tool.GoalReport`). It carries candidate state only; the host commits
/// the real FSM transition after the turn ends.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GoalReport {
    /// One of `continue`, `complete`, or `blocked`.
    pub status: String,
    /// Short explanation (required for continue and blocked).
    pub reason: String,
    /// Optional concrete next step (recommended for continue).
    pub next_action: String,
}

/// Records the report for the active goal turn. The host controller
/// implements it; it is absent in ordinary chat, where the tool fails closed.
pub trait GoalTurnRecorder: Send + Sync {
    /// Validates the report against the turn's goal lifecycle (scope/epoch
    /// binding, idempotency, terminal-state conflicts) and returns the
    /// tool-result text shown to the model.
    fn record_goal_report(
        &self,
        report: GoalReport,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}

/// The recorder bound to one call, when the host installed one (Go
/// `GoalTurnRecorderFromContext`).
pub fn goal_recorder_from(ctx: &CallContext) -> Option<Arc<dyn GoalTurnRecorder>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn GoalTurnRecorder>>()
        .cloned()
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

fn invalid(message: impl Into<String>) -> ToolResult {
    Err(invalid_err(message))
}

/// The one refusal identity both availability paths return, so the identity
/// cannot be present on the gate and missing on the stale-transcript path.
fn no_active_goal_turn() -> Refusal {
    Refusal::code(
        CODE_GOAL_NO_ACTIVE_TURN,
        "update_goal is only available while an active goal turn is running — no goal state was changed",
    )
}

/// The `update_goal` tool (Go `updateGoal`).
#[derive(Debug, Default, Clone, Copy)]
pub struct UpdateGoalTool;

/// The one instance a composition root registers.
pub fn update_goal_tool() -> Arc<dyn Tool> {
    Arc::new(UpdateGoalTool)
}

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default)]
    status: String,
    #[serde(default)]
    reason: String,
    #[serde(default, rename = "next_action")]
    next_action: String,
    #[serde(default)]
    completion: Completion,
}

#[derive(Deserialize, Default)]
struct Completion {
    #[serde(default)]
    verified: Vec<String>,
    #[serde(default)]
    unverified: Vec<String>,
    #[serde(default)]
    risks: Vec<String>,
}

impl Tool for UpdateGoalTool {
    fn name(&self) -> &str {
        UPDATE_GOAL_TOOL_NAME
    }

    fn description(&self) -> &str {
        UPDATE_GOAL_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(GO_SCHEMA).expect("the update_goal schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let args = match args.as_object() {
            Some(_) => args.clone(),
            None => Value::Object(Default::default()),
        };
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| invalid_err(format!("invalid update_goal args: {e}")))?;

        let status = parsed.status.trim().to_lowercase();
        match status.as_str() {
            "continue" | "complete" | "blocked" => {}
            other => {
                return invalid(format!(
                    "update_goal: status must be one of continue|complete|blocked, got {other:?} — no goal state was changed"
                ));
            }
        }
        if (status == "continue" || status == "blocked") && parsed.reason.trim().is_empty() {
            return invalid(format!(
                "update_goal: reason is required for {status} — no goal state was changed"
            ));
        }
        for (i, command) in parsed.completion.verified.iter().enumerate() {
            if command.trim().is_empty() {
                return invalid(format!(
                    "update_goal: completion.verified[{i}] is empty — cite the command as it actually ran, or leave the list out"
                ));
            }
        }

        let Some(recorder) = goal_recorder_from(ctx) else {
            return Err(no_active_goal_turn().into());
        };
        let result = recorder.record_goal_report(GoalReport {
            status: status.clone(),
            reason: parsed.reason.trim().to_owned(),
            next_action: parsed.next_action.trim().to_owned(),
        });
        let (result, err) = match result {
            Ok(text) => (text, None),
            Err(err) => (String::new(), Some(err)),
        };
        if let Some(err) = err {
            return Err(err);
        }
        if status != "complete" {
            return Ok(result);
        }
        let note = completion_note(
            parsed.completion.verified.len(),
            parsed.completion.unverified.len(),
            parsed.completion.risks.len(),
        );
        Ok(result + &note)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "contextual",
                Arc::new(GoalContextual) as Arc<dyn ContextualTool>,
            )
            .with(
                "contextual_reasoner",
                Arc::new(GoalReasoner) as Arc<dyn ContextualReasoner>,
            )
            .with(
                "host_required",
                Arc::new(HostGoalMode) as Arc<dyn HostRequired>,
            )
            .with(
                "plan_mode_safe",
                Arc::new(PlanSafe) as Arc<dyn PlanModeClassifier>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct GoalContextual;
impl Capability for GoalContextual {}
impl ContextualTool for GoalContextual {
    fn provider_visible(&self, ctx: &CallContext) -> bool {
        goal_recorder_from(ctx).is_some()
    }
}

struct GoalReasoner;
impl Capability for GoalReasoner {}
impl ContextualTool for GoalReasoner {
    fn provider_visible(&self, ctx: &CallContext) -> bool {
        goal_recorder_from(ctx).is_some()
    }
}
impl ContextualReasoner for GoalReasoner {
    fn unavailable(&self, _ctx: &CallContext) -> Refusal {
        no_active_goal_turn()
    }
}

struct HostGoalMode;
impl Capability for HostGoalMode {}
impl HostRequired for HostGoalMode {
    fn host_required_by(&self) -> String {
        FEATURE_GOAL_MODE.to_owned()
    }
}

struct PlanSafe;
impl Capability for PlanSafe {}
impl PlanModeClassifier for PlanSafe {
    fn plan_mode_safe(&self) -> bool {
        true
    }
}

/// Tells the model what its own completion account will contribute to the
/// host's completion report. The claim is reconciled against real receipts
/// after the turn; it is recorded, never trusted (Go `completionNote`).
pub fn completion_note(verified: usize, unverified: usize, risks: usize) -> String {
    if verified + unverified + risks == 0 {
        return " No completion account was given: the report will carry only what the host could observe.".to_owned();
    }
    format!(
        " Completion account recorded ({verified} verified, {unverified} unverified, {risks} risks); the verified commands are reconciled against this session's receipts."
    )
}

/// The schema as one JSON value, for tests and contract reads.
pub fn schema_value() -> Value {
    serde_json::from_str(GO_SCHEMA).expect("the update_goal schema is valid JSON")
}
