//! TOOL-044: legacy delegation name recognition (docs/contracts/tools.md §3.9).
//!
//! `task`, `read_only_task`, `parallel_tasks` and `fleet` are **not registered
//! tools** in the reference product. The Rust port must not register them; it
//! reproduces every name-recognition surface the Go product still carries,
//! because each one is observable contract: guidance prompts, hook translation,
//! child registry frozen lists, evidence parsing, sub-agent hook firing, doctor
//! classification, description prose and golden fixtures.
//!
//! Go oracles: `internal/ext/hook/hook_claude_agent.go`,
//! `internal/ext/hook/hook.go`, `internal/runtime/agent/child_registry.go`,
//! `internal/safety/evidence/evidence.go`, `internal/runtime/agent/execute_one.go`,
//! `internal/runtime/agent/agent.go`, `internal/frontend/doctor/skill_health.go`,
//! `internal/runtime/delegation/subagent_result.go`,
//! `internal/runtime/delegation/nested_sink.go`,
//! `internal/runtime/writeclaim/scheduler.go`,
//! `internal/runtime/usecap/testdata/capability_list.json`,
//! `cmd/e2ebench/rounds.go`.

#![forbid(unsafe_code)]

use serde_json::Value;

/// Legacy delegation tool name: `task`.
pub const LEGACY_TASK: &str = "task";
/// Legacy delegation tool name: `read_only_task`.
pub const LEGACY_READ_ONLY_TASK: &str = "read_only_task";
/// Legacy delegation tool name: `parallel_tasks`.
pub const LEGACY_PARALLEL_TASKS: &str = "parallel_tasks";
/// Legacy delegation tool name: `fleet`.
pub const LEGACY_FLEET: &str = "fleet";

/// The four legacy names the reference product never registers, in the order
/// §3.9 lists them.
pub fn legacy_delegation_names() -> [&'static str; 4] {
    [
        LEGACY_TASK,
        LEGACY_READ_ONLY_TASK,
        LEGACY_PARALLEL_TASKS,
        LEGACY_FLEET,
    ]
}

/// Whether `name` is one of the four legacy delegation names. Recognition
/// matches the bare spelling only; namespaced MCP/extension tools are not
/// legacy delegation calls.
pub fn is_legacy_delegation_name(name: &str) -> bool {
    legacy_delegation_names().contains(&name)
}

// --- Surface 1: guidance prompts that name an unregistered tool -------------
//
// Divergence surfaces (docs/parity/BREAKS.md): the prompt text is observable
// behavior, so it is reproduced byte-for-byte; a provider that follows it
// fails through the same unknown-tool path as the reference.

/// `/plan-exec` routing rule: "Research/exploration across modules = use
/// parallel_tasks" (`session/control/turn_submit.go:535`).
pub const GUIDANCE_PLAN_EXEC_RESEARCH: &str =
    "Research/exploration across modules = use parallel_tasks";

/// `/plan-exec` routing rule: "Dispatch each batch via parallel_tasks — each
/// sub-agent gets one module’s context" (`turn_submit.go:536`).
pub const GUIDANCE_PLAN_EXEC_DISPATCH: &str =
    "Dispatch each batch via parallel_tasks — each sub-agent gets one module’s context";

/// `/prometheus` planner prompt tail: "For independent research directions,
/// use parallel_tasks before planning." (`session/control/controller.go:697`).
pub const GUIDANCE_PROMETHEUS_PARALLEL: &str =
    "For independent research directions, use parallel_tasks before planning.";

// --- Surface 2: hook Claude-compat translation -------------------------------

/// Every tool that spawns a subagent, mapped to Claude's one Agent tool
/// (`hook_claude_agent.go`). A hook matching "Agent" must see all of them or
/// it misses whichever entry point is left out.
pub fn claude_spawning_tools() -> &'static [&'static str] {
    &[
        LEGACY_TASK,
        LEGACY_READ_ONLY_TASK,
        LEGACY_PARALLEL_TASKS,
        "explore",
        "locate",
        "research",
        "review",
        "security_review",
    ]
}

