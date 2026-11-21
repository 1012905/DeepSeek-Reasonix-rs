//! Persistence baseline (Go: save.go `sessionPersistState` and
//! the mark/persist methods on `*Session`).
//!
//! The baseline remembers what the last save put on disk: canonical
//! path, content digest, version, revision, and rewrite epoch. It
//! decides whether a snapshot can skip saving, feeds `PersistedState`
//! views, and gates observer notifications.

use std::fs;
use std::path::Path;

use super::Session;
use super::canonical::canonical_session_save_path;
use crate::sessionstore::eventlog::writer_id::digest_string;
use crate::store::naming::{session_event_log, session_event_log_damaged, session_meta};

/// In-memory record of the last persisted snapshot (Go:
/// `sessionPersistState`). `ok` distinguishes "never saved"
/// from "saved with revision 0".
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionPersistState {
    pub(crate) path: String,
    pub(crate) digest: [u8; 32],
    pub(crate) version: u64,
    pub(crate) revision: i64,
    pub(crate) revision_known: bool,
    pub(crate) save_verified: bool,
    pub(crate) ok: bool,
    pub(crate) disk: PersistedDisk,
}

/// One persisted-file stamp (Go: `fileStampOnDisk`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FileStampOnDisk {
    pub(crate) size: i64,
    pub(crate) modified_nanos: i64,
    pub(crate) here: bool,
}

/// Stamps of the sidecar files the fast baseline verified (Go:
/// `persistedDisk`); the stamps ride along so a later save can
/// detect concurrent writers.
#[derive(Debug, Clone, Default)]
pub(crate) struct PersistedDisk {
    pub(crate) stamp: [FileStampOnDisk; 4],
    pub(crate) n: i64,
    pub(crate) armed: bool,
}

/// Public view of the persistence baseline (Go: `PersistedState`
/// struct and the method of the same name).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedState {
    pub digest: [u8; 32],
    pub digest_hex: String,
    pub revision: i64,
    pub revision_known: bool,
    pub rewrite_epoch: i64,
    pub append_only_tail: bool,
    pub unchanged_since_persisted: bool,
}

impl Session {
    /// Whether the current in-memory state already matches the last
    /// save end to end (Go: `snapshotUpToDate`). A session that
    /// only gained rewrite-epoch metadata (compaction and friends) is
    /// still up to date when the persisted rewrite epoch caught up.
    pub fn snapshot_up_to_date(&self, path: &str) -> bool {
        let key = canonical_session_save_path(path);
        let st = self.read();
        st.persisted.ok
            && st.persisted.save_verified
            && st.persisted.path == key
            && st.persisted.version == st.version
            && st.persisted.revision_known
            && st.rewrite_version == st.persisted_rewrite_version
            && !st.normalized_dirty
            && !st.event_log_damaged
    }

    /// Clones the baseline for a canonical path, or a default (never
    /// saved) baseline when the key does not match (Go: the persisted
    /// lookup pattern in save.go).
    pub(crate) fn persist_state(&self, path: &str) -> SessionPersistState {
        let key = canonical_session_save_path(path);
        let st = self.read();
        if st.persisted.ok && st.persisted.path == key {
            st.persisted.clone()
        } else {
            SessionPersistState::default()
        }
    }

    /// Public snapshot of what was last persisted for `path` (Go:
    /// `PersistedState`); `None` when never saved there.
    pub fn persisted_state(&self, path: &str) -> Option<PersistedState> {
        let persisted = self.persist_state(path);
        if !persisted.ok {
            return None;
        }
        let st = self.read();
        Some(PersistedState {
            digest: persisted.digest,
            digest_hex: digest_string(&persisted.digest),
            revision: persisted.revision,
            revision_known: persisted.revision_known,
            rewrite_epoch: st.persisted_rewrite_version,
            append_only_tail: st.rewrite_version == st.persisted_rewrite_version,
            unchanged_since_persisted: persisted.version == st.version,
        })
    }

