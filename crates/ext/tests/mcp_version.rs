//! MCP version/era negotiation oracles (MCP-002, MCP-003).
//!
//! Ported from `internal/ext/plugin/protocol_version_test.go`,
//! `protocol_modern_test.go`, and `protocol_version.go`.

use reasonix_ext::mcp::{
    CODE_HEADER_MISMATCH, CODE_MISSING_CLIENT_CAPABILITY, CODE_UNSUPPORTED_PROTOCOL_VERSION,
    DISCOVER_METHOD, ERA_CODES, META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_PROTOCOL_VERSION,
    MODERN_PROTOCOL_VERSION, MODERN_VERSIONS, OLDEST_PROTOCOL_VERSION, PROTOCOL_VERSION,
    SUPPORTED_PROTOCOL_VERSIONS, TASKS_EXTENSION, TransportKind, VersionError, check_input_bounds,
    classify_probe_result, client_capabilities, client_meta, connect_era, discover_result,
    initialize_params, negotiated_version, pick_era, with_meta,
};
use reasonix_ext::mcp::{Era, EraPlan, ProbeReply, RpcError};
use serde_json::json;

#[test]
fn supported_versions_are_newest_first() {
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS[0], "2025-11-25");
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS[3], OLDEST_PROTOCOL_VERSION);
    assert_eq!(PROTOCOL_VERSION, "2025-11-25");
    assert_eq!(MODERN_PROTOCOL_VERSION, "2026-07-28");
    assert_eq!(MODERN_VERSIONS, ["2026-07-28"]);
    assert_eq!(DISCOVER_METHOD, "server/discover");
}

#[test]
fn negotiated_version_reads_an_older_server() {
    // Go TestNegotiatedVersionReadsAnOlderServer.
    assert_eq!(negotiated_version("").unwrap(), "2024-11-05");
    assert_eq!(negotiated_version("2025-03-26").unwrap(), "2025-03-26");
    assert_eq!(negotiated_version("2025-06-18").unwrap(), "2025-06-18");
    assert_eq!(negotiated_version("2024-11-05").unwrap(), "2024-11-05");
    assert_eq!(negotiated_version("2025-11-25").unwrap(), "2025-11-25");
}

#[test]
fn negotiated_version_refuses_a_future_revision_by_identity() {
    for reply in ["2099-01-01", "2026-07-28", "v1", "2025-11-26"] {
        let err = negotiated_version(reply).unwrap_err();
        assert_eq!(
            err,
            VersionError::UnsupportedProtocolVersion {
                detail: Some(reply.into())
            }
        );
        assert!(err.to_string().contains("does not support"));
    }
}

#[test]
fn era_codes_are_in_the_json_rpc_implementation_range() {
    assert_eq!(
        ERA_CODES,
        [
            CODE_HEADER_MISMATCH,
            CODE_MISSING_CLIENT_CAPABILITY,
            CODE_UNSUPPORTED_PROTOCOL_VERSION
        ]
    );
    assert_eq!(CODE_HEADER_MISMATCH, -32020);
    assert_eq!(CODE_MISSING_CLIENT_CAPABILITY, -32021);
    assert_eq!(CODE_UNSUPPORTED_PROTOCOL_VERSION, -32022);
}

#[test]
fn client_meta_byte_shape_is_frozen() {
    // Confirm Go's encoding/json object-key ordering: sorted keys.
    let meta = client_meta(MODERN_PROTOCOL_VERSION, "");
    assert_eq!(
        meta.to_string(),
        r#"{"io.modelcontextprotocol/clientCapabilities":{"elicitation":{"form":{},"url":{}},"extensions":{"io.modelcontextprotocol/tasks":{}}},"io.modelcontextprotocol/clientInfo":{"name":"reasonix","version":"dev"},"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#
    );
    assert_eq!(
        META_CLIENT_CAPABILITIES,
        "io.modelcontextprotocol/clientCapabilities"
    );
    assert_eq!(META_CLIENT_INFO, "io.modelcontextprotocol/clientInfo");
    assert_eq!(
        META_PROTOCOL_VERSION,
        "io.modelcontextprotocol/protocolVersion"
    );
    assert_eq!(TASKS_EXTENSION, "io.modelcontextprotocol/tasks");
}

