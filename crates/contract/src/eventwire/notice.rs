//! Wire-stable notice codes and the localization fallback, mirroring
//! `event/notice_codes.go`. A code names a notice; its English text is what a
//! reader sees when no catalogue has wording for that code.

/// Machine-readable notice identifiers. Values are never renamed or reused.
pub mod code {
    pub const SESSION_TREE_CHANGED: &str = "session_tree_changed";
    pub const CHECKPOINT_RECORDING_FAILED: &str = "checkpoint_recording_failed";
    pub const TURN_IDENTITY_MISMATCH: &str = "turn_identity_mismatch";
    pub const PLAN_AUTO_APPROVED: &str = "plan_auto_approved";
    pub const EMPTY_FINAL: &str = "empty_final";
    pub const EXECUTOR_HANDOFF: &str = "executor_handoff";
    pub const TOOL_BUDGET: &str = "tool_budget";
    pub const PROMPT_QUEUED: &str = "prompt_queued";
    pub const LOOP_GUARD: &str = "loop_guard";
    pub const PERSEVERATION_LOOP: &str = "perseveration_loop";
    pub const PERSEVERATION_RETRY: &str = "perseveration_retry";
    pub const PROGRESS_GUARD: &str = "progress_guard";
    pub const EVIDENCE_NUDGE: &str = "evidence_nudge";
    pub const REASONING_GOVERNOR: &str = "reasoning_governor";
    pub const WORKSPACE_LEASE: &str = "workspace_lease";
    pub const HOOK_BLOCKED: &str = "hook_blocked";
    pub const HOOK_WARNED: &str = "hook_warned";
    pub const HOOK_FAILED: &str = "hook_failed";
    pub const HOOK_UNEVALUABLE: &str = "hook_unevaluable";
    pub const CANCELLED_TURN: &str = "cancelled_turn_display";
    pub const UNAPPLIED_STEER: &str = "unapplied_steer";
    pub const SESSION_RECOVERY_FORKED: &str = "session_recovery_forked";
    pub const SESSION_RECOVERY_ADOPTED: &str = "session_recovery_adopted";
    pub const SESSION_RECOVERY_ADOPTED_COVERED: &str = "session_recovery_adopted_covered";
    pub const SESSION_RECOVERY_DEPTH_CAP: &str = "session_recovery_depth_cap";
    pub const SESSION_SHUTDOWN_RECOVERY_FORKED: &str = "session_shutdown_recovery_forked";
    pub const VERIFICATION_STALLED: &str = "verification_stalled";
    pub const SETTING_CHANGED: &str = "setting_changed";
    pub const SETTING_OPEN: &str = "setting_open";
    pub const ASK_TIMEOUT_INVALID: &str = "ask_timeout_invalid";
    pub const DECISION_RECEIPT: &str = "decision_receipt";
    pub const CONTEXT_EDITING_FALLBACK: &str = "context_editing_fallback";
    pub const WORKSPACE_LEASE_RESUMED: &str = "workspace_lease_resumed";
    pub const WORKSPACE_LEASE_ABANDONED: &str = "workspace_lease_abandoned";
    pub const PERMISSION_SAVED: &str = "permission_saved";
    pub const PERMISSION_COVERED: &str = "permission_covered";
    pub const PERMISSION_SAVE_FAILED: &str = "permission_save_failed";
    pub const SUSPECTED_INJECTION: &str = "suspected_injection";
    pub const CONTEXT_REPORT: &str = "context_report";
    pub const PROJECT_PROGRAMS_AWAITING_APPROVAL: &str = "project_programs_awaiting_approval";
    pub const PROJECT_PROGRAM_CHANGED: &str = "project_program_changed";
    pub const PROJECT_MCP_AWAITING_APPROVAL: &str = "project_mcp_awaiting_approval";
    pub const PROJECT_MCP_CHANGED: &str = "project_mcp_changed";
    pub const MCP_TOOLS_HELD: &str = "mcp_tools_held";
    pub const MCP_TOOLS_FOLLOWED: &str = "mcp_tools_followed";
    pub const MCP_SCOPE_UPGRADE: &str = "mcp_scope_upgrade";
    pub const PEER_MESSAGE_ARRIVED: &str = "peer_message_arrived";
    pub const SESSION_CONTINUED_FROM_1X: &str = "session_continued_from_1x";
    pub const APPROVAL_MODE_UNRECOGNIZED: &str = "approval_mode_unrecognized";
    pub const PERMISSION_RULES_DORMANT: &str = "permission_rules_dormant";
    pub const INPUT_REFUSED: &str = "input_refused";
    pub const AWAIT_USER: &str = "await_user";
    pub const GOAL_RETRY: &str = "goal_retry";
    pub const GOAL_RETRY_GAVE_UP: &str = "goal_retry_gave_up";
    pub const EGRESS_AUTO_ALLOWED: &str = "egress_auto_allowed";
    pub const AWAIT_TIMED_OUT: &str = "await_timed_out";
    pub const AWAIT_GAVE_UP: &str = "await_gave_up";
    pub const UNKNOWN_COMMAND: &str = "unknown_command";
    pub const DISPLAY_CURRENCY: &str = "display_currency";
    pub const LANGUAGE_OVERRIDDEN: &str = "language_overridden";
    pub const DEFAULT_MODEL_UNAVAILABLE: &str = "default_model_unavailable";
    pub const INHERITED_SUBAGENT_EFFORT_DROPPED: &str = "inherited_subagent_effort_dropped";
    pub const BRANCH_MODEL_UNAVAILABLE: &str = "branch_model_unavailable";
    pub const MEMORY_MIGRATION_BACKUP: &str = "memory_migration_backup";
    pub const COMPACTED: &str = "compacted";
    pub const COMPACT_DECLINED: &str = "compact_declined";
    pub const COMPACT_FAILED: &str = "compact_failed";
    pub const COMPACT_HELD: &str = "compact_held";
    pub const CONTEXT_COMPOSITION: &str = "context_composition";
    pub const SUBAGENT_WAITING: &str = "subagent_waiting";
    pub const EXTENSION_SKIPPED: &str = "extension_skipped";
    pub const INBOX_RECOVERED: &str = "inbox_recovered";
    pub const INBOX_UNRECOVERED: &str = "inbox_unrecovered";
    pub const COMPACTION_TELEMETRY: &str = "compaction_telemetry";
    pub const USER_TURNS_DROPPED: &str = "user_turns_dropped";
    pub const KEPT_TURNS_FOLDED: &str = "kept_turns_folded";
    pub const JOB_STARTED: &str = "job_started";
    pub const JOB_FINISHED: &str = "job_finished";
    pub const JOB_KILLED: &str = "job_killed";
    pub const JOB_FAILED: &str = "job_failed";
    pub const JOB_STALLED: &str = "job_stalled";
    pub const WEB_SEARCH_MODEL_BAD_REF: &str = "web_search_model_bad_ref";
    pub const WEB_SEARCH_MODEL_NOT_ADDED: &str = "web_search_model_not_added";
    pub const WEB_SEARCH_MODEL_REMOVED: &str = "web_search_model_removed";
    pub const WEB_SEARCH_MODEL_UNSUPPORTED: &str = "web_search_model_unsupported";
    pub const WEB_SEARCH_MODEL_NO_CREDENTIALS: &str = "web_search_model_no_credentials";
    pub const WEB_SEARCH_BACKEND_FALLBACK: &str = "web_search_backend_fallback";
    pub const WEB_SEARCH_BACKEND_UNUSABLE: &str = "web_search_backend_unusable";
    pub const WEB_FETCH_BACKEND_FALLBACK: &str = "web_fetch_backend_fallback";
    pub const WEB_FETCH_BACKEND_UNUSABLE: &str = "web_fetch_backend_unusable";
    pub const COMPOSITION_TOOLS_CHANGED: &str = "composition_tools_changed";
    pub const FOLD_NOT_REUSED_HISTORY: &str = "fold_not_reused_history";
    pub const FOLD_NOT_REUSED_LINEAGE: &str = "fold_not_reused_lineage";
}

