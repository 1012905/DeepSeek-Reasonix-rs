//! Wire-stable vocabularies carried by the event payloads, mirroring the
//! constants in `event/event.go`, `event/progress_watch.go`, `event/receipt.go`
//! and `event/tool_issuer.go`.

/// Notice severity. Only the warning tier is distinguished; everything else
/// is information.
pub mod level {
    pub const INFO: &str = "info";
    pub const WARN: &str = "warn";
}

/// Notice recipient. The empty default is every frontend; `operator` is
/// runtime maintenance and must not be forwarded as end-user chat.
pub mod audience {
    pub const DEFAULT: &str = "";
    pub const OPERATOR: &str = "operator";
}

/// Who initiated a tool call. Model work and assistant work are read from
/// this, not from the call's wording.
pub mod issuer {
    pub const MODEL: &str = "model";
    pub const HOST: &str = "host";
    pub const USER: &str = "user";
    pub const PROVIDER: &str = "provider";
}

pub fn is_model_work(issuer: &str) -> bool {
    issuer == issuer::MODEL
}

pub fn is_assistant_work(issuer: &str) -> bool {
    issuer == issuer::MODEL || issuer == issuer::PROVIDER
}

/// Billable usage sources. Empty means executor for wire compatibility.
pub mod usage_source {
    pub const EXECUTOR: &str = "executor";
    pub const PLANNER: &str = "planner";
    pub const SUBAGENT: &str = "subagent";
    pub const COMPACTION: &str = "compaction";
    pub const CLASSIFIER: &str = "classifier";
    pub const TITLE: &str = "title";
    pub const CAPABILITY_ROUTER: &str = "capability-router";
    pub const RECOVERY_REVIEWER: &str = "recovery-reviewer";
    pub const GOAL_EVALUATOR: &str = "goal-evaluator";
    pub const PROMPT_REFINE: &str = "prompt-refine";
    pub const COMMIT_MESSAGE: &str = "commit-message";
    pub const INJECTION_SCREEN: &str = "injection-screen";
    pub const ADVISOR: &str = "advisor";
    pub const BEST_OF: &str = "best-of";
    pub const BEST_OF_JUDGE: &str = "best-of-judge";
    pub const DEEPSEEK_SEARCH: &str = "deepseek-search";
    pub const DEEPSEEK_FETCH: &str = "deepseek-fetch";
    pub const SIDE: &str = "side";

    pub const ALL: &[&str] = &[
        EXECUTOR,
        PLANNER,
        SUBAGENT,
        COMPACTION,
        CLASSIFIER,
        TITLE,
        CAPABILITY_ROUTER,
        RECOVERY_REVIEWER,
        GOAL_EVALUATOR,
        PROMPT_REFINE,
        COMMIT_MESSAGE,
        INJECTION_SCREEN,
        ADVISOR,
        BEST_OF,
        BEST_OF_JUDGE,
        DEEPSEEK_SEARCH,
        DEEPSEEK_FETCH,
        SIDE,
    ];
}

/// How a tool result was fitted into context.
pub mod bound_kind {
    pub const WHOLE: &str = "";
    pub const SPILLED: &str = "spilled";
    pub const WINDOWED: &str = "windowed";
    pub const TRUNCATED: &str = "truncated";
}

/// Retry connection phase; empty means an older emitter.
pub mod retry_scope {
    pub const HEADERS: &str = "headers";
    pub const STREAM: &str = "stream";
}

/// Speculative sampling attempt lifecycle.
pub mod stream_attempt {
    pub const BEGIN: &str = "begin";
    pub const DISCARD: &str = "discard";
    pub const COMMIT: &str = "commit";
    pub mod reason {
        pub const CONNECTION_RESET: &str = "connection_reset";
        pub const PREMATURE_EOF: &str = "premature_eof";
        pub const IDLE_TIMEOUT: &str = "idle_timeout";
    }
}

/// Structured ask reasons: these two are the only legal values.
pub mod ask_reason {
    pub const USER_DECISION: &str = "user_decision";
    pub const MISSING_VALUE: &str = "missing_value";
}

/// External origin kinds for an ask.
pub mod ask_origin {
    pub const MCP: &str = "mcp";
}

/// Approval surface kinds. An empty kind is an ordinary tool approval.
pub mod approval_kind {
    pub const TOOL: &str = "tool";
    pub const PLAN: &str = "plan";
    pub const RECOVERY: &str = "recovery";
}

