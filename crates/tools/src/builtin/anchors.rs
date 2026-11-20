//! Anchors for the write-side confinement half (confine.go confineWrite /
//! confinePreview) whose dependency machines — SessionDataGuard,
//! ManagedConfigPaths, the session-temp manager — live in the safety/state
//! slices this wave did not migrate. The signatures and semantics are
//! declared here so the read tools and future writers share one identity;
//! the bodies are not faked.

#![forbid(unsafe_code)]

pub use super::confine::confine;
use crate::refusal::Refusal;
use thiserror::Error;

/// `workspace.write_outside_scope` (confine.go).
pub const CODE_WRITE_OUTSIDE_SCOPE: &str = "workspace.write_outside_scope";

/// The boundary error `confine` produces (Go: a typed Refusal or a
/// "resolve %s: %w" wrapper).
#[derive(Debug, Error)]
pub enum ConfineError {
    /// Target resolved outside every write root: the typed refusal.
    #[error("{0}")]
    Outside(Refusal),
    /// The target itself could not be resolved.
    #[error("resolve {target}: {message}")]
    Resolve { target: String, message: String },
    /// The target named a machine other than one a root lives on.
    #[error("{0}")]
    Network(Refusal),
}

impl ConfineError {
    /// Whether the error is the network-path refusal (Go `isNetworkRefusal`).
    pub fn is_network(&self) -> bool {
        matches!(self, ConfineError::Network(_))
    }
}

/// Anchor: confine.go `confineWrite` — the write-tool boundary check.
/// Order is frozen: `confine` (workspace) first; inside the roots or under
/// the session temp → `guard.Check(target)`; a network refusal passes
/// through; a non-managed target gets the confinement error plus the
/// session-temp alternative; a managed config file needs the session guard
/// and then a fresh per-write human approval (`ManagedConfigPaths.approve`);
/// without an approver it fails closed with the original confinement error
/// semantics. Depends on SessionDataGuard (safety slice, §7.2),
/// ManagedConfigPaths (write-tools slice, §7.5) and the session-temp
/// manager (state slice) — none migrated yet, so the body is an anchor, not
/// an implementation. The read tools never call it.
#[allow(dead_code)]
pub fn confine_write_side(_target: &str) -> ConfineError {
    ConfineError::Outside(Refusal::code(
        CODE_WRITE_OUTSIDE_SCOPE,
        "anchor: confine_write_side is completed by the write-tools slice",
    ))
}

/// Anchor for `confineWrite`'s first stage only: workspace confinement with
/// the network refusal distinguished, exactly what a caller holding no
/// guard/managed/temp can reproduce today. Everything after those branches
/// is the unmigrated machine.
pub fn confine_write_side_resolved(roots: &[String], target: &str) -> Result<(), ConfineError> {
    match confine(roots, target) {
        Ok(()) => Ok(()),
        Err(ConfineError::Network(r)) => Err(ConfineError::Network(r)),
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_side_refusal_carries_the_workspace_code() {
        let root = if cfg!(windows) { "C:\\ws" } else { "/ws" };
        let target = if cfg!(windows) {
            "C:\\other\\x.txt"
        } else {
            "/other/x.txt"
        };
        let err = confine_write_side_resolved(&[String::from(root)], target).unwrap_err();
        match err {
            ConfineError::Outside(r) => assert_eq!(r.code, CODE_WRITE_OUTSIDE_SCOPE),
            other => panic!("want outside refusal, got {other:?}"),
        }
    }
}
