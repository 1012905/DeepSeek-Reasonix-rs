//! The planning face (Go `internal/tools/builtin/todo.go`, `completestep.go`):
//! `todo_write` records the serial task list and `complete_step` signs the one
//! current item off. The list state machine itself lives in [`plan_evidence`];
//! ledger receipts, baseline continuity and plan-mode activation are host
//! seams.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::abi::{
    CallContext, Capability, CapabilitySet, ContextualReasoner, ContextualTool, PlanModeClassifier,
    SequentialTool, Tool, ToolResult,
};
use crate::builtin::plan_evidence::{
    TodoItem, TodoStepMatch, advance_serial_todo, first_unfinished_sub_step, in_progress_todo,
    incomplete_todos, match_step, match_step_id, match_todo_identity,
    preserves_completed_todo_positions, same_todos, todo_citation, todo_step_ids,
    validate_serial_todos,
};
use crate::refusal::Refusal;
use crate::registry::{FEATURE_PLAN_STEPS, HostRequired};

/// `todo_write`, the tool name the model calls.
pub const TODO_WRITE_TOOL_NAME: &str = "todo_write";
/// `complete_step`, the tool name the model calls.
pub const COMPLETE_STEP_TOOL_NAME: &str = "complete_step";

/// `plan.mode_active` — complete_step is only available after plan approval.
pub const CODE_PLAN_MODE_ACTIVE: &str = "plan.mode_active";

/// Go `todoWrite.Description()`, verbatim.
pub const TODO_WRITE_DESCRIPTION: &str = "Record and update a structured task list for the current work. Send the COMPLETE list every call — it replaces the previous one. Use it to plan multi-step work and show progress: keep exactly one item in_progress at a time, and flip an item to completed the moment it's done (don't batch completions). Skip it for trivial single-step tasks. The list is two-level: a `level` 0 item is a PHASE (a milestone) and the `level` 1 items after it are its concrete sub-steps. COPY `step_id` VERBATIM for every item that already has one: it is how a completion stays attached to its step when you retitle it, insert a step above it, or reorder the list. Give a new item a fresh unique id (e.g. \"plan_step_07\"); never reuse or renumber an existing one.";

/// Go `completeStep.Description()`, verbatim.
pub const COMPLETE_STEP_DESCRIPTION: &str = "Mark ONE step of an approved plan as done. Call it as you finish each step. The host advances the list for you: it marks this step completed and moves the next to in_progress, so a todo_write that marks such an item completed on its own is REJECTED. Sign off one step per tool-call ROUND, not one per call: the sign-off promotes the next item, and a second sign-off sent beside it signs against the list as it stood before.";

const TODO_WRITE_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "todos":{
    "type":"array",
    "description":"The complete task list, in order. Replaces any previous list.",
    "items":{
      "type":"object",
      "properties":{
        "content":{"type":"string","description":"Imperative description of the task."},
        "status":{"type":"string","enum":["pending","in_progress","completed"],"description":"Task state. Keep at most one in_progress."},
        "activeForm":{"type":"string","description":"Present-continuous form shown while the task is in progress (e.g. \"Running tests\")."},
        "level":{"type":"integer","minimum":0,"maximum":1,"description":"0 = phase/milestone, 1 = a sub-step of the phase above it. Omit for a flat list."},
        "step_id":{"type":"string","description":"Stable identity for this item, e.g. \"plan_step_02\". Copy it verbatim from the item's previous entry so completions stay attached across retitles, insertions, and reordering; use a fresh unique id for a genuinely new item."}
      },
      "required":["content","status"]
    }
  }
},
"required":["todos"]
}"#;

const COMPLETE_STEP_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "step_id":{"type":"string","description":"PREFERRED: the stable step_id of the task-list item this completes, e.g. \"plan_step_02\". Unlike a title or a number it survives retitles, insertions, and reordering, so cite it whenever the item has one."},
  "step":{"type":"string","description":"Which plan step this completes — its title or number, matching the task list. Use only when the item has no step_id."},
  "step_index":{"type":"integer","minimum":1,"description":"Optional 1-based task-list item number. Use only when the item has no step_id; an index goes stale the moment a step is inserted above it."},
  "result":{"type":"string","description":"What is now true or changed as a result of finishing this step."},
  "notes":{"type":"string","description":"Optional caveats, follow-ups, or anything deferred."}
},
"required":["result"]
}"#;