/// Auto-guard change classification on a recovery approval.
pub mod recovery_change_kind {
    pub const SAME_STRATEGY: &str = "same_strategy";
    pub const STRATEGY: &str = "strategy";
    pub const SCOPE: &str = "scope";
    pub const RISK: &str = "risk";
    pub const UNCERTAIN: &str = "uncertain";
}

/// Extension surface kinds. `request` is reserved: blocking prompts route
/// through AskRequest.
pub mod extension_surface {
    pub const STATUS: &str = "status";
    pub const CARD: &str = "card";
    pub const FORM: &str = "form";
    pub const NOTIFICATION: &str = "notification";
    pub const REQUEST: &str = "request";
    pub const PANEL: &str = "panel";
    pub const VIEW: &str = "view";
    pub const EXPIRED: &str = "surface.expired";
}

/// Extension view tones say what a node means, not what colour it is.
pub mod extension_severity {
    pub const INFO: &str = "info";
    pub const WARN: &str = "warn";
    pub const ERROR: &str = "error";
}

/// Completion summary verdicts, review states and presets.
pub mod completion {
    pub mod verdict {
        pub const COMPLETE: &str = "complete";
        pub const PARTIAL: &str = "partial";
        pub const BLOCKED: &str = "blocked";
        pub const CONTINUE: &str = "continue";
    }
    pub mod review {
        pub const NONE: &str = "none";
        pub const PASSED: &str = "passed";
        pub const WARNED: &str = "warned";
        pub const FAILED: &str = "failed";
        pub const UNAVAILABLE: &str = "unavailable";
    }
    pub mod preset {
        pub const LIGHT: &str = "light";
        pub const BALANCED: &str = "balanced";
        pub const DELIVERY: &str = "delivery";
    }
}

/// Completion receipt severities, lightest first. None is the empty string.
pub mod receipt_severity {
    pub const NONE: &str = "";
    pub const NOTICE: &str = "notice";
    pub const WARN: &str = "warn";
}

/// Gap kinds the host grades as notice. Any unfamiliar kind is warn: an
/// unfamiliar finding must not drop to a whisper.
pub const RECEIPT_GAP_NOTICE_KINDS: &[&str] = &[
    "stale_verification",
    "inconclusive_verification",
    "unreviewed_change",
    "declared_unverified",
    "mod_effects_unobserved",
    "outside_observed_root",
];

pub fn receipt_gap_severity(kind: &str) -> &'static str {
    if RECEIPT_GAP_NOTICE_KINDS.contains(&kind) {
        receipt_severity::NOTICE
    } else {
        receipt_severity::WARN
    }
}

/// Compaction triggers and boundaries.
pub mod compaction {
    pub mod trigger {
        pub const AUTO: &str = "auto";
        pub const MANUAL: &str = "manual";
    }
    pub mod boundary {
        pub const CAPACITY: &str = "capacity";
        pub const ECONOMIC: &str = "economic";
    }
}

/// Progress-watch causes.
pub mod progress_watch {
    pub const ROUNDS: &str = "rounds";
    pub const TOKENS: &str = "tokens";
    pub const PERSEVERATION: &str = "perseveration";
}

/// Workspace watch states.
pub mod watch_state {
    pub const ACTIVE: &str = "active";
    pub const DEGRADED: &str = "degraded";
    pub const UNAVAILABLE: &str = "unavailable";
}

/// Guardian outcome and risk vocabulary. Wire names stay snake_case.
pub mod guardian {
    pub mod outcome {
        pub const ALLOW: &str = "allow";
        pub const DENY: &str = "deny";
    }
    pub mod risk {
        pub const LOW: &str = "low";
        pub const MEDIUM: &str = "medium";
        pub const HIGH: &str = "high";
        pub const CRITICAL: &str = "critical";
    }
}

/// Turn outcomes. `recovery_paused` is legacy recorded sessions only: a
/// reader renders it, but no writer should emit it.
pub mod turn_outcome {
    pub const NO_PROGRESS: &str = "no_progress";
    pub const RECOVERY_PAUSED: &str = "recovery_paused";
}

/// Turn phase names.
pub mod turn_phase {
    pub const WORKING: &str = "working";
    pub const CHECKING: &str = "checking";
    pub const VERIFYING: &str = "verifying";
    pub const REVIEWING: &str = "reviewing";
}
