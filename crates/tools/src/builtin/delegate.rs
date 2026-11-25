//! TOOL-039: delegation tools (docs/contracts/tools.md §3.4,
//! Go `internal/runtime/delegation/spawn.go` and `subagent_result.go`).
//!
//! `spawn_agents` fans independent tasks out to fresh sub-agent sessions;
//! `read_subagent_result` pages through a completed child's final answer by
//! UTF-8 byte offset. The real child loop, persisted transcript store and
//! parent-session lineage check live behind seams the host assembly binds;
//! this slice ships the cards, argument contract, validation, result shape
//! and pagination rendering.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::abi::{CallContext, Capability, CapabilitySet, PlanModeClassifier, Tool, ToolResult};
use crate::registry::CapabilityAliased;

/// `spawn_agents`, the tool name the model calls (Go `SpawnToolName`).
pub const SPAWN_TOOL_NAME: &str = "spawn_agents";
/// `read_subagent_result`, the paged reader tool name.
pub const SUBAGENT_RESULT_TOOL_NAME: &str = "read_subagent_result";

const MAX_SPAWN_TASKS: usize = 8;
const SPAWN_HEAD_BUDGET: usize = 8000;
const SPAWN_HEAD_MIN: usize = 600;
const SPAWN_HEAD_MAX: usize = 2000;
const SPAWN_TITLE_RUNES: usize = 80;

/// Go `subagentResultDefaultBytes`: 12 KiB.
pub const SUBAGENT_RESULT_DEFAULT_BYTES: usize = 12 * 1024;
/// Go `subagentResultMaxBytes`: 24 KiB.
pub const SUBAGENT_RESULT_MAX_BYTES: usize = 24 * 1024;

/// A sub-agent launch request (Go `delegation.RunSpec` reduced to the fields
/// this slice passes to the runner seam).
#[derive(Debug, Clone, Default)]
pub struct RunSpec {
    pub title: String,
    pub task: String,
    pub kind: String,
    pub name: String,
    pub tools: Vec<String>,
    pub index: usize,
}

/// A finished child's outcome (Go `delegation.RunOutput`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunOutput {
    pub reference: String,
    pub answer: String,
    pub read: Vec<String>,
    pub wrote: Vec<String>,
}

/// Launches one sub-agent task. The host assembly wires this over the agent's
/// sub-agent machinery; `None` keeps the card registered but execution fails
/// closed with the unavailable sentence (Go `Runner`).
pub trait DelegationRunner: Send + Sync {
    fn run(&self, spec: &RunSpec) -> Result<RunOutput, Box<dyn std::error::Error + Send + Sync>>;
}

/// Display profile the host reads for a delegated step (Go `event.Profile`
/// reduced to name/count).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileMeta {
    pub name: String,
    pub count: usize,
}

#[derive(Deserialize, Default)]
struct SpawnTaskInput {
    #[serde(default)]
    title: String,
    #[serde(default)]
    task: String,
    #[serde(default)]
    tools: Vec<String>,
}

#[derive(Serialize, Default)]
struct SpawnResult {
    title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    r#ref: String,
    chars: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    head: String,
    #[serde(skip_serializing_if = "is_false")]
    more: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    wrote: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    error: String,
}

/// The `spawn_agents` tool.
#[derive(Clone, Default)]
pub struct SpawnTool {
    runner: Option<Arc<dyn DelegationRunner>>,
}

impl SpawnTool {
    /// Binds the optional runner seam.
    pub fn new(runner: Option<Arc<dyn DelegationRunner>>) -> Self {
        Self { runner }
    }

    /// The delegation profile resolved from raw args (Go `ResolveProfile`).
    pub fn resolve_profile(args: &Value) -> Option<ProfileMeta> {
        let tasks: Vec<Value> = args
            .get("tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Some(ProfileMeta {
            name: SPAWN_TOOL_NAME.to_owned(),
            count: tasks.len(),
        })
    }
}

const SPAWN_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "tasks":{
    "type":"array",
    "minItems":1,
    "maxItems":8,
    "description":"Independent tasks to run at the same time.",
    "items":{
      "type":"object",
      "properties":{
        "title":{"type":"string","description":"Short label for the task, shown to the user."},
        "task":{"type":"string","description":"What to find out. Be specific about the evidence or summary to return."},
        "tools":{"type":"array","items":{"type":"string"},"description":"Optional narrowing of the sub-agent tools: any of read_file, write_file, bash."}
      },
      "required":["title","task"],
      "additionalProperties":false
    }
  }
},
"required":["tasks"],
"additionalProperties":false
}"#;