#[test]
fn client_capabilities_include_roots_only_with_a_workspace() {
    let without = client_capabilities("");
    assert!(without.get("roots").is_none());
    assert!(without.get("elicitation").is_some());
    assert!(without["extensions"][TASKS_EXTENSION].is_object());

    let with = client_capabilities("C:/work/proj");
    assert!(with.get("roots").is_some());
    assert_eq!(
        with,
        json!({
            "elicitation": {"form": {}, "url": {}},
            "extensions": {"io.modelcontextprotocol/tasks": {}},
            "roots": {}
        })
    );
}

#[test]
fn with_meta_preserves_existing_meta_without_mutating_params() {
    let params = json!({"name": "x", "_meta": {"progressToken": "t1"}});
    let meta = client_meta(MODERN_PROTOCOL_VERSION, "");
    let merged = with_meta(Some(&params), &meta).unwrap();
    assert_eq!(merged["name"], "x");
    assert_eq!(merged["_meta"]["progressToken"], "t1");
    assert_eq!(
        merged["_meta"][META_PROTOCOL_VERSION],
        MODERN_PROTOCOL_VERSION
    );
    // Caller's map is untouched.
    assert!(params["_meta"].get(META_PROTOCOL_VERSION).is_none());

    let none = with_meta(None, &meta).unwrap();
    assert_eq!(none["_meta"], meta);
}

#[test]
fn discover_result_requires_non_empty_supported_versions() {
    assert!(discover_result(&json!({"supportedVersions": []})).is_none());
    assert!(discover_result(&json!({"supportedVersions": "garbage"})).is_none());
    assert!(discover_result(&json!({})).is_none());
    let d = discover_result(&json!({
        "supportedVersions": ["2026-07-28"],
        "capabilities": {"tools": {}},
        "instructions": "  modern test server  "
    }))
    .unwrap();
    assert_eq!(d.supported_versions, ["2026-07-28"]);
    assert!(d.capabilities.contains_key("tools"));
    assert_eq!(d.instructions, "modern test server");
}

#[test]
fn pick_era_prefers_shared_modern_then_legacy_then_refuses() {
    // Go TestModernServerVersionSelection.
    let pick = |v: &[&str]| -> Result<EraPlan, VersionError> {
        pick_era(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>(), None)
    };
    assert_eq!(
        pick(&["2026-07-28"]).unwrap(),
        EraPlan::Modern {
            era: Era::modern("2026-07-28"),
            discover: None
        }
    );
    assert_eq!(
        pick(&["2099-01-01", "2025-11-25"]).unwrap(),
        EraPlan::Legacy
    );
    // A future-only server that shares no supported legacy revision is refused.
    let err = pick(&["2099-01-01"]).unwrap_err();
    assert!(matches!(
        err,
        VersionError::UnsupportedProtocolVersion { .. }
    ));
    assert!(err.to_string().contains("2099-01-01"));
}

