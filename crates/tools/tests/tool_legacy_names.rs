//! TOOL-044 legacy delegation names (docs/contracts/tools.md §3.9; Go
//! `internal/ext/hook/hook_claude_agent.go`, `hook.go`,
//! `internal/runtime/agent/child_registry.go`, `safety/evidence/evidence.go`,
//! `runtime/agent/execute_one.go`, `runtime/agent/agent.go`,
//! `frontend/doctor/skill_health.go`, `runtime/delegation/*`,
//! `runtime/writeclaim/scheduler.go`, `usecap/testdata/capability_list.json`,
//! `cmd/e2ebench/rounds.go`).
//!
//! Covers the eight recognition surfaces: guidance prompts, hook translation,
//! child-registry frozen lists, evidence parsing, sub-agent hook firing,
//! doctor classification, description prose, and comments/fixtures.

use reasonix_tools::builtin::legacy_names::{
    FIXTURE_FLEET_ID, FIXTURE_PARALLEL_TASKS_ID, FIXTURE_READ_ONLY_TASK_ID,
    GUIDANCE_PLAN_EXEC_DISPATCH, GUIDANCE_PLAN_EXEC_RESEARCH, GUIDANCE_PROMETHEUS_PARALLEL,
    LEGACY_FLEET, LEGACY_PARALLEL_TASKS, LEGACY_READ_ONLY_TASK, LEGACY_TASK, NESTED_SINK_DOC,
    READ_SUBAGENT_RESULT_DESCRIPTION, SCHEDULER_COMMENT, claude_default_description,
    claude_spawning_tools, e2ebench_delegation_tools, is_background_task_call,
    is_builtin_or_meta_tool, is_legacy_delegation_name, is_review_task_receipt,
    joined_parallel_task_prompts, legacy_delegation_names, read_only_subagent_workflow_tools,
    subagent_always_hidden_tools, subagent_hooks_fire, subagent_job_tools, subagent_meta_tools,
    subagent_recursive_tools, task_profile_from_args,
};
use serde_json::json;

#[test]
fn four_legacy_names_are_recognized_and_never_namespaced() {
    assert_eq!(
        legacy_delegation_names(),
        [
            LEGACY_TASK,
            LEGACY_READ_ONLY_TASK,
            LEGACY_PARALLEL_TASKS,
            LEGACY_FLEET
        ]
    );
    assert!(is_legacy_delegation_name("task"));
    assert!(is_legacy_delegation_name("fleet"));
    assert!(!is_legacy_delegation_name("mcp__server__task"));
    assert!(!is_legacy_delegation_name("review"));
}

#[test]
fn surface_one_guidance_prompts_keep_the_legacy_spelling() {
    assert_eq!(
        GUIDANCE_PLAN_EXEC_RESEARCH,
        "Research/exploration across modules = use parallel_tasks"
    );
    assert_eq!(
        GUIDANCE_PLAN_EXEC_DISPATCH,
        "Dispatch each batch via parallel_tasks — each sub-agent gets one module’s context"
    );
    assert_eq!(
        GUIDANCE_PROMETHEUS_PARALLEL,
        "For independent research directions, use parallel_tasks before planning."
    );
}

#[test]
fn surface_two_claude_spawning_list_and_default_descriptions() {
    let spawning = claude_spawning_tools();
    assert!(spawning.contains(&"task"));
    assert!(spawning.contains(&"read_only_task"));
    assert!(spawning.contains(&"parallel_tasks"));
    assert!(spawning.contains(&"review"));
    assert_eq!(
        claude_default_description("task"),
        Some("Run delegated subagent task")
    );
    assert_eq!(
        claude_default_description("read_only_task"),
        Some("Run read-only research task")
    );
    assert_eq!(
        claude_default_description("parallel_tasks"),
        Some("Run parallel subagent tasks")
    );
    assert_eq!(
        claude_default_description("explore"),
        Some("Explore the codebase")
    );
    assert_eq!(claude_default_description("nope"), None);
}

#[test]
fn joined_parallel_prompts_flattens_prompt_fields_blank_line_separated() {
    let tasks = json!([
        {"prompt": "first direction"},
        {"prompt": "second direction"},
        {"no_prompt": true},
    ]);
    assert_eq!(
        joined_parallel_task_prompts(&tasks),
        "first direction\n\nsecond direction"
    );
    assert!(joined_parallel_task_prompts(&json!("not an array")).is_empty());
    assert!(joined_parallel_task_prompts(&json!([])).is_empty());
}

