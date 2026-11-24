//! ACP-025: `session/load` + `session/resume` replay decisions, live branch
//! refusals, after-response order, and mode-state vocabulary
//! (`session.go:297-347`; `service.go:486-659`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::methods::{SESSION_LOAD, SESSION_RESUME};
use reasonix_acp::session_open::{
    LiveOpen, OPEN_AFTER_RESPONSE, OpenBranch, PersistedOpen, REPLAY_LOAD, REPLAY_RESUME,
    decide_open, same_dirs, session_modes_state,
};

#[test]
fn load_replays_history_resume_does_not() {
    const _LOAD: () = assert!(REPLAY_LOAD);
    const _RESUME: () = assert!(!REPLAY_RESUME);

    let live = LiveOpen {
        cleanup_pending: false,
        additional_directories: vec!["/a".to_owned()],
        current_mode_id: "normal".to_owned(),
    };
    let loaded = decide_open(
        SESSION_LOAD,
        "s1",
        Some(&live),
        None,
        &["/a".to_owned()],
        REPLAY_LOAD,
    )
    .unwrap();
    assert!(loaded.replay);

    let resumed = decide_open(
        SESSION_RESUME,
        "s1",
        Some(&live),
        None,
        &["/a".to_owned()],
        REPLAY_RESUME,
    )
    .unwrap();
    assert!(!resumed.replay);
}

#[test]
fn live_branch_reconnect_keeps_current_mode() {
    let live = LiveOpen {
        cleanup_pending: false,
        additional_directories: vec!["/a".to_owned(), "/b".to_owned()],
        current_mode_id: "plan".to_owned(),
    };
    let decision = decide_open(
        SESSION_LOAD,
        "s1",
        Some(&live),
        None,
        &["/b".to_owned(), "/a".to_owned()],
        REPLAY_LOAD,
    )
    .unwrap();
    assert_eq!(decision.branch, OpenBranch::Live);
    assert_eq!(decision.live_mode_id.as_deref(), Some("plan"));
}

#[test]
fn cleanup_pending_live_and_persisted_refuse_as_unknown_session() {
    let pending_live = LiveOpen {
        cleanup_pending: true,
        additional_directories: vec![],
        current_mode_id: "normal".to_owned(),
    };
    let err = decide_open(SESSION_LOAD, "s1", Some(&pending_live), None, &[], true).unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(err.message, "session/load: unknown session s1");

    let pending_meta = PersistedOpen {
        cleanup_pending: true,
    };
    let err = decide_open(SESSION_RESUME, "s1", None, Some(&pending_meta), &[], false).unwrap_err();
    assert_eq!(err.message, "session/resume: unknown session s1");
}

#[test]
fn additional_directory_mismatch_carries_stable_data_code() {
    let live = LiveOpen {
        cleanup_pending: false,
        additional_directories: vec!["/a".to_owned()],
        current_mode_id: "normal".to_owned(),
    };
    let err = decide_open(
        SESSION_LOAD,
        "s1",
        Some(&live),
        None,
        &["/z".to_owned()],
        true,
    )
    .unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(
        err.message,
        "session/load: the session is open with other additional directories; close it first"
    );
    assert_eq!(
        err.data.as_ref().unwrap()["code"],
        "acp.additional_directories.session_open"
    );
    assert!(same_dirs(
        &["/a".to_owned(), "/b".to_owned()],
        &["/b".to_owned(), "/a".to_owned()]
    ));
    assert!(!same_dirs(
        &["/a".to_owned()],
        &["/a".to_owned(), "/b".to_owned()]
    ));
}

#[test]
fn persisted_and_unknown_branches_resolve_without_live_mode() {
    let persisted = PersistedOpen::default();
    let open = decide_open(SESSION_LOAD, "s1", None, Some(&persisted), &[], true).unwrap();
    assert_eq!(open.branch, OpenBranch::Persisted);
    assert!(open.live_mode_id.is_none());

    let missing = decide_open(SESSION_RESUME, "s1", None, None, &[], false).unwrap();
    assert_eq!(missing.branch, OpenBranch::Unknown);
}

#[test]
fn after_response_order_matches_reference_on_both_handlers() {
    assert_eq!(
        OPEN_AFTER_RESPONSE,
        [
            "send_available_commands",
            "send_usage_update",
            "publish_mcp_status",
        ]
    );
}

#[test]
fn mode_state_catalog_is_verbatim_with_normal_fallback() {
    let state = session_modes_state(Some("goal"));
    assert_eq!(state.current_mode_id, "goal");
    assert_eq!(state.available_modes.len(), 3);
    let plan = &state.available_modes[1];
    assert_eq!(plan.id, "plan");
    assert_eq!(plan.name, "Plan");
    assert_eq!(
        plan.description.as_deref(),
        Some("Research and propose a plan before making changes")
    );

    let fallback = session_modes_state(Some("not-a-mode"));
    assert_eq!(fallback.current_mode_id, "normal");
}

#[test]
fn invalid_session_id_refuses_before_any_branch_lookup() {
    let err = decide_open(SESSION_LOAD, "has space", None, None, &[], true).unwrap_err();
    assert_eq!(err.message, "session/load: invalid sessionId");
}