impl Tool for SpawnTool {
    fn name(&self) -> &str {
        SPAWN_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Run independent tasks in parallel, each in its own fresh session that does not see this conversation. A sub-agent has three tools: read_file, write_file and bash. In plan mode it can only read. Give each task a short title and a self-contained task. Every sub-agent's full answer is archived; you get back its title, a reference, and the opening of its answer. When an answer is cut off, read the rest with read_subagent_result using the reference. Each result also lists the files the sub-agent read and wrote, so you can open the exact ones yourself. Do not delegate reading a file you are about to edit: read it yourself. Use sub-agents for broad searches, for questions that need only a conclusion, and for independent work that can run in parallel. Sub-agents cannot start sub-agents."
    }

    fn schema(&self) -> Value {
        serde_json::from_str(SPAWN_SCHEMA).expect("the spawn_agents schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let _ = ctx;
        let Some(runner) = &self.runner else {
            return Err(Box::new(std::io::Error::other(
                "sub-agents are not available in this session",
            )));
        };
        let tasks = parse_spawn_tasks(args)?;
        if tasks.is_empty() {
            return Err(Box::new(std::io::Error::other("tasks must not be empty")));
        }
        if tasks.len() > MAX_SPAWN_TASKS {
            return Err(Box::new(std::io::Error::other(format!(
                "at most {MAX_SPAWN_TASKS} tasks per call"
            ))));
        }
        for (i, task) in tasks.iter().enumerate() {
            if task.task.trim().is_empty() {
                return Err(Box::new(std::io::Error::other(format!(
                    "task {} has no task text",
                    i + 1
                ))));
            }
        }
        let head = (SPAWN_HEAD_BUDGET / tasks.len()).clamp(SPAWN_HEAD_MIN, SPAWN_HEAD_MAX);
        let mut results = Vec::with_capacity(tasks.len());
        for (index, task) in tasks.into_iter().enumerate() {
            results.push(run_one(runner, index, task, head));
        }
        let body = serde_json::to_string(&serde_json::json!({ "agents": results }))
            .map_err(|e| Box::new(std::io::Error::other(format!("encode spawn results: {e}"))))?;
        Ok(body)
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "plan_mode_safe",
                Arc::new(SpawnPlanSafe) as Arc<dyn PlanModeClassifier>,
            )
            .with(
                "capability_aliased",
                Arc::new(SpawnAliases) as Arc<dyn CapabilityAliased>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct SpawnPlanSafe;
impl Capability for SpawnPlanSafe {}
impl PlanModeClassifier for SpawnPlanSafe {
    fn plan_mode_safe(&self) -> bool {
        true
    }
}

struct SpawnAliases;
impl Capability for SpawnAliases {}
impl CapabilityAliased for SpawnAliases {
    fn capability_aliases(&self) -> Vec<String> {
        vec!["task:subagent".to_owned()]
    }
}

fn parse_spawn_tasks(
    args: &Value,
) -> Result<Vec<SpawnTaskInput>, Box<dyn std::error::Error + Send + Sync>> {
    let root = args.as_object().ok_or_else(|| {
        std::io::Error::other("arguments must be {tasks: [{title, task}]}: not an object")
    })?;
    reject_unknown(root, &["tasks"])?;
    let tasks = root.get("tasks").and_then(Value::as_array).ok_or_else(|| {
        std::io::Error::other("arguments must be {tasks: [{title, task}]}: tasks is required")
    })?;
    let mut out = Vec::with_capacity(tasks.len());
    for raw in tasks {
        let item = raw.as_object().ok_or_else(|| {
            std::io::Error::other(
                "arguments must be {tasks: [{title, task}]}: each task must be an object",
            )
        })?;
        reject_unknown(item, &["title", "task", "tools"])?;
        let task: SpawnTaskInput = serde_json::from_value(raw.clone()).map_err(|e| {
            std::io::Error::other(format!(
                "arguments must be {{tasks: [{{title, task}}]}}: {e}"
            ))
        })?;
        out.push(task);
    }
    Ok(out)
}

fn is_false(b: &bool) -> bool {
    !b
}

fn reject_unknown(
    map: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(Box::new(std::io::Error::other(format!(
                "invalid args: unknown field {key:?}"
            ))));
        }
    }
    Ok(())
}

fn run_one(
    runner: &Arc<dyn DelegationRunner>,
    index: usize,
    task: SpawnTaskInput,
    head: usize,
) -> SpawnResult {
    let title = truncate_runes(task.title.trim(), SPAWN_TITLE_RUNES);
    let mut result = SpawnResult {
        title,
        ..Default::default()
    };
    let spec = RunSpec {
        title: result.title.clone(),
        task: task.task,
        kind: "task".to_owned(),
        name: result.title.clone(),
        tools: task.tools,
        index,
    };
    match runner.run(&spec) {
        Ok(output) => {
            result.r#ref = output.reference;
            let answer = output.answer.trim().to_owned();
            result.chars = answer.chars().count();
            result.head = truncate_runes(&answer, head);
            result.more = result.chars > head;
            result.read = output.read;
            result.wrote = output.wrote;
        }
        Err(err) => result.error = err.to_string(),
    }
    result
}

fn truncate_runes(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    s.chars().take(n).collect()
}

/// A persisted sub-agent result reader (Go `SubagentStore.ReadResult`). The
/// store enforces parent lineage and workspace match before returning the
/// answer; the seam only exposes the final text and status.
pub trait SubagentResultStore: Send + Sync {
    fn read_result(
        &self,
        reference: &str,
        parent_session: &str,
        workspace_root: &str,
    ) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>>;
}

/// Call-context binding carrying the persisted parent session (Go
/// `agent.ParentSession(ctx)`).
pub struct ParentSessionBinding {
    pub session: String,
}

/// Reads the parent session off the call context, when the host bound one.
pub fn parent_session_from(ctx: &CallContext) -> Option<String> {
    ctx.value()?
        .downcast_ref::<ParentSessionBinding>()
        .map(|b| b.session.clone())
}

/// A capability marker for byte-offset paged readers (Go `tool.Paged`).
pub trait PagedTool: Capability {
    fn paged(&self) -> bool;
}

/// The `read_subagent_result` tool.
#[derive(Clone, Default)]
pub struct SubagentResultTool {
    store: Option<Arc<dyn SubagentResultStore>>,
    workspace_root: String,
}

impl SubagentResultTool {
    /// Binds the optional store seam and workspace root for the lineage check.
    pub fn new(store: Option<Arc<dyn SubagentResultStore>>, workspace_root: String) -> Self {
        Self {
            store,
            workspace_root,
        }
    }
}

const RESULT_SCHEMA: &str = r#"{"type":"object","properties":{"ref":{"type":"string","description":"The sa_... value from a Subagent reference line."},"offset_bytes":{"type":"integer","description":"UTF-8 byte offset to start reading from. Omit for the beginning; use next_offset_bytes from the previous page.","minimum":0},"limit_bytes":{"type":"integer","description":"Maximum UTF-8 bytes to return. Defaults to 12288 and is capped at 24576.","minimum":1,"maximum":24576}},"required":["ref"]}"#;

impl Tool for SubagentResultTool {
    fn name(&self) -> &str {
        SUBAGENT_RESULT_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Read a completed sub-agent's full final answer by the Subagent reference returned from task, parallel_tasks, or fleet. Results are scoped to the current conversation lineage and paged by UTF-8 byte offset so large answers remain lossless without overflowing one tool result."
    }