/// Every shipped code, in declaration order, for completeness checks.
pub const NOTICE_CODES: &[&str] = &[
    code::SESSION_TREE_CHANGED,
    code::CHECKPOINT_RECORDING_FAILED,
    code::TURN_IDENTITY_MISMATCH,
    code::PLAN_AUTO_APPROVED,
    code::EMPTY_FINAL,
    code::EXECUTOR_HANDOFF,
    code::TOOL_BUDGET,
    code::PROMPT_QUEUED,
    code::LOOP_GUARD,
    code::PERSEVERATION_LOOP,
    code::PERSEVERATION_RETRY,
    code::PROGRESS_GUARD,
    code::EVIDENCE_NUDGE,
    code::REASONING_GOVERNOR,
    code::WORKSPACE_LEASE,
    code::HOOK_BLOCKED,
    code::HOOK_WARNED,
    code::HOOK_FAILED,
    code::HOOK_UNEVALUABLE,
    code::CANCELLED_TURN,
    code::UNAPPLIED_STEER,
    code::SESSION_RECOVERY_FORKED,
    code::SESSION_RECOVERY_ADOPTED,
    code::SESSION_RECOVERY_ADOPTED_COVERED,
    code::SESSION_RECOVERY_DEPTH_CAP,
    code::SESSION_SHUTDOWN_RECOVERY_FORKED,
    code::VERIFICATION_STALLED,
    code::SETTING_CHANGED,
    code::SETTING_OPEN,
    code::ASK_TIMEOUT_INVALID,
    code::DECISION_RECEIPT,
    code::CONTEXT_EDITING_FALLBACK,
    code::WORKSPACE_LEASE_RESUMED,
    code::WORKSPACE_LEASE_ABANDONED,
    code::PERMISSION_SAVED,
    code::PERMISSION_COVERED,
    code::PERMISSION_SAVE_FAILED,
    code::SUSPECTED_INJECTION,
    code::CONTEXT_REPORT,
    code::PROJECT_PROGRAMS_AWAITING_APPROVAL,
    code::PROJECT_PROGRAM_CHANGED,
    code::PROJECT_MCP_AWAITING_APPROVAL,
    code::PROJECT_MCP_CHANGED,
    code::MCP_TOOLS_HELD,
    code::MCP_TOOLS_FOLLOWED,
    code::MCP_SCOPE_UPGRADE,
    code::PEER_MESSAGE_ARRIVED,
    code::SESSION_CONTINUED_FROM_1X,
    code::APPROVAL_MODE_UNRECOGNIZED,
    code::PERMISSION_RULES_DORMANT,
    code::INPUT_REFUSED,
    code::AWAIT_USER,
    code::GOAL_RETRY,
    code::GOAL_RETRY_GAVE_UP,
    code::EGRESS_AUTO_ALLOWED,
    code::AWAIT_TIMED_OUT,
    code::AWAIT_GAVE_UP,
    code::UNKNOWN_COMMAND,
    code::DISPLAY_CURRENCY,
    code::LANGUAGE_OVERRIDDEN,
    code::DEFAULT_MODEL_UNAVAILABLE,
    code::INHERITED_SUBAGENT_EFFORT_DROPPED,
    code::BRANCH_MODEL_UNAVAILABLE,
    code::MEMORY_MIGRATION_BACKUP,
    code::COMPACTED,
    code::COMPACT_DECLINED,
    code::COMPACT_FAILED,
    code::COMPACT_HELD,
    code::CONTEXT_COMPOSITION,
    code::SUBAGENT_WAITING,
    code::EXTENSION_SKIPPED,
    code::INBOX_RECOVERED,
    code::INBOX_UNRECOVERED,
    code::COMPACTION_TELEMETRY,
    code::USER_TURNS_DROPPED,
    code::KEPT_TURNS_FOLDED,
    code::JOB_STARTED,
    code::JOB_FINISHED,
    code::JOB_KILLED,
    code::JOB_FAILED,
    code::JOB_STALLED,
    code::WEB_SEARCH_MODEL_BAD_REF,
    code::WEB_SEARCH_MODEL_NOT_ADDED,
    code::WEB_SEARCH_MODEL_REMOVED,
    code::WEB_SEARCH_MODEL_UNSUPPORTED,
    code::WEB_SEARCH_MODEL_NO_CREDENTIALS,
    code::WEB_SEARCH_BACKEND_FALLBACK,
    code::WEB_SEARCH_BACKEND_UNUSABLE,
    code::WEB_FETCH_BACKEND_FALLBACK,
    code::WEB_FETCH_BACKEND_UNUSABLE,
    code::COMPOSITION_TOOLS_CHANGED,
    code::FOLD_NOT_REUSED_HISTORY,
    code::FOLD_NOT_REUSED_LINEAGE,
];

pub const LEVEL_INFO: &str = "info";
pub const LEVEL_WARN: &str = "warn";
pub const AUDIENCE_DEFAULT: &str = "";
pub const AUDIENCE_OPERATOR: &str = "operator";

/// Reports whether a code is one the kernel ships.
pub fn is_known(code: &str) -> bool {
    NOTICE_CODES.contains(&code)
}

/// Resolve a notice's copy. The catalogue is consulted only for a known code;
/// an empty or unknown code, or a catalogue without wording, keeps the
/// kernel's English text.
pub fn localized_or_text<'a>(
    code: &str,
    localized: impl FnOnce(&str) -> Option<&'a str>,
    text: &'a str,
) -> &'a str {
    if !code.is_empty()
        && is_known(code)
        && let Some(found) = localized(code)
    {
        return found;
    }
    text
}
