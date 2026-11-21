//! The bounded display read model (Go: `session_display_read_model.go`).
//!
//! The `.jsonl` transcript is a random-read model of the authoritative event
//! log. Desktop history uses this path only while that model is missing or
//! stale: it reads the authoritative transcript without resume-time
//! normalization, and it can republish the compatibility model plus its
//! display index from the log.

use std::io::{self, Write};
use std::path::Path;

use reasonix_contract::provider::Message;

use super::branch::{record_session_content_revision, session_content_revision};
use super::display_index::{
    build_session_display_index, load_session_display_index, refresh_session_display_index,
    write_session_display_index,
};
use super::eventlog::writer_id::digest_string;
use super::eventlog::{
    append_session_append_event, append_session_replace_event, compact_session_event_log,
    session_event_log_oversized, session_event_log_size, write_session_event_index,
};
use super::session::{
    digest_session_messages, load_session_messages, with_session_save_path_lock,
    write_session_messages,
};
use crate::sessionstore::session_lock::lock_session_file;
use crate::store::naming::session_display_index;

/// The authoritative event log has a torn tail: only the normal save path
/// heals it, and a derived read model must never publish the replayable prefix
/// as if it were complete (Go: `ErrSessionDisplayReadModelDamaged`).
pub const ERR_SESSION_DISPLAY_READ_MODEL_DAMAGED: &str =
    "session display read model source is damaged";

/// The ledger identity of a persisted transcript, as the display path reports
/// it (Go: the `PersistedState` projection in `session_display_read_model.go`).
/// The revision is meaningful only when `revision_known` is set, which happens
/// exactly when the ledger digest matches the transcript's content digest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayReadModelState {
    /// Content digest of the authoritative transcript.
    pub digest: [u8; 32],
    /// `digest` in the hex form sidecars store.
    pub digest_hex: String,
    /// CAS ledger revision.
    pub revision: i64,
    /// Whether the ledger digest matched, making the revision trustworthy.
    pub revision_known: bool,
}

/// Returns the authoritative persisted transcript without applying
/// resume-time normalization (Go: `LoadSessionDisplayMessages`). The bool is
/// `false` when the log's tail is damaged.
pub fn load_session_display_messages(
    path: &Path,
) -> io::Result<(Vec<Message>, DisplayReadModelState, bool)> {
    with_session_save_path_lock(&path.to_string_lossy(), || {
        load_session_display_messages_unlocked(path)
    })
}

fn load_session_display_messages_unlocked(
    path: &Path,
) -> io::Result<(Vec<Message>, DisplayReadModelState, bool)> {
    let (msgs, _from_events, damaged) =
        load_session_messages(path).map_err(|err| io::Error::other(err.to_string()))?;
    let digest = digest_session_messages(&msgs).map_err(io::Error::other)?;
    let (revision, ledger_digest) =
        session_content_revision(path).map_err(|err| io::Error::other(err.to_string()))?;
    let mut state = DisplayReadModelState {
        digest,
        digest_hex: digest_string(&digest),
        ..DisplayReadModelState::default()
    };
    if !ledger_digest.is_empty() && ledger_digest == state.digest_hex {
        state.revision = revision;
        state.revision_known = true;
    }
    Ok((msgs, state, !damaged))
}

/// Atomically refreshes the compatibility JSONL read model and publishes its
/// display index from the authoritative event log (Go:
/// `RepairSessionDisplayReadModel`). It shares BOTH save locks with
/// `Session::save`, so a background migration can never interleave between an
/// event append and its revision/index publication.
pub fn repair_session_display_read_model(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::other("empty session path"));
    }
    with_session_save_path_lock(&path.to_string_lossy(), || {
        let _file_lock = lock_session_file(path).map_err(|err| match err {
            crate::sessionstore::session_lock::SessionFileLockError::Held => {
                io::Error::other("lock session file: session file lock held")
            }
            crate::sessionstore::session_lock::SessionFileLockError::Io(err) => {
                io::Error::other(format!("lock session file: {err}"))
            }
        })?;
        let (msgs, state, repairable) = load_session_display_messages_unlocked(path)?;
        if !repairable {
            return Err(io::Error::other(ERR_SESSION_DISPLAY_READ_MODEL_DAMAGED));
        }
        write_session_messages(path, &msgs)
            .map_err(|err| io::Error::other(format!("write session display read model: {err}")))?;
        let idx =
            build_session_display_index(&msgs, state.revision, state.revision_known, &state.digest)
                .ok_or_else(|| io::Error::other("encode session display index"))?;
        write_session_display_index(&session_display_index(path), &idx)
    })
}

