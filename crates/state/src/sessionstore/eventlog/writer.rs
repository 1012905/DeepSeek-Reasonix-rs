//! Event-log writers: append/replace/append records and the fold
//! decision (session_events.go 166-187, 555-668).
//!
//! appendSessionEvent publishes through open+append so a crash tears
//! only its own record, and every image payload moves into the blob
//! directory first: the record naming a blob can never reach disk
//! before the blob it names. compactSessionEventLog rewrites the log as
//! one replace event through tmp+fsync+rename, then sweeps blobs the
//! published log no longer names.
//!
//! Known deviation: Go surfaces f.Close() errors; Rust closes files on
//! drop, so a close failure after a successful fsync is not reported
//! (std provides no checked close for regular files).

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use reasonix_contract::gojson;
use reasonix_contract::provider::Message;

use super::images::{
    ExternalizeError, externalize_session_images, inline_image_bytes, sweep_session_image_blobs,
};
use super::record::{CreatedAt, SessionEventImages, composition_event_schema};
use super::writer_id::{digest_string, session_writer_id};
use super::{
    SESSION_EVENT_LOG_COMPACT_FACTOR, SESSION_EVENT_LOG_COMPACT_FLOOR,
    SESSION_EVENT_LOG_FOLD_BYTES, SESSION_EVENT_TYPE_APPEND, SESSION_EVENT_TYPE_REPLACE,
};
use crate::store;

/// One event record as a writer stamps it (Go: sessionEventRecord).
/// Zero fields serialize away exactly like Go's omitempty tags; a zero
/// created_at is replaced with the current time by
/// [append_session_event].
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SessionEventRecord {
    /// Record schema; [append_session_event] stamps
    /// [composition_event_schema] over whatever the caller set.
    pub schema_version: i64,
    /// "replace" or "append" ([SESSION_EVENT_TYPE_REPLACE] /
    /// [SESSION_EVENT_TYPE_APPEND]).
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "gojson::is_zero_i64")]
    pub revision: i64,
    #[serde(skip_serializing_if = "gojson::is_zero_i64")]
    pub base_revision: i64,
    #[serde(skip_serializing_if = "gojson::is_zero_i64")]
    pub message_index: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content_digest: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub writer_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// When the record was written; always serialized (Go: time.Time,
    /// no omitempty).
    pub created_at: CreatedAt,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub image_blobs: Vec<SessionEventImages>,
}

/// Error publishing an event record (session_events.go 555-668).
#[derive(Debug)]
pub enum SessionEventWriteError {
    /// The session path is empty: "empty session event log path".
    EmptyPath,
    /// Image externalization failed (blob write refused).
    Externalize(ExternalizeError),
    /// "encode session event: {0}".
    Encode(serde_json::Error),
    /// "{context}: {source}" — the open/protect/append wrappers.
    Context {
        context: &'static str,
        source: io::Error,
    },
    /// Raw io error with no wrapper (Go: mkdir/sync/atomic-write).
    Io(io::Error),
}

impl std::fmt::Display for SessionEventWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "empty session event log path"),
            Self::Externalize(err) => write!(f, "{err}"),
            Self::Encode(err) => write!(f, "encode session event: {err}"),
            Self::Context { context, source } => write!(f, "{context}: {source}"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SessionEventWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EmptyPath => None,
            Self::Externalize(err) => Some(err),
            Self::Encode(err) => Some(err),
            Self::Context { source, .. } => Some(source),
            Self::Io(err) => Some(err),
        }
    }
}

/// Renders an io error the way Go's os.PathError does, so wrapped writer
/// errors keep the "op path: cause" shape.
fn path_error(op: &str, path: &Path, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{op} {}: {err}", path.display()))
}

/// Size of the session's event log, 0 when missing or unreadable (Go:
/// sessionEventLogSize).
pub fn session_event_log_size(session_path: &Path) -> i64 {
    let path = store::session_event_log(session_path);
    if path.as_os_str().is_empty() {
        return 0;
    }
    match std::fs::metadata(&path) {
        Ok(info) if !info.is_dir() => info.len() as i64,
        _ => 0,
    }
}

/// Whether the log has outgrown the transcript it describes and should
/// be folded (Go: sessionEventLogOversized): past the fold size with
/// another half-fold to drop, or past max(compact floor, compact
/// factor x live content) with data-URL images discounted (a folded
/// log holds them outside itself).
pub fn session_event_log_oversized(log_size: i64, content_bytes: i64, msgs: &[Message]) -> bool {
    if log_size > SESSION_EVENT_LOG_FOLD_BYTES
        && log_size - (content_bytes - inline_image_bytes(msgs)) > SESSION_EVENT_LOG_FOLD_BYTES / 2
    {
        return true;
    }
    let mut limit = SESSION_EVENT_LOG_COMPACT_FLOOR;
    let scaled = content_bytes * SESSION_EVENT_LOG_COMPACT_FACTOR;
    if scaled > limit {
        limit = scaled;
    }
    log_size > limit
}

