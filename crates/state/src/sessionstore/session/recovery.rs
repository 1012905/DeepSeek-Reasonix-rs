//! Recovery branches: when a save would clobber a newer transcript, the
//! stale runtime forks its unsaved turns into a bounded recovery file
//! instead of overwriting the winner (Go: save.go saveRecoveryBranch,
//! recovery_isolated.go, save_recovery_helpers.go).
//!
//! Three entry points share one pipeline. save_recovery_branch is the
//! autosave path: it skips the fork when the disk transcript already
//! covers the snapshot and refuses to fork past the chain-depth cap.
//! save_shutdown_recovery_branch and save_conflict_recovery_branch
//! bypass both gates: an unsaved tail must survive a shutdown or a lost
//! write race no matter what. Lane allocation dedupes forks by content
//! digest, pins each writer to its own lane, and rotates on collision,
//! so two stale runtimes never share a recovery file.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use reasonix_contract::provider::Message;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::Session;
use super::canonical::with_session_save_path_lock;
use super::digest::digest_session_messages;
use super::save::{
    LoadError, WriteMessagesError, is_cleanup_pending, load_session_unlocked,
    write_session_messages,
};
use super::storage_eq::{messages_have_prefix, messages_have_prefix_with_compatible_system};
use crate::sessionstore::branch::{
    BRANCH_META_COUNTS_VERSION, BranchMeta, BranchMetaError, branch_id, load_branch_meta,
    save_branch_meta_keep_in_flight_turn,
};
use crate::sessionstore::compaction_state::{
    RecallLedger, covered_prefix_hash, load_compaction_state, save_compaction_state,
};
use crate::sessionstore::eventlog::index::write_session_event_index;
use crate::sessionstore::eventlog::probe::probe_session_event_log;
use crate::sessionstore::eventlog::writer::{
    SessionEventWriteError, append_session_replace_event, compact_session_event_log,
};
use crate::sessionstore::eventlog::writer_id::{
    digest_string, new_session_writer_id, session_writer_id,
};
use crate::sessionstore::session_lock::{SessionFileLockError, lock_session_file};
use crate::sessionstore::sessionpreview::session_preview_from_messages;
use reasonix_core::fileutil::mkdir_all_mode;

/// Cap on how deep a chain of recovery forks may nest (Go
/// SessionRecoveryMaxDepth).
pub const SESSION_RECOVERY_MAX_DEPTH: i64 = 3;

/// Default recovery branch title when the caller does not name it (Go
/// RecoveryBranchDefaultName).
pub const RECOVERY_BRANCH_DEFAULT_NAME: &str = "Recovered unsaved changes from stale runtime";

/// How many isolated lane paths we try before giving up (Go
/// writeRecoveryBranchIsolated).
const RECOVERY_PATH_ATTEMPTS: usize = 8;

/// Byte budget for the parent stem inside a recovery filename (Go
/// maxRecoveryParentStemBytes).
const MAX_RECOVERY_PARENT_STEM_BYTES: usize = 80;

/// Event-log record reason stamped on every recovery write.
const RECOVERY_EVENT_REASON: &str = "recovery";

/// Fallback recovery reason when the caller passes none.
const FALLBACK_RECOVERY_REASON: &str = "session snapshot conflict";