/// A valid serial list the rules' refusals end with, so a model does not have to
/// guess a whole legal shape from the violations alone (Go `validSerialListExample`).
const VALID_SERIAL_LIST_EXAMPLE: &str = "; a valid serial list: {\"todos\":[{\"content\":\"Phase\",\"status\":\"pending\",\"level\":0},{\"content\":\"First sub-step\",\"status\":\"in_progress\",\"level\":1},{\"content\":\"Second sub-step\",\"status\":\"pending\",\"level\":1}]}; a flat list needs the first unfinished item in_progress";

/// Host-side task-list state the two tools read against (Go `evidence.LatestTodos`,
/// `UnverifiedCompletedTodos`, plan-replacement authorization). The host composes
/// ledger receipts and any session-stored baseline behind this seam.
pub trait PlanHost: Send + Sync {
    /// The latest recorded task list (ledger receipts first, state fallback).
    fn latest_todos(&self) -> Vec<TodoItem>;
    /// Completed items in `current` with no matching successful complete_step
    /// receipt this turn, plus whether a receipt baseline exists.
    fn unverified_completed(&self, current: &[TodoItem]) -> (Vec<TodoStepMatch>, bool);
    /// Whether this call carries plan-replacement authorization, which waives
    /// the continuity/id-preservation rewrites (Go `HasPlanReplacementAuthorization`).
    fn plan_replacement_authorized(&self) -> bool {
        false
    }
}

/// The plan host bound to one call, when installed.
pub fn plan_host_from(ctx: &CallContext) -> Option<Arc<dyn PlanHost>> {
    ctx.value()?.downcast_ref::<Arc<dyn PlanHost>>().cloned()
}

/// Reads whether the planning phase is active (Go `planmode.Active`). Absent
/// binding means not active, matching the Go context's zero value.
pub trait PlanModeReader: Send + Sync {
    fn plan_active(&self) -> bool;
}

pub fn plan_mode_from(ctx: &CallContext) -> Option<Arc<dyn PlanModeReader>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn PlanModeReader>>()
        .cloned()
}

fn plan_active(ctx: &CallContext) -> bool {
    plan_mode_from(ctx).is_some_and(|r| r.plan_active())
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

fn invalid(message: impl Into<String>) -> ToolResult {
    Err(invalid_err(message))
}

#[derive(Deserialize, Default)]
struct TodoWriteArgs {
    #[serde(default)]
    todos: Vec<TodoItemRaw>,
}

#[derive(Deserialize, Default)]
struct TodoItemRaw {
    #[serde(default)]
    content: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "activeForm", default)]
    active_form: String,
    #[serde(default)]
    level: i64,
    #[serde(rename = "step_id", default)]
    step_id: String,
}

impl TodoItemRaw {
    fn to_evidence(&self) -> TodoItem {
        TodoItem {
            content: self.content.clone(),
            status: self.status.clone(),
            active_form: self.active_form.clone(),
            level: self.level,
            step_id: self.step_id.trim().to_owned(),
        }
    }
}

/// The `todo_write` tool (Go `todoWrite`).
#[derive(Debug, Default, Clone, Copy)]
pub struct TodoWriteTool;

/// The one instance a composition root registers.
pub fn todo_write_tool() -> Arc<dyn Tool> {
    Arc::new(TodoWriteTool)
}

#[derive(Deserialize, Default)]
struct CompleteStepArgs {
    #[serde(rename = "step_id", default)]
    step_id: String,
    #[serde(default)]
    step: String,
    #[serde(rename = "step_index", default)]
    step_index: i64,
    #[serde(default)]
    result: String,
}

/// The `complete_step` tool (Go `completeStep`).
#[derive(Debug, Default, Clone, Copy)]
pub struct CompleteStepTool;

/// The one instance a composition root registers.
pub fn complete_step_tool() -> Arc<dyn Tool> {
    Arc::new(CompleteStepTool)
}

/// The plan face for one composition root.
pub fn plan_tools() -> Vec<Arc<dyn Tool>> {
    vec![todo_write_tool(), complete_step_tool()]
}

impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        TODO_WRITE_TOOL_NAME
    }

    fn description(&self) -> &str {
        TODO_WRITE_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(TODO_WRITE_SCHEMA).expect("the todo_write schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: TodoWriteArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;

        let mut done = 0usize;
        let mut active = 0usize;
        let mut pending = 0usize;
        for (i, t) in parsed.todos.iter().enumerate() {
            if t.content.is_empty() {
                return invalid(format!("todo {}: content is required", i + 1));
            }
            if t.level < 0 || t.level > 1 {
                return invalid(format!(
                    "todo {}: invalid level {} (want 0 phase | 1 sub-step)",
                    i + 1,
                    t.level
                ));
            }
            match t.status.as_str() {
                "completed" => done += 1,
                "in_progress" => active += 1,
                "pending" | "" => pending += 1,
                other => {
                    return invalid(format!(
                        "todo {}: invalid status {other:?} (want pending|in_progress|completed)",
                        i + 1
                    ));
                }
            }
        }

        let evidence: Vec<TodoItem> = parsed.todos.iter().map(|t| t.to_evidence()).collect();

        validate_serial_todos(&evidence)
            .map_err(|e| invalid_err(format!("{e}{VALID_SERIAL_LIST_EXAMPLE}")))?;
        verify_unique_step_ids(&evidence)?;

        let host = plan_host_from(ctx);
        let authorized = host
            .as_ref()
            .is_some_and(|h| h.plan_replacement_authorized());
        if !authorized {
            let baseline = host.as_ref().map_or(Vec::new(), |h| h.latest_todos());
            if let Err(e) = verify_todo_current_continuity(&baseline, &evidence) {
                return Err(invalid_err(e));
            }
            if let Err(e) = verify_step_ids_preserved(&baseline, &evidence) {
                return Err(invalid_err(e));
            }
        }
        if let Err(e) = verify_completed_todo_positions(&host, &evidence) {
            return Err(invalid_err(e));
        }
        if let Err(e) = verify_todo_completion_transitions(&host, &evidence) {
            return Err(invalid_err(e));
        }

        let baseline = host.as_ref().map_or(Vec::new(), |h| h.latest_todos());
        let hint = signable_step_hint(&evidence);
        if same_todos(&baseline, &evidence) {
            return Ok(format!(
                "Task list unchanged: {} total — {} completed, {} in progress, {} pending. The host already holds this list.{hint}",
                evidence.len(),
                done,
                active,
                pending
            ));
        }
        Ok(format!(
            "Todos updated: {} total — {} completed, {} in progress, {} pending.{hint}",
            evidence.len(),
            done,
            active,
            pending
        ))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "sequential",
                Arc::new(PlanSequential) as Arc<dyn SequentialTool>,
            )
            .with(
                "host_required",
                Arc::new(HostPlanSteps) as Arc<dyn HostRequired>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct PlanSequential;
impl Capability for PlanSequential {}
impl SequentialTool for PlanSequential {
    fn sequential(&self, _ctx: &CallContext, _args: &Value) -> bool {
        true
    }
}

struct HostPlanSteps;
impl Capability for HostPlanSteps {}
impl HostRequired for HostPlanSteps {
    fn host_required_by(&self) -> String {
        FEATURE_PLAN_STEPS.to_owned()
    }
}

fn signable_step_hint(todos: &[TodoItem]) -> String {
    let (m, ok) = in_progress_todo(todos);
    if !ok || m.step_id.is_empty() {
        return String::new();
    }
    format!(
        " The in_progress item is {:?} — cite that step_id when you sign it off.",
        m.step_id
    )
}

fn verify_unique_step_ids(todos: &[TodoItem]) -> ToolResult {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, todo) in todos.iter().enumerate() {
        let id = todo.step_id.trim();
        if id.is_empty() {
            continue;
        }
        if let Some(&prev) = seen.get(id) {
            return invalid(format!(
                "todo {} {:?} reuses step_id {id:?}, already claimed by todo {}; give a new item its own id",
                i + 1,
                todo.content,
                prev + 1
            ));
        }
        seen.insert(id.to_owned(), i);
    }
    Ok(String::new())
}

fn verify_step_ids_preserved(previous: &[TodoItem], next: &[TodoItem]) -> Result<(), String> {
    if previous.is_empty() {
        return Ok(());
    }
    for todo in previous {
        if todo.step_id.is_empty() {
            continue;
        }
        if match_step_id(&todo.step_id, next).1 {
            continue;
        }
        let (match_, found) = match_todo_identity(todo, next);
        if !found || !match_.step_id.is_empty() {
            continue;
        }
        return Err(format!(
            "todo {} {:?} dropped its step_id {:?}; re-send it with step_id {:?} so its completion stays attached across retitles and reordering",
            match_.index, match_.content, todo.step_id, todo.step_id
        ));
    }
    Ok(())
}

fn verify_todo_current_continuity(previous: &[TodoItem], todos: &[TodoItem]) -> Result<(), String> {
    if previous.is_empty() {
        return Ok(());
    }
    for (i, todo) in previous.iter().enumerate() {
        if todo.status.trim() != "in_progress" {
            continue;
        }
        let (match_, found) = match_todo_identity(todo, todos);
        if !found {
            return Err(format!(
                "current todo {} {:?} cannot be removed or replaced while it is in_progress; complete it with complete_step before changing the remaining list",
                i + 1,
                todo.content
            ));
        }
        if match_.status == "pending" || match_.status.is_empty() {
            return Err(format!(
                "current todo {} {:?} cannot move back to pending; keep it in_progress or complete it with complete_step",
                i + 1,
                todo.content
            ));
        }
    }
    Ok(())
}

fn verify_completed_todo_positions(
    host: &Option<Arc<dyn PlanHost>>,
    todos: &[TodoItem],
) -> Result<(), String> {
    let previous = host.as_ref().map_or(Vec::new(), |h| h.latest_todos());
    if previous.is_empty() {
        return Ok(());
    }
    for (i, todo) in todos.iter().enumerate() {
        if todo.status != "completed" {
            continue;
        }
        let (match_, found) = match_todo_identity(todo, &previous);
        if !found || match_.index != i + 1 {
            return Err(format!(
                "completed todo {} {:?} cannot be inserted, duplicated, or reordered; preserve the completed prefix and sign off the current item with complete_step",
                i + 1,
                todo.content
            ));
        }
    }
    if !incomplete_todos(&previous).is_empty()
        && !preserves_completed_todo_positions(&previous, todos)
    {
        return Err("completed task history cannot be removed, changed, or reordered while the plan is active; preserve every completed item at its original position".to_owned());
    }
    Ok(())
}

fn verify_todo_completion_transitions(
    host: &Option<Arc<dyn PlanHost>>,
    todos: &[TodoItem],
) -> Result<(), String> {
    let Some(host) = host else { return Ok(()) };
    let (missing, has_baseline) = host.unverified_completed(todos);
    if !has_baseline {
        let previous = host.latest_todos();
        if !previous.is_empty() {
            for (i, todo) in todos.iter().enumerate() {
                if todo.status != "completed" {
                    continue;
                }
                let (match_, found) = match_todo_identity(todo, &previous);
                if !found || match_.status != "completed" {
                    return Err(format!(
                        "todo {} {:?} cannot become completed without signing off the current item with complete_step",
                        i + 1,
                        todo.content
                    ));
                }
            }
            return Ok(());
        }
        for (i, todo) in todos.iter().enumerate() {
            if todo.status == "completed" {
                return Err(format!(
                    "initial todo {} {:?} cannot start completed; establish the task list before doing the work, then sign off the current item with complete_step",
                    i + 1,
                    todo.content
                ));
            }
        }
        return Ok(());
    }
    if missing.is_empty() {
        return Ok(());
    }
    const HINT: &str = "; sign each finished item off with complete_step first — it advances the list for you, so send this todo_write again only to change something else about it";
    if missing.len() == 1 {
        let m = &missing[0];
        return Err(format!(
            "todo {} {:?} is newly completed but has no matching successful complete_step receipt in this turn{HINT}",
            m.index, m.content
        ));
    }
    Err(format!(
        "{} todos are newly completed but have no matching successful complete_step receipts in this turn{HINT}",
        missing.len()
    ))
}

impl Tool for CompleteStepTool {
    fn name(&self) -> &str {
        COMPLETE_STEP_TOOL_NAME
    }

    fn description(&self) -> &str {
        COMPLETE_STEP_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(COMPLETE_STEP_SCHEMA).expect("the complete_step schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: CompleteStepArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;

        let step = complete_step_identity(&parsed.step_id, &parsed.step, parsed.step_index);
        if step.is_empty() {
            return invalid(
                "step_id, step, or step_index is required — cite the task-list item you are completing, preferring its stable step_id",
            );
        }
        if parsed.step_index < 0 {
            return invalid("step_index must be a positive 1-based task-list number");
        }
        if parsed.result.trim().is_empty() {
            return invalid(
                "result is required — state what is now true after finishing this step",
            );
        }

        let host = plan_host_from(ctx);
        let todos = host.as_ref().map_or(Vec::new(), |h| h.latest_todos());
        let (todo_match, has_todo) = verify_todo_step(&step, &todos)?;

        Ok(format!(
            "Step {step:?} done.{}",
            plan_advance_status(ctx, &todos, &todo_match, has_todo)
        ))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "contextual",
                Arc::new(CompleteContextual) as Arc<dyn ContextualTool>,
            )
            .with(
                "contextual_reasoner",
                Arc::new(CompleteReasoner) as Arc<dyn ContextualReasoner>,
            )
            .with(
                "sequential",
                Arc::new(PlanSequential) as Arc<dyn SequentialTool>,
            )
            .with(
                "host_required",
                Arc::new(HostPlanSteps) as Arc<dyn HostRequired>,
            )
            .with(
                "plan_mode_safe",
                Arc::new(PlanUnsafe) as Arc<dyn PlanModeClassifier>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct CompleteContextual;
impl Capability for CompleteContextual {}
impl ContextualTool for CompleteContextual {
    fn provider_visible(&self, ctx: &CallContext) -> bool {
        !plan_active(ctx)
    }
}

struct CompleteReasoner;
impl Capability for CompleteReasoner {}
impl ContextualTool for CompleteReasoner {
    fn provider_visible(&self, ctx: &CallContext) -> bool {
        !plan_active(ctx)
    }
}
impl ContextualReasoner for CompleteReasoner {
    fn unavailable(&self, _ctx: &CallContext) -> Refusal {
        Refusal::code(
            CODE_PLAN_MODE_ACTIVE,
            "blocked: complete_step is only available after plan approval. While planning, keep task state with todo_write and present the plan for user approval.",
        )
    }
}

struct PlanUnsafe;
impl Capability for PlanUnsafe {}
impl PlanModeClassifier for PlanUnsafe {
    fn plan_mode_safe(&self) -> bool {
        false
    }
}

fn complete_step_identity(step_id: &str, step: &str, step_index: i64) -> String {
    if !step_id.trim().is_empty() {
        return step_id.trim().to_owned();
    }
    if step_index > 0 {
        return step_index.to_string();
    }
    step.trim().to_owned()
}

fn verify_todo_step(
    step: &str,
    todos: &[TodoItem],
) -> Result<(TodoStepMatch, bool), Box<dyn std::error::Error + Send + Sync>> {
    if todos.is_empty() {
        return Ok((TodoStepMatch::default(), false));
    }
    let (match_, found) = match_step(step, todos);
    if !found {
        let all_completed = todos.iter().all(|t| t.status.trim() == "completed");
        if all_completed {
            let last = todos.len() - 1;
            return Err(invalid_err(format!(
                "step {step:?} has no matching todo_write item and every current todo is already completed; this is a renewal sign-off, so retry complete_step with step_index {} (the final existing todo {:?}) — do not invent a new step or rewrite the completed list",
                last + 1,
                todos[last].content
            )));
        }
        let ids = todo_step_ids(todos);
        if !ids.is_empty() {
            return Err(invalid_err(format!(
                "step {step:?} has no matching todo_write item in the current task list; cite the item's stable step_id — available ids: {} (list: {})",
                ids.join(", "),
                todo_list_inventory(todos)
            )));
        }
        return Err(invalid_err(format!(
            "step {step:?} has no matching todo_write item in the current task list; cite a todo verbatim or by number: {}",
            todo_list_inventory(todos)
        )));
    }
    match match_.status.as_str() {
        "in_progress" => {
            if let (unfinished, true) = first_unfinished_sub_step(todos, match_.index as isize - 1)
                && unfinished >= 0
            {
                return Err(invalid_err(format!(
                    "step {step:?} matches phase {} {:?} whose sub-steps are unfinished; complete sub-step {} {:?} first, then sign the phase off",
                    match_.index,
                    match_.content,
                    unfinished as usize + 1,
                    todos[unfinished as usize].content
                )));
            }
            Ok((match_, true))
        }
        "completed" => Ok((match_, true)),
        "" | "pending" => {
            let mut current = String::new();
            for (i, todo) in todos.iter().enumerate() {
                if todo.status.trim() != "in_progress" {
                    continue;
                }
                current = format!("; finish todo {} {:?} first", i + 1, todo.content);
                if todo.level == 1 {
                    break;
                }
            }
            Err(invalid_err(format!(
                "step {step:?} matches pending todo {} {:?}; complete_step only signs the current in_progress item{current}",
                match_.index, match_.content
            )))
        }
        other => Err(invalid_err(format!(
            "step {step:?} matches todo {} ({:?}) but its status is {other:?}; complete_step requires in_progress or completed",
            match_.index, match_.content,
        ))),
    }
}

fn plan_advance_status(
    ctx: &CallContext,
    todos: &[TodoItem],
    match_: &TodoStepMatch,
    has_todo: bool,
) -> String {
    if !has_todo {
        return " The host recorded the sign-off; there is no task list to advance.".to_owned();
    }
    if let Some((terminal, next)) = plan_advance_after(ctx, todos, match_) {
        if terminal {
            return " Every step in the task list is now complete. Do not open new work under this plan.".to_owned();
        }
        if !next.content.is_empty() {
            return format!(
                " The host advanced the task list; the next step is {} {:?}.",
                next.index, next.content
            );
        }
    }
    if match_.status == "completed" {
        return " The matched todo was already completed; the task list is unchanged.".to_owned();
    }
    " The host advanced the task list; continue with the next step.".to_owned()
}

fn plan_advance_after(
    ctx: &CallContext,
    todos: &[TodoItem],
    match_: &TodoStepMatch,
) -> Option<(bool, TodoStepMatch)> {
    let _ = ctx;
    if todos.is_empty() || match_.index == 0 || match_.index > todos.len() {
        return None;
    }
    let mut advanced = todos.to_vec();
    advance_serial_todo(&mut advanced, match_.index - 1);
    for (i, todo) in advanced.iter().enumerate() {
        if todo.status.trim() == "in_progress" {
            return Some((
                false,
                TodoStepMatch {
                    found: true,
                    index: i + 1,
                    content: todo.content.clone(),
                    status: todo.status.clone(),
                    active_form: todo.active_form.clone(),
                    step_id: todo.step_id.clone(),
                },
            ));
        }
    }
    Some((true, TodoStepMatch::default()))
}

fn todo_list_inventory(todos: &[TodoItem]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(todos.len());
    for (i, t) in todos.iter().enumerate() {
        let mut content = t.content.clone();
        let runes: Vec<char> = content.chars().collect();
        if runes.len() > 60 {
            content = runes[..60].iter().collect::<String>() + "…";
        }
        parts.push(todo_citation(&t.step_id, i + 1, &format!("{content:?}")));
        if parts.len() == 12 && todos.len() > 12 {
            parts.push(format!("… {} more", todos.len() - 12));
            break;
        }
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_valid_json() {
        let _ = TodoWriteTool.schema();
        let _ = CompleteStepTool.schema();
    }

    #[test]
    fn plan_advance_after_replays_the_host_advance() {
        let todos = vec![
            TodoItem {
                content: "First".into(),
                status: "in_progress".into(),
                ..Default::default()
            },
            TodoItem {
                content: "Second".into(),
                status: "pending".into(),
                ..Default::default()
            },
        ];
        let m = TodoStepMatch {
            found: true,
            index: 1,
            content: "First".into(),
            status: "in_progress".into(),
            ..Default::default()
        };
        let (terminal, next) = plan_advance_after(&CallContext::default(), &todos, &m)
            .expect("advance reports a result");
        assert!(!terminal);
        assert_eq!(next.index, 2);
        assert_eq!(next.content, "Second");
    }

    #[test]
    fn plan_advance_after_terminal_when_none_left() {
        let todos = vec![TodoItem {
            content: "Only".into(),
            status: "in_progress".into(),
            ..Default::default()
        }];
        let m = TodoStepMatch {
            found: true,
            index: 1,
            content: "Only".into(),
            status: "in_progress".into(),
            ..Default::default()
        };
        let (terminal, next) = plan_advance_after(&CallContext::default(), &todos, &m)
            .expect("advance reports a result");
        assert!(terminal);
        assert!(next.content.is_empty());
    }

    #[test]
    fn inventory_truncates_long_contents_and_extra_count() {
        let todos: Vec<TodoItem> = (1..=13)
            .map(|i| TodoItem {
                content: format!(
                    "step number {i} with a deliberately long description that keeps going"
                ),
                status: "pending".into(),
                ..Default::default()
            })
            .collect();
        let inv = todo_list_inventory(&todos);
        assert!(inv.contains("… 1 more"));
        assert!(inv.contains("…"));
    }
}
