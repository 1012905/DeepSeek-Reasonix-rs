//! ACP-009: permission round-trip, option kinds, outcome mapping, ceiling filter.

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::permission::{
    POSTURE_ORDER, PermissionDecision, PermissionGrant, PermissionOption, PermissionOptionKind,
    PermissionOutcomeBody, PermissionRequestParams, PermissionRequestResult, PermissionToolCall,
    above_ceiling_error, ask_options, filter_posture_choices, fresh_options, persist_option,
    resolve_permission, standard_options,
};

#[test]
fn request_params_wire_shape() {
    let params = PermissionRequestParams {
        session_id: "s1".to_owned(),
        tool_call: PermissionToolCall {
            tool_call_id: "gate-7".to_owned(),
            name: Some("bash".to_owned()),
            title: Some("bash ls".to_owned()),
            kind: Some("execute".to_owned()),
            status: Some("pending".to_owned()),
            content: None,
            raw_input: None,
            locations: None,
            meta: None,
        },
        options: standard_options("Allow ls for this session"),
    };
    let v = serde_json::to_value(&params).unwrap();
    assert_eq!(v["sessionId"], "s1");
    assert_eq!(v["toolCall"]["toolCallId"], "gate-7");
    assert_eq!(v["toolCall"]["status"], "pending");
    assert!(v["toolCall"].get("content").is_none());
    assert_eq!(v["options"].as_array().unwrap().len(), 3);
}

#[test]
fn four_option_kinds_round_trip_wire_values() {
    for (kind, wire) in [
        (PermissionOptionKind::AllowOnce, "allow_once"),
        (PermissionOptionKind::AllowAlways, "allow_always"),
        (PermissionOptionKind::RejectOnce, "reject_once"),
        (PermissionOptionKind::RejectAlways, "reject_always"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(wire));
    }
}

#[test]
fn persist_option_keeps_allow_always_kind() {
    let option = persist_option("network write");
    assert_eq!(option.option_id, "allow_persist");
    assert_eq!(option.kind, PermissionOptionKind::AllowAlways);
}

#[test]
fn selected_options_map_to_grants_everything_else_denies() {
    let selected = |id: &str| PermissionRequestResult {
        outcome: PermissionOutcomeBody {
            outcome: "selected".to_owned(),
            option_id: Some(id.to_owned()),
        },
    };
    assert_eq!(
        resolve_permission(&selected("allow_once")),
        PermissionDecision::Grant(PermissionGrant {
            allow: true,
            session: false,
            persist: false,
        })
    );
    assert_eq!(
        resolve_permission(&selected("allow_always")),
        PermissionDecision::Grant(PermissionGrant {
            allow: true,
            session: true,
            persist: false,
        })
    );
    assert_eq!(
        resolve_permission(&selected("allow_persist")),
        PermissionDecision::Grant(PermissionGrant {
            allow: true,
            session: true,
            persist: true,
        })
    );

    let cancelled = PermissionRequestResult {
        outcome: PermissionOutcomeBody {
            outcome: "cancelled".to_owned(),
            option_id: None,
        },
    };
    assert_eq!(resolve_permission(&cancelled), PermissionDecision::Deny);
    assert_eq!(
        resolve_permission(&selected("bogus")),
        PermissionDecision::Deny
    );
    assert_eq!(
        resolve_permission(&PermissionRequestResult::default()),
        PermissionDecision::Deny
    );
}

#[test]
fn ask_options_number_entries_and_append_cancel() {
    let options = ask_options(
        "ask-1",
        &[
            ("First".to_owned(), None),
            ("Second".to_owned(), Some("desc".to_owned())),
        ],
    );
    let ids: Vec<_> = options.iter().map(|o| o.option_id.as_str()).collect();
    assert_eq!(ids, vec!["ask-1:1", "ask-1:2", "ask-1:cancel"]);
    assert_eq!(options[0].kind, PermissionOptionKind::AllowOnce);
    assert_eq!(options[2].kind, PermissionOptionKind::RejectOnce);
    assert_eq!(options[1].name, "Second - desc");
}

#[test]
fn standard_and_fresh_option_sets() {
    let standard = standard_options("Allow x for this session");
    assert_eq!(standard.len(), 3);
    assert_eq!(standard[0].option_id, "allow_once");
    assert_eq!(standard[1].option_id, "allow_always");
    assert_eq!(standard[2].option_id, "reject_once");

    let fresh = fresh_options();
    assert_eq!(fresh.len(), 2);
}

#[test]
fn ceiling_filters_posture_choices_not_permission_options() {
    let choices: Vec<String> = POSTURE_ORDER.iter().map(|s| s.to_string()).collect();
    let unrestricted = filter_posture_choices(&choices, "");
    assert_eq!(unrestricted, vec!["ask", "auto", "yolo"]);

    let capped = filter_posture_choices(&choices, "ask");
    assert_eq!(capped, vec!["readOnly", "dontAsk", "ask"]);

    // The permission option list itself carries no ceiling filtering.
    let options = standard_options("x");
    assert_eq!(options.len(), 3);
}

#[test]
fn above_ceiling_error_structure() {
    let err = above_ceiling_error("session/set_mode", "yolo", "ask");
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(
        err.message,
        "session/set_mode: tool approval yolo is above the ceiling this agent was started with (ask)"
    );
    let data = err.data.expect("data present");
    assert_eq!(data["code"], "acp.tool_approval.above_ceiling");
    assert_eq!(data["ceiling"], "ask");
}

#[test]
fn permission_option_serialization_shape() {
    let option = PermissionOption {
        option_id: "allow_once".to_owned(),
        name: "Allow".to_owned(),
        kind: PermissionOptionKind::AllowOnce,
    };
    let v = serde_json::to_value(&option).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"optionId": "allow_once", "name": "Allow", "kind": "allow_once"})
    );
}
