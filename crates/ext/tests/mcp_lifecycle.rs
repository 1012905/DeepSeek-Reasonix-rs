//! Spec/Host lifecycle and StartPolicy/phase oracles (MCP-004, MCP-005).
//!
//! Ported from `internal/ext/plugin/plugin.go:38-165,250-575,769-1326`,
//! `runtime_spec.go`, `process_mode_test.go`, `host_concurrency_test.go`.

use std::collections::BTreeMap;

use reasonix_ext::mcp::{
    DEFAULT_CALL_TIMEOUT_MS, DEFAULT_START_CONCURRENCY, DEFAULT_START_TIMEOUT_MS, Host,
    LifecycleError, McpProcessMode, START_PHASES, Spec, StartPhase, StartPolicy, StartupFailure,
    TransportKind, build_start_plan, canonical_mcp_runtime_transport, check_deferred_generation,
    effective_concurrency, new_startup_failure, parse_transport_type, spawn_key,
};

#[test]
fn defaults_match_go_constants() {
    assert_eq!(DEFAULT_START_CONCURRENCY, 8);
    assert_eq!(DEFAULT_START_TIMEOUT_MS, 5_000);
    assert_eq!(DEFAULT_CALL_TIMEOUT_MS, 300_000);
}

#[test]
fn process_mode_defaults_to_host_and_only_confined_is_wrapped() {
    // Go TestResolvedProcessMode / process_mode_test.go.
    let spec = Spec::new("m");
    assert_eq!(spec.resolved_process_mode(), McpProcessMode::Host);
    assert_eq!(McpProcessMode::parse("confined"), McpProcessMode::Confined);
    assert_eq!(McpProcessMode::parse(""), McpProcessMode::Host);
    assert_eq!(McpProcessMode::parse("anything"), McpProcessMode::Host);
    assert_eq!(McpProcessMode::Host.as_str(), "host");
    assert_eq!(McpProcessMode::Confined.as_str(), "confined");
}

#[test]
fn transport_type_parsing_matches_go_new_transport() {
    assert_eq!(parse_transport_type("").unwrap(), TransportKind::Stdio);
    assert_eq!(parse_transport_type("stdio").unwrap(), TransportKind::Stdio);
    assert_eq!(
        parse_transport_type("http").unwrap(),
        TransportKind::StreamableHttp
    );
    assert_eq!(
        parse_transport_type("streamable-http").unwrap(),
        TransportKind::StreamableHttp
    );
    assert_eq!(
        parse_transport_type("streamable_http").unwrap(),
        TransportKind::StreamableHttp
    );
    assert_eq!(parse_transport_type("sse").unwrap(), TransportKind::Sse);
    let err = parse_transport_type("ftp").unwrap_err();
    assert_eq!(err, LifecycleError::UnknownTransportType("ftp".into()));
    assert!(err.to_string().contains("stdio|http|sse"));
}

#[test]
fn canonical_runtime_transport_normalizes_aliases() {
    assert_eq!(canonical_mcp_runtime_transport(""), "stdio");
    assert_eq!(canonical_mcp_runtime_transport("stdio"), "stdio");
    assert_eq!(canonical_mcp_runtime_transport("http"), "streamable-http");
    assert_eq!(
        canonical_mcp_runtime_transport("streamable_http"),
        "streamable-http"
    );
    assert_eq!(
        canonical_mcp_runtime_transport("streamable-http"),
        "streamable-http"
    );
    assert_eq!(canonical_mcp_runtime_transport("sse"), "sse");
    assert_eq!(canonical_mcp_runtime_transport("Custom"), "custom");
}

#[test]
fn timeouts_resolve_without_mutating_the_spec() {
    let mut spec = Spec::new("m");
    assert_eq!(
        spec.effective_startup_timeout_ms(),
        DEFAULT_START_TIMEOUT_MS
    );
    assert_eq!(spec.effective_call_timeout_ms(), DEFAULT_CALL_TIMEOUT_MS);

    spec.default_startup_timeout_ms = 9_000;
    spec.default_call_timeout_ms = 100;
    assert_eq!(spec.effective_startup_timeout_ms(), 9_000);
    assert_eq!(spec.effective_call_timeout_ms(), 100);

    spec.startup_timeout_ms = 1_234;
    spec.call_timeout_ms = 2_345;
    assert_eq!(spec.effective_startup_timeout_ms(), 1_234);
    assert_eq!(spec.effective_call_timeout_ms(), 2_345);
}

#[test]
fn authorization_is_the_single_source_and_tools_have_no_trust_bit() {
    let mut spec = Spec::new("m");
    assert!(!spec.server_authorized());
    spec.authorized = true;
    assert!(spec.server_authorized());
}

#[test]
fn disabled_tools_filter_server_local_names_before_namespacing() {
    let mut spec = Spec::new("m");
    spec.disabled_tools = vec!["secret".into()];
    assert!(!spec.tool_enabled("secret"));
    assert!(spec.tool_enabled("public"));
}

#[test]
fn start_policy_wrappers_match_go_semantics() {
    let all = StartPolicy::start_all();
    assert_eq!(all.concurrency, Some(8));
    assert!(all.abort_on_error);
    assert_eq!(all.per_plugin_timeout_ms, 0);

    let available = StartPolicy::start_available();
    assert_eq!(available.concurrency, Some(8));
    assert!(!available.abort_on_error);
    assert_eq!(available.per_plugin_timeout_ms, 5_000);

    let zero = StartPolicy::default();
    assert_eq!(zero.concurrency, None);
    assert!(!zero.abort_on_error);
}

