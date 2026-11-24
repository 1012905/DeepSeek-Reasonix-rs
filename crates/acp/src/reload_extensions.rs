//! ACP-041 `_reasonix.io/session/reloadExtensions` coalescing and rebuild
//! (`service.go:700-848`, `session.go:476-493`, `reload_extensions_test.go`).
//!
//! A reload rebuilds the agent runtime in place while the transcript, approval
//! grants, goal, and recovery state carry over. A turn or rebuild in flight
//! coalesces exactly one queued reload, drained when the session goes idle; a
//! failed build keeps the outgoing controller fully usable; the outgoing
//! controller's resources are released only after the swap publishes the
//! replacement. The real factory seam, controller rebuild, runtime state
//! resolution, and maintenance channel are runtime wiring; this module owns the
//! params/result DTOs, the busy-table decision, the drain gate, the stage order,
//! and the error identities.

use serde::{Deserialize, Serialize};

use crate::errors::{RpcError, RpcErrorCode};

/// Vendor-namespaced runtime-reload method (`protocol.go:557`); aliased to the
/// registered constant in [`crate::methods`] so the registry cannot drift.
pub const RELOAD_EXTENSIONS_METHOD: &str = crate::methods::SESSION_RELOAD_EXTENSIONS;

/// Inbound `session/reloadExtensions` parameters (`protocol.go:541-544`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionReloadExtensionsParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// Outcome body: `queued` rides only when the reload was coalesced behind an
/// in-flight turn or rebuild (`protocol.go:546-551`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionReloadExtensionsResult {
    #[serde(rename = "queued", skip_serializing_if = "Option::is_none")]
    pub queued: Option<bool>,
}

/// Immediate rebuild ack: no `queued` field on the wire.
pub fn immediate_result() -> SessionReloadExtensionsResult {
    SessionReloadExtensionsResult::default()
}

/// Coalesced-reload ack: `queued: true`.
pub fn queued_result() -> SessionReloadExtensionsResult {
    SessionReloadExtensionsResult { queued: Some(true) }
}

// ---- Busy precheck ------------------------------------------------------------

/// Runtime snapshot the precheck reads before rebuilding
/// (`service.go:733-753`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReloadBusy {
    /// Session was deleted while the request waited.
    pub deleted: bool,
    /// Controller has a prompt awaiting a response.
    pub pending_prompt: bool,
    /// Session-side runner is active.
    pub active_runner: bool,
    /// Controller reports a running turn.
    pub controller_running: bool,
    /// Background job count still draining.
    pub background_jobs: usize,
    /// A previous rebuild still holds the maintenance channel.
    pub maintenance: bool,
}

/// Precheck outcome: rebuild now, coalesce behind active work, or reject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadDecision {
    /// Idle enough to rebuild now.
    Proceed,
    /// Active turn, running runner, or ongoing maintenance: queue one reload.
    Queue,
    /// Rejected before any controller mutation.
    Reject(RpcError),
}

/// `-32600` deleted session (`service.go:736`).
pub fn deleted_error() -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidRequest,
        format!("{RELOAD_EXTENSIONS_METHOD}: session is deleted"),
    )
}

/// `-32600` active-work refusal, sharing the `session config:` prefix with the
/// config-switch path (`session.go:564-568`).
pub fn active_work_error(message: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidRequest,
        format!("session config: {message}"),
    )
}

/// Precheck order: deleted first, then pending prompt, then background jobs,
/// then the active/maintenance queue branch (`service.go:734-753`).
pub fn precheck(busy: &ReloadBusy) -> ReloadDecision {
    if busy.deleted {
        return ReloadDecision::Reject(deleted_error());
    }
    if busy.pending_prompt {
        return ReloadDecision::Reject(active_work_error(
            "answer pending prompts before reloading the runtime",
        ));
    }
    if !busy.active_runner && !busy.controller_running && busy.background_jobs > 0 {
        return ReloadDecision::Reject(active_work_error(
            "stop background jobs before reloading the runtime",
        ));
    }
    if busy.active_runner || busy.controller_running || busy.maintenance {
        return ReloadDecision::Queue;
    }
    ReloadDecision::Proceed
}