/// Advances the JSONL random-read model in place when the previous display
/// index proves it is exactly the authoritative prefix (Go:
/// `appendSessionDisplayReadModel`). `false` asks the caller to leave the old
/// model and index untouched and let background repair rebuild them; it never
/// guesses from file size alone.
pub fn append_session_display_read_model(
    path: &Path,
    msgs: &[Message],
    append_from: i64,
    base_revision: i64,
) -> io::Result<bool> {
    if append_from <= 0 || append_from > msgs.len() as i64 {
        return Ok(false);
    }
    let index_path = session_display_index(path);
    let Ok(idx) = load_session_display_index(&index_path) else {
        return Ok(false);
    };
    if idx.message_count != append_from || !idx.revision_known || idx.revision != base_revision {
        return Ok(false);
    }
    let Ok(info) = std::fs::metadata(path) else {
        return Ok(false);
    };
    if info.is_dir() || idx.transcript_size != info.len() as i64 {
        return Ok(false);
    }
    let Ok(index_info) = std::fs::metadata(&index_path) else {
        return Ok(false);
    };
    if index_info.is_dir() {
        return Ok(false);
    }
    match (index_info.modified(), info.modified()) {
        (Ok(index_at), Ok(model_at)) if index_at >= model_at => {}
        _ => return Ok(false),
    }

    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    let original_size = info.len();
    let outcome =
        write_model_tail(&mut file, &msgs[append_from as usize..]).and_then(|()| file.sync_all());
    drop(file);
    if let Err(err) = outcome {
        // The model must not keep a partially appended tail.
        truncate_model(path, original_size);
        return Err(err);
    }
    // Some filesystems expose coarse mtimes. Ensure the subsequently
    // published index cannot appear older than this append generation.
    if let Ok(handle) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = handle.set_modified(std::time::SystemTime::now());
    }
    Ok(true)
}

/// Encodes one message per line into an already opened append handle.
fn write_model_tail(file: &mut std::fs::File, msgs: &[Message]) -> io::Result<()> {
    for msg in msgs {
        let line = reasonix_contract::gojson::marshal(msg)
            .map_err(|err| io::Error::other(format!("encode session display message: {err}")))?;
        file.write_all(format!("{line}\n").as_bytes())?;
    }
    Ok(())
}

/// Rolls a failed append back to the model size it started at.
fn truncate_model(path: &Path, original_size: u64) {
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_len(original_size);
    }
}

/// Publishes one append-only save and returns the revision it recorded: the
/// event log first, then the compatibility read model, then the index. A
/// display-side failure is warn-only — the transcript is already durable —
/// while an event-log failure fails the save (Go: the appendOnly arm of
/// saveLocked).
pub fn publish_append_only_save(
    path: &str,
    msgs: &[Message],
    digest: &[u8; 32],
    content_bytes: i64,
    base_revision: i64,
    append_from: i64,
) -> Result<i64, EventWriteError> {
    let session = Path::new(path);
    let log_size = session_event_log_size(session);
    let display_model_current = if log_size == 0 {
        append_session_replace_event(session, msgs, digest, base_revision, "snapshot")
            .map_err(EventWriteError::Event)?;
        append_or_warn(session, msgs, append_from, base_revision)
    } else if session_event_log_oversized(log_size, content_bytes, msgs) {
        // Fold the history into one replace event and refresh the random-read
        // model atomically. Normal appends keep it current below too.
        compact_session_event_log(session, msgs, digest, base_revision, "compact")
            .map_err(EventWriteError::Event)?;
        write_session_messages(session, msgs).map_err(EventWriteError::Model)?;
        true
    } else {
        let tail = &msgs[append_from as usize..];
        append_session_append_event(session, append_from, tail, digest, base_revision)
            .map_err(EventWriteError::Event)?;
        append_or_warn(session, msgs, append_from, base_revision)
    };
    // A declined in-place append must not leave the checkpoint describing the
    // transcript from before this save; republishing it also re-arms the
    // append fast path for every later save.
    let (display_model_current, index_from) =
        settle_display_read_model(session, msgs, display_model_current, append_from);
    let revision = record_session_content_revision(session, digest, base_revision)
        .map_err(|err| EventWriteError::Revision(err.to_string()))?;
    warn_event_index_write(path, msgs, digest, revision);
    if display_model_current
        && let Err(err) = refresh_session_display_index(session, msgs, digest, revision, index_from)
    {
        warn_display_index_write(path, &err);
    }
    Ok(revision)
}

