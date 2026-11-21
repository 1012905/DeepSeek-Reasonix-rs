//! Session-level artifact naming and removal.
//!
//! Parity target: `reference/.../internal/state/store/session.go`
//! (docs/contracts/session-format.md 1, 20.1).

pub mod artifacts;
pub mod naming;

pub use artifacts::{RemoveArtifactsError, remove_session_artifacts};
pub use naming::{
    JSONL_SIDECAR_SUFFIXES, is_session_transcript_name, is_subagent_transcript_name,
    is_v4_session_id, session_adjudication, session_blobs_dir, session_blobs_dir_for_event_log,
    session_checkpoint_dir, session_cleanup_pending, session_conflict_log, session_context,
    session_display_index, session_event_index, session_event_log, session_event_log_damaged,
    session_execution, session_goal_state, session_inbox_dir, session_jobs_dir, session_lease_info,
    session_lease_lock, session_lock_file, session_meta, session_outputs_dir,
    session_path_for_event_log, session_path_for_outputs_dir, session_recovery_state,
    session_sidecar_dirs, session_sidecar_files, session_sidecar_moves, session_stem,
    session_superseded, session_v4_removed_marker, session_wire_log, session_wire_log_meta,
    v4_sessions_root,
};
