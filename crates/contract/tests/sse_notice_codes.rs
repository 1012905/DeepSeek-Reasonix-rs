//! Notice-code registry completeness and Detail decode guards (ledger
//! SSE-014..SSE-015). The code set is pinned against the Go source inventory;
//! the decode cases pin the kernel's English fallback on a malformed Detail.

use std::collections::HashSet;

use reasonix_contract::eventwire::notice::{self, NOTICE_CODES, code, is_known, localized_or_text};
use reasonix_contract::eventwire::notice_payload::*;

const GO_GOLDEN_CODES: &[&str] = &[
    "session_tree_changed",
    "checkpoint_recording_failed",
    "turn_identity_mismatch",
    "plan_auto_approved",
    "empty_final",
    "executor_handoff",
    "tool_budget",
    "prompt_queued",
    "loop_guard",
    "perseveration_loop",
    "perseveration_retry",
    "progress_guard",
    "evidence_nudge",
    "reasoning_governor",
    "workspace_lease",
    "hook_blocked",
    "hook_warned",
    "hook_failed",
    "hook_unevaluable",
    "cancelled_turn_display",
    "unapplied_steer",
    "session_recovery_forked",
    "session_recovery_adopted",
    "session_recovery_adopted_covered",
    "session_recovery_depth_cap",
    "session_shutdown_recovery_forked",
    "verification_stalled",
    "setting_changed",
    "setting_open",
    "ask_timeout_invalid",
    "decision_receipt",
    "context_editing_fallback",
    "workspace_lease_resumed",
    "workspace_lease_abandoned",
    "permission_saved",
    "permission_covered",
    "permission_save_failed",
    "suspected_injection",
    "context_report",
    "project_programs_awaiting_approval",
    "project_program_changed",
    "project_mcp_awaiting_approval",
    "project_mcp_changed",
    "mcp_tools_held",
    "mcp_tools_followed",
    "mcp_scope_upgrade",
    "peer_message_arrived",
    "session_continued_from_1x",
    "approval_mode_unrecognized",
    "permission_rules_dormant",
    "input_refused",
    "await_user",
    "goal_retry",
    "goal_retry_gave_up",
    "egress_auto_allowed",
    "await_timed_out",
    "await_gave_up",
    "unknown_command",
    "display_currency",
    "language_overridden",
    "default_model_unavailable",
    "inherited_subagent_effort_dropped",
    "branch_model_unavailable",
    "memory_migration_backup",
    "compacted",
    "compact_declined",
    "compact_failed",
    "compact_held",
    "context_composition",
    "subagent_waiting",
    "extension_skipped",
    "inbox_recovered",
    "inbox_unrecovered",
    "compaction_telemetry",
    "user_turns_dropped",
    "kept_turns_folded",
    "job_started",
    "job_finished",
    "job_killed",
    "job_failed",
    "job_stalled",
    "web_search_model_bad_ref",
    "web_search_model_not_added",
    "web_search_model_removed",
    "web_search_model_unsupported",
    "web_search_model_no_credentials",
    "web_search_backend_fallback",
    "web_search_backend_unusable",
    "web_fetch_backend_fallback",
    "web_fetch_backend_unusable",
    "composition_tools_changed",
    "fold_not_reused_history",
    "fold_not_reused_lineage",
];

#[test]
fn registry_covers_every_go_code_without_repeats() {
    let got: HashSet<&str> = NOTICE_CODES.iter().copied().collect();
    let want: HashSet<&str> = GO_GOLDEN_CODES.iter().copied().collect();
    assert_eq!(got, want, "notice-code set drifted from Go notice_codes.go");
    assert_eq!(NOTICE_CODES.len(), got.len(), "notice codes must be unique");
    assert_eq!(NOTICE_CODES.len(), 93);
    assert!(is_known(code::EMPTY_FINAL));
    assert!(!is_known("no_such_code"));
    assert!(!is_known(""));
}

