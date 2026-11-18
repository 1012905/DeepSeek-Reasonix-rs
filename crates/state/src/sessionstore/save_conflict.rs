//! Session snapshot conflict errors (Go: `save_conflict.go`).
//!
//! A save is refused with a conflict error when the on-disk transcript
//! moved past the snapshot the saving runtime holds. The error chains the
//! `ERR_SESSION_SNAPSHOT_CONFLICT` sentinel (Go:
//! `errors.Is` against `ErrSessionSnapshotConflict`).

use std::fmt;

use reasonix_contract::provider::Message;

use super::session::storage_eq::{
    messages_equal_for_storage, messages_have_prefix, messages_have_prefix_with_compatible_system,
};

/// The sentinel every snapshot conflict wraps (Go:
/// `ErrSessionSnapshotConflict`).
pub const ERR_SESSION_SNAPSHOT_CONFLICT: &str = "session snapshot conflicts with newer transcript";

/// How the on-disk transcript relates to the snapshot (Go:
/// `SessionSnapshotConflictKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSnapshotConflictKind {
    /// The disk transcript extends the snapshot (Go: `stale_prefix`).
    StalePrefix,
    /// Disk and snapshot share no storage relationship (Go:
    /// `diverged`).
    Diverged,
}

impl SessionSnapshotConflictKind {
    /// The wire name of the kind (Go: the kind string).
    pub fn as_str(self) -> &'static str {
        match self {
            SessionSnapshotConflictKind::StalePrefix => "stale_prefix",
            SessionSnapshotConflictKind::Diverged => "diverged",
        }
    }
}

/// A save refused because the on-disk transcript moved past the snapshot
/// (Go: `SessionSnapshotConflictError`).
///
/// Display texts match the Go implementation byte for byte because they
/// surface in user-visible diagnostics. No message content ever leaves
/// the diagnostic: only counts, revisions, and roles do.
#[derive(Debug, Clone)]
pub struct SessionSnapshotConflictError {
    /// Session path the save targeted.
    pub path: String,
    /// The detected relationship.
    pub kind: SessionSnapshotConflictKind,
    /// Messages in the on-disk transcript.
    pub existing_messages: usize,
    /// Messages in the snapshot being saved.
    pub snapshot_messages: usize,
    /// Revision the saving runtime last saw (Go: `BaseRevision`).
    pub base_revision: i64,
    /// Revision the disk transcript is at (Go: `DiskRevision`).
    pub disk_revision: i64,
    /// First index where storage diverged, `-1` when one side is a
    /// prefix of the other (Go: `DivergedAt`).
    pub diverged_at: i64,
    /// Role of the disk message at `diverged_at` (empty when
    /// `-1`).
    pub disk_role: String,
    /// Role of the snapshot message at `diverged_at` (empty when
    /// `-1`).
    pub snapshot_role: String,
}

impl SessionSnapshotConflictError {
    fn from_parts(
        path: &str,
        kind: SessionSnapshotConflictKind,
        existing: &[Message],
        snapshot: &[Message],
        base_revision: i64,
        disk_revision: i64,
    ) -> Self {
        let (diverged_at, disk_role, snapshot_role) = first_storage_divergence(existing, snapshot);
        Self {
            path: path.to_string(),
            kind,
            existing_messages: existing.len(),
            snapshot_messages: snapshot.len(),
            base_revision,
            disk_revision,
            diverged_at,
            disk_role,
            snapshot_role,
        }
    }
}

impl fmt::Display for SessionSnapshotConflictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            SessionSnapshotConflictKind::StalePrefix => write!(
                f,
                "{ERR_SESSION_SNAPSHOT_CONFLICT}: {} has {} messages at revision {}; \
                 stale snapshot has {} messages from revision {}",
                self.path,
                self.existing_messages,
                self.disk_revision,
                self.snapshot_messages,
                self.base_revision
            ),
            SessionSnapshotConflictKind::Diverged => write!(
                f,
                "{ERR_SESSION_SNAPSHOT_CONFLICT}: {} diverged on disk \
                 ({} messages, revision {}) from snapshot ({} messages, revision {})",
                self.path,
                self.existing_messages,
                self.disk_revision,
                self.snapshot_messages,
                self.base_revision
            ),
        }
    }
}

impl std::error::Error for SessionSnapshotConflictError {}

/// Builds a conflict error, picking the kind from the storage
/// relationship: a prefix relationship reports `StalePrefix`,
/// anything else `Diverged` (Go: `snapshotConflict`).
pub fn snapshot_conflict(
    path: &str,
    existing: &[Message],
    next: &[Message],
    base_revision: i64,
    disk_revision: i64,
) -> SessionSnapshotConflictError {
    let kind = if messages_have_prefix(existing, next)
        || messages_have_prefix_with_compatible_system(existing, next)
    {
        SessionSnapshotConflictKind::StalePrefix
    } else {
        SessionSnapshotConflictKind::Diverged
    };
    snapshot_conflict_of_kind(path, kind, existing, next, base_revision, disk_revision)
}

