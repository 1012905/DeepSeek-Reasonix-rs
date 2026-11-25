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
//! Deferred to the slices that own them: the 1.x DAG load path (SESS-033),
//! the v4 import inside `prepare_session_path` (SESS-036..038), and the
//! overlong-name sidecar reconciliation (SESS-023's rename arm, which is in
//! cleanup_pending.rs).

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
mod tests {
    use std::fs::{self, File};
    use std::sync::RwLock;
    use std::time::{Duration, SystemTime};

    use reasonix_contract::provider::Role;

    use super::*;
    use crate::sessionstore::branch::{BRANCH_META_COUNTS_VERSION, load_branch_meta};
    use crate::sessionstore::eventlog::read_session_event_index;
    use crate::sessionstore::eventlog::replay_session_event_log;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::session::{
        RecoveryBranchOptions, SessionState, load_session, load_session_messages,
        load_session_unlocked,
    };
    use crate::sessionstore::session_lease::{
        next_session_write_generation, try_acquire_session_lease,
    };
    use crate::store::naming::{session_event_log, session_meta};
    use reasonix_contract::gojson;
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

    fn write_dag(session: &Path, user_content: &str) -> Vec<u8> {
        fs::write(session, b"").expect("write checkpoint");
        let records = [
            format!(
                "{{\"schema_version\":2,\"at\":\"2026-09-25T13:00:00Z\",\"type\":\"message\",\"id\":\"u\",\"parent\":\"\",\"head\":\"main\",\"msgs\":[{{\"role\":\"user\",\"content\":{}}}]}}",
                serde_json::to_string(user_content).expect("content JSON")
            ),
            "{\"schema_version\":2,\"at\":\"2026-09-25T13:00:01Z\",\"type\":\"message\",\"id\":\"a\",\"parent\":\"u\",\"head\":\"main\",\"msgs\":[{\"role\":\"assistant\",\"content\":\"answer\"}]}".to_string(),
        ];
        let body = format!("{}\n", records.join("\n"));
        fs::write(session_event_log(session), body.as_bytes()).expect("write DAG");
        body.into_bytes()
    }

    #[test]
    fn unchanged_foreign_dag_save_is_typed_and_writes_nothing() {
        let dir = tempdir();
        let path = dir.path().join("foreign.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let before = write_dag(&path, "same question");

        let session = load_session(&path_str).expect("load foreign DAG");
        let err = session
            .save(&path_str)
            .expect_err("unchanged foreign save refused");
        assert!(matches!(
            err,
            SaveError::DagUnchanged { ref path } if path == &path_str
        ));
        assert_eq!(fs::read(session_event_log(&path)).expect("log"), before);
        assert_eq!(fs::read(&path).expect("checkpoint"), b"");
    }

    #[test]
    fn changed_foreign_dag_save_is_refused_without_writing_a_sidecar() {
        let dir = tempdir();
        let path = dir.path().join("foreign.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let before = write_dag(&path, "stored question");

        let session = session_of(vec![user_message("different question")]);
        let err = session
            .save(&path_str)
            .expect_err("changed foreign save refused");
        assert!(matches!(
            err,
            SaveError::DagForeign { ref path } if path == &path_str
        ));
        assert_eq!(fs::read(session_event_log(&path)).expect("log"), before);
        assert!(!session_meta(&path).exists());
    }

    #[test]
    fn an_unknown_record_opens_but_refuses_a_rewrite() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("hello")]);
        session.save(&path_str).expect("first save");

        // A newer build appended a record this one does not understand.
        let log = session_event_log(&path);
        let mut body = fs::read_to_string(&log).expect("read log");
        body.push_str(
            "{\"schema_version\":1,\"type\":\"mutate\",\"messages\":[{\"role\":\"user\",\"content\":\"future\"}]}
",
        );
        fs::write(&log, &body).expect("append unknown record");

        // The read tolerates it and keeps the known transcript.
        let loaded = load_session(&path_str).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.snapshot()[0].content, "hello");

        // A fold would drop it, so it is refused.
        let grown = session_of(loaded.snapshot());
        grown.add(assistant_message("answer"));
        let err = grown
            .save_rewrite_compact(&path_str)
            .expect_err("fold refused");
        match &err {
            SaveError::UnknownEventType { path, event_type } => {
                assert!(path.ends_with("s.jsonl"));
                assert_eq!(event_type, "mutate");
            }
            other => panic!("expected an unknown-event-type refusal, got: {other}"),
        }
        assert_eq!(fs::read_to_string(&log).expect("unread log"), body);

        // An append still lands: the record stays in place.
        let appended = session_of(loaded.snapshot());
        appended.add(assistant_message("answer"));
        appended.save(&path_str).expect("append allowed");
        let replay = replay_session_event_log(&log).expect("replay");
        assert_eq!(replay.unknown_records, vec!["mutate".to_string()]);
        assert_eq!(replay.msgs.len(), 2);
    }

    #[test]
    fn the_display_index_tracks_every_save() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("one")]);
        session.save(&path_str).expect("first save");

        let index_path = session_display_index(&path);
        let first = load_session_display_index(&index_path).expect("index");
        assert_eq!(first.message_count, 1);
        assert_eq!(first.authored_turns, 1);
        assert!(first.revision_known);

        session.add(assistant_message("two"));
        session.add(user_message("three"));
        session.save(&path_str).expect("append save");
        let second = load_session_display_index(&index_path).expect("index");
        assert_eq!(second.message_count, 3);
        assert_eq!(second.authored_turns, 2);
        assert_eq!(second.revision, first.revision + 1);
        // The prefix entries survive the incremental extension unchanged.
        assert_eq!(second.entries[0], first.entries[0]);
        let transcript = fs::metadata(&path).expect("stat").len() as i64;
        assert_eq!(second.transcript_size, transcript);
    }

    #[test]
    fn the_read_model_append_keeps_the_transcript_current() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = session_of(vec![user_message("one")]);
        session.save(&path_str).expect("first save");
        let before =
            crate::sessionstore::session::load_session_messages_from_jsonl(&path).expect("model");
        assert_eq!(before.len(), 1);

        session.add(assistant_message("two"));
        session.save(&path_str).expect("append save");
        let after =
            crate::sessionstore::session::load_session_messages_from_jsonl(&path).expect("model");
        assert_eq!(after.len(), 2);
        assert_eq!(after[1].content, "two");
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