/// Errors surfaced while forking a recovery branch (Go
/// ErrSessionRecovery* plus the wrapped helper failures).
#[derive(Debug, Error)]
pub enum RecoveryBranchError {
    #[error("empty original session path")]
    EmptyOriginalPath,
    #[error("session recovery not needed")]
    NotNeeded,
    #[error("session recovery chain depth exceeded: {path} is already {depth} recovery forks deep")]
    DepthExceeded { path: String, depth: i64 },
    #[error("lock original session file: {0}")]
    LockOriginal(#[source] SessionFileLockError),
    #[error("lock recovery session file: {0}")]
    LockRecovery(#[source] SessionFileLockError),
    #[error("create recovery session dir: {0}")]
    CreateDir(#[source] io::Error),
    #[error("allocate isolated recovery lane: too many existing collisions")]
    TooManyCollisions,
    #[error(transparent)]
    Digest(#[from] serde_json::Error),
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Meta(#[from] BranchMetaError),
    #[error(transparent)]
    EventWrite(#[from] SessionEventWriteError),
    #[error(transparent)]
    WriteMessages(#[from] WriteMessagesError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Caller options for a recovery fork (Go RecoveryBranchOptions).
#[derive(Debug, Clone, Default)]
pub struct RecoveryBranchOptions {
    /// Session the fork diverged from.
    pub original_path: String,
    /// Optional fork title; blank falls back to
    /// RECOVERY_BRANCH_DEFAULT_NAME.
    pub name: String,
    /// Optional machine-facing reason; blank falls back to
    /// FALLBACK_RECOVERY_REASON.
    pub reason: String,
    /// Seed branch metadata (Go opts.BranchMeta).
    pub branch_meta: BranchMeta,
}

/// Result of a successful recovery fork (Go RecoveryBranchInfo).
#[derive(Debug, Clone)]
pub struct RecoveryBranchInfo {
    /// Path of the recovery transcript that now holds the snapshot.
    pub path: PathBuf,
    /// Hex content digest of the saved snapshot.
    pub digest: String,
    /// True when an identical fork already existed and was reused.
    pub existing: bool,
    /// Persisted branch metadata of the fork.
    pub meta: BranchMeta,
    /// Preview text of the first user turn.
    pub preview: String,
    /// Number of user turns in the snapshot.
    pub turns: i64,
}
impl Session {
    /// Forks the snapshot into a recovery branch when the on-disk
    /// transcript diverges (Go SaveRecoveryBranch). Skips the fork when
    /// the disk transcript already covers the snapshot, and refuses to
    /// fork past SESSION_RECOVERY_MAX_DEPTH.
    pub fn save_recovery_branch(
        &self,
        opts: RecoveryBranchOptions,
    ) -> Result<RecoveryBranchInfo, RecoveryBranchError> {
        self.save_recovery_branch_inner(opts, false)
    }

    /// Forks the snapshot on shutdown, bypassing the coverage check and
    /// the depth cap (Go SaveShutdownRecoveryBranch): an unsaved tail
    /// must survive shutdown no matter what.
    pub fn save_shutdown_recovery_branch(
        &self,
        opts: RecoveryBranchOptions,
    ) -> Result<RecoveryBranchInfo, RecoveryBranchError> {
        self.save_recovery_branch_inner(opts, true)
    }

    /// Forks the snapshot after a lost write race, bypassing the
    /// coverage check and the depth cap (Go SaveConflictRecoveryBranch).
    pub fn save_conflict_recovery_branch(
        &self,
        opts: RecoveryBranchOptions,
    ) -> Result<RecoveryBranchInfo, RecoveryBranchError> {
        self.save_recovery_branch_inner(opts, true)
    }

    fn save_recovery_branch_inner(
        &self,
        opts: RecoveryBranchOptions,
        shutdown: bool,
    ) -> Result<RecoveryBranchInfo, RecoveryBranchError> {
        let original = opts.original_path.trim().to_string();
        if original.is_empty() {
            return Err(RecoveryBranchError::EmptyOriginalPath);
        }
        let (msgs, version, rewrite_version) = self.snapshot_with_version();
        let (preview, turns) = session_preview_from_messages(&msgs);
        if turns == 0 {
            return Err(RecoveryBranchError::NotNeeded);
        }
        let digest = digest_session_messages(&msgs)?;
        let digest_text = digest_string(&digest);
        let loaded: Option<Session> = if shutdown {
            None
        } else {
            with_session_save_path_lock(&original, || {
                let _file_guard = lock_session_file(Path::new(&original))
                    .map_err(RecoveryBranchError::LockOriginal)?;
                let loaded = match load_session_unlocked(Path::new(&original)) {
                    Ok(session) => Some(session),
                    Err(LoadError::Probe(err)) if err.kind() == io::ErrorKind::NotFound => None,
                    Err(err) => return Err(RecoveryBranchError::Load(err)),
                };
                drop(_file_guard);
                Ok(loaded)
            })?
        };
        if let Some(current) = &loaded {
            let existing = current.snapshot();
            let existing_digest = digest_session_messages(&existing)?;
            let mut covered = existing_digest == digest
                || messages_have_prefix(&existing, &msgs)
                || messages_have_prefix_with_compatible_system(&existing, &msgs);
            if !covered {
                // The loaded session may have repaired the transcript on
                // load; its raw pre-repair messages are the honest
                // comparison basis for coverage (Go: the raw rescue).
                let (normalized_dirty, raw_messages) = {
                    let st = current.read();
                    (st.normalized_dirty, st.raw_messages.clone())
                };
                if normalized_dirty && !raw_messages.is_empty() {
                    let raw_digest = digest_session_messages(&raw_messages)?;
                    covered = raw_digest == digest
                        || messages_have_prefix(&raw_messages, &msgs)
                        || messages_have_prefix_with_compatible_system(&raw_messages, &msgs);
                }
            }
            if covered {
                return Err(RecoveryBranchError::NotNeeded);
            }
        }
        let parent_depth: i64 = match load_branch_meta(Path::new(&original)) {
            Ok(Some(meta)) if meta.recovered => {
                if meta.recovery_depth <= 0 {
                    1
                } else {
                    meta.recovery_depth
                }
            }
            _ => 0,
        };
        if parent_depth >= SESSION_RECOVERY_MAX_DEPTH && !shutdown {
            return Err(RecoveryBranchError::DepthExceeded {
                path: original.clone(),
                depth: parent_depth,
            });
        }
        let recovery_depth = (parent_depth + 1).min(SESSION_RECOVERY_MAX_DEPTH);
        self.write_recovery_branch_isolated(
            &original,
            &opts,
            &msgs,
            &digest,
            version,
            rewrite_version,
            &preview,
            turns,
            &digest_text,
            recovery_depth,
            shutdown,
        )
    }

    #[allow(clippy::too_many_arguments)] // mirrors Go writeRecoveryBranchIsolated
    fn write_recovery_branch_isolated(
        &self,
        original_path: &str,
        opts: &RecoveryBranchOptions,
        msgs: &[Message],
        digest: &[u8; 32],
        version: u64,
        rewrite_version: i64,
        preview: &str,
        turns: i64,
        digest_text: &str,
        recovery_depth: i64,
        shutdown: bool,
    ) -> Result<RecoveryBranchInfo, RecoveryBranchError> {
        let mut taken: HashSet<PathBuf> = HashSet::new();
        for _ in 0..RECOVERY_PATH_ATTEMPTS {
            let (path, lane) =
                self.isolated_recovery_session_path(original_path, digest_text, &taken);
            match self.write_recovery_branch_at_path(
                &path,
                opts,
                msgs,
                digest,
                version,
                rewrite_version,
                preview,
                turns,
                digest_text,
                recovery_depth,
                shutdown,
            )? {
                Some(info) => return Ok(info),
                None => {
                    taken.insert(path);
                    self.rotate_recovery_lane(&lane);
                }
            }
        }
        Err(RecoveryBranchError::TooManyCollisions)
    }

    fn isolated_recovery_session_path(
        &self,
        original_path: &str,
        digest_text: &str,
        taken: &HashSet<PathBuf>,
    ) -> (PathBuf, String) {
        let lane = self.recovery_lane();
        if let Some(sibling) = sibling_recovery_branch_with_digest(original_path, digest_text)
            && !taken.contains(&sibling)
        {
            return (sibling, lane);
        }
        if lane.is_empty() {
            return (fixed_writer_recovery_session_path(original_path), lane);
        }
        (recovery_session_path_for_lane(original_path, &lane), lane)
    }

    fn rotate_recovery_lane(&self, current: &str) {
        let mut st = self.write();
        if st.recovery_lane != current {
            return;
        }
        st.recovery_lane = new_session_writer_id();
    }

    #[allow(clippy::too_many_arguments)] // mirrors Go writeRecoveryBranchAtPath
    fn write_recovery_branch_at_path(
        &self,
        path: &Path,
        opts: &RecoveryBranchOptions,
        msgs: &[Message],
        digest: &[u8; 32],
        version: u64,
        rewrite_version: i64,
        preview: &str,
        turns: i64,
        digest_text: &str,
        recovery_depth: i64,
        shutdown: bool,
    ) -> Result<Option<RecoveryBranchInfo>, RecoveryBranchError> {
        with_session_save_path_lock(&path.to_string_lossy(), || {
            // Both guards live to the end of this closure, matching the
            // Go defers in writeRecoveryBranchAtPath.
            let _file_guard = lock_session_file(path).map_err(RecoveryBranchError::LockRecovery)?;
            let current = match load_session_unlocked(path) {
                Ok(session) => Some(session),
                Err(LoadError::Probe(err)) if err.kind() == io::ErrorKind::NotFound => None,
                Err(err) => return Err(RecoveryBranchError::Load(err)),
            };
            if let Some(current) = current {
                let existing = current.snapshot();
                let existing_digest = digest_session_messages(&existing)?;
                if existing_digest == *digest {
                    // An identical fork already exists; adopt it.
                    inherit_compaction_state(&opts.original_path, path, msgs);
                    let meta = save_recovery_branch_meta(
                        path,
                        opts,
                        preview,
                        turns,
                        digest_text,
                        recovery_depth,
                    )?;
                    self.mark_persisted(
                        &path.to_string_lossy(),
                        digest,
                        version,
                        meta.revision,
                        rewrite_version,
                    );
                    return Ok(Some(RecoveryBranchInfo {
                        path: path.to_path_buf(),
                        digest: digest_text.to_string(),
                        existing: true,
                        meta,
                        preview: preview.to_string(),
                        turns,
                    }));
                }
                if !messages_have_prefix(msgs, &existing)
                    && !messages_have_prefix_with_compatible_system(msgs, &existing)
                {
                    return Ok(None);
                }
            }
            mkdir_all_mode(&file_dir(path), 0o755).map_err(RecoveryBranchError::CreateDir)?;
            let probe = probe_session_event_log(path)?;
            if probe.native {
                write_recovery_event_log(path, msgs, digest, shutdown)?;
            }
            write_session_messages(path, msgs)?;
            inherit_compaction_state(&opts.original_path, path, msgs);
            let meta =
                save_recovery_branch_meta(path, opts, preview, turns, digest_text, recovery_depth)?;
            if let Err(err) = write_session_event_index(path, msgs.len(), digest, meta.revision) {
                tracing::warn!(
                    path = %path.to_string_lossy(),
                    error = %err,
                    "session: keeping recovery branch after event index write failure"
                );
            }
            self.mark_persisted(
                &path.to_string_lossy(),
                digest,
                version,
                meta.revision,
                rewrite_version,
            );
            Ok(Some(RecoveryBranchInfo {
                path: path.to_path_buf(),
                digest: digest_text.to_string(),
                existing: false,
                meta,
                preview: preview.to_string(),
                turns,
            }))
        })
    }
}

/// Writes the metadata sidecar of a fresh or adopted recovery branch
/// (Go saveRecoveryBranchMeta). Returns the reloaded sidecar when
/// possible, so callers observe exactly what landed on disk.
fn save_recovery_branch_meta(
    path: &Path,
    opts: &RecoveryBranchOptions,
    preview: &str,
    turns: i64,
    digest_text: &str,
    recovery_depth: i64,
) -> Result<BranchMeta, BranchMetaError> {
    let mut meta = opts.branch_meta.clone();
    meta.id = branch_id(&path.to_string_lossy());
    if meta.name.trim().is_empty() {
        let name = first_non_empty(&[opts.name.trim(), RECOVERY_BRANCH_DEFAULT_NAME]);
        meta.name = name.to_string();
    }
    if meta.parent_id.trim().is_empty() {
        meta.parent_id = branch_id(&opts.original_path);
    }
    meta.recovery_root_id = recovery_root_of(&opts.original_path, &meta.parent_id);
    meta.preview = preview.to_string();
    meta.turns = turns;
    meta.schema_version = BRANCH_META_COUNTS_VERSION;
    meta.recovered = true;
    let reason = first_non_empty(&[opts.reason.trim(), FALLBACK_RECOVERY_REASON]);
    meta.recovery_reason = reason.to_string();
    meta.recovery_digest = digest_text.to_string();
    meta.recovery_depth = recovery_depth;
    if meta.revision == 0 {
        meta.revision = 1;
    }
    if meta.content_digest.trim().is_empty() {
        meta.content_digest = digest_text.to_string();
    }
    if meta.writer_id.trim().is_empty() {
        meta.writer_id = session_writer_id().to_string();
    }
    save_branch_meta_keep_in_flight_turn(path, meta.clone())?;
    load_branch_meta(path).map(|stored| stored.unwrap_or(meta))
}

/// Appends (or, for isolated shutdown forks, compacts) the recovery
/// event-log record (Go writeRecoveryEventLog).
fn write_recovery_event_log(
    path: &Path,
    msgs: &[Message],
    digest: &[u8; 32],
    isolated: bool,
) -> Result<(), SessionEventWriteError> {
    if isolated {
        compact_session_event_log(path, msgs, digest, 0, RECOVERY_EVENT_REASON)
    } else {
        append_session_replace_event(path, msgs, digest, 0, RECOVERY_EVENT_REASON)
    }
}

/// Copies the compaction sidecar from the original session into the
/// recovery fork when the fork still covers the projection (Go
/// InheritCompactionState).
fn inherit_compaction_state(source_path: &str, target_path: &Path, msgs: &[Message]) -> bool {
    if matches!(load_compaction_state(target_path), Ok(Some(_))) {
        return true;
    }
    let mut st = match load_compaction_state(Path::new(source_path)) {
        Ok(Some(st)) => st,
        _ => return false,
    };
    let n = st.projection.covered_count;
    if st.projection.messages.is_empty()
        || n <= 0
        || n as usize > msgs.len()
        || st.projection.covered_prefix_hash.is_empty()
        || covered_prefix_hash(msgs, n) != st.projection.covered_prefix_hash
    {
        return false;
    }
    st.recall = RecallLedger::default();
    st.blocked_input_hash = String::new();
    st.blocked_reason = String::new();
    if st
        .last_receipt
        .as_ref()
        .is_some_and(|receipt| receipt.status != "applied")
    {
        st.last_receipt = None;
    }
    match save_compaction_state(target_path, st) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(
                path = %target_path.to_string_lossy(),
                error = %err,
                "session: context projection not inherited"
            );
            false
        }
    }
}

/// The recovery path pinned to this writer identity (Go
/// fixedWriterRecoverySessionPath).
fn fixed_writer_recovery_session_path(original_path: &str) -> PathBuf {
    recovery_session_path_for_lane(original_path, session_writer_id())
}

/// Derives the recovery path for one lane: a session id keeps its lane
/// suffix once it carries one, so a lane re-enters the same fork (Go
/// recoverySessionPathForLane).
fn recovery_session_path_for_lane(original_path: &str, lane: &str) -> PathBuf {
    let writer_digest = Sha256::digest(lane.as_bytes());
    let suffix = format!("-recovery-{}", hex_bytes(&writer_digest[..8]));
    let id = branch_id(original_path);
    if id.ends_with(&suffix) {
        return PathBuf::from(original_path);
    }
    file_dir(Path::new(original_path)).join(format!(
        "{}{}.jsonl",
        recovery_parent_stem(&id),
        suffix
    ))
}

/// Finds an existing recovery fork of the original session whose
/// recovery digest matches the snapshot (Go
/// siblingRecoveryBranchWithDigest).
fn sibling_recovery_branch_with_digest(original_path: &str, digest_text: &str) -> Option<PathBuf> {
    if digest_text.trim().is_empty() {
        return None;
    }
    let dir = file_dir(Path::new(original_path));
    let prefix = format!(
        "{}-recovery-",
        recovery_parent_stem(&branch_id(original_path))
    );
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return None,
    };
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            name.starts_with(&prefix) && name.ends_with(".jsonl")
        })
        .map(|entry| entry.path())
        .collect();
    candidates.sort();
    for path in candidates {
        if path == Path::new(original_path) || is_cleanup_pending(&path.to_string_lossy()) {
            continue;
        }
        if let Ok(Some(meta)) = load_branch_meta(&path)
            && meta.recovered
            && meta.recovery_digest == digest_text
        {
            return Some(path);
        }
    }
    None
}

/// Resolves the lineage root recorded in the fork chain (Go
/// recoveryRootOf).
fn recovery_root_of(original_path: &str, parent_id: &str) -> String {
    if let Ok(Some(meta)) = load_branch_meta(Path::new(original_path))
        && meta.recovered
    {
        let root = meta.recovery_root_id.trim();
        if !root.is_empty() {
            return root.to_string();
        }
        let parent = meta.parent_id.trim();
        if !parent.is_empty() {
            return parent.to_string();
        }
    }
    parent_id.to_string()
}

/// Builds the parent stem for recovery filenames: a recovery fork id is
/// cut at its marker and long ids are compacted, each stamped with a
/// stable hex digest of the parent (Go recoveryParentStem).
pub(crate) fn recovery_parent_stem(parent: &str) -> String {
    let parent = parent.trim();
    if parent.is_empty() {
        return "session".to_string();
    }
    let sum = Sha256::digest(parent.as_bytes());
    if let Some((before, _)) = parent.split_once("-recovery-") {
        let base = before.trim_matches(['-', '_', '.', ' ']);
        let base = if base.is_empty() { "session" } else { base };
        let base = truncate_utf8_bytes(base, MAX_RECOVERY_PARENT_STEM_BYTES)
            .trim_matches(['-', '_', '.', ' ']);
        let base = if base.is_empty() { "session" } else { base };
        return format!("{base}-{}", hex_bytes(&sum[..6]));
    }
    if parent.len() <= MAX_RECOVERY_PARENT_STEM_BYTES {
        return parent.to_string();
    }
    let prefix = truncate_utf8_bytes(parent, MAX_RECOVERY_PARENT_STEM_BYTES)
        .trim_matches(['-', '_', '.', ' ']);
    let prefix = if prefix.is_empty() { "session" } else { prefix };
    format!("{prefix}-{}", hex_bytes(&sum[..6]))
}

/// Truncates a string to at most max bytes without splitting a UTF-8
/// sequence (Go truncateUTF8Bytes).
fn truncate_utf8_bytes(s: &str, max: usize) -> &str {
    if max == 0 {
        return "";
    }
    if s.len() <= max {
        return s;
    }
    for (i, ch) in s.char_indices() {
        if i + ch.len_utf8() > max {
            return &s[..i];
        }
    }
    s
}

/// First value that is not blank after trimming; returns the original
/// untrimmed value (Go firstNonEmpty).
fn first_non_empty<'a>(values: &[&'a str]) -> &'a str {
    for value in values {
        if !value.trim().is_empty() {
            return value;
        }
    }
    ""
}