/// Claude Agent's required description for a Reasonix tool that omits its
/// own (`hook_claude_agent.go`). Stable operation labels; the complete task
/// stays in prompt for hook policy decisions.
pub fn claude_default_description(name: &str) -> Option<&'static str> {
    match name {
        LEGACY_TASK => Some("Run delegated subagent task"),
        LEGACY_READ_ONLY_TASK => Some("Run read-only research task"),
        LEGACY_PARALLEL_TASKS => Some("Run parallel subagent tasks"),
        "explore" => Some("Explore the codebase"),
        "locate" => Some("Locate code in the codebase"),
        "research" => Some("Research external references"),
        "review" => Some("Review the current changes"),
        "security_review" => Some("Review security risks"),
        _ => None,
    }
}

/// Flattens a `parallel_tasks` `tasks` array into one prompt string, blank-line
/// separated (`hook.go:joinedParallelTaskPrompts`). Malformed or empty input
/// yields an empty string.
pub fn joined_parallel_task_prompts(tasks: &Value) -> String {
    let Some(items) = tasks.as_array() else {
        return String::new();
    };
    let mut prompts: Vec<String> = Vec::new();
    for item in items {
        let prompt = item
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if !prompt.is_empty() {
            prompts.push(prompt.to_owned());
        }
    }
    prompts.join("\n\n")
}

// --- Surface 3: child registry frozen lists ---------------------------------

/// Tools a recursive sub-agent may keep (`child_registry.go`
/// `subagentRecursiveTools`). `read_skill` is deliberately absent: it renders
/// playbook text inline and cannot recurse.
pub fn subagent_recursive_tools() -> &'static [&'static str] {
    &[
        LEGACY_TASK,
        LEGACY_READ_ONLY_TASK,
        "run_skill",
        "read_only_skill",
        "explore",
        "locate",
        "research",
        "review",
        "security_review",
    ]
}

/// Tools a sub-agent never inherits (`subagentAlwaysHiddenTools`).
pub fn subagent_always_hidden_tools() -> &'static [&'static str] {
    &[
        LEGACY_PARALLEL_TASKS,
        LEGACY_FLEET,
        "read_subagent_result",
        "list_subagents",
        "install_skill",
        "install_source",
        "apply_isolated",
        "discard_isolated",
    ]
}

/// Background-job tools a sub-agent reaches through the job surface
/// (`subagentJobTools`).
pub fn subagent_job_tools() -> &'static [&'static str] {
    &["wait", "bash_output", "kill_shell"]
}

/// Read-only sub-agent workflow surface (`readOnlySubagentWorkflowTools`).
pub fn read_only_subagent_workflow_tools() -> &'static [&'static str] {
    &["connect_tool_source"]
}

/// Meta tools spawned agents should not inherit: the recursive list plus
/// `read_skill` plus the always-hidden list (`SubagentMetaTools`).
pub fn subagent_meta_tools() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = subagent_recursive_tools().to_vec();
    out.push("read_skill");
    out.extend_from_slice(subagent_always_hidden_tools());
    out
}

// --- Surface 4: evidence parsing --------------------------------------------

/// Whether a successful receipt counts as a review (`evidence.go`
/// `successfulForegroundReviewReceipt`). A `task` receipt with profile
/// `review` counts only when it is not a background call; the caller already
/// checked `Success`.
pub fn is_review_task_receipt(tool_name: &str, profile: &str, args: &Value) -> bool {
    if tool_name == "review" {
        return true;
    }
    if tool_name != LEGACY_TASK || profile != "review" {
        return false;
    }
    let run_in_background = args
        .get("run_in_background")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    !run_in_background
}