/// Appends one event record to the session's event log (Go:
/// appendSessionEvent). Every data-URL image moves into the blob
/// directory before the record is encoded; sync fsyncs the log so a
/// power cut cannot lose the record.
pub fn append_session_event(
    session_path: &Path,
    mut rec: SessionEventRecord,
    sync: bool,
) -> Result<(), SessionEventWriteError> {
    let path = store::session_event_log(session_path);
    if path.as_os_str().is_empty() {
        return Err(SessionEventWriteError::EmptyPath);
    }
    let external = externalize_session_images(session_path, &rec.messages)
        .map_err(SessionEventWriteError::Externalize)?;
    if let Some(messages) = external.messages {
        rec.messages = messages;
    }
    rec.image_blobs = external.image_blobs;
    reasonix_core::fileutil::crash("wal-append", &path.to_string_lossy());
    reasonix_core::fileutil::mkdir_all_mode(super::parent_or_dot(&path), 0o755)
        .map_err(SessionEventWriteError::Io)?;
    rec.schema_version = composition_event_schema(&rec.messages, &rec.image_blobs);
    if rec.created_at == CreatedAt::zero() {
        rec.created_at = super::record::now_utc();
    }
    if rec.writer_id.is_empty() {
        rec.writer_id = session_writer_id().to_owned();
    }
    let mut buf = gojson::marshal(&rec).map_err(SessionEventWriteError::Encode)?;
    buf.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| SessionEventWriteError::Context {
            context: "open session event log",
            source: path_error("open", &path, err),
        })?;
    // The event log carries the complete transcript. Chmod after
    // opening so upgrading a pre-v0.53-boundary 0644 sidecar tightens
    // the existing inode before any unredacted message is appended;
    // OpenFile's perm only applies when the file is newly created.
    reasonix_core::fileutil::set_file_mode(&path, 0o600).map_err(|err| {
        SessionEventWriteError::Context {
            context: "protect session event log",
            source: path_error("chmod", &path, err),
        }
    })?;
    file.write_all(buf.as_bytes())
        .map_err(|err| SessionEventWriteError::Context {
            context: "append session event",
            source: path_error("write", &path, err),
        })?;
    if sync {
        file.sync_all()
            .map_err(|err| SessionEventWriteError::Io(path_error("sync", &path, err)))?;
    }
    Ok(())
}

/// Appends a replace event carrying the whole transcript, fsynced (Go:
/// appendSessionReplaceEvent). Replace events mark intentional history
/// rewrites; they are rare.
pub fn append_session_replace_event(
    session_path: &Path,
    msgs: &[Message],
    digest: &[u8; 32],
    base_revision: i64,
    reason: &str,
) -> Result<(), SessionEventWriteError> {
    append_session_event(
        session_path,
        SessionEventRecord {
            event_type: SESSION_EVENT_TYPE_REPLACE.to_owned(),
            revision: base_revision + 1,
            base_revision,
            messages: msgs.to_vec(),
            content_digest: digest_string(digest),
            reason: reason.to_owned(),
            ..SessionEventRecord::default()
        },
        true,
    )
}

/// Appends an append event for msgs, fsynced (Go:
/// appendSessionAppendEvent). An empty slice writes nothing.
pub fn append_session_append_event(
    session_path: &Path,
    message_index: i64,
    msgs: &[Message],
    digest: &[u8; 32],
    base_revision: i64,
) -> Result<(), SessionEventWriteError> {
    if msgs.is_empty() {
        return Ok(());
    }
    append_session_event(
        session_path,
        SessionEventRecord {
            event_type: SESSION_EVENT_TYPE_APPEND.to_owned(),
            revision: base_revision + 1,
            base_revision,
            message_index,
            messages: msgs.to_vec(),
            content_digest: digest_string(digest),
            ..SessionEventRecord::default()
        },
        true,
    )
}