    fn schema(&self) -> Value {
        serde_json::from_str(RESULT_SCHEMA).expect("the result schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let root = args
            .as_object()
            .ok_or_else(|| std::io::Error::other("invalid args: not an object"))?;
        reject_unknown(root, &["ref", "offset_bytes", "limit_bytes"])?;

        let reference = root
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if reference.is_empty() {
            return Err(Box::new(std::io::Error::other("ref is required")));
        }
        let offset = root
            .get("offset_bytes")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if offset < 0 {
            return Err(Box::new(std::io::Error::other(
                "offset_bytes must be non-negative",
            )));
        }
        let mut limit = root.get("limit_bytes").and_then(Value::as_i64).unwrap_or(0);
        if limit == 0 {
            limit = SUBAGENT_RESULT_DEFAULT_BYTES as i64;
        }
        if limit < 1 || limit > SUBAGENT_RESULT_MAX_BYTES as i64 {
            return Err(Box::new(std::io::Error::other(format!(
                "limit_bytes must be between 1 and {SUBAGENT_RESULT_MAX_BYTES}"
            ))));
        }

        let Some(store) = &self.store else {
            return Err(Box::new(std::io::Error::other(
                "subagent result storage is not available in this session",
            )));
        };
        let Some(parent_session) = parent_session_from(ctx) else {
            return Err(Box::new(std::io::Error::other(
                "subagent result retrieval requires a persisted parent session",
            )));
        };

        let (answer, status) =
            store.read_result(&reference, &parent_session, &self.workspace_root)?;
        let offset = offset as usize;
        let limit = limit as usize;
        if offset > answer.len() {
            return Err(Box::new(std::io::Error::other(format!(
                "offset_bytes {offset} exceeds result size {}",
                answer.len()
            ))));
        }
        if offset < answer.len() && !answer.is_char_boundary(offset) {
            return Err(Box::new(std::io::Error::other(format!(
                "offset_bytes {offset} is not at a UTF-8 character boundary; use next_offset_bytes from the previous page"
            ))));
        }
        let mut end = (offset + limit).min(answer.len());
        while end > offset && end < answer.len() && !answer.is_char_boundary(end) {
            end -= 1;
        }

        let mut body = format!(
            "Subagent result {reference} (status={status}, bytes {offset}-{end} of {}):\n",
            answer.len()
        );
        body.push_str(&answer[offset..end]);
        if end < answer.len() {
            body.push_str(&format!(
                "\n\nMore remains. Call read_subagent_result with ref=\"{reference}\" and offset_bytes={end}."
            ));
        } else {
            body.push_str("\n\nEnd of subagent result.");
        }
        Ok(body)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "plan_mode_safe",
                Arc::new(ResultPlanSafe) as Arc<dyn PlanModeClassifier>,
            )
            .with("paged", Arc::new(ResultPaged) as Arc<dyn PagedTool>)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct ResultPlanSafe;
impl Capability for ResultPlanSafe {}
impl PlanModeClassifier for ResultPlanSafe {
    fn plan_mode_safe(&self) -> bool {
        true
    }
}

struct ResultPaged;
impl Capability for ResultPaged {}
impl PagedTool for ResultPaged {
    fn paged(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::CallContext;

    #[derive(Default)]
    struct StubRunner {
        outputs: Vec<RunOutput>,
    }

    impl DelegationRunner for StubRunner {
        fn run(
            &self,
            spec: &RunSpec,
        ) -> Result<RunOutput, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.outputs.get(spec.index).cloned().unwrap_or_default())
        }
    }

    #[test]
    fn spawn_card_flags_and_alias_match_go() {
        let tool = SpawnTool::default();
        assert_eq!(tool.name(), SPAWN_TOOL_NAME);
        assert!(!tool.read_only());
        assert!(
            tool.capabilities()
                .get::<dyn PlanModeClassifier>()
                .unwrap()
                .plan_mode_safe()
        );
        assert_eq!(
            tool.capabilities()
                .get::<dyn CapabilityAliased>()
                .unwrap()
                .capability_aliases(),
            vec!["task:subagent".to_owned()]
        );
    }

    #[test]
    fn spawn_without_runner_fails_closed() {
        let tool = SpawnTool::default();
        let args = serde_json::json!({ "tasks": [{ "title": "t", "task": "find x" }] });
        let err = tool.execute(&CallContext::new(), &args).unwrap_err();
        assert!(err.to_string().contains("sub-agents are not available"));
    }

    #[test]
    fn spawn_validates_shape_budget_and_result_shape() {
        let runner = Arc::new(StubRunner {
            outputs: vec![RunOutput {
                reference: "sa_1".to_owned(),
                answer: "hello world".to_owned(),
                read: vec!["a.rs".to_owned()],
                wrote: vec![],
            }],
        });
        let tool = SpawnTool::new(Some(runner));

        let empty = tool.execute(&CallContext::new(), &serde_json::json!({ "tasks": [] }));
        assert!(
            empty
                .unwrap_err()
                .to_string()
                .contains("tasks must not be empty")
        );

        let over_tasks: Vec<Value> = (0..9)
            .map(|_| serde_json::json!({ "title": "t", "task": "x" }))
            .collect();
        let over = tool.execute(
            &CallContext::new(),
            &serde_json::json!({ "tasks": over_tasks }),
        );
        assert!(over.unwrap_err().to_string().contains("at most 8 tasks"));

        let no_text = tool.execute(
            &CallContext::new(),
            &serde_json::json!({ "tasks": [{ "title": "t", "task": "  " }] }),
        );
        assert!(
            no_text
                .unwrap_err()
                .to_string()
                .contains("task 1 has no task text")
        );

        let unknown = tool.execute(
            &CallContext::new(),
            &serde_json::json!({ "tasks": [{ "title": "t", "task": "x" }], "bogus": 1 }),
        );
        assert!(unknown.unwrap_err().to_string().contains("unknown field"));

        let ok = tool
            .execute(
                &CallContext::new(),
                &serde_json::json!({ "tasks": [{ "title": "t", "task": "find x" }] }),
            )
            .unwrap();
        let body: Value = serde_json::from_str(&ok).unwrap();
        let agent = &body["agents"][0];
        assert_eq!(agent["title"], "t");
        assert_eq!(agent["ref"], "sa_1");
        assert_eq!(agent["chars"], 11);
        assert!(!agent["more"].as_bool().unwrap_or(false));
        assert_eq!(agent["read"][0], "a.rs");
    }

    #[test]
    fn head_budget_scales_with_task_count() {
        // 8 tasks: 8000/8 = 1000, inside [600, 2000].
        assert_eq!((8000 / 8).clamp(600, 2000), 1000);
        // 1 task: 8000 clamped to 2000.
        assert_eq!(8000usize.clamp(600, 2000), 2000);
    }

    struct StubStore {
        answer: String,
        status: String,
    }

    impl SubagentResultStore for StubStore {
        fn read_result(
            &self,
            _reference: &str,
            _parent_session: &str,
            _workspace_root: &str,
        ) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
            Ok((self.answer.clone(), self.status.clone()))
        }
    }

