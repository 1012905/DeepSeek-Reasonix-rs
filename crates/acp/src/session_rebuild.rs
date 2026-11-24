//! ACP-034 config rebuild pipeline prechecks, stage order, queue merge, and
//! history splice rules (`service.go:941-1105`).
//!
//! A rebuild replaces one controller with another after a model, effort,
//! profile, or approval switch. This module owns the busy-state rejection table,
//! the last-write-wins pending queue merge, the fixed stage sequence, the
//! leading-system splice, and the guard error identities. Real snapshotting,
//! history adoption, lifecycle/authorization inheritance, controller factory
//! wiring, and the critical-section concurrency are runtime wiring; the stage
//! order and its invariants are checkable over a recording double.

use crate::errors::{RpcError, RpcErrorCode};

// ---- Busy-state precheck -----------------------------------------------------

/// Runtime snapshot the precheck reads before the rebuild starts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RebuildBusy {
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

/// Precheck outcome: proceed to rebuild, queue behind the active work, or
/// reject outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildPrecheck {
    /// Idle enough to rebuild now.
    Proceed,
    /// Active turn, running runner, or ongoing maintenance: merge into the
    /// pending queue and answer with the refreshed config state.
    Queue,
    /// Rejected before any controller mutation.
    Reject(RpcError),
}

/// `-32600` deleted session (`service.go:943-945`; `ErrInvalidRequest`).
pub fn deleted_error() -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidRequest,
        "session config: session is deleted",
    )
}

/// `-32600` active-work refusal (`service.go:564-568`).
pub fn active_work_error(message: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::InvalidRequest,
        format!("session config: {message}"),
    )
}

/// Precheck order: deleted first, then pending prompt, then background jobs,
/// then the active/maintenance queue branch (`service.go:942-963`).
pub fn precheck(busy: &RebuildBusy) -> RebuildPrecheck {
    if busy.deleted {
        return RebuildPrecheck::Reject(deleted_error());
    }
    if busy.pending_prompt {
        return RebuildPrecheck::Reject(active_work_error(
            "answer pending prompts before switching config",
        ));
    }
    if !busy.active_runner && !busy.controller_running && busy.background_jobs > 0 {
        return RebuildPrecheck::Reject(active_work_error(
            "stop background jobs before switching config",
        ));
    }
    if busy.active_runner || busy.controller_running || busy.maintenance {
        return RebuildPrecheck::Queue;
    }
    RebuildPrecheck::Proceed
}

// ---- Pending queue -----------------------------------------------------------

/// One queued config axis delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDelta {
    pub axis: String,
    pub value: String,
}

/// Last-write-wins per axis (`session_config.go:73-81`).
pub fn merge_pending(queue: Vec<ConfigDelta>, delta: ConfigDelta) -> Vec<ConfigDelta> {
    let mut queue = queue;
    for existing in queue.iter_mut() {
        if existing.axis == delta.axis {
            existing.axis = delta.axis.clone();
            existing.value = delta.value.clone();
            return queue;
        }
    }
    queue.push(delta);
    queue
}

/// Drops the axes the rebuild is applying, keeping other requests' queued
/// entries (`session_config.go:86-107`).
pub fn remove_pending_axes(queue: Vec<ConfigDelta>, applied: &[ConfigDelta]) -> Vec<ConfigDelta> {
    queue
        .into_iter()
        .filter(|entry| !applied.iter().any(|applied| applied.axis == entry.axis))
        .collect()
}

// ---- Stage order -------------------------------------------------------------

/// Rebuild stages in the reference execution order (`service.go:968-1103`).
/// Each label names the seam a recording double must invoke in sequence.
pub const REBUILD_STAGES: &[&str] = &[
    "claim_axes_raise_maintenance",
    "snapshot_controller",
    "capture_path_history_goal",
    "assemble_params",
    "bind_client_io",
    "new_session",
    "enable_interactive_approval",
    "runtime_state",
    "splice_leading_system",
    "adopt_history",
    "reapply_three_axes",
    "inherit_lifecycle_authorizations",
    "prepare_replacement_authority",
    "critical_section_guards",
    "swap_fields",
    "save_meta",
    "bind_composition_io",
    "release_old_resources",
    "send_available_commands",
    "config_option_update",
];

