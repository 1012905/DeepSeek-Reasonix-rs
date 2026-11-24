//! ACP-005: inbound wire types round-trip, required shapes and dual-shape env.

use reasonix_acp::wire::{
    ContentBlock, McpServerSpec, SessionConfigOption, SessionIdParams, SessionInfo,
    SessionListParams, SessionListResult, SessionMode, SessionModeState, SessionModelState,
    SessionNewParams, SessionNewResult, SessionOpenParams, SessionOpenResult, SessionPromptParams,
    SessionPromptResult, SessionSetModeParams, SessionSteerParams, SessionSteerResult,
    SetSessionConfigOptionParams, SetSessionModelParams, StopReason,
};

fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let v = serde_json::to_value(value).unwrap();
    let back: T = serde_json::from_value(v).unwrap();
    assert_eq!(value, &back);
}

#[test]
fn session_new_minimal_and_full_round_trip() {
    let minimal = SessionNewParams::default();
    round_trip(&minimal);

    let full = SessionNewParams {
        cwd: Some("/proj".to_owned()),
        additional_directories: Some(vec!["/other".to_owned()]),
        mcp_servers: Some(vec![McpServerSpec {
            name: "fs".to_owned(),
            r#type: Some("local".to_owned()),
            command: Some("mcp-fs".to_owned()),
            args: Some(vec!["--root".to_owned(), "/proj".to_owned()]),
            env: None,
            url: None,
            headers: None,
        }]),
    };
    round_trip(&full);

    let result = SessionNewResult {
        session_id: "123e4567-e89b-42d3-a562-426614174000".to_owned(),
        models: Some(SessionModelState {
            available_models: vec![],
            current_model_id: "gpt-x".to_owned(),
        }),
        modes: Some(SessionModeState {
            current_mode_id: "plan".to_owned(),
            available_modes: vec![SessionMode {
                id: "plan".to_owned(),
                name: "Plan".to_owned(),
                description: Some("Plan mode".to_owned()),
            }],
        }),
        config_options: Some(vec![SessionConfigOption::default()]),
    };
    round_trip(&result);
}

#[test]
fn load_resume_open_params_and_results_round_trip() {
    let params = SessionOpenParams {
        session_id: "sess-1".to_owned(),
        cwd: Some("/proj".to_owned()),
        additional_directories: None,
        mcp_servers: None,
    };
    round_trip(&params);

    let result = SessionOpenResult::default();
    let v = serde_json::to_value(&result).unwrap();
    assert!(v.get("sessionId").is_none());
    round_trip(&result);
}

#[test]
fn control_and_prompt_dtos_round_trip() {
    round_trip(&SetSessionModelParams {
        session_id: "s1".to_owned(),
        model_id: "gpt-x".to_owned(),
    });
    round_trip(&SetSessionConfigOptionParams {
        session_id: "s1".to_owned(),
        config_id: "tool_approval".to_owned(),
        value: "ask".to_owned(),
    });
    round_trip(&SessionSetModeParams {
        session_id: "s1".to_owned(),
        mode_id: "plan".to_owned(),
    });
    round_trip(&SessionIdParams {
        session_id: "s1".to_owned(),
    });
    round_trip(&SessionIdParams {
        session_id: "s1".to_owned(),
    });

    let prompt = SessionPromptParams {
        session_id: "s1".to_owned(),
        prompt: vec![ContentBlock {
            block_type: "text".to_owned(),
            text: Some("hello".to_owned()),
            ..Default::default()
        }],
    };
    round_trip(&prompt);

    let result = SessionPromptResult {
        stop_reason: StopReason::EndTurn,
        meta: None,
    };
    round_trip(&result);

    let steer = SessionSteerParams {
        session_id: "s1".to_owned(),
        prompt: vec![ContentBlock::default()],
    };
    round_trip(&steer);
    round_trip(&SessionSteerResult {
        item_id: Some("item-1".to_owned()),
        disposition: Some("steer_accepted".to_owned()),
    });
}

#[test]
fn list_result_omits_next_cursor_when_unset() {
    let result = SessionListResult {
        sessions: vec![SessionInfo {
            session_id: "s1".to_owned(),
            cwd: "/proj".to_owned(),
            title: Some("t".to_owned()),
            updated_at: Some("2026-11-24T00:00:00Z".to_owned()),
            meta: None,
        }],
        next_cursor: None,
    };
    let v = serde_json::to_value(&result).unwrap();
    assert!(v.get("nextCursor").is_none());
    round_trip(&result);

    let params = SessionListParams {
        cwd: Some("/proj".to_owned()),
        cursor: None,
    };
    round_trip(&params);
}

#[test]
fn mcp_env_dual_shape_and_invalid_messages() {
    let spec: McpServerSpec = serde_json::from_value(serde_json::json!({
        "name": "local", "command": "mcp-fs",
        "env": [{"name": "HOME", "value": "/tmp"}],
        "headers": {"Authorization": "Bearer t"}
    }))
    .unwrap();
    assert_eq!(spec.env.unwrap().0["HOME"], "/tmp");
    assert_eq!(spec.headers.unwrap().0["Authorization"], "Bearer t");

    let blank = serde_json::from_value::<McpServerSpec>(serde_json::json!({
        "name": "s", "env": [{"name": "  ", "value": "v"}]
    }));
    assert!(
        blank
            .unwrap_err()
            .to_string()
            .contains("env[0].name is required")
    );

    let number = serde_json::from_value::<McpServerSpec>(serde_json::json!({
        "name": "s", "env": 7
    }));
    assert!(
        number
            .unwrap_err()
            .to_string()
            .contains("env must be an array of {name,value} objects")
    );

    let headers = serde_json::from_value::<McpServerSpec>(serde_json::json!({
        "name": "s", "headers": 7
    }));
    assert!(
        headers
            .unwrap_err()
            .to_string()
            .contains("headers must be an array of {name,value} objects")
    );
}

#[test]
fn stop_reason_wire_values() {
    assert_eq!(
        serde_json::to_value(StopReason::EndTurn).unwrap(),
        serde_json::json!("end_turn")
    );
    assert_eq!(
        serde_json::to_value(StopReason::Cancelled).unwrap(),
        serde_json::json!("cancelled")
    );
}
