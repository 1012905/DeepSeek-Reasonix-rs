//! Anchors for the write-side confinement half (confine.go confineWrite /
//! confinePreview). `confine_write` now lives in [`super::confine`] with
//! its guard/managed/session-temp dependency machines reachable through
//! hook traits; the anchors left here are the error identity and the
//! first-stage check for callers holding no hooks.

#![forbid(unsafe_code)]

pub use super::confine::{WriteHooks, confine, confine_write, under_session_temp};
use crate::refusal::Refusal;
use thiserror::Error;

/// `workspace.write_outside_scope` (confine.go).
pub const CODE_WRITE_OUTSIDE_SCOPE: &str = "workspace.write_outside_scope";

/// The boundary error `confine` produces (Go: a typed Refusal or a
/// "resolve %s: %w" wrapper). The refusals stay `#[source]`-reachable so a
/// caller walking the chain finds the identity Go's errors.As finds.
#[derive(Debug, Error)]
pub enum ConfineError {
    /// Target resolved outside every write root: the typed refusal.
    #[error("{0}")]
    Outside(#[source] Refusal),
    /// The target itself could not be resolved.
    #[error("resolve {target}: {message}")]
    Resolve { target: String, message: String },
    /// The target named a machine other than one a root lives on.
    #[error("{0}")]
    Network(#[source] Refusal),
}

impl ConfineError {
    /// Whether the error is the network-path refusal (Go `isNetworkRefusal`).
    pub fn is_network(&self) -> bool {
        matches!(self, ConfineError::Network(_))
    }
}

/// First stage of `confineWrite` only: workspace confinement with the
/// network refusal distinguished, exactly what a caller holding no
/// guard/managed/temp hooks reproduces. Callers with hooks use
/// [`super::confine::confine_write`] directly.
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