/// Builds a conflict error with a caller-picked kind (Go:
/// `snapshotConflictOfKind`).
pub fn snapshot_conflict_of_kind(
    path: &str,
    kind: SessionSnapshotConflictKind,
    existing: &[Message],
    next: &[Message],
    base_revision: i64,
    disk_revision: i64,
) -> SessionSnapshotConflictError {
    SessionSnapshotConflictError::from_parts(
        path,
        kind,
        existing,
        next,
        base_revision,
        disk_revision,
    )
}

/// Finds the first index where disk and snapshot disagree in storage,
/// with the roles of both messages there (Go:
/// `firstStorageDivergence`). Returns `(-1, "", "")` when
/// one side is a prefix of the other. Message content never leaves this
/// comparison.
pub fn first_storage_divergence(disk: &[Message], snapshot: &[Message]) -> (i64, String, String) {
    let shared = disk.len().min(snapshot.len());
    for i in 0..shared {
        if !messages_equal_for_storage(&disk[i], &snapshot[i]) {
            return (
                i as i64,
                disk[i].role.as_str().to_string(),
                snapshot[i].role.as_str().to_string(),
            );
        }
    }
    (-1, String::new(), String::new())
}

#[cfg(test)]
mod tests {
    use reasonix_contract::provider::{Message, Role};

    use super::*;

    fn msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            ..Message::default()
        }
    }

    #[test]
    fn stale_prefix_display_matches_go() {
        let existing = vec![msg(Role::USER, "a"), msg(Role::ASSISTANT, "b")];
        let next = vec![msg(Role::USER, "a")];
        let err = snapshot_conflict("/s/x.jsonl", &existing, &next, 3, 9);
        assert_eq!(err.kind, SessionSnapshotConflictKind::StalePrefix);
        assert_eq!(
            err.to_string(),
            "session snapshot conflicts with newer transcript: /s/x.jsonl has 2 \
             messages at revision 9; stale snapshot has 1 messages from revision 3"
        );
    }

    #[test]
    fn diverged_display_matches_go() {
        let existing = vec![msg(Role::USER, "a"), msg(Role::ASSISTANT, "x")];
        let next = vec![msg(Role::USER, "a"), msg(Role::ASSISTANT, "y")];
        let err = snapshot_conflict("/s/y.jsonl", &existing, &next, 5, 12);
        assert_eq!(err.kind, SessionSnapshotConflictKind::Diverged);
        assert_eq!(
            err.to_string(),
            "session snapshot conflicts with newer transcript: /s/y.jsonl diverged on \
             disk (2 messages, revision 12) from snapshot (2 messages, revision 5)"
        );
    }

    #[test]
    fn conflict_of_kind_overrides_detection() {
        let existing = vec![msg(Role::USER, "a")];
        let next = vec![msg(Role::USER, "a"), msg(Role::ASSISTANT, "b")];
        // Disk extends the snapshot: auto-detection says stale_prefix,
        // but OfKind pins the kind.
        let err = snapshot_conflict_of_kind(
            "/s/z.jsonl",
            SessionSnapshotConflictKind::Diverged,
            &existing,
            &next,
            1,
            2,
        );
        assert_eq!(err.kind, SessionSnapshotConflictKind::Diverged);
        assert!(err.to_string().contains("diverged on disk"));
    }

    #[test]
    fn divergence_reports_index_and_roles() {
        let disk = vec![msg(Role::USER, "a"), msg(Role::ASSISTANT, "x")];
        let snapshot = vec![msg(Role::USER, "a"), msg(Role::TOOL, "y")];
        let (at, disk_role, snapshot_role) = first_storage_divergence(&disk, &snapshot);
        assert_eq!(at, 1);
        assert_eq!(disk_role, "assistant");
        assert_eq!(snapshot_role, "tool");
    }

    #[test]
    fn divergence_of_prefix_is_minus_one() {
        let disk = vec![msg(Role::USER, "a")];
        let snapshot = vec![msg(Role::USER, "a"), msg(Role::ASSISTANT, "b")];
        let (at, disk_role, snapshot_role) = first_storage_divergence(&disk, &snapshot);
        assert_eq!(at, -1);
        assert_eq!(disk_role, "");
        assert_eq!(snapshot_role, "");
    }

    #[test]
    fn kind_wire_names_match_go() {
        assert_eq!(
            SessionSnapshotConflictKind::StalePrefix.as_str(),
            "stale_prefix"
        );
        assert_eq!(SessionSnapshotConflictKind::Diverged.as_str(), "diverged");
    }
}
