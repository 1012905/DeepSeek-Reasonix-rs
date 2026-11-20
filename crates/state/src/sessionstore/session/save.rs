//! The session save pipeline (Go: save.go and its helpers —
//! save_fast_baseline.go, save_supersede.go, save_listing_projection.go,
//! the session file lock, the load chain in session_events.go /
//! session_checkpoint_agreement.go / load_context.go, heal from
//! save_recovery_helpers.go, and the CAS conflicts from save_conflict.go).
//!
//! Contract: docs/contracts/session-format.md 7 (SESS-016..019, SESS-026).
//!
//! Save ordering invariant (7.3): the event log mutates FIRST — a crash
//! leaves the newer transcript authoritative. The checkpoint rewrite and
//! the branch-meta revision ledger follow.
//!
//! Deferred to the slices that own them: recovery branches (SESS-020),
//! the v4 import / 1.x DAG load path and sidecar reconciliation
//! (SESS-023), leases (SESS-025), and the
//! display-index / display-read-model sidecars (SESS-028 — the full
//! checkpoint rewrite stands in for the read-model writes, so the
//! durable outcome is identical; only the in-place append optimization
//! is skipped).

use std::borrow::Cow;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use reasonix_contract::gojson;
use reasonix_contract::provider::{Message, normalize_session_messages};
use reasonix_core::fileutil::{create_temp_file, mkdir_all_mode, replace_file};
use thiserror::Error;

use crate::sessionstore::branch::{
    BRANCH_META_COUNTS_VERSION, BranchMetaError, load_branch_meta_retry,
    record_session_content_revision, session_content_revision, update_branch_meta,
};
use crate::sessionstore::eventlog::record::now_utc;
use crate::sessionstore::eventlog::writer_id::digest_string;
use crate::sessionstore::eventlog::{
    DegradedCheckpointError, ExternalizeError, RepairError, ReplayError,
    SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION, SessionEventLogProbe, SessionEventReplay,
    SessionEventWriteError, append_session_append_event, append_session_replace_event,
    compact_session_event_log, composition_removed, holds_any_image, is_data_url,
    probe_session_event_log, refuse_degraded_checkpoint_rewrite, repair_session_event_log_tail,
    replay_session_event_log, session_event_log_oversized, session_event_log_size,
    session_image_key, write_session_event_index, write_session_image_blob,
};
use crate::sessionstore::preview::migrate_legacy_provider_content;
use crate::sessionstore::save_conflict::{
    SessionSnapshotConflictError, SessionSnapshotConflictKind, snapshot_conflict,
    snapshot_conflict_of_kind,
};
use crate::sessionstore::session_lease::{
    SessionWriteAuthoritySave, WriteAuthorityError, observe_unleased_session_write,
};
use crate::sessionstore::session_lock::{SessionFileLockError, lock_session_file};
use crate::sessionstore::sessionpreview::session_preview_from_messages;
use crate::store::naming::{
    session_blobs_dir, session_cleanup_pending, session_event_log, session_event_log_damaged,
    session_meta, session_sidecar_files, session_superseded,
};

use super::baseline::{FileStampOnDisk, PersistedDisk};
use super::canonical::{canonical_session_save_path, with_session_save_path_lock};
use super::digest::{
    digest_and_size_session_messages, digest_session_messages, digest_size_and_prefix,
};
use super::storage_eq::{
    append_covers_persisted_baseline, messages_have_prefix,
    messages_have_prefix_with_compatible_system,
};
use super::{Session, SessionState};

/// The save flavor (Go: sessionSaveMode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionSaveMode {
    /// Save the current snapshot (Go: sessionSaveModeSnapshot).
    Snapshot,
    /// Rewrite the event-log history from the snapshot (Go:
    /// sessionSaveModeRewrite).
    Rewrite,
    /// Rewrite and compact the event log (Go:
    /// sessionSaveModeRewriteCompact).
    RewriteCompact,
}

/// One CAS decision for a save (Go: snapshotWriteDecision).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SnapshotWriteDecision {
    /// The revision the save claims.
    pub(crate) revision: i64,
    /// The snapshot is already durable; only bookkeeping may remain.
    pub(crate) up_to_date: bool,
    /// The snapshot is a pure append onto the on-disk log.
    pub(crate) append_only: bool,
    /// First message index the append starts from.
    pub(crate) append_from: i64,
    /// The event log needs a full rewrite (it is damaged).
    pub(crate) repair_log: bool,
    /// The save supersedes another runtime's transcript (Go:
    /// supersedingDecision; leases are SESS-025/027).
    pub(crate) supersedes: bool,
    /// Content is durable but the branch-meta ledger digest is stale.
    pub(crate) ledger_stale: bool,
}

