//! Controller surface the hub read side consumes: the read half of Go
//! `control.SessionAPI` (docs/contract.md §4.1), frozen so serve and the
//! host can implement it before the controller machine itself migrates.
//!
//! Write-side interface groups (turn control, approvals, inbox, settings,
//! jobs, ...) join here only when their machines migrate; none of them may
//! be stubbed with fake success (docs/contract.md §3).

/// What the hub reads off one open session: where it saves, what it
/// drives, and which folder owns it (Go `control.SessionAPI` `Lifecycle`).
pub trait Controller: Send + Sync {
    /// The transcript this controller drives, empty when none is bound yet
    /// (Go: `SessionPath`).
    fn session_path(&self) -> String;

    /// The directory new transcripts land in (Go: `SessionDir`).
    fn session_dir(&self) -> String;

    /// The folder the session works in (Go: `WorkspaceRoot`).
    fn workspace_root(&self) -> String;
}

/// Event fan-out handle for one runtime. The broadcaster entity lives in
/// serve; session names the subscriber surface in [`crate::fanout`] so
/// `Hub::broadcaster` is stable across the write-side migration
/// (docs/contract.md §4.1).
pub use crate::fanout::BroadcasterHandle;