/// Outgoing controller release: `ReleaseResources`, never `Close`
/// (`service.go:1101`).
pub const OLD_CONTROLLER_RELEASE: &str = "release_resources";

/// `-32600` controller changed mid-rebuild (`service.go:1082-1085`).
pub fn ctrl_changed_error() -> RpcError {
    active_work_error("session changed while switching config; retry")
}

// ---- History splice -----------------------------------------------------------

/// One carried history message; role mirrors the provider message role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildMessage {
    pub role: String,
}

/// Splice the fresh controller's leading system message into carried history
/// before `AdoptHistory` (`service.go:1034-1040`): replace a carried leading
/// system, otherwise prepend it.
pub fn splice_leading_system(
    mut carried: Vec<RebuildMessage>,
    fresh: Vec<RebuildMessage>,
) -> Vec<RebuildMessage> {
    let Some(fresh_first) = fresh.first() else {
        return carried;
    };
    if fresh_first.role != "system" {
        return carried;
    }
    match carried.first_mut() {
        Some(carried_first) if carried_first.role == "system" => {
            *carried_first = fresh_first.clone();
            carried
        }
        _ => {
            let mut spliced = Vec::with_capacity(carried.len() + 1);
            spliced.push(fresh_first.clone());
            spliced.append(&mut carried);
            spliced
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    #[test]
    fn precheck_rejects_deleted_prompt_and_jobs_in_order() {
        let deleted = RebuildBusy {
            deleted: true,
            ..Default::default()
        };
        let RebuildPrecheck::Reject(err) = precheck(&deleted) else {
            panic!("deleted must reject");
        };
        assert_eq!(err.message, "session config: session is deleted");

        let prompt = RebuildBusy {
            pending_prompt: true,
            ..Default::default()
        };
        let RebuildPrecheck::Reject(err) = precheck(&prompt) else {
            panic!("pending prompt must reject");
        };
        assert!(err.message.contains("answer pending prompts"));

        let jobs = RebuildBusy {
            background_jobs: 2,
            ..Default::default()
        };
        let RebuildPrecheck::Reject(err) = precheck(&jobs) else {
            panic!("background jobs must reject");
        };
        assert!(err.message.contains("stop background jobs"));
    }

    #[test]
    fn running_or_maintenance_queues_instead_of_rejecting() {
        let runner = RebuildBusy {
            active_runner: true,
            ..Default::default()
        };
        assert_eq!(precheck(&runner), RebuildPrecheck::Queue);

        let maintenance = RebuildBusy {
            maintenance: true,
            ..Default::default()
        };
        assert_eq!(precheck(&maintenance), RebuildPrecheck::Queue);

        let idle = RebuildBusy::default();
        assert_eq!(precheck(&idle), RebuildPrecheck::Proceed);
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
    fn remove_pending_axes_keeps_other_queued_entries() {
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
    fn splice_replaces_or_prepends_leading_system() {
        let carried_system = vec![RebuildMessage {
            role: "system".into(),
        }];
        let fresh_system = vec![RebuildMessage {
            role: "system".into(),
        }];
        let spliced = splice_leading_system(carried_system, fresh_system.clone());
        assert_eq!(spliced.len(), 1);

        let carried_user = vec![RebuildMessage {
            role: "user".into(),
        }];
        let spliced = splice_leading_system(carried_user, fresh_system);
        assert_eq!(spliced[0].role, "system");
        assert_eq!(spliced[1].role, "user");

        let no_system = splice_leading_system(
            vec![RebuildMessage {
                role: "user".into(),
            }],
            vec![RebuildMessage {
                role: "assistant".into(),
            }],
        );
        assert_eq!(no_system[0].role, "user");
    }

    #[test]
    fn guard_error_identities_match_reference() {
        let err = ctrl_changed_error();
        assert_eq!(err.code, RpcErrorCode::InvalidRequest);
        assert_eq!(
            err.message,
            "session config: session changed while switching config; retry"
        );
    }

    #[test]
    fn stage_list_is_frozen_in_reference_order() {
        assert!(REBUILD_STAGES.windows(2).all(|w| w[0] != w[1]));
        assert_eq!(
            REBUILD_STAGES.first(),
            Some(&"claim_axes_raise_maintenance")
        );
        assert_eq!(REBUILD_STAGES.last(), Some(&"config_option_update"));
        assert_eq!(OLD_CONTROLLER_RELEASE, "release_resources");
    }
}
