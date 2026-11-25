//! The session save pipeline (Go: save.go and its helpers —
//! save_fast_baseline.go, save_supersede.go, and the CAS conflicts from
//! save_conflict.go). The load chain lives in `load.rs`, the checkpoint
//! writes in `checkpoint.rs`, and the derived sidecars in
//! `display_index.rs` / `display_read_model.rs`.
//!
//! Contract: docs/contracts/session-format.md 7 (SESS-016..019, SESS-026).
//!
//! Save ordering invariant (7.3): the event log mutates FIRST — a crash
//! leaves the newer transcript authoritative. The checkpoint rewrite and
//! the branch-meta revision ledger follow.
//!
//! The 1.x DAG guard and sessions-v4 import run before the native save path;
//! overlong-name sidecar reconciliation lives in `cleanup_pending.rs`.

use std::io;
use std::path::{Path, PathBuf};

use reasonix_contract::provider::Message;
use reasonix_core::fileutil::mkdir_all_mode;
use thiserror::Error;

use crate::sessionstore::branch::{
    BranchMetaError, record_session_content_revision, session_content_revision,
};
use crate::sessionstore::dag::DagError;
use crate::sessionstore::display_index::{
    load_session_display_index, refresh_session_display_index,
};
use crate::sessionstore::display_read_model::{
    publish_append_only_save, warn_display_index_write, warn_event_index_write,
};
use crate::sessionstore::eventlog::writer_id::digest_string;
use crate::sessionstore::eventlog::{
    DegradedCheckpointError, ExternalizeError, RepairError, ReplayError, SessionEventWriteError,
    append_session_replace_event, compact_session_event_log, composition_removed,
    probe_session_event_log, repair_session_event_log_tail, session_event_log_oversized,
    session_event_log_size,
};
use crate::sessionstore::save_conflict::{
    SessionSnapshotConflictError, SessionSnapshotConflictKind, snapshot_conflict,
    snapshot_conflict_of_kind,
};
use crate::sessionstore::session_lease::{
    SessionWriteAuthoritySave, WriteAuthorityError, observe_unleased_session_write,
};
use crate::sessionstore::session_lock::{SessionFileLockError, lock_session_file};
use crate::sessionstore::v4_import::ImportError;
use crate::store::naming::session_display_index;

use super::Session;
use super::baseline::{PersistedDisk, stamp_baseline};
use super::canonical::{canonical_session_save_path, with_session_save_path_lock};
use super::checkpoint::{
    heal_empty_checkpoint_from_wal, keep_superseded_transcript, persist_session_listing_projection,
    probe_refuses_save, session_artifact_exists, write_session_messages,
};
use super::digest::{digest_session_messages, digest_size_and_prefix};
use super::load::load_snapshot_baseline;
use super::storage_eq::{
    append_covers_persisted_baseline, messages_have_prefix,
    messages_have_prefix_with_compatible_system,
};

/// The directory of a path (Go: filepath.Dir), "." for bare names.
pub(crate) fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

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
    #[error(transparent)]
    Dag(#[from] DagError),
    #[error(transparent)]
    V4Import(Box<ImportError>),
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
    #[error("session event log written by Reasonix 1.x already holds this transcript: {path}")]
    DagUnchanged { path: String },
    #[error("session event log for {path} uses schema {version}; this build supports up to {max}")]
    FutureSchema {
        path: String,
        version: i64,
        max: i64,
    },
    #[error("decode session event log {path}: unsupported event type {event_type:?}")]
    UnknownEventType { path: String, event_type: String },
    #[error(transparent)]
    DisplayModelWrite(#[from] crate::sessionstore::display_read_model::EventWriteError),
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
    ///
    /// A snapshot save reports the previous display index's message count as
    /// the append boundary, so the observer can tell an append from a rewrite
    /// without re-reading the transcript.
    fn save_observed(&self, path: &str, mode: SessionSaveMode) -> Result<(), SaveError> {
        let append_from = if mode == SessionSaveMode::Snapshot {
            load_session_display_index(&session_display_index(Path::new(path)))
                .map(|index| index.message_count)
                .unwrap_or(-1)
        } else {
            -1i64
        };
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
        probe_refuses_save(path, &probe, &msgs)?;
        // (9) Repair a torn tail before deciding anything on top of it.
        if probe.native && probe.size > 0 {
            repair_session_event_log_tail(Path::new(path))?;
        }
        // (10) The CAS decision against the persisted baseline.
        let owned_rewrite = mode != SessionSaveMode::Snapshot;
        let decision =
            self.snapshot_decision(path, &msgs, digest, prefix, version, owned_rewrite)?;
        // (10b) A rewrite or fold must not bury a record it cannot reproduce.
        // The decision is computed first: an append or an up-to-date no-op
        // leaves every existing record in place.
        let append_shape = decision.append_only && probe.native;
        if (mode == SessionSaveMode::RewriteCompact || (!decision.up_to_date && !append_shape))
            && let Some(event_type) =
                crate::sessionstore::eventlog::unrewritable_record_type(Path::new(path), &probe)
        {
            return Err(SaveError::UnknownEventType {
                path: path.to_string(),
                event_type,
            });
        }
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
                if let Err(err) =
                    refresh_session_display_index(Path::new(path), &msgs, &digest, revision, -1)
                {
                    warn_display_index_write(path, &err);
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
            publish_append_only_save(
                path,
                &msgs,
                &digest,
                content_bytes,
                decision.revision,
                decision.append_from,
            )?;
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
        // The display index is a pure derived sidecar: a write failure is
        // warn-only and never fails a save whose transcript already landed.
        if let Err(err) =
            refresh_session_display_index(Path::new(path), &msgs, &digest, revision, -1)
        {
            warn_display_index_write(path, &err);
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
    pub(crate) fn load_agrees_with_ledger(
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

#[cfg(test)]
#[path = "save_tests.rs"]
mod tests;
