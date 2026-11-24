//! ACP-003 initialize capabilities / protocolVersion, ACP-004 authenticate.

use reasonix_acp::caps::{authenticate, initialize_result, negotiate_protocol_version};
use reasonix_acp::errors::RpcErrorCode;
use serde_json::Value;

#[test]
fn initialize_result_top_level_field_order_and_content() {
    let result = initialize_result("reasonix", "0.3.0");
    let bytes = serde_json::to_vec(&result).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();

    let order = [
        "protocolVersion",
        "agentCapabilities",
        "agentInfo",
        "authMethods",
    ];
    let mut prev = 0;
    for key in order {
        let idx = text.find(key).expect("{key} present");
        assert!(idx >= prev, "{key} out of order");
        prev = idx;
    }

    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["protocolVersion"], 1);
    assert_eq!(value["agentInfo"]["name"], "reasonix");
    assert_eq!(value["agentInfo"]["version"], "0.3.0");
    assert_eq!(value["agentCapabilities"]["loadSession"], true);
    assert_eq!(
        value["agentCapabilities"]["promptCapabilities"]["image"],
        true
    );
    assert_eq!(
        value["agentCapabilities"]["promptCapabilities"]["audio"],
        false
    );
    assert_eq!(
        value["agentCapabilities"]["promptCapabilities"]["embeddedContext"],
        true
    );
    assert_eq!(value["agentCapabilities"]["mcpCapabilities"]["http"], true);
    assert_eq!(value["agentCapabilities"]["mcpCapabilities"]["sse"], false);
}

#[test]
fn initialize_meta_vendor_block_and_eleven_schema_keys() {
    let result = initialize_result("reasonix", "0.3.0");
    let value = serde_json::to_value(&result).unwrap();
    let meta = &value["agentCapabilities"]["_meta"];

    let vendor = &meta["reasonix.io"];
    assert_eq!(
        vendor["sessionSteer"]["method"],
        "_reasonix.io/session/steer"
    );
    assert_eq!(vendor["sessionInbox"]["schemaVersion"], 1);
    assert_eq!(
        vendor["sessionInbox"]["methods"].as_object().unwrap().len(),
        9
    );
    assert_eq!(
        vendor["sessionReloadExtensions"]["method"],
        "_reasonix.io/session/reloadExtensions"
    );
    assert_eq!(vendor["extensionSurface"]["supported"], true);
    assert_eq!(vendor["extensionSurface"]["schemaVersion"], 1);

    let schema_keys = [
        "_reasonix.io/session/status",
        "_reasonix.io/session/status_update",
        "_reasonix.io/session/mcpStatus",
        "_reasonix.io/session/mcpStatus_update",
        "_reasonix.io/session/info",
        "_reasonix.io/session/mcpServers",
        "_reasonix.io/session/mcp/reconnect",
        "_reasonix.io/session/mcp/disconnect",
        "_reasonix.io/session/extension/submitForm",
        "_reasonix.io/session/goal/resume",
        "_reasonix.io/session/goal/pause",
    ];
    for key in schema_keys {
        assert_eq!(meta[key]["schemaVersion"], 1, "missing {key}");
    }
}

#[test]
fn protocol_version_is_always_one_without_negotiation() {
    assert_eq!(negotiate_protocol_version(99), 1);
    assert_eq!(negotiate_protocol_version(0), 1);
    assert_eq!(negotiate_protocol_version(i32::MAX), 1);
}

#[test]
fn authenticate_accepts_reasonix_setup_only_and_returns_empty_object() {
    let ok = authenticate(Some(&serde_json::json!({"methodId": "reasonix-setup"})));
    assert!(ok.is_ok());
    assert_eq!(
        serde_json::to_value(ok.unwrap()).unwrap(),
        serde_json::json!({})
    );

    let trimmed = authenticate(Some(&serde_json::json!({"methodId": "  reasonix-setup  "})));
    assert!(trimmed.is_ok());

    let bad = authenticate(Some(&serde_json::json!({"methodId": "other"}))).unwrap_err();
    assert_eq!(bad.code, RpcErrorCode::InvalidParams);
    assert_eq!(bad.message, "authenticate: unknown methodId other");

    // The unknown id in the message is the raw, untrimmed value.
    let spaced = authenticate(Some(&serde_json::json!({"methodId": "  other  "}))).unwrap_err();
    assert_eq!(spaced.message, "authenticate: unknown methodId   other  ");

    let malformed = authenticate(Some(&serde_json::json!({"methodId": 42}))).unwrap_err();
    assert_eq!(malformed.code, RpcErrorCode::InvalidParams);
    assert!(malformed.message.starts_with("authenticate: "));
}

#[test]
fn vendor_opt_in_tolerates_malformed_blocks() {
    let caps: reasonix_acp::caps::ClientCapabilities = serde_json::from_value(serde_json::json!({
        "_meta": {"reasonix.io": {"extensionSurface": {"supported": true}}}
    }))
    .unwrap();
    assert!(caps.extension_surface_supported());
    assert!(!caps.status_updates_supported());

    for shape in [
        serde_json::json!({}),
        serde_json::json!({"_meta": {"reasonix.io": null}}),
        serde_json::json!({"_meta": {"reasonix.io": {"extensionSurface": true}}}),
        serde_json::json!({"_meta": {"reasonix.io": {"extensionSurface": {"supported": 1}}}}),
        serde_json::json!({"_meta": {"reasonix.io": {"extensionSurface": {"supported": false}}}}),
    ] {
        let caps: reasonix_acp::caps::ClientCapabilities = serde_json::from_value(shape).unwrap();
        assert!(!caps.extension_surface_supported());
    }
}
