//! ACP-034: config rebuild precheck table, pending queue merge, history splice,
//! stage order, and guard error identities (`service.go:941-1105`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_rebuild::{
    ConfigDelta, OLD_CONTROLLER_RELEASE, REBUILD_STAGES, RebuildBusy, RebuildMessage,
    RebuildPrecheck, active_work_error, ctrl_changed_error, deleted_error, merge_pending, precheck,
    remove_pending_axes, splice_leading_system,
};

#[test]
fn deleted_is_rejected_first() {
    let busy = RebuildBusy {
        deleted: true,
        pending_prompt: true,
        background_jobs: 3,
        ..Default::default()
    };
    let RebuildPrecheck::Reject(err) = precheck(&busy) else {
        panic!("deleted must reject");
    };
    assert_eq!(err.message, "session config: session is deleted");
}

#[test]
fn pending_prompt_and_background_jobs_reject_with_verbatim_messages() {
    let RebuildPrecheck::Reject(err) = precheck(&RebuildBusy {
        pending_prompt: true,
        ..Default::default()
    }) else {
        panic!("pending prompt must reject");
    };
    assert_eq!(
        err.message,
        "session config: answer pending prompts before switching config"
    );

    let RebuildPrecheck::Reject(err) = precheck(&RebuildBusy {
        background_jobs: 2,
        ..Default::default()
    }) else {
        panic!("background jobs must reject");
    };
    assert_eq!(
        err.message,
        "session config: stop background jobs before switching config"
    );
}

#[test]
fn runner_maintenance_or_controller_running_queues_instead_of_rejecting() {
    assert_eq!(
        precheck(&RebuildBusy {
            active_runner: true,
            ..Default::default()
        }),
        RebuildPrecheck::Queue
    );
    assert_eq!(
        precheck(&RebuildBusy {
            controller_running: true,
            ..Default::default()
        }),
        RebuildPrecheck::Queue
    );
    assert_eq!(
        precheck(&RebuildBusy {
            maintenance: true,
            ..Default::default()
        }),
        RebuildPrecheck::Queue
    );
    assert_eq!(precheck(&RebuildBusy::default()), RebuildPrecheck::Proceed);
}

#[test]
fn queue_merge_is_last_write_wins_per_axis() {
    let queue = vec![
        ConfigDelta {
            axis: "model".into(),
            value: "old".into(),
        },
        ConfigDelta {
            axis: "effort".into(),
            value: "low".into(),
        },
    ];
    let merged = merge_pending(
        queue,
        ConfigDelta {
            axis: "model".into(),
            value: "new".into(),
        },
    );
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].value, "new");

    let appended = merge_pending(
        merged,
        ConfigDelta {
            axis: "profile".into(),
            value: "fast".into(),
        },
    );
    assert_eq!(appended.len(), 3);
}

#[test]
fn remove_pending_axes_drops_only_applied_axes() {
    let queue = vec![
        ConfigDelta {
            axis: "model".into(),
            value: "a".into(),
        },
        ConfigDelta {
            axis: "effort".into(),
            value: "b".into(),
        },
    ];
    let applied = vec![ConfigDelta {
        axis: "model".into(),
        value: "a".into(),
    }];
    let kept = remove_pending_axes(queue, &applied);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].axis, "effort");
}

#[test]
fn leading_system_is_replaced_or_prepended_before_adopt_history() {
    let carried_system = vec![RebuildMessage {
        role: "system".into(),
    }];
    let fresh_system = vec![RebuildMessage {
        role: "system".into(),
    }];
    let spliced = splice_leading_system(carried_system, fresh_system);
    assert_eq!(spliced.len(), 1);

    let carried_user = vec![RebuildMessage {
        role: "user".into(),
    }];
    let spliced = splice_leading_system(
        carried_user,
        vec![RebuildMessage {
            role: "system".into(),
        }],
    );
    assert_eq!(spliced[0].role, "system");
    assert_eq!(spliced[1].role, "user");

    let untouched = splice_leading_system(
        vec![RebuildMessage {
            role: "user".into(),
        }],
        vec![RebuildMessage {
            role: "assistant".into(),
        }],
    );
    assert_eq!(untouched[0].role, "user");
}

#[test]
fn stage_list_matches_reference_order_without_duplicates() {
    assert!(REBUILD_STAGES.windows(2).all(|w| w[0] != w[1]));
    assert_eq!(REBUILD_STAGES[0], "claim_axes_raise_maintenance");
    assert_eq!(
        REBUILD_STAGES[REBUILD_STAGES.len() - 1],
        "config_option_update"
    );
    // Snapshot precedes history capture; authority preparation precedes swap.
    let snapshot = REBUILD_STAGES
        .iter()
        .position(|s| *s == "snapshot_controller")
        .unwrap();
    let capture = REBUILD_STAGES
        .iter()
        .position(|s| *s == "capture_path_history_goal")
        .unwrap();
    let authority = REBUILD_STAGES
        .iter()
        .position(|s| *s == "prepare_replacement_authority")
        .unwrap();
    let swap = REBUILD_STAGES
        .iter()
        .position(|s| *s == "swap_fields")
        .unwrap();
    assert!(snapshot < capture);
    assert!(authority < swap);
}

#[test]
fn old_controller_is_released_not_closed() {
    assert_eq!(OLD_CONTROLLER_RELEASE, "release_resources");
}

#[test]
fn guard_error_identities_match_reference() {
    let deleted = deleted_error();
    assert_eq!(deleted.code, RpcErrorCode::InvalidRequest);

    let changed = ctrl_changed_error();
    assert_eq!(changed.code, RpcErrorCode::InvalidRequest);
    assert_eq!(
        changed.message,
        "session config: session changed while switching config; retry"
    );

    let active = active_work_error("answer pending prompts before switching config");
    assert_eq!(active.code, RpcErrorCode::InvalidRequest);
    assert_eq!(
        active.message,
        "session config: answer pending prompts before switching config"
    );
}