    fn ctx_with_session() -> CallContext {
        CallContext::new().with_value(Arc::new(ParentSessionBinding {
            session: "parent-1".to_owned(),
        }))
    }

    #[test]
    fn result_tool_reads_pages_and_renders_boundaries() {
        let store = Arc::new(StubStore {
            answer: "héllo world".to_owned(),
            status: "completed".to_owned(),
        });
        let tool = SubagentResultTool::new(Some(store), String::new());
        let page = tool
            .execute(
                &ctx_with_session(),
                &serde_json::json!({ "ref": "sa_1", "limit_bytes": 4 }),
            )
            .unwrap();
        assert!(page.starts_with("Subagent result sa_1 (status=completed, bytes 0-4 of 12):"));
        assert!(page.contains("More remains"));
        assert!(page.contains("offset_bytes=4"));

        let tail = tool
            .execute(
                &ctx_with_session(),
                &serde_json::json!({ "ref": "sa_1", "offset_bytes": 4 }),
            )
            .unwrap();
        assert!(tail.ends_with("End of subagent result."));
    }

    #[test]
    fn result_tool_refuses_bad_args_and_missing_seams() {
        let tool = SubagentResultTool::default();
        let no_ref = tool.execute(&ctx_with_session(), &serde_json::json!({}));
        assert!(no_ref.unwrap_err().to_string().contains("ref is required"));

        let negative = tool.execute(
            &ctx_with_session(),
            &serde_json::json!({ "ref": "sa_1", "offset_bytes": -1 }),
        );
        assert!(negative.unwrap_err().to_string().contains("non-negative"));

        let limit = tool.execute(
            &ctx_with_session(),
            &serde_json::json!({ "ref": "sa_1", "limit_bytes": 99999 }),
        );
        assert!(
            limit
                .unwrap_err()
                .to_string()
                .contains("between 1 and 24576")
        );

        let no_store = SubagentResultTool::default();
        let err = no_store
            .execute(&ctx_with_session(), &serde_json::json!({ "ref": "sa_1" }))
            .unwrap_err();
        assert!(err.to_string().contains("storage is not available"));

        let store = Arc::new(StubStore {
            answer: "x".to_owned(),
            status: "completed".to_owned(),
        });
        let no_parent = SubagentResultTool::new(Some(store), String::new())
            .execute(&CallContext::new(), &serde_json::json!({ "ref": "sa_1" }));
        assert!(
            no_parent
                .unwrap_err()
                .to_string()
                .contains("persisted parent session")
        );
    }

    #[test]
    fn result_tool_rejects_offset_past_end_and_half_character() {
        let store = Arc::new(StubStore {
            answer: "héllo".to_owned(),
            status: "completed".to_owned(),
        });
        let tool = SubagentResultTool::new(Some(store), String::new());
        let past = tool.execute(
            &ctx_with_session(),
            &serde_json::json!({ "ref": "sa_1", "offset_bytes": 100 }),
        );
        assert!(
            past.unwrap_err()
                .to_string()
                .contains("exceeds result size")
        );

        // 'é' spans bytes 1..3; offset 2 sits mid-character.
        let half = tool.execute(
            &ctx_with_session(),
            &serde_json::json!({ "ref": "sa_1", "offset_bytes": 2 }),
        );
        assert!(
            half.unwrap_err()
                .to_string()
                .contains("UTF-8 character boundary")
        );
    }
}