/// Reads the `profile` field off a `task` receipt's args
/// (`evidence.go:471`). Returns `None` when the field is absent or not a
/// string.
pub fn task_profile_from_args(args: &Value) -> Option<String> {
    args.get("profile")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

// --- Surface 5: sub-agent hooks ---------------------------------------------

/// Parses `{"run_in_background": bool}` off the call args
/// (`agent.go:isBackgroundTaskCall`). A missing or unparsable field reads
/// false: an unmarked call is a foreground delegation.
pub fn is_background_task_call(args: &Value) -> bool {
    args.get("run_in_background")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether the `SubagentStart`/`SubagentStop` hooks fire for this call
/// (`execute_one.go:682`): the tool is the legacy `task` name and the call is
/// not a background task.
pub fn subagent_hooks_fire(tool_name: &str, args: &Value) -> bool {
    tool_name == LEGACY_TASK && !is_background_task_call(args)
}

// --- Surface 6: doctor name classification ----------------------------------

/// Built-in and meta tool names the doctor recognizes for health reporting
/// (`skill_health.go:isBuiltinOrMetaTool`). Classification covers names listed
/// in skills; it is not proof of registration.
pub fn is_builtin_or_meta_tool(name: &str) -> bool {
    const BUILTIN_OR_META: &[&str] = &[
        "bash",
        "read_file",
        "write_file",
        "edit_file",
        "grep",
        "glob",
        "ls",
        "todo_write",
        "complete_step",
        "ask",
        LEGACY_TASK,
        LEGACY_READ_ONLY_TASK,
        LEGACY_PARALLEL_TASKS,
        LEGACY_FLEET,
        "run_skill",
        "read_skill",
        "read_only_skill",
        "explore",
        "locate",
        "research",
        "review",
        "security_review",
        "web_fetch",
        "multi_edit",
        "move_file",
        "code_index",
        "wait",
        "bash_output",
        "kill_shell",
    ];
    BUILTIN_OR_META.contains(&name) || name.starts_with("mcp__") || name.starts_with("lsp_")
}

// --- Surface 7: description prose ------------------------------------------

/// `read_subagent_result` description, which still names the legacy sources
/// (`subagent_result.go:41`).
pub const READ_SUBAGENT_RESULT_DESCRIPTION: &str = "Read a completed sub-agent's full final answer by the Subagent reference \
returned from task, parallel_tasks, or fleet. Results are scoped to the current \
conversation lineage and paged by UTF-8 byte offset so large answers remain \
lossless without overflowing one tool result.";

/// `nestedSink` doc line: the event view a fleet or parallel_tasks item runs
/// under (`nested_sink.go:9`).
pub const NESTED_SINK_DOC: &str = "nestedSink is the event view a fleet or parallel_tasks item runs under: tool \
events are re-parented, usage is attributed to the sub-agent, everything else \
is dropped.";

// --- Surface 8: comments and fixtures ----------------------------------------

/// `SubagentScheduler` doc comment naming the legacy delegation surfaces
/// (`writeclaim/scheduler.go:65`).
pub const SCHEDULER_COMMENT: &str = "SubagentScheduler is a session-scoped concurrency controller shared by task, \
fleet, parallel_tasks, profile skills, and nested sub-agents.";

/// Golden fixture capability id for `fleet`
/// (`usecap/testdata/capability_list.json`).
pub const FIXTURE_FLEET_ID: &str = "tool:fleet";
/// Golden fixture capability id for `parallel_tasks`.
pub const FIXTURE_PARALLEL_TASKS_ID: &str = "tool:parallel_tasks";
/// Golden fixture capability id for `read_only_task`.
pub const FIXTURE_READ_ONLY_TASK_ID: &str = "tool:read_only_task";

/// Benchmark delegation cost classification (`cmd/e2ebench/rounds.go:121`):
/// calls whose cost story is the delegation itself, not the local
/// mutation/verification the batch would otherwise classify as.
pub fn e2ebench_delegation_tools() -> &'static [&'static str] {
    &[LEGACY_TASK, LEGACY_PARALLEL_TASKS, LEGACY_FLEET, "research"]
}
