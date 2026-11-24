//! ACP-026: `session/prompt` flattening, stop-reason mapping, vendor meta, and
//! the inbox drain decision table (`session.go:353-427`; `inbox_drain.go:15-47`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_prompt::{
    AdmissionGate, DrainAction, DrainItem, DrainRound, DrainRunError, DrainSnapshot, PromptOutcome,
    TURN_DEFER_STEPS, TURN_POST_STEPS, admission_refusal, admit, classify_drain_error,
    classify_run, clip_turn_message, empty_prompt_error, flatten_prompt, next_drain_action,
    prompt_meta, turn_failed_error, unknown_session_error,
};
use reasonix_acp::wire::ContentBlock;

#[test]
fn flatten_joins_text_and_counts_byte_carrying_blocks() {
    let blocks = vec![
        ContentBlock {
            block_type: "text".to_owned(),
            text: Some("first".to_owned()),
            ..Default::default()
        },
        ContentBlock {
            block_type: "resource".to_owned(),
            resource: Some(reasonix_acp::wire::ResourceContents {
                uri: "r".to_owned(),
                text: Some("embedded".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ContentBlock {
            block_type: "image".to_owned(),
            data: Some("blob".to_owned()),
            ..Default::default()
        },
    ];
    let prompt = flatten_prompt(&blocks);
    assert_eq!(prompt.text, "first\n\nembedded");
    assert_eq!(prompt.attachment_blocks, 1);
}

#[test]
fn stop_mapping_prefers_end_turn_then_cancelled_then_failed() {
    assert_eq!(classify_run(false, false), PromptOutcome::EndTurn);
    assert_eq!(classify_run(false, true), PromptOutcome::EndTurn);
    assert_eq!(classify_run(true, true), PromptOutcome::Cancelled);
    assert_eq!(classify_run(true, false), PromptOutcome::TurnFailed);
}

#[test]
fn turn_failed_error_carries_turn_failed_data_code_and_clips() {
    let err = turn_failed_error("boom");
    assert_eq!(err.code, RpcErrorCode::Internal);
    assert_eq!(err.data.as_ref().unwrap()["code"], "turn_failed");
    assert!(clip_turn_message(&"y".repeat(3_000)).len() <= 2_048);
}

#[test]
fn transcript_meta_present_only_when_pinned() {
    assert!(prompt_meta("").is_none());
    let meta = prompt_meta("/sessions/s1.jsonl").unwrap();
    assert_eq!(meta["reasonix.io"]["transcriptPath"], "/sessions/s1.jsonl");
}

#[test]
fn drain_stops_on_paused_or_empty_queued() {
    let paused = DrainSnapshot {
        paused: true,
        items: vec![DrainItem {
            id: "i1".to_owned(),
            state: "queued".to_owned(),
        }],
    };
    assert_eq!(next_drain_action(&paused), DrainAction::Stop);

    let none_queued = DrainSnapshot {
        paused: false,
        items: vec![DrainItem {
            id: "i1".to_owned(),
            state: "done".to_owned(),
        }],
    };
    assert_eq!(next_drain_action(&none_queued), DrainAction::Stop);
}

#[test]
fn drain_runs_first_queued_in_fifo_snapshot_order() {
    let snapshot = DrainSnapshot {
        paused: false,
        items: vec![
            DrainItem {
                id: "i1".to_owned(),
                state: "done".to_owned(),
            },
            DrainItem {
                id: "i2".to_owned(),
                state: "queued".to_owned(),
            },
            DrainItem {
                id: "i3".to_owned(),
                state: "queued".to_owned(),
            },
        ],
    };
    assert_eq!(
        next_drain_action(&snapshot),
        DrainAction::RunNext("i2".to_owned())
    );
}

#[test]
fn drain_errors_refresh_snapshot_or_propagate_by_class() {
    assert_eq!(
        classify_drain_error(DrainRunError::NotFound),
        DrainRound::Refreshed
    );
    assert_eq!(
        classify_drain_error(DrainRunError::InvalidState),
        DrainRound::Refreshed
    );
    assert_eq!(
        classify_drain_error(DrainRunError::Other("boom".to_owned())),
        DrainRound::Propagated("boom".to_owned())
    );
}

#[test]
fn drain_loop_drives_refresh_then_queued_then_stop() {
    let rounds = [
        DrainSnapshot {
            paused: false,
            items: vec![DrainItem {
                id: "i1".to_owned(),
                state: "queued".to_owned(),
            }],
        },
        DrainSnapshot {
            paused: false,
            items: vec![
                DrainItem {
                    id: "i1".to_owned(),
                    state: "done".to_owned(),
                },
                DrainItem {
                    id: "i2".to_owned(),
                    state: "queued".to_owned(),
                },
            ],
        },
        DrainSnapshot {
            paused: false,
            items: vec![],
        },
    ];
    let mut actions = Vec::new();
    for snapshot in &rounds {
        actions.push(next_drain_action(snapshot));
    }
    assert_eq!(actions[0], DrainAction::RunNext("i1".to_owned()));
    assert_eq!(actions[1], DrainAction::RunNext("i2".to_owned()));
    assert_eq!(actions[2], DrainAction::Stop);
}

#[test]
fn prompt_error_templates_are_verbatim() {
    assert_eq!(
        unknown_session_error("s9").message,
        "session/prompt: unknown session s9"
    );
    assert_eq!(
        admission_refusal().message,
        "session/prompt: session already has an active prompt"
    );
    assert_eq!(empty_prompt_error().message, "session/prompt: empty prompt");
}

#[test]
fn begin_refuses_active_deleted_maintenance_and_pending_config_equally() {
    assert!(admit(&AdmissionGate::default()).is_ok());

    for gate in [
        AdmissionGate {
            active: true,
            ..Default::default()
        },
        AdmissionGate {
            deleted: true,
            ..Default::default()
        },
        AdmissionGate {
            maintenance: true,
            ..Default::default()
        },
        AdmissionGate {
            pending_config: true,
            ..Default::default()
        },
    ] {
        let err = admit(&gate).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidRequest);
        assert_eq!(
            err.message,
            "session/prompt: session already has an active prompt"
        );
    }
}

#[test]
fn deferred_and_post_turn_step_orders_match_reference() {
    assert_eq!(
        TURN_DEFER_STEPS,
        ["clear_turn_context", "finish_turn", "cancel_context"]
    );
    assert_eq!(
        TURN_POST_STEPS,
        [
            "finish_turn_status",
            "publish_status",
            "send_usage_update",
            "persist_after_turn",
        ]
    );
}