/// Drain gate (`drainPendingReload`, `service.go:858-862`): the queued reload
/// runs only when the session is idle, not deleted, not in maintenance, and has
/// no queued config switch.
pub fn can_drain(
    pending_reload: bool,
    deleted: bool,
    active_runner: bool,
    maintenance: bool,
    pending_config: bool,
) -> bool {
    pending_reload && !deleted && !active_runner && !maintenance && !pending_config
}

// ---- Error identities ---------------------------------------------------------

/// `-32600` the factory does not implement the rebuild seam
/// (`service.go:702-704`).
pub fn unavailable_error() -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidRequest,
        format!("{RELOAD_EXTENSIONS_METHOD}: runtime reload is unavailable in this session"),
    )
}

/// `-32602` unknown session (`session.go:489-491`).
pub fn unknown_session_error(session_id: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidParams,
        format!("{RELOAD_EXTENSIONS_METHOD}: unknown session {session_id}"),
    )
}

/// `-32603` pre-rebuild snapshot failure (`service.go:773-775`).
pub fn snapshot_before_reload_error(detail: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::Internal,
        format!("{RELOAD_EXTENSIONS_METHOD}: snapshot before reload: {detail}"),
    )
}

/// `-32603` the outgoing controller does not expose the rebuild path
/// (`service.go:781-783`).
pub fn not_rebuildable_error() -> RpcError {
    RpcError::new(
        RpcErrorCode::Internal,
        format!("{RELOAD_EXTENSIONS_METHOD}: session controller does not support rebuild"),
    )
}

/// `-32603` rebuild build failure (`service.go:800-802`).
pub fn rebuild_failed_error(detail: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::Internal,
        format!("{RELOAD_EXTENSIONS_METHOD}: {detail}"),
    )
}

/// `-32603` runtime-state recomputation failure (`service.go:809-811`).
pub fn runtime_state_error(detail: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::Internal,
        format!("{RELOAD_EXTENSIONS_METHOD}: runtime state: {detail}"),
    )
}

/// `-32600` controller changed mid-reload (`service.go:827-830`).
pub fn ctrl_changed_error() -> RpcError {
    active_work_error("session changed while reloading; retry")
}

/// Pending-config or queued-reload failure sentence (`session_config.go:441-447`).
/// Reported as a LevelWarn notice plus a `config_option_update`, never as a
/// JSON-RPC error body; `when` is one of `after turn`, `after maintenance`, or
/// `after queued reload`.
pub fn pending_switch_notice(when: &str, err: &str) -> String {
    format!("session config switch failed {when}: {err}")
}

// ---- Rebuild seam and stage order ---------------------------------------------

/// Controller handle the rebuild reads its recovery path from.
pub trait RebuildController {
    /// Session transcript path reported by the outgoing controller.
    fn session_path(&self) -> &str;
}

/// Factory rebuild seam; absent makes the method report unavailable.
pub trait SessionRebuilder {
    /// Builds the replacement from the outgoing controller; the `String` is the
    /// kernel error message, wrapped as an internal failure.
    fn rebuild(
        &mut self,
        old: &dyn RebuildController,
    ) -> Result<Box<dyn RebuildController>, String>;
}

/// Rebuild stages in reference execution order (`service.go:773-846`). The
/// outgoing controller is released only after the swap publishes the
/// replacement; the client gets a fresh `available_commands_update` without
/// waiting for the next turn.
pub const RELOAD_STAGES: &[&str] = &[
    "snapshot_controller",
    "capture_path_after_snapshot",
    "assemble_rebuild_params",
    "bind_client_io",
    "rebuild_session",
    "enable_interactive_approval",
    "runtime_state",
    "prepare_replacement_authority",
    "critical_section_guards",
    "swap_fields",
    "save_meta",
    "bind_composition_io",
    "release_old_resources",
    "send_available_commands",
];