#[test]
fn probe_result_classification_covers_every_go_branch() {
    // Result with supported versions → pickEra.
    let modern = classify_probe_result(ProbeReply::Result(json!({
        "supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}}
    })));
    assert!(matches!(modern, EraPlan::Modern { .. }));

    // Unparseable/empty result → legacy.
    assert_eq!(
        classify_probe_result(ProbeReply::Result(json!({"echo": "legacy"}))),
        EraPlan::Legacy
    );
    assert_eq!(
        classify_probe_result(ProbeReply::Result(json!({"supportedVersions": []}))),
        EraPlan::Legacy
    );

    // No reply → errProbeUnanswered (represented as Unanswered).
    assert_eq!(
        classify_probe_result(ProbeReply::NoReply),
        EraPlan::Unanswered
    );
    assert_eq!(
        classify_probe_result(ProbeReply::Cancelled),
        EraPlan::Failed(VersionError::Cancelled)
    );

    // Non -32022 RPC error → legacy (Go TestLegacyServerUsingAModernCodeStillGetsTheHandshake).
    for code in [CODE_HEADER_MISMATCH, CODE_MISSING_CLIENT_CAPABILITY] {
        let err = RpcError::new(code, "unknown method");
        assert_eq!(
            classify_probe_result(ProbeReply::RpcError(err)),
            EraPlan::Legacy
        );
    }

    // -32022 without a supported list → legacy.
    assert_eq!(
        classify_probe_result(ProbeReply::RpcError(RpcError::with_data(
            CODE_UNSUPPORTED_PROTOCOL_VERSION,
            "no",
            json!({"supported": "garbage"})
        ))),
        EraPlan::Legacy
    );
    assert_eq!(
        classify_probe_result(ProbeReply::RpcError(RpcError::new(
            CODE_UNSUPPORTED_PROTOCOL_VERSION,
            "no"
        ))),
        EraPlan::Legacy
    );

    // -32022 with a supported list → pickEra.
    let picked = classify_probe_result(ProbeReply::RpcError(RpcError::with_data(
        CODE_UNSUPPORTED_PROTOCOL_VERSION,
        "Unsupported protocol version",
        json!({"supported": ["2026-07-28"]}),
    )));
    assert!(matches!(picked, EraPlan::Modern { .. }));
}

#[test]
fn connect_era_never_speaks_modern_over_sse() {
    let modern_plan = EraPlan::Modern {
        era: Era::modern(MODERN_PROTOCOL_VERSION),
        discover: None,
    };
    assert_eq!(
        connect_era(TransportKind::Sse, modern_plan.clone()).unwrap(),
        Era::legacy()
    );
    assert_eq!(
        connect_era(TransportKind::Stdio, modern_plan).unwrap(),
        Era::modern(MODERN_PROTOCOL_VERSION)
    );
    assert_eq!(
        connect_era(TransportKind::StreamableHttp, EraPlan::Legacy).unwrap(),
        Era::legacy()
    );
    assert_eq!(
        connect_era(TransportKind::Stdio, EraPlan::Unanswered).unwrap(),
        Era::legacy()
    );
    assert!(
        connect_era(
            TransportKind::Stdio,
            EraPlan::Failed(VersionError::UnsupportedProtocolVersion { detail: None })
        )
        .is_err()
    );
}

#[test]
fn initialize_params_follow_the_legacy_handshake_shape() {
    let params = initialize_params("");
    assert_eq!(params["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(
        params["clientInfo"],
        json!({"name": "reasonix", "version": "dev"})
    );
    assert!(params["capabilities"].get("roots").is_none());
    assert_eq!(
        params["capabilities"]["elicitation"],
        json!({"form": {}, "url": {}})
    );

    let with_roots = initialize_params("C:/work/proj");
    assert_eq!(
        with_roots["capabilities"]["roots"],
        json!({"listChanged": false})
    );
}

#[test]
fn modern_input_round_bounds_match_go_constants() {
    assert!(check_input_bounds(0, 16, 1 << 20).is_ok());
    assert_eq!(
        check_input_bounds(0, 17, 0).unwrap_err(),
        VersionError::InputOverBounds
    );
    assert_eq!(
        check_input_bounds(0, 0, (1 << 20) + 1).unwrap_err(),
        VersionError::InputOverBounds
    );
    assert_eq!(
        check_input_bounds(3, 0, 0).unwrap_err(),
        VersionError::InputRoundsExhausted
    );
}

#[test]
fn typed_input_errors_have_stable_identity() {
    assert_eq!(
        VersionError::InputRequired {
            method: "elicitation/create".into()
        },
        VersionError::InputRequired {
            method: "elicitation/create".into()
        }
    );
    assert!(
        VersionError::InputRequired { method: "x".into() }
            .to_string()
            .contains("does not provide")
    );
    assert!(
        VersionError::ProbeUnanswered
            .to_string()
            .contains("server/discover")
    );
}
