//! ACP-017: `session/info` and `session/mcpServers` shapes, scope subtree, and
//! the split between mcpServers entries and the mcpStatus snapshot.

use reasonix_acp::session_info::{
    ReasonixConfiguredServer, ReasonixMcpServers, ReasonixSessionCache, ReasonixSessionInfo,
    ReasonixSessionScope, SessionMcpServersParams,
};

#[test]
fn info_tree_serializes_declared_order_with_scope_subtree() {
    let info = ReasonixSessionInfo {
        schema_version: 1,
        session_id: "s1".to_owned(),
        label: "main".to_owned(),
        model_ref: "anthropic/claude".to_owned(),
        workspace_root: "/proj".to_owned(),
        scope: ReasonixSessionScope {
            root: "/proj".to_owned(),
            name: "workspace".to_owned(),
            label: None,
            key: "workspace:/proj".to_owned(),
            repo: false,
            trees: None,
            branch: None,
            overrides: 0,
            current: None,
        },
        agent_preset: "builtin:general".to_owned(),
        auto_approve_tools: false,
        cache: ReasonixSessionCache::new(3, 1),
    };
    let text = serde_json::to_string(&info).unwrap();
    let expected = "{\"schemaVersion\":1,\"sessionId\":\"s1\",\"label\":\"main\",\"modelRef\":\"anthropic/claude\",\"workspaceRoot\":\"/proj\",\"scope\":{\"root\":\"/proj\",\"name\":\"workspace\",\"key\":\"workspace:/proj\",\"repo\":false,\"overrides\":0},\"agentPreset\":\"builtin:general\",\"autoApproveTools\":false,\"cache\":{\"hitTokens\":3,\"missTokens\":1,\"hitRatio\":0.75}}";
    assert_eq!(text, expected);
}

#[test]
fn mcp_servers_entries_carry_no_status_or_error_fields() {
    let listing = ReasonixMcpServers {
        schema_version: 1,
        session_id: "s1".to_owned(),
        servers: vec![
            ReasonixConfiguredServer {
                name: "fs".to_owned(),
                enabled: true,
                local_override: false,
                pending: false,
                changed: false,
                tools: 4,
            },
            ReasonixConfiguredServer {
                name: "git".to_owned(),
                enabled: false,
                local_override: true,
                pending: true,
                changed: true,
                tools: 0,
            },
        ],
    };
    let text = serde_json::to_string(&listing).unwrap();
    let expected = "{\"schemaVersion\":1,\"sessionId\":\"s1\",\"servers\":[{\"name\":\"fs\",\"enabled\":true,\"localOverride\":false,\"pending\":false,\"changed\":false,\"tools\":4},{\"name\":\"git\",\"enabled\":false,\"localOverride\":true,\"pending\":true,\"changed\":true,\"tools\":0}]}";
    assert_eq!(text, expected);

    let value = serde_json::to_value(&listing).unwrap();
    for entry in value["servers"].as_array().unwrap() {
        for forbidden in [
            "state",
            "status",
            "error",
            "httpStatus",
            "pendingReason",
            "ready",
            "connecting",
            "failed",
            "disabled",
            "standby",
            "idle",
        ] {
            assert!(
                entry.get(forbidden).is_none(),
                "{} leaked into mcpServers",
                forbidden
            );
        }
    }
}

#[test]
fn mcp_servers_params_omit_optional_name() {
    let bare = SessionMcpServersParams {
        session_id: "s1".to_owned(),
        name: None,
    };
    assert_eq!(
        serde_json::to_string(&bare).unwrap(),
        "{\"sessionId\":\"s1\"}"
    );

    let named = SessionMcpServersParams {
        session_id: "s1".to_owned(),
        name: Some("fs".to_owned()),
    };
    assert_eq!(
        serde_json::to_string(&named).unwrap(),
        "{\"sessionId\":\"s1\",\"name\":\"fs\"}"
    );
}
