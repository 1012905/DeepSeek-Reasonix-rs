//! Approval-flag conflict resolution, shared by `run` and the TUI.
//!
//! Parity target: `internal/frontend/cli/cli_flags.go:211-232`
//! (`resolveRunPermissionMode`). `--dangerously-skip-permissions` shares the
//! `--yolo` pointer in the reference, so it counts as the yolo spelling here
//! too. The three pairwise conflicts map to the frozen messages in [`crate::text`].

use crate::flags::ParsedFlags;
use crate::text::{AUTO_PERMISSION_CONFLICT, AUTO_YOLO_CONFLICT, YOLO_PERMISSION_CONFLICT};

/// The pairwise approval conflict a rejected invocation hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalConflict {
    AutoYolo,
    AutoMode,
    YoloMode,
}

impl ApprovalConflict {
    /// Frozen reference message for the conflict (without the `error:` prefix).
    pub fn message(self) -> &'static str {
        match self {
            ApprovalConflict::AutoYolo => AUTO_YOLO_CONFLICT,
            ApprovalConflict::AutoMode => AUTO_PERMISSION_CONFLICT,
            ApprovalConflict::YoloMode => YOLO_PERMISSION_CONFLICT,
        }
    }
}

/// Whether the yolo spelling is set: `--yolo` or its alias
/// `--dangerously-skip-permissions`.
pub fn yolo_active(flags: &ParsedFlags) -> bool {
    flags.bool("yolo") || flags.bool("dangerously-skip-permissions")
}

/// Resolves the approval triple. `mode_explicit` is true when
/// `--permission-mode` was written on the command line, not just defaulted.
pub fn resolve_approval(
    auto: bool,
    yolo: bool,
    mode_explicit: bool,
) -> Result<(), ApprovalConflict> {
    if auto && yolo {
        return Err(ApprovalConflict::AutoYolo);
    }
    if auto && mode_explicit {
        return Err(ApprovalConflict::AutoMode);
    }
    if yolo && mode_explicit {
        return Err(ApprovalConflict::YoloMode);
    }
    Ok(())
}

/// Convenience wrapper over a parsed flag set: returns the conflict, if any,
/// for the flags a command registered.
pub fn approval_conflict(flags: &ParsedFlags) -> Option<ApprovalConflict> {
    resolve_approval(
        flags.bool("auto"),
        yolo_active(flags),
        flags.changed("permission-mode"),
    )
    .err()
}
