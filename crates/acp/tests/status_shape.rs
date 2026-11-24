//! ACP-015: `session/status` ReasonixSessionStatus v1 tree, field order,
//! conditional goal/runtime blocks, and sensitive-field omission.

use reasonix_acp::status::{
    ReasonixFinalReadiness, ReasonixGoalRuntime, ReasonixSessionStatus, ReasonixStatusGoal,
    ReasonixStatusUpdate, ReasonixStatusUsage, ReasonixTurnOutcome, ReasonixUsage,
    SessionRuntimeState, SessionSandboxState,
};

fn full_status() -> ReasonixSessionStatus {
    ReasonixSessionStatus {
        schema_version: 1,
        sequence: 7,
        session_id: "s1".to_owned(),
        state: "running".to_owned(),
        model: "anthropic/claude".to_owned(),
        effort: "high".to_owned(),
        mode: "goal".to_owned(),
        work_mode: "economy".to_owned(),
        planner_mode: "off".to_owned(),
        goal: ReasonixStatusGoal {
            status: "blocked".to_owned(),
            objective: Some("finish docs".to_owned()),
            runtime: Some(ReasonixGoalRuntime {
                turns_used: 3,
                turns_limit: 0,
                tokens_used: 1200,
                requests_used: None,
                work_duration_ms: None,
                tokens_limit: 0,
                no_progress_turns: 1,
                no_progress_limit: 0,
                last_reason: None,
                stop_cause: None,
                budget_extensions: 0,
            }),
        },
        phase: "working".to_owned(),
        turn_outcome: ReasonixTurnOutcome {
            kind: "blocked".to_owned(),
            reason: Some("token budget".to_owned()),
        },
        final_readiness: ReasonixFinalReadiness {
            ready_for_review: false,
            summary: String::new(),
            risks: Vec::new(),
        },
        sandbox: SessionSandboxState {
            mode: "off".to_owned(),
            engine: String::new(),
            available: false,
            workspace_root: "/proj".to_owned(),
            write_roots: Vec::new(),
            network_enabled: false,
        },
        usage: ReasonixStatusUsage {
            turn: ReasonixUsage::default(),
            cumulative: ReasonixUsage::default(),
        },
    }
}

#[test]
fn status_tree_keeps_top_level_declared_order() {
    let text = serde_json::to_string(&full_status()).unwrap();
    let prefix = "{\"schemaVersion\":1,\"sequence\":7,\"sessionId\":\"s1\",\"state\":\"running\",\"model\":\"anthropic/claude\",\"effort\":\"high\",\"mode\":\"goal\",\"workMode\":\"economy\",\"plannerMode\":\"off\",\"goal\":{\"status\":\"blocked\",\"objective\":\"finish docs\",\"runtime\":{\"turnsUsed\":3,\"turnsLimit\":0,\"tokensUsed\":1200,\"tokensLimit\":0,\"noProgressTurns\":1,\"noProgressLimit\":0,\"budgetExtensions\":0}},\"phase\":\"working\",\"turnOutcome\":{\"kind\":\"blocked\",\"reason\":\"token budget\"},\"finalReadiness\":{\"readyForReview\":false,\"summary\":\"\",\"risks\":[]},\"sandbox\":{\"mode\":\"off\",\"engine\":\"\",\"available\":false,\"workspaceRoot\":\"/proj\",\"writeRoots\":[],\"networkEnabled\":false},\"usage\":{";
    assert!(text.starts_with(prefix), "got: {text}");
}

#[test]
fn goal_runtime_deprecated_counters_ride_as_zero_on_the_wire() {
    let value = serde_json::to_value(full_status()).unwrap();
    let runtime = &value["goal"]["runtime"];
    for field in [
        "turnsLimit",
        "tokensLimit",
        "noProgressLimit",
        "budgetExtensions",
    ] {
        assert_eq!(runtime[field], 0, "{field} must ride even when deprecated");
    }
    assert!(runtime.get("requestsUsed").is_none());
    assert!(runtime.get("workDurationMs").is_none());
}

#[test]
fn empty_goal_and_outcome_omit_conditional_blocks() {
    let mut status = full_status();
    status.goal = ReasonixStatusGoal {
        status: "none".to_owned(),
        ..Default::default()
    };
    status.turn_outcome = ReasonixTurnOutcome::default();
    let value = serde_json::to_value(&status).unwrap();
    let goal = &value["goal"];
    assert_eq!(goal["status"], "none");
    assert!(goal.get("objective").is_none());
    assert!(goal.get("runtime").is_none());
    assert!(value["turnOutcome"].get("reason").is_none());
}

#[test]
fn snapshot_omits_sensitive_payloads() {
    let text = serde_json::to_string(&full_status()).unwrap();
    for forbidden in [
        "credentials",
        "environment",
        "transcript",
        "mcpServers",
        "command",
        "args",
        "reasoningText",
    ] {
        assert!(
            !text.contains(forbidden),
            "sensitive field leaked: {forbidden}"
        );
    }
}

#[test]
fn status_update_envelope_keeps_order() {
    let update = ReasonixStatusUpdate {
        schema_version: 1,
        sequence: 7,
        session_id: "s1".to_owned(),
        event: "turn_done".to_owned(),
        status: full_status(),
    };
    let text = serde_json::to_string(&update).unwrap();
    assert!(text.starts_with(
        "{\"schemaVersion\":1,\"sequence\":7,\"sessionId\":\"s1\",\"event\":\"turn_done\",\"status\":{"
    ));
}

#[test]
fn runtime_state_defaults_planner_mode_and_sandbox() {
    let state = SessionRuntimeState::default();
    let value = serde_json::to_value(&state).unwrap();
    assert!(value["plannerMode"].is_string());
    assert!(value["sandbox"]["workspaceRoot"].is_string());
}