#[test]
fn surface_three_child_registry_frozen_lists_match_the_reference() {
    assert!(subagent_recursive_tools().contains(&"task"));
    assert!(subagent_recursive_tools().contains(&"explore"));
    assert!(!subagent_recursive_tools().contains(&"read_skill"));

    assert!(subagent_always_hidden_tools().contains(&LEGACY_PARALLEL_TASKS));
    assert!(subagent_always_hidden_tools().contains(&LEGACY_FLEET));
    assert!(subagent_always_hidden_tools().contains(&"read_subagent_result"));

    assert_eq!(subagent_job_tools(), ["wait", "bash_output", "kill_shell"]);
    assert_eq!(read_only_subagent_workflow_tools(), ["connect_tool_source"]);

    let meta = subagent_meta_tools();
    assert!(meta.contains(&"read_skill"));
    assert!(meta.contains(&LEGACY_TASK));
    assert!(meta.contains(&LEGACY_FLEET));
}

#[test]
fn surface_four_evidence_recognizes_review_task_receipts_and_profiles() {
    // A review tool receipt counts directly.
    assert!(is_review_task_receipt("review", "", &json!({})));
    // A task receipt with profile review and no background flag counts.
    assert!(is_review_task_receipt(
        "task",
        "review",
        &json!({"run_in_background": false})
    ));
    // Background task receipts do not count as foreground reviews.
    assert!(!is_review_task_receipt(
        "task",
        "review",
        &json!({"run_in_background": true})
    ));
    // Plain task receipts do not.
    assert!(!is_review_task_receipt("task", "plan", &json!({})));

    assert_eq!(
        task_profile_from_args(&json!({"profile": "review"})),
        Some("review".to_owned())
    );
    assert_eq!(task_profile_from_args(&json!({})), None);
}

#[test]
fn surface_five_subagent_hooks_fire_on_foreground_task_only() {
    assert!(subagent_hooks_fire("task", &json!({})));
    assert!(subagent_hooks_fire(
        "task",
        &json!({"run_in_background": false})
    ));
    assert!(!subagent_hooks_fire(
        "task",
        &json!({"run_in_background": true})
    ));
    assert!(!subagent_hooks_fire("review", &json!({})));
    assert!(!is_background_task_call(&json!({})));
    assert!(is_background_task_call(&json!({"run_in_background": true})));
}

#[test]
fn surface_six_doctor_classifies_builtin_meta_and_namespaced_names() {
    assert!(is_builtin_or_meta_tool("bash"));
    assert!(is_builtin_or_meta_tool(LEGACY_TASK));
    assert!(is_builtin_or_meta_tool(LEGACY_PARALLEL_TASKS));
    assert!(is_builtin_or_meta_tool("mcp__server__tool"));
    assert!(is_builtin_or_meta_tool("lsp__rust__hover"));
    assert!(!is_builtin_or_meta_tool("completely_custom_tool"));
}

#[test]
fn surface_seven_prose_constraints_keep_legacy_sources() {
    assert!(READ_SUBAGENT_RESULT_DESCRIPTION.contains("task"));
    assert!(READ_SUBAGENT_RESULT_DESCRIPTION.contains("parallel_tasks"));
    assert!(READ_SUBAGENT_RESULT_DESCRIPTION.contains("fleet"));
    assert!(NESTED_SINK_DOC.contains("fleet"));
    assert!(NESTED_SINK_DOC.contains("parallel_tasks"));
}

#[test]
fn surface_eight_comments_and_fixture_ids_track_the_reference() {
    assert!(SCHEDULER_COMMENT.contains("task"));
    assert!(SCHEDULER_COMMENT.contains("fleet"));
    assert_eq!(FIXTURE_FLEET_ID, "tool:fleet");
    assert_eq!(FIXTURE_PARALLEL_TASKS_ID, "tool:parallel_tasks");
    assert_eq!(FIXTURE_READ_ONLY_TASK_ID, "tool:read_only_task");

    let delegation = e2ebench_delegation_tools();
    assert!(delegation.contains(&LEGACY_TASK));
    assert!(delegation.contains(&LEGACY_PARALLEL_TASKS));
    assert!(delegation.contains(&LEGACY_FLEET));
    assert!(delegation.contains(&"research"));
}