#[test]
fn detail_is_payload_matches_go_set() {
    let payload_codes: Vec<&str> = NOTICE_CODES
        .iter()
        .copied()
        .filter(|c| detail_is_payload(c))
        .collect();
    assert_eq!(payload_codes.len(), 15);
    for c in &[
        code::UNAPPLIED_STEER,
        code::EXTENSION_SKIPPED,
        code::INBOX_RECOVERED,
        code::INBOX_UNRECOVERED,
        code::PERMISSION_RULES_DORMANT,
        code::SUBAGENT_WAITING,
        code::CONTEXT_COMPOSITION,
        code::COMPACTION_TELEMETRY,
        code::USER_TURNS_DROPPED,
        code::KEPT_TURNS_FOLDED,
        code::JOB_STARTED,
        code::JOB_FINISHED,
        code::JOB_KILLED,
        code::JOB_FAILED,
        code::JOB_STALLED,
    ] {
        assert!(detail_is_payload(c), "{c} must be a payload code");
    }
    assert!(!detail_is_payload(code::EMPTY_FINAL));
    assert!(!detail_is_payload(code::COMPACTED));
    assert!(!detail_is_payload(""));
}

#[test]
fn localization_falls_back_to_the_kernel_text() {
    let catalogue = |c: &str| -> Option<&'static str> {
        if c == code::COMPACTED {
            Some("translated")
        } else {
            None
        }
    };
    assert_eq!(
        localized_or_text(code::COMPACTED, catalogue, "kernel"),
        "translated"
    );
    assert_eq!(
        localized_or_text(code::EMPTY_FINAL, catalogue, "kernel"),
        "kernel"
    );
    assert_eq!(
        localized_or_text("no_such_code", catalogue, "kernel"),
        "kernel"
    );
    assert_eq!(localized_or_text("", catalogue, "kernel"), "kernel");
    assert_eq!(notice::LEVEL_INFO, "info");
    assert_eq!(notice::LEVEL_WARN, "warn");
}

#[test]
fn decode_guards_refuse_non_payloads_with_typed_identity() {
    assert!(decode_extension_skipped(r#"{"extension":"x","point":"p","reason":"r"}"#).is_ok());
    assert_eq!(
        decode_extension_skipped(r#"{"extension":"","point":"p","reason":"r"}"#),
        Err(DetailDecodeError::GuardFailed)
    );
    assert_eq!(
        decode_extension_skipped("not json"),
        Err(DetailDecodeError::NotPayload)
    );

    assert!(decode_inbox_recovered(r#"{"count":1}"#).is_ok());
    assert_eq!(
        decode_inbox_recovered(r#"{"count":0}"#),
        Err(DetailDecodeError::GuardFailed)
    );
    assert!(decode_inbox_unrecovered(r#"{"dir":"/x"}"#).is_ok());
    assert_eq!(
        decode_inbox_unrecovered(r#"{"dir":""}"#),
        Err(DetailDecodeError::GuardFailed)
    );

    assert!(decode_job_notice(r#"{"kind":"shell","id":"j1"}"#).is_ok());
    assert_eq!(
        decode_job_notice(r#"{"kind":"shell","id":""}"#),
        Err(DetailDecodeError::GuardFailed)
    );

    assert!(decode_compaction_notice(r#"{"mode":"summarized"}"#).is_ok());
    assert_eq!(
        decode_compaction_notice(r#"{"mode":""}"#),
        Err(DetailDecodeError::GuardFailed)
    );
    for err in [
        decode_compaction_notice("null").unwrap_err(),
        decode_compaction_notice("[]").unwrap_err(),
        decode_compaction_notice("\"text\"").unwrap_err(),
    ] {
        assert!(err.is_not_payload());
    }
}

#[test]
fn unguarded_decoders_tolerate_missing_fields_as_go_does() {
    assert_eq!(
        decode_permission_rules_dormant("{}").unwrap(),
        PermissionRulesDormant::default()
    );
    assert_eq!(
        decode_context_composition("{}").unwrap(),
        ContextComposition::default()
    );
    assert_eq!(
        decode_subagent_waiting("{}").unwrap(),
        SubagentWaiting::default()
    );
    assert_eq!(decode_turns_dropped("{}").unwrap(), TurnsDropped::default());
    assert_eq!(
        decode_inbox_recovered("null").unwrap_err(),
        DetailDecodeError::GuardFailed
    );
}