/// Failures of the transcript checkpoint write (Go: the errors of
/// writeSessionMessages). The image-payload guard runs first: a
/// degraded checkpoint holding the only copy of an image is never
/// rewritten.
#[derive(Debug, Error)]
pub enum WriteMessagesError {
    #[error(transparent)]
    DegradedCheckpoint(#[from] DegradedCheckpointError),
    #[error("create session tmp: {0}")]
    CreateTmp(#[source] io::Error),
    #[error("encode message: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("encode message: {0}")]
    EncodeIo(#[source] io::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Failures while loading a stored session (Go: the load chain's
/// errors, plus the image-blob guards from session_image_blobs.go).
#[derive(Debug, Error)]
pub enum LoadError {
    #[error(transparent)]
    Probe(#[from] io::Error),
    #[error(transparent)]
    Replay(#[from] ReplayError),
    #[error("decode {path}: {source}")]
    Decode {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("unowned session event log reader is not available in this build")]
    UnownedEventLog,
    #[error(
        "session image payload unavailable: this runtime lost images the stored session still holds; reopen it"
    )]
    ImagesLost,
    #[error("session image payload unavailable: {0}")]
    ImagesRestore(#[from] ExternalizeError),
}

/// Failures of a save (Go: the errors saveLocked surfaces).
#[derive(Debug, Error)]
pub enum SaveError {
    #[error("empty session path")]
    EmptyPath,
    #[error("create session dir: {0}")]
    CreateSessionDir(#[source] io::Error),
    #[error("lock session file: {0}")]
    LockSessionFile(#[from] SessionFileLockError),
    #[error("repair session event log: {0}")]
    RepairEventLog(#[from] RepairError),
    #[error(transparent)]
    Conflict(#[from] SessionSnapshotConflictError),
    #[error(transparent)]
    WriteAuthority(#[from] WriteAuthorityError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    EventWrite(#[from] SessionEventWriteError),
    #[error(transparent)]
    Revision(#[from] BranchMetaError),
    #[error(transparent)]
    Probe(#[from] io::Error),
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error("rebuild checkpoint from WAL: {0}")]
    HealFromWal(#[source] WriteMessagesError),
    #[error(transparent)]
    WriteMessages(#[from] WriteMessagesError),
    #[error("file already exists")]
    Exists,
    #[error("session event log is written by Reasonix 1.x: {path}")]
    DagForeign { path: String },
    #[error("session event log for {path} uses schema {version}; this build supports up to {max}")]
    FutureSchema {
        path: String,
        version: i64,
        max: i64,
    },
}

impl Session {
    /// Persists the current snapshot (Go: Save / SaveSnapshot).
    pub fn save(&self, path: &str) -> Result<(), SaveError> {
        self.save_observed(path, SessionSaveMode::Snapshot)
    }

    /// Persists without the up-to-date fast path (Go: SaveRewrite).
    pub fn save_rewrite(&self, path: &str) -> Result<(), SaveError> {
        self.save_observed(path, SessionSaveMode::Rewrite)
    }

    /// Persists and compacts the event log (Go: SaveRewriteCompact).
    pub fn save_rewrite_compact(&self, path: &str) -> Result<(), SaveError> {
        self.save_observed(path, SessionSaveMode::RewriteCompact)
    }

    /// Persists only when nothing exists yet (Go: SaveIfAbsent).
    pub fn save_if_absent(&self, path: &str) -> Result<(), SaveError> {
        Self::with_session_save_locks(path, || {
            if session_artifact_exists(Path::new(path)) {
                return Err(SaveError::Exists);
            }
            self.save_locked(path, SessionSaveMode::Snapshot)
        })?;
        self.notify_persisted(path, false, -1);
        Ok(())
    }

    /// Save plus the persist-observer notification (Go: saveObserved).
    fn save_observed(&self, path: &str, mode: SessionSaveMode) -> Result<(), SaveError> {
        // Go reads the display index to recover append-from (SESS-028);
        // the read model is deferred in this build, so append-from stays -1.
        let append_from = -1i64;
        self.save_with_locks(path, mode)?;
        self.notify_persisted(path, mode != SessionSaveMode::Snapshot, append_from);
        Ok(())
    }

    fn save_with_locks(&self, path: &str, mode: SessionSaveMode) -> Result<(), SaveError> {
        if path.is_empty() {
            return Err(SaveError::EmptyPath);
        }
        Self::with_session_save_locks(path, || self.save_locked(path, mode))
    }

    /// Serializes one save behind the per-path locks (Go:
    /// withSessionSaveLocks): the in-process save-path mutex, the
    /// session directory, and the cross-process session file lock.
    fn with_session_save_locks<R>(
        path: &str,
        f: impl FnOnce() -> Result<R, SaveError>,
    ) -> Result<R, SaveError> {
        if path.trim().is_empty() {
            return Err(SaveError::EmptyPath);
        }
        with_session_save_path_lock(path, || {
            mkdir_all_mode(&parent_dir(Path::new(path)), 0o755)
                .map_err(SaveError::CreateSessionDir)?;
            let _guard = lock_session_file(Path::new(path)).map_err(SaveError::LockSessionFile)?;
            f()
        })
    }

    /// The 14-step save pipeline (Go: saveLocked). The event log mutates
    /// FIRST: a crash between the log append and the checkpoint rewrite
    /// leaves a newer authoritative transcript, never an older one.
    #[allow(clippy::too_many_lines)] // faithful port of Go saveLocked: 14 steps kept inline
    fn save_locked(&self, path: &str, mode: SessionSaveMode) -> Result<(), SaveError> {
        let _authority_save = self.require_write_authority_for_save(path)?;
        // (2) An unleased write is recorded for the write-authority
        // probe (SESS-025); a held lease short-circuits the record.
        observe_unleased_session_write(path, mode);
        // (3) Rebuild a missing or empty checkpoint from the event log.
        heal_empty_checkpoint_from_wal(Path::new(path))?;
        // (4) Fast path: the snapshot matches the last save on disk.
        if mode == SessionSaveMode::Snapshot && self.snapshot_up_to_date(path) {
            return Ok(());
        }
        // (5) The in-memory snapshot and its version bookkeeping.
        let (msgs, version, rewrite_version) = self.snapshot_with_version();
        // (6) Content digest plus the prefix digest the fast baseline armed.
        let (digest, content_bytes, prefix) =
            digest_size_and_prefix(&msgs, self.disk_prefix_len(path))?;
        // (7) The event log must be one this build owns.
        let probe = probe_session_event_log(Path::new(path))?;
        // (8) Refuse future schemas and foreign 1.x DAG logs.
        probe_refuses_save(path, &probe)?;
        // (9) Repair a torn tail before deciding anything on top of it.
        if probe.native && probe.size > 0 {
            repair_session_event_log_tail(Path::new(path))?;
        }
        // (10) The CAS decision against the persisted baseline.
        let owned_rewrite = mode != SessionSaveMode::Snapshot;
        let decision =
            self.snapshot_decision(path, &msgs, digest, prefix, version, owned_rewrite)?;
        // (11) Up to date: heal a stale ledger, never touch the transcript.
        if decision.up_to_date && mode != SessionSaveMode::RewriteCompact {
            if decision.ledger_stale {
                let revision =
                    record_session_content_revision(Path::new(path), &digest, decision.revision)?;
                if let Err(err) = write_session_messages(Path::new(path), &msgs) {
                    tracing::warn!(
                        path = path,
                        error = %err,
                        "session: keeping save after display read-model repair failure"
                    );
                }
                if probe.native {
                    warn_event_index_write(path, &msgs, &digest, revision);
                }
                self.mark_persisted_with_listing(
                    path,
                    &digest,
                    version,
                    revision,
                    rewrite_version,
                    &msgs,
                );
                return Ok(());
            }
            self.mark_persisted_with_listing(
                path,
                &digest,
                version,
                decision.revision,
                rewrite_version,
                &msgs,
            );
            return Ok(());
        }
        // (12) Append-only: extend the event log in place.
        if decision.append_only && probe.native && mode != SessionSaveMode::RewriteCompact {
            let log_size = session_event_log_size(Path::new(path));
            if log_size == 0 {
                append_session_replace_event(
                    Path::new(path),
                    &msgs,
                    &digest,
                    decision.revision,
                    "snapshot",
                )?;
                warn_only_checkpoint_append(path, &msgs);
            } else if session_event_log_oversized(log_size, content_bytes, &msgs) {
                compact_session_event_log(
                    Path::new(path),
                    &msgs,
                    &digest,
                    decision.revision,
                    "compact",
                )?;
                write_session_messages(Path::new(path), &msgs)?;
            } else {
                let tail = &msgs[decision.append_from as usize..];
                append_session_append_event(
                    Path::new(path),
                    decision.append_from,
                    tail,
                    &digest,
                    decision.revision,
                )?;
                warn_only_checkpoint_append(path, &msgs);
            }
            let revision =
                record_session_content_revision(Path::new(path), &digest, decision.revision)?;
            warn_event_index_write(path, &msgs, &digest, revision);
            self.mark_persisted_with_listing(
                path,
                &digest,
                version,
                revision,
                rewrite_version,
                &msgs,
            );
            return Ok(());
        }
        // (13) Full rewrite: replace or compact the log, then the checkpoint.
        let base_revision = decision.revision;
        let repair_log = decision.repair_log;
        let reason = match mode {
            SessionSaveMode::Rewrite => "rewrite",
            SessionSaveMode::RewriteCompact => "rewrite-compact",
            SessionSaveMode::Snapshot => "snapshot",
        };
        let reason = if repair_log { "repair" } else { reason };
        let log_size = session_event_log_size(Path::new(path));
        if probe.native {
            if mode == SessionSaveMode::RewriteCompact
                || repair_log
                || composition_removed(probe.schema_version, &msgs)
                || session_event_log_oversized(log_size, content_bytes, &msgs)
            {
                compact_session_event_log(Path::new(path), &msgs, &digest, base_revision, reason)?;
            } else {
                append_session_replace_event(
                    Path::new(path),
                    &msgs,
                    &digest,
                    base_revision,
                    reason,
                )?;
            }
        }
        keep_superseded_transcript(path, decision.supersedes);
        write_session_messages(Path::new(path), &msgs)?;
        let revision = record_session_content_revision(Path::new(path), &digest, base_revision)?;
        if probe.native {
            warn_event_index_write(path, &msgs, &digest, revision);
        }
        // (14) Mark the baseline and refresh the listing projection.
        self.mark_persisted_with_listing(path, &digest, version, revision, rewrite_version, &msgs);
        Ok(())
    }

    /// The CAS check against the session as loaded from disk (Go:
    /// checkSnapshotWrite). Anything not append-shaped is a conflict
    /// unless this session owns the baseline or the rewrite is owned.
    #[allow(clippy::too_many_lines)] // faithful port of Go checkSnapshotWrite (13 steps)
    fn check_snapshot_write(
        &self,
        path: &str,
        next: &[Message],
        next_digest: [u8; 32],
        next_version: u64,
        allow_owned_rewrite: bool,
    ) -> Result<SnapshotWriteDecision, SaveError> {
        // (1) The on-disk session; a missing baseline is a fresh save.
        let current = match load_snapshot_baseline(Path::new(path), next) {
            Ok(current) => current,
            Err(LoadError::Probe(err)) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(SnapshotWriteDecision::default());
            }
            Err(err) => return Err(err.into()),
        };
        // (2) The branch-meta ledger revision and content digest.
        let (current_revision, current_ledger_digest) = session_content_revision(Path::new(path))?;
        // (3) The baseline this session object claims.
        let base_state = self.persist_state(path);
        // (4) What is actually on disk, plus its raw pre-repair shape.
        let (normalized_dirty, event_log_damaged, raw_messages) = {
            let st = current.read();
            (
                st.normalized_dirty,
                st.event_log_damaged,
                st.raw_messages.clone(),
            )
        };
        let mut existing = current.snapshot();
        let existing_digest = digest_session_messages(&existing)?;
        let raw_differs = normalized_dirty && !raw_messages.is_empty();
        let mut raw = Vec::new();
        let mut raw_digest = existing_digest;
        if raw_differs {
            raw = raw_messages;
            raw_digest = digest_session_messages(&raw)?;
        }
        // (5) The shape of the write relative to the stored transcript.
        let mut content_unchanged = existing_digest == next_digest;
        let mut exact_append = messages_have_prefix(next, &existing);
        let mut append_shaped = content_unchanged
            || exact_append
            || messages_have_prefix_with_compatible_system(next, &existing);
        let mut repair_pending = normalized_dirty;
        // (6) Raw rescue: the load normalized the transcript in memory.
        if !append_shaped && raw_differs {
            let raw_unchanged = raw_digest == next_digest;
            let raw_append = messages_have_prefix(next, &raw);
            if raw_unchanged
                || raw_append
                || messages_have_prefix_with_compatible_system(next, &raw)
            {
                existing = raw.clone();
                content_unchanged = raw_unchanged;
                exact_append = raw_append;
                append_shaped = true;
                repair_pending = false;
            }
        }
        // (7) Owned rescue: this session holds the writable baseline.
        if !append_shaped
            && base_state.ok
            && base_state.revision_known
            && base_state.revision == current_revision
            && !content_unchanged
            && self.owns_writable_baseline(
                path,
                existing_digest,
                raw_digest,
                raw_differs,
                current_revision,
                &current_ledger_digest,
                next_version,
            )
        {
            append_shaped = true;
        }
        // (8) Append-shaped: guard against a foreign newer ledger.
        if append_shaped {
            if base_state.ok
                && base_state.revision_known
                && current_revision != base_state.revision
                && !content_unchanged
                && !append_covers_persisted_baseline(next, &existing, &base_state.digest)
            {
                return Err(SaveError::Conflict(snapshot_conflict(
                    path,
                    &existing,
                    next,
                    base_state.revision,
                    current_revision,
                )));
            }
            let mut decision = SnapshotWriteDecision {
                revision: current_revision,
                up_to_date: content_unchanged && !repair_pending && !event_log_damaged,
                repair_log: event_log_damaged,
                ..SnapshotWriteDecision::default()
            };
            if decision.up_to_date
                && !current_ledger_digest.is_empty()
                && current_ledger_digest != digest_string(&next_digest)
            {
                decision.ledger_stale = true;
            }
            if exact_append
                && !content_unchanged
                && existing.len() < next.len()
                && !event_log_damaged
                && !repair_pending
            {
                decision.append_only = true;
                decision.append_from = existing.len() as i64;
            }
            return Ok(decision);
        }
        // (9) An owned rewrite bypasses the conflict machinery.
        if allow_owned_rewrite
            && self.owns_writable_baseline(
                path,
                existing_digest,
                raw_digest,
                raw_differs,
                current_revision,
                &current_ledger_digest,
                next_version,
            )
        {
            return Ok(SnapshotWriteDecision {
                revision: current_revision,
                repair_log: event_log_damaged,
                ..SnapshotWriteDecision::default()
            });
        }
        // (10) The leaseholder decision (Go: leaseholderDecision): a
        // bound write authority refuses the save with a typed error
        // or hands out a superseding write.
        if let Some(decision) =
            self.leaseholder_decision(path, current_revision, event_log_damaged)?
        {
            return Ok(decision);
        }
        // (11) Still prefix-shaped is a stale snapshot; the rest diverged.
        let prefix_shaped = messages_have_prefix(&existing, next)
            || messages_have_prefix_with_compatible_system(&existing, next)
            || (raw_differs
                && (messages_have_prefix(&raw, next)
                    || messages_have_prefix_with_compatible_system(&raw, next)));
        let kind = if prefix_shaped {
            SessionSnapshotConflictKind::StalePrefix
        } else {
            SessionSnapshotConflictKind::Diverged
        };
        // (12)+(13) Refuse the stale or diverged snapshot.
        Err(SaveError::Conflict(snapshot_conflict_of_kind(
            path,
            kind,
            &existing,
            next,
            base_state.revision,
            current_revision,
        )))
    }

    /// Whether the persisted baseline is owned by this snapshot (Go:
    /// ownsPersistedState): same digest and a matching-or-unknown ledger
    /// revision.
    fn owns_persisted_state(
        &self,
        path: &str,
        existing_digest: [u8; 32],
        existing_revision: i64,
        existing_ledger_digest: &str,
        next_version: u64,
    ) -> bool {
        let state = self.persist_state(path);
        if !state.ok || state.version > next_version || state.digest != existing_digest {
            return false;
        }
        if !state.revision_known || existing_revision == 0 || state.revision == existing_revision {
            return true;
        }
        existing_ledger_digest == digest_string(&existing_digest)
    }

    /// The writable-baseline ownership test (Go: recovery_isolated.go
    /// ownsWritableBaseline): the persisted digest matches, or the raw
    /// pre-repair shape matches, or the generation-bound write
    /// authority (hasValidWriteAuthority) covers the path.
    #[allow(clippy::too_many_arguments)] // faithful port of Go ownsWritableBaseline
    fn owns_writable_baseline(
        &self,
        path: &str,
        existing_digest: [u8; 32],
        raw_digest: [u8; 32],
        raw_differs: bool,
        existing_revision: i64,
        existing_ledger_digest: &str,
        next_version: u64,
    ) -> bool {
        if self.owns_persisted_state(
            path,
            existing_digest,
            existing_revision,
            existing_ledger_digest,
            next_version,
        ) {
            return true;
        }
        if raw_differs
            && self.owns_persisted_state(
                path,
                raw_digest,
                existing_revision,
                existing_ledger_digest,
                next_version,
            )
        {
            return true;
        }
        // Go's third block: a still-valid write authority owns the
        // baseline when the persisted state is otherwise consistent.
        let state = self.persist_state(path);
        if !state.ok || !state.revision_known || state.version > next_version {
            return false;
        }
        if existing_revision != 0 && state.revision != existing_revision {
            return false;
        }
        self.has_valid_write_authority(path)
    }

    /// Open the authority-guarded save window (Go:
    /// `requireWriteAuthorityForSave`): no requirement means no
    /// guard; a missing authority is a typed error.
    pub(crate) fn require_write_authority_for_save(
        &self,
        path: &str,
    ) -> Result<Option<SessionWriteAuthoritySave>, WriteAuthorityError> {
        let (authority, required) = {
            let state = self.read();
            (state.write_auth.clone(), state.auth_required)
        };
        if !required {
            return Ok(None);
        }
        let Some(authority) = authority else {
            return Err(WriteAuthorityError::Missing);
        };
        Ok(Some(authority.begin_save(path)?))
    }

    /// Whether a valid write authority covers `path` (Go:
    /// `hasValidWriteAuthority`).
    fn has_valid_write_authority(&self, path: &str) -> bool {
        self.read()
            .write_auth
            .as_ref()
            .is_some_and(|authority| authority.covers(path))
    }

    /// The typed authority error for `path`, if any (Go:
    /// `authorityErrorForPath`).
    fn authority_error_for_path(&self, path: &str) -> Option<WriteAuthorityError> {
        let (authority, required) = {
            let state = self.read();
            (state.write_auth.clone(), state.auth_required)
        };
        if !required {
            return None;
        }
        let Some(authority) = authority else {
            return Some(WriteAuthorityError::Missing);
        };
        if authority.covers(path) {
            return None;
        }
        if authority.path().trim().is_empty() || authority.generation() == 0 {
            return Some(WriteAuthorityError::Missing);
        }
        Some(WriteAuthorityError::Stale)
    }

    /// The leaseholder decision (Go: save_supersede.go
    /// `leaseholderDecision`): a bound write authority either refuses
    /// the save with a typed error or hands out a superseding write.
    fn leaseholder_decision(
        &self,
        path: &str,
        revision: i64,
        repair_log: bool,
    ) -> Result<Option<SnapshotWriteDecision>, SaveError> {
        if let Some(err) = self.authority_error_for_path(path) {
            return Err(SaveError::WriteAuthority(err));
        }
        if self.has_valid_write_authority(path) {
            return Ok(Some(SnapshotWriteDecision {
                revision,
                repair_log,
                supersedes: true,
                ..SnapshotWriteDecision::default()
            }));
        }
        Ok(None)
    }

    /// The armed fast baseline's answer without touching the disk (Go:
    /// save_fast_baseline.go fastSnapshotDecision).
    fn fast_snapshot_decision(
        &self,
        path: &str,
        next: &[Message],
        prefix: [u8; 32],
    ) -> Option<SnapshotWriteDecision> {
        let st = self.persist_state(path);
        if !st.ok || !st.save_verified || !st.revision_known || !st.disk.armed {
            return None;
        }
        let n = st.disk.n;
        if n > next.len() as i64
            || prefix != st.digest
            || stamp_baseline(Path::new(path)) != st.disk.stamp
        {
            return None;
        }
        if next
            .iter()
            .any(|m| !m.images.is_empty() || !m.unavailable_images.is_empty())
        {
            return None;
        }
        Some(SnapshotWriteDecision {
            revision: st.revision,
            up_to_date: next.len() == n as usize,
            append_only: next.len() > n as usize,
            append_from: n,
            ..SnapshotWriteDecision::default()
        })
    }

    /// The fast baseline first, then the full CAS check (Go:
    /// snapshotDecision).
    fn snapshot_decision(
        &self,
        path: &str,
        next: &[Message],
        next_digest: [u8; 32],
        prefix: [u8; 32],
        next_version: u64,
        allow_owned_rewrite: bool,
    ) -> Result<SnapshotWriteDecision, SaveError> {
        if let Some(decision) = self.fast_snapshot_decision(path, next, prefix) {
            return Ok(decision);
        }
        self.check_snapshot_write(path, next, next_digest, next_version, allow_owned_rewrite)
    }

    /// Arms the fast baseline after a verified save (Go: armDiskBaseline).
    /// The stamp is taken before the state lock, mirroring Go.
    fn arm_disk_baseline(&self, path: &str, digest: &[u8; 32], n: i64) {
        let stamp = stamp_baseline(Path::new(path));
        let mut st = self.write();
        if !st.persisted.ok
            || !st.persisted.save_verified
            || st.persisted.path != canonical_session_save_path(path)
            || &st.persisted.digest != digest
        {
            return;
        }
        st.persisted.disk = PersistedDisk {
            stamp,
            n,
            armed: true,
        };
    }

    /// The armed fast-baseline message count (Go: diskPrefixLen), or -1.
    fn disk_prefix_len(&self, path: &str) -> i64 {
        let st = self.persist_state(path);
        if !st.ok || !st.save_verified || !st.revision_known || !st.disk.armed {
            return -1;
        }
        st.disk.n
    }

    /// Whether a loaded session agrees with the branch-meta ledger (Go:
    /// loadAgreesWithLedger).
    fn load_agrees_with_ledger(
        &self,
        ledger_present: bool,
        content_digest: &str,
        digest: [u8; 32],
    ) -> bool {
        let st = self.read();
        ledger_present
            && !st.normalized_dirty
            && !st.event_log_damaged
            && content_digest.trim() == digest_string(&digest)
    }

    /// Marks the baseline and refreshes the listing projection (Go:
    /// markPersistedWithListing): markPersisted, then the branch-meta
    /// preview/turns counts, then arm the fast baseline.
    fn mark_persisted_with_listing(
        &self,
        path: &str,
        digest: &[u8; 32],
        version: u64,
        revision: i64,
        rewrite_version: i64,
        msgs: &[Message],
    ) {
        self.mark_persisted(path, digest, version, revision, rewrite_version);
        persist_session_listing_projection(Path::new(path), msgs);
        self.arm_disk_baseline(path, digest, msgs.len() as i64);
    }
}

/// Whether any durable artifact of the session exists (Go:
/// sessionArtifactExists): the transcript, a sidecar, or an event log
/// this build recognizes. A probe failure counts as existing.
pub fn session_artifact_exists(session_path: &Path) -> bool {
    if session_path.symlink_metadata().is_ok() {
        return true;
    }
    for sidecar in session_sidecar_files(session_path) {
        if sidecar == session_event_log(session_path) {
            match probe_session_event_log(session_path) {
                // An unreadable log is treated as present.
                Err(_) => return true,
                Ok(probe) => {
                    if !probe.native {
                        continue;
                    }
                }
            }
        }
        if sidecar.symlink_metadata().is_ok() {
            return true;
        }
    }
    false
}

/// Writes the transcript checkpoint (Go: writeSessionMessages): a temp
/// file in the session directory, one HTML-escaped JSON message per
/// line, fsync, then the atomic replace. The image-payload guard runs
/// first: a degraded checkpoint holding the only copy of an image is
/// never rewritten.
pub fn write_session_messages(
    session_path: &Path,
    msgs: &[Message],
) -> Result<(), WriteMessagesError> {
    refuse_degraded_checkpoint_rewrite(session_path, msgs)?;
    let (mut file, tmp_path) = create_temp_file(&parent_dir(session_path), ".session.", ".tmp")
        .map_err(WriteMessagesError::CreateTmp)?;
    for msg in msgs {
        let line = gojson::marshal(msg).map_err(WriteMessagesError::Encode)?;
        if let Err(err) = file
            .write_all(line.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
        {
            let _ = fs::remove_file(&tmp_path);
            return Err(WriteMessagesError::EncodeIo(err));
        }
    }
    if let Err(err) = file.sync_all() {
        let _ = fs::remove_file(&tmp_path);
        return Err(WriteMessagesError::Io(err));
    }
    drop(file);
    if let Err(err) = replace_file(&tmp_path, session_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(WriteMessagesError::Io(err));
    }
    Ok(())
}

/// Keeps the overwritten transcript for the lease audit trail (Go:
/// keepSupersededTranscript). Warn-only: failures never block a save.
fn keep_superseded_transcript(path: &str, superseding: bool) {
    if !superseding || path.is_empty() {
        return;
    }
    let current = match fs::read(path) {
        Ok(current) => current,
        Err(_) => return,
    };
    if current.is_empty() {
        return;
    }
    let dest = session_superseded(Path::new(path));
    if dest.as_os_str().is_empty() {
        return;
    }
    let stamp = serde_json::to_string(&now_utc())
        .unwrap_or_default()
        .trim_matches('"')
        .to_string();
    let header = format!("// superseded at {stamp} by the runtime holding this session's lease\n");
    #[cfg(unix)]
    let mut options = {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut options = OpenOptions::new();
        options.mode(0o600);
        options
    };
    #[cfg(not(unix))]
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    let mut file = match options.open(&dest) {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!(
                path = path,
                error = %err,
                "session: could not keep the superseded transcript"
            );
            return;
        }
    };
    if let Err(err) = file
        .write_all(header.as_bytes())
        .and_then(|()| file.write_all(&current))
    {
        tracing::warn!(
            path = path,
            error = %err,
            "session: could not keep the superseded transcript"
        );
    }
}

/// Loads a session under the save-path lock (Go: LoadSession).
///
/// Not ported: `PrepareSessionPath` (the 1.x v4 import) belongs to the
/// deferred SESS-023 load-path slice; Go's `lockSessionSavePath` maps to
/// `with_session_save_path_lock`, and the session file lock is not taken
/// here — Go's LoadSession does not take it either.
pub fn load_session(path: &str) -> Result<Session, LoadError> {
    with_session_save_path_lock(path, || load_session_unlocked(Path::new(path)))
}

/// Loads a session without taking the file locks (Go:
/// loadSessionUnlocked): replay the event log, normalize, and anchor the
/// persisted baseline against the branch-meta ledger.
pub fn load_session_unlocked(session_path: &Path) -> Result<Session, LoadError> {
    let (msgs, _from_events, damaged) = load_session_messages(session_path)?;
    let normalized = normalize_session_messages(&msgs);
    let normalize_dirty = matches!(normalized, Cow::Owned(_));
    let migrated = migrate_legacy_provider_content(&normalized);
    let dirty = normalize_dirty || matches!(migrated, Cow::Owned(_));
    let messages = migrated.into_owned();
    let session = Session {
        state: RwLock::new(SessionState {
            messages,
            event_log_damaged: damaged,
            normalized_dirty: dirty,
            raw_messages: if dirty { msgs } else { Vec::new() },
            ..SessionState::default()
        }),
    };
    // Bind the digest first: the read guard must drop before the
    // mark_persisted* calls below take the write lock (Rust 2024 keeps
    // if-let scrutinee temporaries alive through the arm).
    let anchored = digest_and_size_session_messages(&session.read().messages).ok();
    if let Some((digest, _content_bytes)) = anchored {
        let path = session_path.to_string_lossy();
        let version = session.transcript_version();
        let rewrite_version = session.rewrite_version();
        match load_branch_meta_retry(session_path) {
            Err(_) => {
                session.mark_persisted_revision_unknown(&path, &digest, version, rewrite_version);
            }
            Ok(meta) => {
                let mut revision = 0;
                if let Some(meta) = &meta {
                    revision = meta.revision;
                }
                let ledger_digest = match &meta {
                    Some(meta) => meta.content_digest.as_str(),
                    None => "",
                };
                if session.load_agrees_with_ledger(meta.is_some(), ledger_digest, digest) {
                    session.mark_persisted(&path, &digest, version, revision, rewrite_version);
                } else {
                    session.mark_persisted_from_load(
                        &path,
                        &digest,
                        version,
                        revision,
                        rewrite_version,
                    );
                }
            }
        }
    }
    Ok(session)
}

/// Loads the stored messages (Go: loadSessionMessages): the event log
/// when this build owns it, the JSONL checkpoint otherwise. Returns the
/// messages, whether they came from the event log, and whether the log
/// replay was damaged.
pub fn load_session_messages(session_path: &Path) -> Result<(Vec<Message>, bool, bool), LoadError> {
    let probe = probe_session_event_log(session_path)?;
    if probe.future_schema || probe.dag {
        // The 1.x DAG reader (SESS-023) is not available in this build.
        return Err(LoadError::UnownedEventLog);
    }
    if probe.native && probe.size > 0 {
        let replay = replay_session_event_log(&session_event_log(session_path))?;
        if replay.records > 0 {
            return resolve_replayed_messages(session_path, replay);
        }
        // Records but none replayed: fall back to the JSONL defensively.
        return load_session_messages_from_jsonl(session_path).map(|msgs| (msgs, false, true));
    }
    load_session_messages_from_jsonl(session_path).map(|msgs| (msgs, false, false))
}

/// Reconciles a damaged replay against the checkpoint (Go:
/// resolveReplayedMessages): a checkpoint that extends the replay heals
/// the log's tail; otherwise the decoded replay wins.
fn resolve_replayed_messages(
    session_path: &Path,
    replay: SessionEventReplay,
) -> Result<(Vec<Message>, bool, bool), LoadError> {
    if replay.damaged
        && let Some(healed) = checkpoint_extending_replay(session_path, &replay.msgs)
    {
        return Ok((healed, false, true));
    }
    Ok((replay.msgs, true, replay.damaged))
}

/// Heals a damaged replay from a longer checkpoint (Go:
/// checkpointExtendingReplay): the checkpoint must strictly extend the
/// replayed prefix.
fn checkpoint_extending_replay(session_path: &Path, replayed: &[Message]) -> Option<Vec<Message>> {
    let msgs = match load_session_messages_from_jsonl(session_path) {
        Ok(msgs) => msgs,
        Err(_) => load_session_messages_from_jsonl_keeping_decoded_tail(session_path),
    };
    if msgs.len() <= replayed.len() {
        return None;
    }
    if !messages_have_prefix(&msgs, replayed)
        && !messages_have_prefix_with_compatible_system(&msgs, replayed)
    {
        return None;
    }
    Some(msgs)
}

/// Strict JSONL load (Go: loadSessionMessagesFromJSONL): any decode
/// error names the file.
fn load_session_messages_from_jsonl(session_path: &Path) -> Result<Vec<Message>, LoadError> {
    let data = fs::read(session_path)?;
    let mut msgs = Vec::new();
    for item in serde_json::Deserializer::from_slice(&data).into_iter::<Message>() {
        match item {
            Ok(msg) => msgs.push(msg),
            Err(source) => {
                return Err(LoadError::Decode {
                    path: session_path.to_string_lossy().into_owned(),
                    source,
                });
            }
        }
    }
    Ok(msgs)
}

/// Lenient JSONL load keeping the decoded prefix (Go:
/// loadSessionMessagesFromJSONLKeepingDecodedTail): a torn final line is
/// dropped; any other decode error gives up entirely.
fn load_session_messages_from_jsonl_keeping_decoded_tail(session_path: &Path) -> Vec<Message> {
    let Ok(data) = fs::read(session_path) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&data);
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.iter().rposition(|line| !line.trim().is_empty());
    let mut msgs = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Message>(line) {
            Ok(msg) => msgs.push(msg),
            Err(_) => {
                if Some(i) != last {
                    return Vec::new();
                }
                return msgs;
            }
        }
    }
    msgs
}

/// Loads the baseline session for a CAS check (Go:
/// loadSnapshotBaseline), refusing saves that would drop the only copy
/// of a stored image and restoring data-URL image blobs the caller
/// lost.
pub fn load_snapshot_baseline(session_path: &Path, next: &[Message]) -> Result<Session, LoadError> {
    let current = load_session_unlocked(session_path)?;
    let stored = current.snapshot();
    for (i, m) in next.iter().enumerate() {
        if !m.unavailable_images.is_empty()
            && i < stored.len()
            && holds_any_image(&stored[i], &m.unavailable_images)
        {
            return Err(LoadError::ImagesLost);
        }
    }
    let dir = session_blobs_dir(session_path);
    let mut restored = false;
    for (i, m) in stored.iter().enumerate() {
        if m.unavailable_images.is_empty() || i >= next.len() {
            continue;
        }
        for url in &next[i].images {
            if !is_data_url(url)
                || !m
                    .unavailable_images
                    .iter()
                    .any(|lost| lost.reference == session_image_key(url))
            {
                continue;
            }
            write_session_image_blob(&dir, url)?;
            restored = true;
        }
    }
    if !restored {
        return Ok(current);
    }
    load_session_unlocked(session_path)
}

/// Rebuilds a missing or empty transcript checkpoint from the event log
/// (Go: healEmptyCheckpointFromWAL). Load failures are swallowed: there
/// is nothing to heal from then.
fn heal_empty_checkpoint_from_wal(session_path: &Path) -> Result<(), SaveError> {
    let need_heal = match fs::metadata(session_path) {
        Ok(info) => info.len() == 0,
        Err(err) => err.kind() == io::ErrorKind::NotFound,
    };
    if !need_heal {
        return Ok(());
    }
    let probe = probe_session_event_log(session_path)?;
    if !probe.native || probe.size <= 0 || probe.future_schema {
        return Ok(());
    }
    let Ok((msgs, from_events, damaged)) = load_session_messages(session_path) else {
        return Ok(());
    };
    if !from_events || damaged || msgs.is_empty() {
        return Ok(());
    }
    write_session_messages(session_path, &msgs).map_err(SaveError::HealFromWal)
}

/// Refuses saves onto logs this build cannot own (Go: probeRefusesSave):
/// future schemas and 1.x DAG logs.
fn probe_refuses_save(path: &str, probe: &SessionEventLogProbe) -> Result<(), SaveError> {
    if probe.future_schema {
        return Err(SaveError::FutureSchema {
            path: path.to_string(),
            version: probe.schema_version,
            max: SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION,
        });
    }
    if probe.dag {
        return Err(SaveError::DagForeign {
            path: path.to_string(),
        });
    }
    Ok(())
}

/// Refreshes the branch-meta listing projection (Go:
/// persistSessionListingProjection). Warn-only: a deferred listing never
/// fails the save.
fn persist_session_listing_projection(session_path: &Path, msgs: &[Message]) {
    let (preview, turns) = session_preview_from_messages(msgs);
    let result = update_branch_meta(session_path, false, |meta| {
        meta.preview = preview;
        meta.turns = turns;
        meta.schema_version = BRANCH_META_COUNTS_VERSION;
        Ok(())
    });
    if let Err(err) = result {
        tracing::warn!(
            path = %session_path.display(),
            error = %err,
            "session: listing metadata update deferred"
        );
    }
}

/// Warn-only event index refresh (Go: writeSessionEventIndex + the
/// slog.Warn "session: keeping save after event index write failure").
fn warn_event_index_write(path: &str, msgs: &[Message], digest: &[u8; 32], revision: i64) {
    if let Err(err) = write_session_event_index(Path::new(path), msgs.len(), digest, revision) {
        tracing::warn!(
            path = path,
            error = %err,
            "session: keeping save after event index write failure"
        );
    }
}

/// Warn-only checkpoint rewrite on the append path (Go: the display
/// read-model append; writeSessionMessages stands in for it, SESS-028).
fn warn_only_checkpoint_append(path: &str, msgs: &[Message]) {
    if let Err(err) = write_session_messages(Path::new(path), msgs) {
        tracing::warn!(
            path = path,
            error = %err,
            "session: keeping save after display read-model append failure"
        );
    }
}

/// The directory of a path (Go: filepath.Dir), "." for bare names.
fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// The four file stamps the fast baseline guards on (Go: stampBaseline):
/// the transcript, the event log, the branch meta, and the damaged flag.
fn stamp_baseline(session_path: &Path) -> [FileStampOnDisk; 4] {
    [
        file_stamp(session_path),
        file_stamp(&session_event_log(session_path)),
        file_stamp(&session_meta(session_path)),
        file_stamp(&session_event_log_damaged(session_path)),
    ]
}

/// One os.Stat stamp (Go: fileStampOnDisk): size, mtime, existence.
fn file_stamp(path: &Path) -> FileStampOnDisk {
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
fn modified_nanos(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// Go save.go `IsCleanupPending`: a delayed-cleanup marker hides a
/// logically removed session from resume/list surfaces until delayed
/// physical cleanup has finished.
pub fn is_cleanup_pending(session_path: &str) -> bool {
    let marker = session_cleanup_pending(Path::new(session_path));
    if marker.as_os_str().is_empty() {
        return false;
    }
    marker.exists()
}

/// Go session_v4_removal.go `isV4SessionRemoved`, pinned false here: a
/// `.v4-removed` marker can only exist for a v4-imported session
/// (SESS-036..038), which this store cannot produce yet.
fn is_v4_session_removed(_session_path: &str) -> bool {
    false
}

/// Go save.go `IsVisibleSession`: whether a persisted session should
/// appear on normal user/agent-facing list, restore, and retrieval
/// surfaces.
pub fn is_visible_session(session_path: &str) -> bool {
    !session_path.trim().is_empty()
        && !is_cleanup_pending(session_path)
        && !is_v4_session_removed(session_path)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    use reasonix_contract::provider::Role;

    use super::*;
    use crate::sessionstore::branch::load_branch_meta;
    use crate::sessionstore::eventlog::read_session_event_index;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::session::RecoveryBranchOptions;
    use crate::sessionstore::session_lease::{
        next_session_write_generation, try_acquire_session_lease,
    };
    use reasonix_contract::provider::ToolCall;
    use std::sync::Arc;

    fn user_message(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn assistant_message(content: &str) -> Message {
        Message {
            role: Role::ASSISTANT,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn session_of(messages: Vec<Message>) -> Session {
        Session {
            state: RwLock::new(SessionState {
                messages,
                ..SessionState::default()
            }),
        }
    }

    fn transcript_lines(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .expect("read transcript")
            .lines()
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect()
    }

    #[test]
    fn save_writes_transcript_log_ledger_and_index() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("hello world")]);

        session.save(&path_str).expect("save");

        let lines = transcript_lines(&path);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"role\":\"user\""));
        let replay = replay_session_event_log(&session_event_log(&path)).expect("replay");
        assert_eq!(replay.msgs.len(), 1);
        let meta = load_branch_meta(&path)
            .expect("branch meta io")
            .expect("branch meta present");
        assert_eq!(meta.revision, 1);
        assert!(!meta.content_digest.is_empty());
        assert_eq!(meta.preview, "hello world");
        assert_eq!(meta.turns, 1);
        assert_eq!(meta.schema_version, BRANCH_META_COUNTS_VERSION);
        let index = read_session_event_index(&path)
            .expect("index io")
            .expect("index present");
        assert_eq!(index.message_count, 1);
        assert_eq!(index.revision, 1);
        assert!(session.snapshot_up_to_date(&path_str));
        let persisted = session
            .persisted_state(&path_str)
            .expect("persisted baseline");
        assert_eq!(persisted.revision, 1);
    }

    #[test]
    fn unchanged_save_is_a_fast_path_no_op() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("steady")]);
        session.save(&path_str).expect("first save");
        let log_before = fs::read(session_event_log(&path)).expect("read log");
        let transcript_before = fs::read(&path).expect("read transcript");

        session.save(&path_str).expect("second save");

        assert_eq!(
            fs::read(session_event_log(&path)).expect("read log"),
            log_before
        );
        assert_eq!(fs::read(&path).expect("read transcript"), transcript_before);
        let meta = load_branch_meta(&path)
            .expect("branch meta io")
            .expect("branch meta present");
        assert_eq!(meta.revision, 1);
    }

    #[test]
    fn append_extends_the_log_and_bumps_the_revision() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("one")]);
        session.save(&path_str).expect("first save");

        session.add(assistant_message("hi there"));
        session.save(&path_str).expect("second save");

        assert_eq!(transcript_lines(&path).len(), 2);
        let replay = replay_session_event_log(&session_event_log(&path)).expect("replay");
        assert_eq!(replay.msgs.len(), 2);
        let meta = load_branch_meta(&path)
            .expect("branch meta io")
            .expect("branch meta present");
        assert_eq!(meta.revision, 2);
        let index = read_session_event_index(&path)
            .expect("index io")
            .expect("index present");
        assert_eq!(index.message_count, 2);
        assert_eq!(index.revision, 2);
    }

    #[test]
    fn fast_baseline_decision_tracks_the_armed_stamp() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("fast")]);
        session.save(&path_str).expect("save");

        assert_eq!(session.disk_prefix_len(&path_str), 1);
        let msgs = session.snapshot();
        let (_, _, prefix) =
            digest_size_and_prefix(&msgs, session.disk_prefix_len(&path_str)).expect("digest");
        let decision = session
            .fast_snapshot_decision(&path_str, &msgs, prefix)
            .expect("fast decision");
        assert!(decision.up_to_date);
        assert!(!decision.append_only);

        File::options()
            .write(true)
            .open(session_meta(&path))
            .expect("open branch meta")
            .set_modified(SystemTime::now() + Duration::from_secs(3600))
            .expect("stamp branch meta mtime");
        assert!(
            session
                .fast_snapshot_decision(&path_str, &msgs, prefix)
                .is_none()
        );
    }

    #[test]
    fn diverged_save_is_refused() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let first = session_of(vec![user_message("hello world")]);
        first.save(&path_str).expect("first save");

        let divergent = session_of(vec![user_message("a different turn")]);
        let err = divergent
            .save(&path_str)
            .expect_err("diverged save refused");
        let conflict = match err {
            SaveError::Conflict(conflict) => conflict,
            other => panic!("expected a snapshot conflict, got: {other}"),
        };
        assert_eq!(conflict.kind, SessionSnapshotConflictKind::Diverged);
        assert_eq!(conflict.existing_messages, 1);
        assert_eq!(conflict.snapshot_messages, 1);
        assert_eq!(conflict.base_revision, 0);
        assert_eq!(conflict.disk_revision, 1);
        assert_eq!(conflict.diverged_at, 0);
        assert_eq!(conflict.disk_role, "user");
        assert_eq!(conflict.snapshot_role, "user");
        assert!(conflict.to_string().contains(
            "diverged on disk (1 messages, revision 1) from snapshot (1 messages, revision 0)"
        ));
    }