/// Parent directory of a path, "." for bare names (mirrors the private
/// Go file_dir helpers).
fn file_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Lowercase hex of a byte slice.
fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::branch::{
        InFlightTurnMeta, save_branch_meta, set_session_in_flight_turn,
    };
    use crate::sessionstore::session::{SaveError, load_session};
    use crate::sessionstore::session_lock::try_lock_session_file;
    use reasonix_contract::provider::{Role, ToolCall};

    fn tempdir() -> tempfile::TempDir {
        crate::sessionstore::eventlog::test_support::tempdir()
    }

    fn user(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn assistant(content: &str) -> Message {
        Message {
            role: Role::ASSISTANT,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn opts(path: &Path) -> RecoveryBranchOptions {
        RecoveryBranchOptions {
            original_path: path.to_string_lossy().into_owned(),
            ..RecoveryBranchOptions::default()
        }
    }

    fn diverged_session_pair(dir: &Path, name: &str) -> (PathBuf, Session) {
        let path = dir.join(name);
        let current = Session::new("sys");
        current.add(user("first"));
        current.add(assistant("one"));
        current.add(user("disk second"));
        current.save(&path.to_string_lossy()).expect("save current");
        let stale = Session::new("sys");
        stale.add(user("first"));
        stale.add(assistant("one"));
        stale.add(user(&format!("local {name}")));
        (path, stale)
    }

    fn stamp_recovery_meta(path: &Path, depth: i64) {
        let mut meta = load_branch_meta(path)
            .expect("load meta")
            .expect("meta present");
        meta.recovered = true;
        meta.recovery_depth = depth;
        save_branch_meta(path, meta).expect("save meta");
    }

    #[test]
    fn save_shutdown_recovery_branch_bypasses_held_original_file_lock() {
        let dir = tempdir();
        let path = dir.path().join("session.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let base = Session::new("sys");
        base.add(user("persisted"));
        base.save(&path_str).expect("save base");
        let current = load_session(&path_str).expect("load base");
        current.add(assistant("unsaved shutdown tail"));
        let lock = try_lock_session_file(&path).expect("take session file lock");
        let err = current.save(&path_str).expect_err("save must fail");
        assert!(matches!(err, SaveError::LockSessionFile(_)));
        let info = current
            .save_shutdown_recovery_branch(RecoveryBranchOptions {
                original_path: path_str.clone(),
                reason: "shutdown session file lock timeout".to_string(),
                ..RecoveryBranchOptions::default()
            })
            .expect("shutdown recovery branch");
        drop(lock);
        assert_ne!(info.path, path);
        assert!(info.meta.recovered);
        assert_eq!(
            info.meta.recovery_reason,
            "shutdown session file lock timeout"
        );
        let recovered = load_session(&info.path.to_string_lossy()).expect("load recovery");
        let msgs = recovered.snapshot();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].content, "unsaved shutdown tail");
        let original = load_session(&path_str).expect("load original");
        assert_eq!(original.snapshot().len(), 2);
    }

    #[test]
    fn save_recovery_branch_not_needed_when_raw_transcript_covers_snapshot() {
        let dir = tempdir();
        let path = dir.path().join("session.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let s = Session::new("sys");
        s.add(user("run the build"));
        s.add(Message {
            role: Role::ASSISTANT,
            content: "Running it.".to_string(),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: String::new(),
                arguments: r#"{"cmd":"make"}"#.to_string(),
                ..ToolCall::default()
            }],
            ..Message::default()
        });
        s.add(Message {
            role: Role::TOOL,
            name: "bash".to_string(),
            tool_call_id: "call_1".to_string(),
            content: "ok".to_string(),
            ..Message::default()
        });
        s.save(&path_str).expect("save");
        let err = s.save_recovery_branch(opts(&path)).expect_err("not needed");
        assert!(matches!(err, RecoveryBranchError::NotNeeded));
    }

    #[test]
    fn save_recovery_branch_persists_diverged_snapshot() {
        let dir = tempdir();
        let path = dir.path().join("session.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let current = Session::new("sys");
        current.add(user("first"));
        current.add(assistant("one"));
        current.add(user("disk second"));
        current.save(&path_str).expect("save current");
        let stale = Session::new("sys");
        stale.add(user("first"));
        stale.add(assistant("one"));
        stale.add(user("local second"));
        let err = stale.save(&path_str).expect_err("conflict expected");
        assert!(matches!(err, SaveError::Conflict(_)));
        let info = stale
            .save_recovery_branch(opts(&path))
            .expect("recovery branch");
        assert_ne!(info.path, path);
        assert_eq!(info.turns, 2);
        assert_eq!(info.preview, "first");
        let recovered = load_session(&info.path.to_string_lossy()).expect("load recovery");
        assert_eq!(
            recovered.snapshot().last().map(|m| m.content.as_str()),
            Some("local second")
        );
        let original = load_session(&path_str).expect("load original");
        assert_eq!(
            original.snapshot().last().map(|m| m.content.as_str()),
            Some("disk second")
        );
        let meta = load_branch_meta(&info.path)
            .expect("load meta")
            .expect("meta present");
        assert!(meta.recovered);
        assert_eq!(meta.parent_id, branch_id(&path_str));
        assert_eq!(meta.name, RECOVERY_BRANCH_DEFAULT_NAME);
        assert!(!meta.recovery_digest.is_empty());
        assert_eq!(meta.schema_version, BRANCH_META_COUNTS_VERSION);
        assert_eq!(meta.revision, 1);
        assert_eq!(meta.content_digest, meta.recovery_digest);
        assert!(!meta.writer_id.is_empty());
    }

    #[test]
    fn save_recovery_branch_skips_pure_stale_prefix() {
        let dir = tempdir();
        let path = dir.path().join("session.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let current = Session::new("sys");
        current.add(user("first"));
        current.add(assistant("one"));
        current.add(user("disk second"));
        current.save(&path_str).expect("save current");
        let stale = Session::new("sys");
        stale.add(user("first"));
        stale.add(assistant("one"));
        let err = stale
            .save_recovery_branch(opts(&path))
            .expect_err("not needed");
        assert!(matches!(err, RecoveryBranchError::NotNeeded));
    }

    #[test]
    fn save_recovery_branch_stamps_and_caps_chain_depth() {
        let dir = tempdir();
        let (path, stale) = diverged_session_pair(dir.path(), "session.jsonl");
        let info = stale
            .save_recovery_branch(opts(&path))
            .expect("normal fork");
        assert_eq!(info.meta.recovery_depth, 1);

        let (deeper_path, deeper_stale) = diverged_session_pair(dir.path(), "deeper.jsonl");
        stamp_recovery_meta(&deeper_path, 1);
        let deeper_info = deeper_stale
            .save_recovery_branch(opts(&deeper_path))
            .expect("depth 2 fork");
        assert_eq!(deeper_info.meta.recovery_depth, 2);

        let (legacy_path, legacy_stale) = diverged_session_pair(dir.path(), "legacy.jsonl");
        stamp_recovery_meta(&legacy_path, 0);
        let legacy_info = legacy_stale
            .save_recovery_branch(opts(&legacy_path))
            .expect("legacy fork");
        assert_eq!(legacy_info.meta.recovery_depth, 2);

        let (capped_path, capped_stale) = diverged_session_pair(dir.path(), "capped.jsonl");
        stamp_recovery_meta(&capped_path, SESSION_RECOVERY_MAX_DEPTH);
        let err = capped_stale
            .save_recovery_branch(opts(&capped_path))
            .expect_err("depth cap");
        assert!(matches!(
            err,
            RecoveryBranchError::DepthExceeded { depth, .. } if depth == SESSION_RECOVERY_MAX_DEPTH
        ));
        let forks = fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("capped-recovery-") && name.ends_with(".jsonl")
            })
            .count();
        assert_eq!(forks, 0);
    }

    #[test]
    fn save_recovery_branch_dedupes_by_digest() {
        let dir = tempdir();
        let path = dir.path().join("session.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let current = Session::new("sys");
        current.add(user("first"));
        current.add(assistant("disk"));
        current.save(&path_str).expect("save current");
        let stale = Session::new("sys");
        stale.add(user("first"));
        stale.add(assistant("local"));
        let first = stale.save_recovery_branch(opts(&path)).expect("first fork");
        assert!(!first.existing);
        let second = stale
            .save_recovery_branch(opts(&path))
            .expect("second fork");
        assert_eq!(second.path, first.path);
        assert!(second.existing);
    }

    #[test]
    fn save_recovery_branch_compacts_long_parent_filename() {
        let dir = tempdir();
        let parent_id = "longparent-".repeat(22);
        let path = dir.path().join(format!("{parent_id}.jsonl"));
        let path_str = path.to_string_lossy().into_owned();
        let current = Session::new("sys");
        current.add(user("first"));
        current.add(assistant("disk"));
        current.save(&path_str).expect("save current");
        let stale = Session::new("sys");
        stale.add(user("first"));
        stale.add(assistant("local"));
        let info = stale
            .save_recovery_branch(opts(&path))
            .expect("compact fork");
        let base = info
            .path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        assert!(base.len() <= 140, "base too long: {base}");
        for suffix in [".lock", ".lease.lock", ".lease.json", ".meta"] {
            assert!(
                base.len() + suffix.len() <= 255,
                "{base}{suffix} exceeds 255"
            );
        }
        assert_eq!(info.meta.parent_id, branch_id(&path_str));
    }

    #[test]
    fn save_recovery_branch_does_not_cascade_recovery_filename() {
        let dir = tempdir();
        let path = dir.path().join("session.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let current = Session::new("sys");
        current.add(user("first"));
        current.add(assistant("disk"));
        current.save(&path_str).expect("save current");
        let local = Session::new("sys");
        local.add(user("first"));
        local.add(assistant("local"));
        let first = local.save_recovery_branch(opts(&path)).expect("first fork");
        let recovery_disk = load_session(&first.path.to_string_lossy()).expect("load fork");
        recovery_disk.add(user("disk follow-up"));
        recovery_disk
            .save(&first.path.to_string_lossy())
            .expect("save fork");
        let recovery_local = Session::new("sys");
        recovery_local.add(user("first"));
        recovery_local.add(assistant("local"));
        recovery_local.add(user("local follow-up"));
        let second = recovery_local
            .save_recovery_branch(RecoveryBranchOptions {
                original_path: first.path.to_string_lossy().into_owned(),
                ..RecoveryBranchOptions::default()
            })
            .expect("second fork");
        let base = second
            .path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(base.matches("-recovery-").count(), 1);
        assert!(base.len() <= 140, "base too long: {base}");
    }

    #[test]
    fn save_conflict_recovery_branch_at_cap_stays_bounded() {
        let dir = tempdir();
        let (path, stale) = diverged_session_pair(dir.path(), "session.jsonl");
        stamp_recovery_meta(&path, SESSION_RECOVERY_MAX_DEPTH);
        let mut current = path;
        for tick in 0..6 {
            stale.add(user(&format!("tick {tick}")));
            let err = stale
                .save_recovery_branch(opts(&current))
                .expect_err("depth exceeded");
            assert!(matches!(err, RecoveryBranchError::DepthExceeded { .. }));
            let info = stale
                .save_conflict_recovery_branch(opts(&current))
                .expect("conflict fork");
            current = info.path;
            if tick == 0 {
                set_session_in_flight_turn(
                    &current,
                    InFlightTurnMeta {
                        start_message_index: 2,
                        ..InFlightTurnMeta::default()
                    },
                )
                .expect("stamp in-flight turn");
            }
        }
        let meta = load_branch_meta(&current)
            .expect("load meta")
            .expect("meta present");
        let in_flight = meta.in_flight_turn.expect("in-flight marker");
        assert_eq!(in_flight.start_message_index, 2);
        let forks = fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.contains("-recovery-")
                    && name.ends_with(".jsonl")
                    && !name.ends_with(".events.jsonl")
            })
            .count();
        assert_eq!(forks, 1);
    }

    #[test]
    fn fixed_writer_recovery_session_path_separates_sibling_lineages() {
        let dir = tempdir();
        let session = dir.path().join("session.jsonl");
        let root = fixed_writer_recovery_session_path(&session.to_string_lossy());
        let session_fork = dir.path().join("session-recovery-abcd1234abcd1234.jsonl");
        let fork = fixed_writer_recovery_session_path(&session_fork.to_string_lossy());
        assert_ne!(root, fork);
        assert_eq!(
            fixed_writer_recovery_session_path(&root.to_string_lossy()),
            root
        );
        let root_str = root.to_string_lossy().into_owned();
        let id = branch_id(&root_str);
        let lane_suffix = id.strip_prefix("session-recovery-").expect("writer suffix");
        assert_eq!(lane_suffix.len(), 16);
    }

    #[test]
    fn recovery_lane_collision_preserves_independent_transcript() {
        let dir = tempdir();
        let original = dir.path().join("session.jsonl");
        let first = Session::new("sys");
        first.add(user("first"));
        first.add(assistant("one"));
        let first_info = first
            .save_conflict_recovery_branch(opts(&original))
            .expect("first fork");
        let second = Session::new("sys");
        second.add(user("second"));
        second.add(assistant("two"));
        second.set_recovery_lane(&first.recovery_lane());
        let second_info = second
            .save_conflict_recovery_branch(opts(&original))
            .expect("second fork");
        assert_ne!(second_info.path, first_info.path);
        let kept = load_session(&first_info.path.to_string_lossy()).expect("load first fork");
        assert_eq!(kept.snapshot()[2].content, "one");
        second.add(user("continued"));
        second.add(assistant("again"));
        let again = second
            .save_conflict_recovery_branch(RecoveryBranchOptions {
                original_path: second_info.path.to_string_lossy().into_owned(),
                ..RecoveryBranchOptions::default()
            })
            .expect("third save");
        assert_eq!(again.path, second_info.path);
    }

    #[test]
    fn recovery_parent_stem_trims_cuts_and_compacts() {
        assert_eq!(recovery_parent_stem(""), "session");
        assert_eq!(recovery_parent_stem("   "), "session");
        assert_eq!(recovery_parent_stem("session"), "session");
        let cut = recovery_parent_stem("session-recovery-abcd1234abcd1234");
        let (base, hex) = cut.rsplit_once('-').expect("hex suffix");
        assert_eq!(base, "session");
        assert_eq!(hex.len(), 12);
        assert!(hex.bytes().all(|b| b.is_ascii_hexdigit()));
        let long = recovery_parent_stem(&"x".repeat(120));
        assert_eq!(long.len(), 80 + 1 + 12);
        assert!(long.starts_with(&"x".repeat(80)));
    }

    #[test]
    fn truncate_utf8_bytes_bounds_and_multibyte() {
        assert_eq!(truncate_utf8_bytes("abcdef", 0), "");
        assert_eq!(truncate_utf8_bytes("abcdef", 3), "abc");
        assert_eq!(truncate_utf8_bytes("abcdef", 10), "abcdef");
        assert_eq!(truncate_utf8_bytes("a\u{4e09}b", 2), "a");
        assert_eq!(truncate_utf8_bytes("a\u{4e09}b", 4), "a\u{4e09}");
    }

    #[test]
    fn first_non_empty_skips_blanks_and_keeps_original() {
        assert_eq!(first_non_empty(&["", "  ", "x", "y"]), "x");
        assert_eq!(first_non_empty(&["  kept  "]), "  kept  ");
        assert_eq!(first_non_empty(&["", " "]), "");
    }
}