    /// Marks the in-memory state as persisted by a verified save (Go:
    /// `markPersisted`).
    pub fn mark_persisted(
        &self,
        path: &str,
        digest: &[u8; 32],
        version: u64,
        revision: i64,
        rewrite_version: i64,
    ) {
        self.set_persisted_baseline(path, digest, version, revision, true, true, rewrite_version);
    }

    /// Marks the baseline as loaded from disk; the snapshot has NOT been
    /// verified by a save, so dirty flags survive (Go:
    /// `markPersistedFromLoad`).
    pub fn mark_persisted_from_load(
        &self,
        path: &str,
        digest: &[u8; 32],
        version: u64,
        revision: i64,
        rewrite_version: i64,
    ) {
        self.set_persisted_baseline(
            path,
            digest,
            version,
            revision,
            true,
            false,
            rewrite_version,
        );
    }

    /// Marks the baseline as persisted with an unknown revision (Go:
    /// `markPersistedRevisionUnknown`).
    pub fn mark_persisted_revision_unknown(
        &self,
        path: &str,
        digest: &[u8; 32],
        version: u64,
        rewrite_version: i64,
    ) {
        self.set_persisted_baseline(path, digest, version, 0, false, false, rewrite_version);
    }

    /// Baseline writer shared by the public marks (Go:
    /// `setPersistedBaseline`). The rewrite epoch only moves
    /// forward; a verified save clears the dirty/rerun state while a
    /// loaded baseline leaves it alone.
    #[allow(clippy::too_many_arguments)] // mirrors Go setPersistedBaseline(path, digest, version, revision, revisionKnown, saveVerified, rewriteVersion)
    pub(crate) fn set_persisted_baseline(
        &self,
        path: &str,
        digest: &[u8; 32],
        version: u64,
        revision: i64,
        revision_known: bool,
        save_verified: bool,
        rewrite_version: i64,
    ) {
        let key = canonical_session_save_path(path);
        let mut st = self.write();
        st.persisted = SessionPersistState {
            path: key,
            digest: *digest,
            version,
            revision,
            revision_known,
            save_verified,
            ok: true,
            disk: PersistedDisk::default(),
        };
        if rewrite_version > st.persisted_rewrite_version {
            st.persisted_rewrite_version = rewrite_version;
        }
        if save_verified {
            st.normalized_dirty = false;
            st.raw_messages = Vec::new();
            st.event_log_damaged = false;
        }
    }
}

/// The four file stamps the fast baseline guards on (Go: stampBaseline):
/// the transcript, the event log, the branch meta, and the damaged flag.
pub(crate) fn stamp_baseline(session_path: &Path) -> [FileStampOnDisk; 4] {
    [
        file_stamp(session_path),
        file_stamp(&session_event_log(session_path)),
        file_stamp(&session_meta(session_path)),
        file_stamp(&session_event_log_damaged(session_path)),
    ]
}

/// One os.Stat stamp (Go: fileStampOnDisk): size, mtime, existence.
pub(crate) fn file_stamp(path: &Path) -> FileStampOnDisk {
    match fs::metadata(path) {
        Ok(meta) => FileStampOnDisk {
            size: meta.len() as i64,
            modified_nanos: modified_nanos(&meta),
            here: true,
        },
        Err(_) => FileStampOnDisk::default(),
    }
}