    #[test]
    fn stale_prefix_save_is_refused() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let first = session_of(vec![user_message("hello world")]);
        first.save(&path_str).expect("first save");

        let second = session_of(vec![
            user_message("hello world"),
            assistant_message("more one"),
            assistant_message("more two"),
        ]);
        second.save(&path_str).expect("second save");

        first.add(assistant_message("more one"));
        let err = first.save(&path_str).expect_err("stale save refused");
        let conflict = match err {
            SaveError::Conflict(conflict) => conflict,
            other => panic!("expected a snapshot conflict, got: {other}"),
        };
        assert_eq!(conflict.kind, SessionSnapshotConflictKind::StalePrefix);
        assert_eq!(conflict.diverged_at, -1);
        assert!(conflict.to_string().contains(
            "has 3 messages at revision 2; stale snapshot has 2 messages from revision 1"
        ));
    }

    #[test]
    fn future_schema_log_refuses_the_save() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        fs::write(
            session_event_log(&path),
            "{\"schema_version\":99,\"type\":\"replace\"}\n",
        )
        .expect("write future log");

        let session = session_of(vec![user_message("hello")]);
        let err = session.save(&path_str).expect_err("future schema refused");
        assert!(
            err.to_string()
                .contains("uses schema 99; this build supports up to 3")
        );
    }

    #[test]
    fn dag_log_refuses_the_save_as_foreign() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        fs::write(
            session_event_log(&path),
            "{\"schema_version\":2,\"type\":\"replace\"}\n",
        )
        .expect("write dag log");

        let session = session_of(vec![user_message("hello")]);
        let err = session.save(&path_str).expect_err("dag refused");
        assert!(
            err.to_string()
                .contains("session event log is written by Reasonix 1.x")
        );
    }

    #[test]
    fn plain_jsonl_load_anchors_the_baseline() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let line = gojson::marshal(&user_message("plain")).expect("marshal");
        fs::write(&path, format!("{line}\n")).expect("write transcript");

        let session = load_session_unlocked(&path).expect("load");
        assert_eq!(session.snapshot().len(), 1);
        let persisted = session
            .persisted_state(&path_str)
            .expect("persisted baseline");
        assert_eq!(persisted.revision, 0);
        assert!(persisted.revision_known);
        assert!(!session.snapshot_up_to_date(&path_str));

        session.save(&path_str).expect("save adopts the baseline");
        let meta = load_branch_meta(&path)
            .expect("branch meta io")
            .expect("branch meta present");
        assert_eq!(meta.revision, 0);
        assert!(!session_event_log(&path).exists());
    }

    #[test]
    fn heal_rebuilds_an_empty_checkpoint_from_the_log() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("heal me")]);
        session.save(&path_str).expect("save");
        fs::write(&path, b"").expect("truncate transcript");

        let fresh = session_of(vec![user_message("heal me")]);
        fresh.save(&path_str).expect("heal and save");

        assert_eq!(transcript_lines(&path).len(), 1);
    }

    #[test]
    fn write_session_messages_escapes_html_like_go() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");

        write_session_messages(&path, &[user_message("<b>&</b>")]).expect("write");

        let data = fs::read_to_string(&path).expect("read transcript");
        assert!(data.contains("\\u003c"));
        assert!(data.contains("\\u0026"));
        assert!(!data.contains('<'));
    }

    #[test]
    fn jsonl_decode_errors_name_the_path() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        fs::write(&path, b"{\"role\":\"user\",\"content\":\n").expect("write torn line");

        let err =
            load_session_messages_from_jsonl(&path).expect_err("torn line fails strict decode");
        assert!(err.to_string().starts_with("decode "));
        assert!(err.to_string().contains(path.to_string_lossy().as_ref()));

        let good = gojson::marshal(&user_message("kept")).expect("marshal");
        fs::write(&path, format!("{good}\n{{\"role\":\n")).expect("write valid plus torn");
        let tail = load_session_messages_from_jsonl_keeping_decoded_tail(&path);
        assert_eq!(tail.len(), 1);
    }

    #[test]
    fn save_if_absent_creates_then_refuses() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("absent")]);

        session.save_if_absent(&path_str).expect("create");
        assert!(path.exists());

        let err = session.save_if_absent(&path_str).expect_err("refused");
        assert_eq!(err.to_string(), "file already exists");
    }

    #[test]
    fn superseded_transcripts_are_kept_warn_only() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        fs::write(&path, "line-one\n").expect("write transcript");

        keep_superseded_transcript(&path_str, true);

        let kept = fs::read_to_string(session_superseded(&path)).expect("superseded kept");
        assert!(kept.starts_with("// superseded at "));
        assert!(kept.contains(" by the runtime holding this session's lease\n"));
        assert!(kept.ends_with("line-one\n"));

        let other = dir.path().join("other.jsonl");
        let other_str = other.to_string_lossy().into_owned();
        fs::write(&other, "other\n").expect("write transcript");
        keep_superseded_transcript(&other_str, false);
        assert!(!session_superseded(&other).exists());
    }

    #[test]
    fn file_stamps_track_existence_and_modification() {
        let dir = tempdir();
        let missing = dir.path().join("missing.jsonl");
        assert_eq!(file_stamp(&missing), FileStampOnDisk::default());

        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("stamped")]);
        session.save(&path_str).expect("save");

        let stamps = stamp_baseline(&path);
        assert!(stamps[0].here);
        assert!(stamps[1].here);
        assert!(stamps[2].here);
        assert!(!stamps[3].here);
        assert!(stamps[0].size > 0);
    }

    #[test]
    fn session_artifact_exists_scans_the_sidecars() {
        let dir = tempdir();

        let empty = dir.path().join("empty.jsonl");
        assert!(!session_artifact_exists(&empty));

        let meta_only = dir.path().join("meta-only.jsonl");
        fs::write(session_meta(&meta_only), b"{}\n").expect("write branch meta");
        assert!(session_artifact_exists(&meta_only));

        let legacy = dir.path().join("legacy.jsonl");
        fs::write(
            session_event_log(&legacy),
            "{\"schema_version\":0,\"type\":\"legacy\"}\n",
        )
        .expect("write legacy log");
        assert!(!session_artifact_exists(&legacy));

        let native = dir.path().join("native.jsonl");
        fs::write(
            session_event_log(&native),
            "{\"schema_version\":3,\"type\":\"replace\"}\n",
        )
        .expect("write native log");
        assert!(session_artifact_exists(&native));
    }

    #[test]
    fn empty_paths_are_rejected() {
        let session = session_of(vec![user_message("nope")]);

        assert_eq!(
            session.save("").expect_err("empty path").to_string(),
            "empty session path"
        );
        assert_eq!(
            session.save("   ").expect_err("blank path").to_string(),
            "empty session path"
        );
        assert_eq!(
            session
                .save_if_absent(" ")
                .expect_err("blank path")
                .to_string(),
            "empty session path"
        );
    }

    #[test]
    fn rewrite_mode_rewrites_the_log_history() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("first")]);
        session.save(&path_str).expect("first save");

        session.add(assistant_message("second"));
        session.save_rewrite(&path_str).expect("rewrite save");

        let replay = replay_session_event_log(&session_event_log(&path)).expect("replay");
        assert_eq!(replay.msgs.len(), 2);
        let meta = load_branch_meta(&path)
            .expect("branch meta io")
            .expect("branch meta present");
        assert_eq!(meta.revision, 2);
        assert_eq!(transcript_lines(&path).len(), 2);
    }

    #[test]
    fn cleanup_pending_hides_a_session() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        session_of(vec![user_message("hello world")])
            .save(&path_str)
            .expect("save");

        assert!(is_visible_session(&path_str));
        assert!(!is_visible_session(""));
        assert!(!is_visible_session("   "));
        assert!(!is_cleanup_pending(""));

        let marker = crate::store::naming::session_cleanup_pending(&path);
        std::fs::write(&marker, "{}").expect("write cleanup marker");
        assert!(is_cleanup_pending(&path_str));
        assert!(!is_visible_session(&path_str));

        std::fs::remove_file(&marker).expect("remove cleanup marker");
        assert!(!is_cleanup_pending(&path_str));
        assert!(is_visible_session(&path_str));
    }

    const RESHAPED_DIFF: &str = "@@ -1 +1 @@\n-a\n+b\n";

    fn recovery_jsonl(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read session dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.contains("-recovery-")
                    && name.ends_with(".jsonl")
                    && !name.ends_with(".events.jsonl")
            })
            .collect();
        names.sort();
        names
    }

    fn assistant_message_with_call(content: &str, diff: &str) -> Message {
        let mut message = assistant_message(content);
        message.tool_calls.push(ToolCall {
            id: "call_1".to_string(),
            name: "edit".to_string(),
            arguments: r#"{"path":"f"}"#.to_string(),
            diff: diff.to_string(),
            added: if diff.is_empty() { 0 } else { 1 },
            removed: if diff.is_empty() { 0 } else { 1 },
            ..ToolCall::default()
        });
        message
    }

    #[test]
    fn streaming_append_does_not_diverge() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("");
        session.add(user_message("hello"));
        let mut hi = assistant_message("hi");
        hi.reasoning_content = "think".to_string();
        session.add(hi);
        session.save(&path_str).expect("save");
        for i in 0..20 {
            session.add(user_message(&format!("u{i}")));
            let mut call = assistant_message(&format!("a{i}"));
            call.reasoning_content = format!("r{i}");
            call.tool_calls.push(ToolCall {
                id: format!("c{i}"),
                name: "bash".to_string(),
                arguments: r#"{"cmd":"true"}"#.to_string(),
                ..ToolCall::default()
            });
            session.add(call);
            session.add(Message {
                role: Role::TOOL,
                tool_call_id: format!("c{i}"),
                name: "bash".to_string(),
                content: "ok".to_string(),
                work_duration_ms: i64::from(i),
                ..Message::default()
            });
            let mut done = assistant_message(&format!("done{i}"));
            done.reasoning_content = "more".to_string();
            session.add(done);
            session.save(&path_str).expect("save");
        }
        assert!(recovery_jsonl(dir.path()).is_empty());
    }

    #[test]
    fn lease_held_same_revision_allows_reshaped_prefix() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("");
        session.add(user_message("edit"));
        session.add(assistant_message_with_call("editing", ""));
        session.save(&path_str).expect("save");

        let lease = try_acquire_session_lease(&path_str).expect("lease");
        let authority = lease
            .issue_write_authority(next_session_write_generation())
            .expect("authority");
        session.bind_write_authority(Some(Arc::new(authority.clone())));

        let foreign = vec![
            user_message("edit"),
            assistant_message_with_call("editing", RESHAPED_DIFF),
        ];
        let foreign_digest = digest_session_messages(&foreign).expect("digest");
        let (revision, _) = session_content_revision(&path).expect("revision");
        append_session_replace_event(&path, &foreign, &foreign_digest, revision, "snapshot")
            .expect("append foreign replace");
        write_session_messages(&path, &foreign).expect("reshape disk");

        assert!(session.update_tool_call_preview(&ToolCall {
            id: "call_1".to_string(),
            diff: RESHAPED_DIFF.to_string(),
            added: 1,
            removed: 1,
            ..ToolCall::default()
        }));
        session.add(Message {
            role: Role::TOOL,
            tool_call_id: "call_1".to_string(),
            name: "edit".to_string(),
            content: "ok".to_string(),
            ..Message::default()
        });
        let mut done = assistant_message("done");
        done.reasoning_content = "r".to_string();
        session.add(done);
        session.save(&path_str).expect("save under authority");
        assert!(recovery_jsonl(dir.path()).is_empty());

        for i in 0..10 {
            session.add(user_message(&format!("n{i}")));
            session.add(assistant_message(&format!("r{i}")));
            session.save(&path_str).expect("save");
        }
        assert!(recovery_jsonl(dir.path()).is_empty());
    }

    #[test]
    fn chain_after_recovery_fork_does_not_cascade() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("");
        session.add(user_message("start"));
        session.add(assistant_message("ok"));
        session.save(&path_str).expect("save");
        session.add(user_message("unsaved"));
        session.add(assistant_message("local only tail"));
        let info = session
            .save_recovery_branch(RecoveryBranchOptions {
                original_path: path_str.clone(),
                reason: "snapshot conflict".to_string(),
                ..RecoveryBranchOptions::default()
            })
            .expect("recovery branch");
        let branch_str = info.path.to_string_lossy().into_owned();
        for i in 0..15 {
            session.add(user_message(&format!("u{i}")));
            let mut assistant = assistant_message(&format!("a{i}"));
            assistant.reasoning_content = "x".to_string();
            session.add(assistant);
            session.save(&branch_str).expect("save branch");
        }
        assert_eq!(recovery_jsonl(dir.path()).len(), 1);
    }

    #[test]
    fn rejects_interrupted_foreign_write_without_lease() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("");
        session.add(user_message("base"));
        session.save(&path_str).expect("save");

        let stale = load_session(&path_str).expect("reload");
        let (revision, _) = session_content_revision(&path).expect("revision");
        let mut foreign = stale.snapshot();
        foreign.push(assistant_message("foreign writer tail"));
        let foreign_digest = digest_session_messages(&foreign).expect("digest");
        append_session_replace_event(&path, &foreign, &foreign_digest, revision, "snapshot")
            .expect("append foreign replace");
        write_session_messages(&path, &foreign).expect("foreign tail");

        stale.add(assistant_message("stale writer tail"));
        let err = stale.save(&path_str).expect_err("stale writer refused");
        assert!(matches!(
            &err,
            SaveError::Conflict(conflict) if conflict.kind == SessionSnapshotConflictKind::Diverged
        ));
    }

    #[test]
    fn authority_missing_returns_typed_error() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("");
        session.add(user_message("u"));
        session.add(assistant_message("a"));
        session.save(&path_str).expect("save");
        session.require_write_authority();
        session.clear_write_authority();
        session.add(user_message("more"));
        let err = session.save(&path_str).expect_err("missing authority");
        assert!(matches!(
            err,
            SaveError::WriteAuthority(WriteAuthorityError::Missing)
        ));
        assert!(recovery_jsonl(dir.path()).is_empty());
    }

    #[test]
    fn stale_authority_refused() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("");
        session.add(user_message("u"));
        session.add(assistant_message("a"));
        session.save(&path_str).expect("save");

        let lease = try_acquire_session_lease(&path_str).expect("lease");
        let old_authority = lease
            .issue_write_authority(next_session_write_generation())
            .expect("old authority");
        session.bind_write_authority(Some(Arc::new(old_authority.clone())));
        let new_authority = lease
            .issue_write_authority(next_session_write_generation())
            .expect("new authority");
        assert!(new_authority.valid());

        session.add(user_message("more"));
        session.bind_write_authority(Some(Arc::new(old_authority)));
        let err = session.save(&path_str).expect_err("stale authority");
        assert!(matches!(
            err,
            SaveError::WriteAuthority(WriteAuthorityError::Stale)
        ));
        assert!(recovery_jsonl(dir.path()).is_empty());
    }

    #[test]
    fn zero_byte_checkpoint_heals_from_wal() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("");
        session.add(user_message("u0"));
        session.add(assistant_message("a0"));
        session.save(&path_str).expect("save");

        let lease = try_acquire_session_lease(&path_str).expect("lease");
        let authority = lease
            .issue_write_authority(next_session_write_generation())
            .expect("authority");
        session.bind_write_authority(Some(Arc::new(authority.clone())));

        fs::write(&path, b"").expect("truncate checkpoint");
        for i in 1..=100 {
            session.add(user_message(&format!("u{i}")));
            session.add(assistant_message(&format!("a{i}")));
            session.save(&path_str).expect("autosave");
        }
        assert!(recovery_jsonl(dir.path()).is_empty());
        assert!(fs::metadata(&path).expect("stat checkpoint").len() > 0);
    }

    #[test]
    fn lease_held_without_baseline_rewrites_instead_of_forking() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let base = Session::new("sys");
        base.add(user_message("ask"));
        base.add(assistant_message_with_call("answering", ""));
        base.save(&path_str).expect("save base");

        let rebuilt = Session::new("sys");
        rebuilt.add(user_message("ask"));
        rebuilt.add(assistant_message_with_call("answering", RESHAPED_DIFF));

        let lease = try_acquire_session_lease(&path_str).expect("lease");
        let authority = lease
            .issue_write_authority(next_session_write_generation())
            .expect("authority");
        rebuilt.bind_write_authority(Some(Arc::new(authority.clone())));

        rebuilt.save(&path_str).expect("rewrite under authority");

        let back = load_session(&path_str).expect("reload");
        let got = back.snapshot();
        assert!(got.len() >= 3);
        assert!(!got[2].tool_calls.is_empty());
        assert_ne!(got[2].tool_calls[0].diff, "");
        let (raw, _from_events, _damaged) = load_session_messages(&path).expect("raw load");
        let (_, ledger) = session_content_revision(&path).expect("ledger");
        let digest = digest_session_messages(&raw).expect("digest");
        assert_eq!(ledger, digest_string(&digest));
    }

    #[test]
    fn without_lease_still_refuses_a_reshape() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let base = Session::new("sys");
        base.add(user_message("ask"));
        base.add(assistant_message_with_call("answering", ""));
        base.save(&path_str).expect("save base");

        let rebuilt = Session::new("sys");
        rebuilt.add(user_message("ask"));
        rebuilt.add(assistant_message_with_call("answering", RESHAPED_DIFF));

        let err = rebuilt.save(&path_str).expect_err("reshape refused");
        assert!(matches!(
            &err,
            SaveError::Conflict(conflict) if conflict.kind == SessionSnapshotConflictKind::Diverged
        ));
    }
}