/// Rewrites the log as a single replace event via an atomic
/// tmp+fsync+rename, so readers observe either the old log or the
/// compacted one and never a partial state. It also heals a damaged
/// log by construction (Go: compactSessionEventLog). Blobs the
/// published log no longer names are swept.
pub fn compact_session_event_log(
    session_path: &Path,
    msgs: &[Message],
    digest: &[u8; 32],
    base_revision: i64,
    reason: &str,
) -> Result<(), SessionEventWriteError> {
    let path = store::session_event_log(session_path);
    if path.as_os_str().is_empty() {
        return Err(SessionEventWriteError::EmptyPath);
    }
    let mut rec = SessionEventRecord {
        event_type: SESSION_EVENT_TYPE_REPLACE.to_owned(),
        revision: base_revision + 1,
        base_revision,
        messages: msgs.to_vec(),
        content_digest: digest_string(digest),
        writer_id: session_writer_id().to_owned(),
        reason: reason.to_owned(),
        created_at: super::record::now_utc(),
        ..SessionEventRecord::default()
    };
    let external = externalize_session_images(session_path, &rec.messages)
        .map_err(SessionEventWriteError::Externalize)?;
    if let Some(messages) = external.messages {
        rec.messages = messages;
    }
    rec.image_blobs = external.image_blobs;
    rec.schema_version = composition_event_schema(&rec.messages, &rec.image_blobs);
    let mut buf = gojson::marshal(&rec).map_err(SessionEventWriteError::Encode)?;
    buf.push('\n');
    reasonix_core::fileutil::atomic_write_file(&path, buf.as_bytes(), 0o600)
        .map_err(SessionEventWriteError::Io)?;
    // The published log names every blob still needed; nothing else is.
    sweep_session_image_blobs(session_path, &rec.image_blobs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support;
    use chrono::TimeZone;
    use reasonix_contract::provider::Role;

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::from("user"),
            content: content.to_owned(),
            ..Default::default()
        }
    }

    fn fixed_created_at() -> CreatedAt {
        CreatedAt::from_datetime(
            chrono::FixedOffset::east_opt(0)
                .unwrap()
                .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
                .single()
                .unwrap(),
        )
    }

    #[test]
    fn append_writes_go_shaped_record() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let digest = [7u8; 32];
        append_session_event(
            &session,
            SessionEventRecord {
                event_type: SESSION_EVENT_TYPE_REPLACE.to_owned(),
                revision: 2,
                base_revision: 1,
                messages: vec![user_msg("hi")],
                content_digest: digest_string(&digest),
                writer_id: "writer".to_owned(),
                reason: "why".to_owned(),
                created_at: fixed_created_at(),
                ..SessionEventRecord::default()
            },
            true,
        )
        .unwrap();
        let log = store::session_event_log(&session);
        let bytes = std::fs::read(&log).unwrap();
        let digest_hex = "0707070707070707070707070707070707070707070707070707070707070707";
        let expected = format!(
            "{{\"schema_version\":1,\"type\":\"replace\",\"revision\":2,\"base_revision\":1,\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}],\"content_digest\":\"{digest_hex}\",\"writer_id\":\"writer\",\"reason\":\"why\",\"created_at\":\"2026-01-02T03:04:05Z\"}}\n"
        );
        assert_eq!(std::str::from_utf8(&bytes).unwrap(), expected);
        // A second append lands on the next line and replays.
        append_session_event(
            &session,
            SessionEventRecord {
                event_type: SESSION_EVENT_TYPE_APPEND.to_owned(),
                revision: 3,
                base_revision: 2,
                message_index: 1,
                messages: vec![user_msg("again")],
                content_digest: digest_string(&digest),
                created_at: fixed_created_at(),
                ..SessionEventRecord::default()
            },
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("\"message_index\":1"));
        let replay = super::super::replay_session_event_log(&log).unwrap();
        assert_eq!(replay.msgs.len(), 2);
        assert_eq!(replay.msgs[1].content, "again");
    }

    #[test]
    fn record_serialization_omits_zero_fields() {
        let rec = SessionEventRecord {
            event_type: SESSION_EVENT_TYPE_APPEND.to_owned(),
            created_at: fixed_created_at(),
            ..SessionEventRecord::default()
        };
        let json = gojson::marshal(&rec).unwrap();
        assert_eq!(
            json,
            "{\"schema_version\":0,\"type\":\"append\",\"created_at\":\"2026-01-02T03:04:05Z\"}"
        );
    }

    #[test]
    fn append_stamps_writer_id_and_time() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        append_session_replace_event(&session, &[user_msg("hi")], &[0u8; 32], 4, "test").unwrap();
        let log = store::session_event_log(&session);
        let line = std::fs::read_to_string(&log).unwrap();
        assert!(line.contains("\"revision\":5"));
        assert!(line.contains("\"base_revision\":4"));
        assert!(line.contains("\"reason\":\"test\""));
        assert!(!line.contains("\"created_at\":\"0001-01-01"));
        let writer = session_writer_id();
        assert!(line.contains(&format!("\"writer_id\":\"{writer}\"")));
        // Replay applies the replace record.
        let replay = super::super::replay_session_event_log(&log).unwrap();
        assert_eq!(replay.msgs.len(), 1);
        assert_eq!(replay.msgs[0].content, "hi");
    }

    #[test]
    fn append_empty_session_path_is_refused() {
        let err =
            append_session_replace_event(Path::new(""), &[user_msg("hi")], &[0u8; 32], 1, "test")
                .unwrap_err();
        assert_eq!(err.to_string(), "empty session event log path");
        let err = append_session_append_event(Path::new(""), 0, &[user_msg("hi")], &[0u8; 32], 1)
            .unwrap_err();
        assert_eq!(err.to_string(), "empty session event log path");
    }

    #[test]
    fn append_event_skips_empty_messages() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        append_session_append_event(&session, 0, &[], &[0u8; 32], 1).unwrap();
        assert!(!store::session_event_log(&session).exists());
    }

    #[test]
    fn append_externalizes_images() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let url = "data:image/png;base64,QQ==";
        let key = super::super::images::session_image_key(url);
        let msgs = vec![Message {
            images: vec![url.to_owned()],
            ..user_msg("look")
        }];
        append_session_replace_event(&session, &msgs, &[0u8; 32], 1, "test").unwrap();
        let log = store::session_event_log(&session);
        let line = std::fs::read_to_string(&log).unwrap();
        assert!(line.contains("\"schema_version\":3"));
        assert!(!line.contains("\"images\":["));
        assert!(line.contains(&format!("\"blob\":\"{key}\"")));
        // Replay restores the image from the blob.
        let replay = super::super::replay_session_event_log(&log).unwrap();
        assert_eq!(replay.msgs[0].images, vec![url.to_owned()]);
    }

    #[test]
    fn compact_rewrites_to_one_replace_line() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let digest = [1u8; 32];
        append_session_replace_event(&session, &[user_msg("one")], &digest, 1, "test").unwrap();
        append_session_append_event(&session, 1, &[user_msg("two")], &digest, 2).unwrap();
        // An orphan blob: compact must sweep it.
        let dir = store::session_blobs_dir(&session);
        std::fs::create_dir_all(&dir).unwrap();
        let orphan = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(dir.join(orphan), "junk").unwrap();
        compact_session_event_log(
            &session,
            &[user_msg("one"), user_msg("two")],
            &digest,
            3,
            "fold",
        )
        .unwrap();
        let log = store::session_event_log(&session);
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"type\":\"replace\""));
        assert!(lines[0].contains("\"revision\":4"));
        assert!(lines[0].contains("\"base_revision\":3"));
        assert!(lines[0].contains("\"reason\":\"fold\""));
        assert!(!dir.join(orphan).exists());
        let replay = super::super::replay_session_event_log(&log).unwrap();
        assert_eq!(replay.msgs.len(), 2);
        assert_eq!(replay.msgs[0].content, "one");
        assert_eq!(replay.msgs[1].content, "two");
    }

    #[test]
    fn compact_empty_session_path_is_refused() {
        let err =
            compact_session_event_log(Path::new(""), &[user_msg("hi")], &[0u8; 32], 1, "fold")
                .unwrap_err();
        assert_eq!(err.to_string(), "empty session event log path");
    }

    #[test]
    fn oversized_fold_and_compact_rules() {
        let mib = 1024 * 1024;
        // Under the compact floor.
        assert!(!session_event_log_oversized(1000, 10, &[]));
        // At and past max(256KiB, 4x content).
        assert!(!session_event_log_oversized(256 << 10, 10, &[]));
        assert!(session_event_log_oversized((256 << 10) + 1, 10, &[]));
        assert!(session_event_log_oversized(4 * 65_537 + 1, 65_537, &[]));
        assert!(!session_event_log_oversized(4 * 65_537, 65_537, &[]));
        // Fold: past 128MiB with another 64MiB to drop.
        let fold = 128 * mib;
        assert!(session_event_log_oversized(fold + 1, 0, &[]));
        // Content large enough that folding would not halve the log.
        assert!(!session_event_log_oversized(fold + 1, fold + 1, &[]));
        // Inline images discount the content a fold holds outside.
        let msgs = vec![Message {
            images: vec!["data:image/png;base64,QUJD".to_owned()],
            ..user_msg("x")
        }];
        assert!(session_event_log_oversized(fold + 1, 12, &msgs));
    }

    #[test]
    fn log_size_reports_missing_and_present() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        assert_eq!(session_event_log_size(&session), 0);
        let log = store::session_event_log(&session);
        std::fs::write(&log, "12345\n").unwrap();
        assert_eq!(session_event_log_size(&session), 6);
        // A directory in the way reads as 0.
        let other = tmp.path().join("t.jsonl");
        std::fs::create_dir_all(store::session_event_log(&other)).unwrap();
        assert_eq!(session_event_log_size(&other), 0);
    }
}