/// Errors of [`publish_append_only_save`]: the event log is authoritative, so
/// its failure fails the save; the model and ledger failures keep their
/// existing classes.
#[derive(Debug, thiserror::Error)]
pub enum EventWriteError {
    /// The event log could not be written.
    #[error(transparent)]
    Event(#[from] super::eventlog::SessionEventWriteError),
    /// The compatibility transcript could not be written.
    #[error("{0}")]
    Model(#[source] super::session::WriteMessagesError),
    /// The revision ledger could not be updated.
    #[error("{0}")]
    Revision(String),
}

/// One in-place read-model append whose failure is warn-only (Go: the
/// "keeping save after display read-model append failure" branch).
fn append_or_warn(session: &Path, msgs: &[Message], append_from: i64, base_revision: i64) -> bool {
    match append_session_display_read_model(session, msgs, append_from, base_revision) {
        Ok(current) => current,
        Err(err) => {
            tracing::warn!(
                path = %session.display(),
                error = %err,
                "session: keeping save after display read-model append failure"
            );
            false
        }
    }
}

/// Warn-only event index refresh (Go: writeSessionEventIndex plus the
/// "keeping save after event index write failure" warn).
pub(crate) fn warn_event_index_write(
    path: &str,
    msgs: &[Message],
    digest: &[u8; 32],
    revision: i64,
) {
    if let Err(err) = write_session_event_index(Path::new(path), msgs.len(), digest, revision) {
        tracing::warn!(
            path = path,
            error = %err,
            "session: keeping save after event index write failure"
        );
    }
}

/// Warn-only display index refresh (Go: the "keeping save after display index
/// write failure" warn).
pub(crate) fn warn_display_index_write(path: &str, err: &io::Error) {
    tracing::warn!(
        path = path,
        error = %err,
        "session: keeping save after display index write failure"
    );
}

/// Republishes the read model whole when the in-place append declined (Go:
/// `settleDisplayReadModel`): leaving it would keep a pre-save transcript
/// beside the log as a second lineage. A rewrite failure keeps the save — the
/// transcript and revision are already durable — and returns `(false,
/// append_from)` for background repair; a rebuild returns `(true, -1)` because
/// there is no previous index left to extend.
pub fn settle_display_read_model(
    path: &Path,
    msgs: &[Message],
    current: bool,
    append_from: i64,
) -> (bool, i64) {
    if current {
        return (true, append_from);
    }
    if let Err(err) = write_session_messages(path, msgs) {
        tracing::warn!(
            path = %path.display(),
            error = %err,
            "session: keeping save after display read-model rewrite failure"
        );
        return (false, append_from);
    }
    (true, -1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::display_index::refresh_session_display_index;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::session::{Session, load_session_messages_from_jsonl};
    use reasonix_contract::provider::{Message, Role};

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

    #[test]
    fn repair_republishes_from_the_authoritative_log() {
        let dir = tempdir();
        let path = dir.path().join("repair.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("sys");
        session.add(user("first"));
        session.save(&path_str).expect("save base");
        let old_model = std::fs::read(&path).expect("read model");
        let old_index = std::fs::read(session_display_index(&path)).expect("read index");

        session.add(assistant("new tail"));
        session.save(&path_str).expect("save tail");
        std::fs::write(&path, &old_model).expect("restore stale model");
        std::fs::write(session_display_index(&path), &old_index).expect("restore stale index");

        let (msgs, state, repairable) = load_session_display_messages(&path).expect("load");
        assert!(repairable);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].content, "new tail");
        assert!(state.revision_known);
        assert_eq!(state.digest_hex, digest_string(&state.digest));

        repair_session_display_read_model(&path).expect("repair");
        let repaired = load_session_messages_from_jsonl(&path).expect("read repaired model");
        assert_eq!(repaired.len(), msgs.len());
        let idx = load_session_display_index(&session_display_index(&path)).expect("load index");
        let size = std::fs::metadata(&path).expect("stat").len() as i64;
        assert!(super::super::display_index::validate_session_display_index(
            &idx,
            state.revision,
            state.revision_known,
            &state.digest,
            size
        ));
    }

    #[test]
    fn a_damaged_log_refuses_the_repair() {
        let dir = tempdir();
        let path = dir.path().join("damaged.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        let session = Session::new("sys");
        session.add(user("first"));
        session.save(&path_str).expect("save");
        let log = crate::store::session_event_log(&path);
        let mut log_bytes = std::fs::read(&log).expect("read log");
        log_bytes.extend_from_slice(b"{\"schema_version\":1,\"type\":\"app");
        std::fs::write(&log, &log_bytes).expect("tear log");

        let err = repair_session_display_read_model(&path).expect_err("damaged refused");
        assert_eq!(err.to_string(), ERR_SESSION_DISPLAY_READ_MODEL_DAMAGED);
    }

    #[test]
    fn append_only_when_the_index_proves_the_prefix() {
        let dir = tempdir();
        let path = dir.path().join("append.jsonl");
        let base = [user("one"), assistant("two")];
        write_session_messages(&path, &base).expect("write model");
        let digest = digest_session_messages(&base).expect("digest");
        refresh_session_display_index(&path, &base, &digest, 1, -1).expect("index");

        let next = [user("one"), assistant("two"), user("three")];
        assert!(
            append_session_display_read_model(&path, &next, 2, 1).expect("append in place"),
            "a proven prefix is appended in place"
        );
        let model = load_session_messages_from_jsonl(&path).expect("reread");
        assert_eq!(model.len(), 3);
        assert_eq!(model[2].content, "three");

        // A second append with the same stale index is declined.
        assert!(
            !append_session_display_read_model(&path, &next, 2, 1).expect("declined"),
            "an index that no longer covers the file is not reused"
        );
    }

    #[test]
    fn append_declines_without_an_index_or_proof() {
        let dir = tempdir();
        let path = dir.path().join("noindex.jsonl");
        let msgs = [user("one"), assistant("two")];
        write_session_messages(&path, &msgs).expect("write model");
        assert!(!append_session_display_read_model(&path, &msgs, 1, 1).expect("no index"));
        // Non-append shapes decline without touching the file.
        let digest = digest_session_messages(&msgs).expect("digest");
        refresh_session_display_index(&path, &msgs, &digest, 2, -1).expect("index");
        let before = std::fs::read(&path).expect("read");
        assert!(!append_session_display_read_model(&path, &msgs, 0, 2).expect("zero boundary"));
        assert!(!append_session_display_read_model(&path, &msgs, 5, 2).expect("past end"));
        assert!(!append_session_display_read_model(&path, &msgs, 1, 9).expect("wrong revision"));
        assert_eq!(std::fs::read(&path).expect("read"), before);
    }

    #[test]
    fn settle_republishes_or_keeps_the_save() {
        let dir = tempdir();
        let path = dir.path().join("settle.jsonl");
        let msgs = [user("one"), assistant("two")];
        assert_eq!(settle_display_read_model(&path, &msgs, true, 7), (true, 7));
        let (current, append_from) = settle_display_read_model(&path, &msgs, false, 1);
        assert!(current);
        assert_eq!(append_from, -1);
        assert_eq!(
            load_session_messages_from_jsonl(&path).expect("read").len(),
            2
        );
    }
}