/// The mtime in nanoseconds since the epoch, 0 when unavailable.
pub(crate) fn modified_nanos(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::provider::{Message, Role};

    fn msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            ..Default::default()
        }
    }

    const D: [u8; 32] = [7u8; 32];

    #[test]
    fn snapshot_up_to_date_matrix() {
        let s = Session::new("");
        s.add(msg(Role::USER, "hi"));
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir.path().join("s.jsonl");

        // Never saved -> not up to date.
        assert!(!s.snapshot_up_to_date(&p.to_string_lossy()));

        // Mark from load: revision known but save not verified.
        s.mark_persisted_from_load(&p.to_string_lossy(), &D, s.transcript_version(), 5, 0);
        assert!(!s.snapshot_up_to_date(&p.to_string_lossy()));

        // Verified save with matching version and rewrite epoch.
        s.mark_persisted(&p.to_string_lossy(), &D, s.transcript_version(), 6, 0);
        assert!(s.snapshot_up_to_date(&p.to_string_lossy()));

        // A different path is never up to date.
        assert!(!s.snapshot_up_to_date(&dir.path().join("other.jsonl").to_string_lossy()));

        // Dirty state breaks it again.
        s.write().normalized_dirty = true;
        assert!(!s.snapshot_up_to_date(&p.to_string_lossy()));
        s.write().normalized_dirty = false;

        // An advanced in-memory rewrite epoch breaks it.
        s.increment_rewrite();
        assert!(!s.snapshot_up_to_date(&p.to_string_lossy()));
    }

    #[test]
    fn persisted_state_view() {
        let s = Session::new("");
        s.add(msg(Role::USER, "one"));
        s.add(msg(Role::ASSISTANT, "two"));
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir.path().join("s.jsonl");
        assert!(s.persisted_state(&p.to_string_lossy()).is_none());

        s.mark_persisted(&p.to_string_lossy(), &D, s.transcript_version(), 4, 0);
        let view = s.persisted_state(&p.to_string_lossy()).unwrap();
        assert_eq!(view.digest, D);
        assert_eq!(view.digest_hex, digest_string(&D));
        assert_eq!(view.revision, 4);
        assert!(view.revision_known);
        assert_eq!(view.rewrite_epoch, 0);
        assert!(view.append_only_tail);
        assert!(view.unchanged_since_persisted);

        // Moving the rewrite epoch forward flips append-only tail off.
        s.increment_rewrite();
        let view = s.persisted_state(&p.to_string_lossy()).unwrap();
        assert_eq!(view.rewrite_epoch, 0);
        assert!(!view.append_only_tail);

        // A trailing edit flips unchanged_since_persisted off.
        s.add(msg(Role::USER, "three"));
        let view = s.persisted_state(&p.to_string_lossy()).unwrap();
        assert!(!view.unchanged_since_persisted);
    }

    #[test]
    fn mark_variants_clear_or_keep_dirty() {
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir.path().join("s.jsonl").to_string_lossy().into_owned();

        let from_load = Session::new("");
        from_load.add(msg(Role::USER, "x"));
        from_load.write().normalized_dirty = true;
        from_load.write().event_log_damaged = true;
        from_load.mark_persisted_from_load(&p, &D, from_load.transcript_version(), 1, 0);
        assert!(from_load.read().normalized_dirty);
        assert!(from_load.read().event_log_damaged);

        let verified = Session::new("");
        verified.add(msg(Role::USER, "x"));
        verified.write().normalized_dirty = true;
        verified.write().event_log_damaged = true;
        verified.mark_persisted(&p, &D, verified.transcript_version(), 2, 0);
        assert!(!verified.read().normalized_dirty);
        assert!(!verified.read().event_log_damaged);

        let unknown = Session::new("");
        unknown.add(msg(Role::USER, "x"));
        unknown.mark_persisted_revision_unknown(&p, &D, unknown.transcript_version(), 0);
        let view = unknown.persisted_state(&p).unwrap();
        assert_eq!(view.revision, 0);
        assert!(!view.revision_known);
        assert!(!unknown.snapshot_up_to_date(&p));
    }

    #[test]
    fn rewrite_epoch_moves_forward_only() {
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir.path().join("s.jsonl").to_string_lossy().into_owned();
        let s = Session::new("");
        s.mark_persisted(&p, &D, 1, 1, 5);
        assert_eq!(s.read().persisted_rewrite_version, 5);
        // A load from an older file must not drag the epoch back.
        s.mark_persisted_from_load(&p, &D, 1, 2, 3);
        assert_eq!(s.read().persisted_rewrite_version, 5);
        // Forward moves still land.
        s.mark_persisted(&p, &D, 1, 3, 9);
        assert_eq!(s.read().persisted_rewrite_version, 9);
    }
}