#[test]
fn effective_concurrency_caps_and_preserves_stable_order() {
    assert_eq!(effective_concurrency(StartPolicy::default(), 12), 12);
    assert_eq!(effective_concurrency(StartPolicy::start_all(), 12), 8);
    assert_eq!(effective_concurrency(StartPolicy::start_all(), 3), 3);
    assert_eq!(
        effective_concurrency(
            StartPolicy {
                concurrency: Some(0),
                ..Default::default()
            },
            5
        ),
        5
    );
    assert_eq!(
        effective_concurrency(
            StartPolicy {
                concurrency: Some(99),
                ..Default::default()
            },
            5
        ),
        5
    );
    assert_eq!(
        effective_concurrency(
            StartPolicy {
                concurrency: Some(2),
                ..Default::default()
            },
            5
        ),
        2
    );
}

#[test]
fn start_plan_keeps_spec_order_and_rejects_unknown_transport() {
    let specs = vec![Spec::new("a"), Spec::new("b"), Spec::new("c")];
    let plan = build_start_plan(specs, StartPolicy::start_all()).unwrap();
    assert_eq!(
        plan.entries.iter().map(|e| e.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(plan.effective_concurrency, 3);
    assert_eq!(plan.phases, START_PHASES);

    let bad = vec![Spec {
        transport: "ftp".into(),
        ..Spec::new("bad")
    }];
    assert!(matches!(
        build_start_plan(bad, StartPolicy::default()).unwrap_err(),
        LifecycleError::UnknownTransportType(_)
    ));
}

#[test]
fn start_phases_are_ordered_and_have_stable_names() {
    assert_eq!(
        START_PHASES,
        [
            StartPhase::Launch,
            StartPhase::Initialize,
            StartPhase::ToolsList,
            StartPhase::PromptsResources,
        ]
    );
    assert_eq!(StartPhase::Launch.as_str(), "launch");
    assert_eq!(StartPhase::Initialize.as_str(), "initialize");
    assert_eq!(StartPhase::ToolsList.as_str(), "tools/list");
}

#[test]
fn host_connect_is_duplicate_safe_and_phases_advance() {
    let mut host = Host::new();
    host.connect(Spec::new("mock")).unwrap();
    assert_eq!(host.len(), 1);
    assert!(!host.is_empty());
    assert_eq!(host.names(), vec!["mock"]);
    assert_eq!(host.get("mock").unwrap().generation, 1);

    let dup = host.connect(Spec::new("mock")).unwrap_err();
    assert_eq!(dup, LifecycleError::ServerAlreadyConnected("mock".into()));

    host.advance_phase("mock", StartPhase::ToolsList).unwrap();
    assert!(host.get("mock").unwrap().tools_listed);
    assert_eq!(host.get("mock").unwrap().phase, StartPhase::ToolsList);
}

#[test]
fn host_remove_returns_namespace_prefix_and_bumps_generation() {
    let mut host = Host::new();
    host.connect(Spec::new("search/code")).unwrap();
    let before = host.generation();
    let removal = host.remove("search/code");
    assert!(removal.found);
    assert_eq!(removal.prefix, "mcp__search_code_2fc740__");
    assert!(host.is_empty());
    assert_eq!(host.generation(), before + 1);

    let missing = host.remove("nope");
    assert!(!missing.found);
}

#[test]
fn host_close_is_idempotent_and_refuses_connects() {
    let mut host = Host::new();
    host.close();
    host.close();
    assert!(host.is_closed());
    assert_eq!(
        host.connect(Spec::new("m")).unwrap_err(),
        LifecycleError::HostClosed
    );
}

#[test]
fn deferred_spawn_generation_guards_stale_spawns() {
    assert!(check_deferred_generation(5, 5).is_ok());
    assert_eq!(
        check_deferred_generation(6, 5).unwrap_err(),
        LifecycleError::DeferredSpawnCancelled
    );
    assert_eq!(spawn_key("search/code"), "search_code_2fc740");
}

#[test]
fn startup_failure_records_phase_timing_and_stderr_summary() {
    let failure = new_startup_failure("initialize", 250, "tail".into(), "boom".into());
    assert_eq!(
        failure,
        StartupFailure {
            phase: "initialize",
            elapsed_ms: 250,
            stderr_summary: "tail".into(),
            detail: "boom".into(),
        }
    );
    let err = LifecycleError::Startup {
        server: "m".into(),
        phase: failure.phase,
        detail: failure.detail,
    };
    assert!(err.to_string().contains("m"));
    assert!(err.to_string().contains("initialize"));
}

#[test]
fn spec_carries_runtime_only_fields_without_a_schema_role() {
    let mut spec = Spec::new("codegraph");
    spec.workspace_root = "/project".into();
    spec.dir = "/project".into();
    let mut timeouts = BTreeMap::new();
    timeouts.insert("index".to_string(), 1_000);
    spec.tool_timeouts_ms = timeouts;
    assert_eq!(spec.tool_timeouts_ms.get("index"), Some(&1_000));
    assert_eq!(spec.workspace_root, "/project");
}