/// Outgoing controller release: `ReleaseResources`, never `Close` — the session
/// logically continues, so SessionEnd hooks must not fire mid-reload
/// (`service.go:840-844`).
pub const OLD_CONTROLLER_RELEASE: &str = "release_resources";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    #[test]
    fn precheck_rejects_deleted_prompt_and_jobs_in_order() {
        let deleted = ReloadBusy {
            deleted: true,
            pending_prompt: true,
            background_jobs: 3,
            ..Default::default()
        };
        let ReloadDecision::Reject(err) = precheck(&deleted) else {
            panic!("deleted must reject");
        };
        assert_eq!(err.code, RpcErrorCode::InvalidRequest);
        assert_eq!(
            err.message,
            format!("{RELOAD_EXTENSIONS_METHOD}: session is deleted")
        );

        let prompt = ReloadBusy {
            pending_prompt: true,
            ..Default::default()
        };
        let ReloadDecision::Reject(err) = precheck(&prompt) else {
            panic!("pending prompt must reject");
        };
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
    fn running_maintenance_or_controller_running_queues() {
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
    fn drain_gate_requires_idle_without_pending_config() {
        assert!(can_drain(true, false, false, false, false));
        assert!(!can_drain(false, false, false, false, false));
        assert!(!can_drain(true, true, false, false, false));
        assert!(!can_drain(true, false, true, false, false));
        assert!(!can_drain(true, false, false, true, false));
        assert!(!can_drain(true, false, false, false, true));
    }

    #[test]
    fn result_omits_queued_when_immediate() {
        let immediate = serde_json::to_value(immediate_result()).unwrap();
        assert!(immediate.get("queued").is_none());
        let queued = serde_json::to_value(queued_result()).unwrap();
        assert_eq!(queued["queued"], true);
    }

    #[test]
    fn guard_error_identities_match_reference() {
        assert_eq!(
            unavailable_error().message,
            format!("{RELOAD_EXTENSIONS_METHOD}: runtime reload is unavailable in this session")
        );
        assert_eq!(
            unknown_session_error("nope").code,
            RpcErrorCode::InvalidParams
        );
        assert_eq!(
            snapshot_before_reload_error("conflict").message,
            format!("{RELOAD_EXTENSIONS_METHOD}: snapshot before reload: conflict")
        );
        assert_eq!(not_rebuildable_error().code, RpcErrorCode::Internal);
        assert_eq!(
            rebuild_failed_error("boom").message,
            format!("{RELOAD_EXTENSIONS_METHOD}: boom")
        );
        assert_eq!(
            runtime_state_error("nope").message,
            format!("{RELOAD_EXTENSIONS_METHOD}: runtime state: nope")
        );
        assert_eq!(
            ctrl_changed_error().message,
            "session config: session changed while reloading; retry"
        );
    }

    #[test]
    fn queued_reload_failure_is_a_warn_notice_sentence_not_an_rpc_error() {
        assert_eq!(
            pending_switch_notice("after queued reload", "build failed"),
            "session config switch failed after queued reload: build failed"
        );
    }

    #[test]
    fn stage_list_keeps_release_before_commands_and_without_duplicates() {
        assert!(RELOAD_STAGES.windows(2).all(|w| w[0] != w[1]));
        let release = RELOAD_STAGES
            .iter()
            .position(|s| *s == "release_old_resources")
            .unwrap();
        let commands = RELOAD_STAGES
            .iter()
            .position(|s| *s == "send_available_commands")
            .unwrap();
        assert!(release < commands);
        assert_eq!(OLD_CONTROLLER_RELEASE, "release_resources");
    }
}
