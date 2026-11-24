//! ACP-041 behavior coverage: reloadExtensions params/result, busy coalescing
//! precheck, drain gate, rebuild stage order, and guard error identities
//! (`service.go:700-848`, `protocol.go:541-557`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::reload_extensions::{
    RELOAD_EXTENSIONS_METHOD, RELOAD_STAGES, ReloadBusy, ReloadDecision,
    SessionReloadExtensionsParams, SessionReloadExtensionsResult, active_work_error, can_drain,
    ctrl_changed_error, deleted_error, immediate_result, not_rebuildable_error, precheck,
    queued_result, rebuild_failed_error, runtime_state_error, snapshot_before_reload_error,
    unavailable_error, unknown_session_error,
};

#[test]
fn method_constant_matches_reference() {
    assert_eq!(
        RELOAD_EXTENSIONS_METHOD,
        "_reasonix.io/session/reloadExtensions"
    );
}

#[test]
fn params_and_result_round_trip_with_queued_optional() {
    let params = SessionReloadExtensionsParams {
        session_id: "sess-1".to_owned(),
    };
    let value = serde_json::to_value(&params).unwrap();
    assert_eq!(value["sessionId"], "sess-1");
    let back: SessionReloadExtensionsParams = serde_json::from_value(value).unwrap();
    assert_eq!(back, params);

    let immediate = serde_json::to_value(immediate_result()).unwrap();
    assert!(immediate.get("queued").is_none());
    let queued = serde_json::to_value(queued_result()).unwrap();
    assert_eq!(queued["queued"], true);
    let back: SessionReloadExtensionsResult = serde_json::from_value(queued).unwrap();
    assert_eq!(back, queued_result());
}

#[test]
fn precheck_rejects_deleted_before_prompt_and_jobs() {
    let busy = ReloadBusy {
        deleted: true,
        pending_prompt: true,
        background_jobs: 4,
        ..Default::default()
    };
    let ReloadDecision::Reject(err) = precheck(&busy) else {
        panic!("deleted must reject");
    };
    assert_eq!(err.code, RpcErrorCode::InvalidRequest);
    assert_eq!(
        err.message,
        format!("{RELOAD_EXTENSIONS_METHOD}: session is deleted")
    );
}

#[test]
fn precheck_rejects_pending_prompt_then_background_jobs() {
    let prompt = ReloadBusy {
        pending_prompt: true,
        ..Default::default()
    };
    let ReloadDecision::Reject(err) = precheck(&prompt) else {
        panic!("pending prompt must reject");
    };
    assert_eq!(err.code, RpcErrorCode::InvalidRequest);
    assert_eq!(
        err.message,
        "session config: answer pending prompts before reloading the runtime"
    );

    let jobs = ReloadBusy {
        background_jobs: 2,
        ..Default::default()
    };
    let ReloadDecision::Reject(err) = precheck(&jobs) else {
        panic!("background jobs must reject");
    };
    assert_eq!(
        err.message,
        "session config: stop background jobs before reloading the runtime"
    );
}

#[test]
fn active_runner_controller_running_and_maintenance_coalesce() {
    assert_eq!(
        precheck(&ReloadBusy {
            active_runner: true,
            ..Default::default()
        }),
        ReloadDecision::Queue
    );
    assert_eq!(
        precheck(&ReloadBusy {
            controller_running: true,
            ..Default::default()
        }),
        ReloadDecision::Queue
    );
    assert_eq!(
        precheck(&ReloadBusy {
            maintenance: true,
            ..Default::default()
        }),
        ReloadDecision::Queue
    );
    assert_eq!(precheck(&ReloadBusy::default()), ReloadDecision::Proceed);
}

#[test]
fn drain_gate_keeps_reload_until_session_is_idle_and_config_queue_empty() {
    assert!(can_drain(true, false, false, false, false));
    assert!(!can_drain(false, false, false, false, false));
    assert!(!can_drain(true, true, false, false, false));
    assert!(!can_drain(true, false, true, false, false));
    assert!(!can_drain(true, false, false, true, false));
    assert!(!can_drain(true, false, false, false, true));
}

#[test]
fn rebuild_releases_old_after_swap_before_command_refresh() {
    let release = RELOAD_STAGES
        .iter()
        .position(|s| *s == "release_old_resources")
        .unwrap();
    let commands = RELOAD_STAGES
        .iter()
        .position(|s| *s == "send_available_commands")
        .unwrap();
    let swap = RELOAD_STAGES
        .iter()
        .position(|s| *s == "swap_fields")
        .unwrap();
    assert!(swap < release);
    assert!(release < commands);
    assert!(RELOAD_STAGES.windows(2).all(|w| w[0] != w[1]));
}

#[test]
fn guard_errors_carry_typed_codes_and_verbatim_templates() {
    assert_eq!(unavailable_error().code, RpcErrorCode::InvalidRequest);
    assert_eq!(
        unavailable_error().message,
        format!("{RELOAD_EXTENSIONS_METHOD}: runtime reload is unavailable in this session")
    );
    assert_eq!(
        unknown_session_error("ghost").message,
        format!("{RELOAD_EXTENSIONS_METHOD}: unknown session ghost")
    );
    assert_eq!(
        unknown_session_error("ghost").code,
        RpcErrorCode::InvalidParams
    );
    assert_eq!(
        snapshot_before_reload_error("conflict").message,
        format!("{RELOAD_EXTENSIONS_METHOD}: snapshot before reload: conflict")
    );
    assert_eq!(not_rebuildable_error().code, RpcErrorCode::Internal);
    assert_eq!(
        not_rebuildable_error().message,
        format!("{RELOAD_EXTENSIONS_METHOD}: session controller does not support rebuild")
    );
    assert_eq!(
        rebuild_failed_error("build failed").message,
        format!("{RELOAD_EXTENSIONS_METHOD}: build failed")
    );
    assert_eq!(
        runtime_state_error("no state").message,
        format!("{RELOAD_EXTENSIONS_METHOD}: runtime state: no state")
    );
    assert_eq!(
        ctrl_changed_error().message,
        "session config: session changed while reloading; retry"
    );
    assert_eq!(deleted_error().code, RpcErrorCode::InvalidRequest);
    assert_eq!(
        active_work_error("stop background jobs before reloading the runtime").message,
        "session config: stop background jobs before reloading the runtime"
    );
}
