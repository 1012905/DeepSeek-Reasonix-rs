//! The event-index sidecar (session_events.go 152, 677-749).
//!
//! The index is a small JSON summary written after each successful save:
//! it records the log size the index was built against (the repair path's
//! cheap intact check), the message count and revision for listings, the
//! content digest, the writer identity, and the update time. Go writes it
//! with plain tags (no omitempty) via MarshalIndent plus a trailing
//! newline, through a temp file and ReplaceFile.

use std::io::{self, Write};
use std::path::Path;

use reasonix_contract::go_struct;
use reasonix_contract::gojson::{escape_html, null_default};

use super::SESSION_EVENT_SCHEMA_VERSION;
use super::record::{CreatedAt, deserialize_created_at};
use super::writer_id::{digest_string, session_writer_id};

go_struct! {
    /// The event-index sidecar (session_events.go 152). Go writes it with
    /// plain tags (no omitempty), so every field round-trips and missing
    /// fields decode to zero values.
    pub struct SessionEventIndex : SessionEventIndexWire {
        #[serde(rename = "schema_version", default, deserialize_with = "null_default")]
        pub schema_version: i64,
        #[serde(rename = "log_size", default, deserialize_with = "null_default")]
        pub log_size: i64,
        #[serde(rename = "message_count", default, deserialize_with = "null_default")]
        pub message_count: i64,
        #[serde(rename = "revision", default, deserialize_with = "null_default")]
        pub revision: i64,
        #[serde(rename = "content_digest", default, deserialize_with = "null_default")]
        pub content_digest: String,
        #[serde(rename = "writer_id", default, deserialize_with = "null_default")]
        pub writer_id: String,
        #[serde(rename = "updated_at", default, deserialize_with = "deserialize_created_at")]
        pub updated_at: CreatedAt,
    }
}

/// Reads the event-index sidecar (session_events.go 677). An empty session
/// path has no index at all; a missing file is an error, exactly like Go's
/// ReadFileUTF8. The index is machine-written UTF-8, so a plain UTF-8 read
/// is equivalent; the BOM/encoding cascade of fileutil.ReadFileUTF8 is a
/// platform concern outside this module.
pub fn read_session_event_index(session_path: &Path) -> io::Result<Option<SessionEventIndex>> {
    let path = crate::store::session_event_index(session_path);
    if path.as_os_str().is_empty() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    let idx: SessionEventIndex = serde_json::from_str(&raw)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    if idx.schema_version != SESSION_EVENT_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported session event index schema {}",
                idx.schema_version
            ),
        ));
    }
    Ok(Some(idx))
}

/// Writes the event-index sidecar after a successful save
/// (session_events.go 696). Go passes the whole message slice but uses
/// only its length, so this port takes the count directly; the log must
/// already be on disk, and a session with no event log loses a stale
/// index and returns Ok, exactly like Go. The bytes are MarshalIndent
/// (two-space pretty JSON, HTML-escaped, trailing newline) published via
/// temp file + replace, with no fsync — a torn index is cheap to rebuild
/// because the log stays authoritative.
pub fn write_session_event_index(
    session_path: &Path,
    message_count: usize,
    digest: &[u8; 32],
    revision: i64,
) -> io::Result<()> {
    let index_path = crate::store::session_event_index(session_path);
    if index_path.as_os_str().is_empty() {
        return Ok(());
    }
    let log_len = match std::fs::metadata(crate::store::session_event_log(session_path)) {
        Ok(info) => info.len(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // No event log: remove a stale index from a previous save.
            return reasonix_core::fileutil::remove(&index_path);
        }
        Err(err) => return Err(err),
    };
    let idx = SessionEventIndex {
        schema_version: SESSION_EVENT_SCHEMA_VERSION,
        log_size: log_len as i64,
        message_count: message_count as i64,
        revision,
        content_digest: digest_string(digest),
        writer_id: session_writer_id().to_string(),
        updated_at: CreatedAt::from_datetime(chrono::Utc::now().fixed_offset()),
    };
    let pretty = serde_json::to_string_pretty(&idx)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let mut data = escape_html(&pretty);
    data.push('\n');
    let dir = match index_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    reasonix_core::fileutil::mkdir_all_mode(&dir, 0o755)?;
    let (mut tmp, tmp_path) =
        reasonix_core::fileutil::create_temp_file(&dir, ".session-event-index.", ".tmp")?;
    if let Err(err) = tmp.write_all(data.as_bytes()) {
        drop(tmp);
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }
    drop(tmp);
    if let Err(err) = reasonix_core::fileutil::replace_file(&tmp_path, &index_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support;

    fn write_log(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
        let session = dir.path().join("s.jsonl");
        let log = crate::store::session_event_log(&session);
        std::fs::write(&log, body).unwrap();
        session
    }

    #[test]
    fn write_then_read_round_trip() {
        let tmp = test_support::tempdir();
        let session = write_log(&tmp, "0123456789");
        let digest = [7u8; 32];
        write_session_event_index(&session, 4, &digest, 9).unwrap();
        let idx = read_session_event_index(&session).unwrap().unwrap();
        assert_eq!(idx.schema_version, 1);
        assert_eq!(idx.log_size, 10);
        assert_eq!(idx.message_count, 4);
        assert_eq!(idx.revision, 9);
        assert_eq!(idx.content_digest, digest_string(&digest));
        assert!(!idx.writer_id.is_empty());
        assert!(idx.updated_at.get() > CreatedAt::zero().get());
    }

    #[test]
    fn written_bytes_are_pretty_with_trailing_newline() {
        let tmp = test_support::tempdir();
        let session = write_log(&tmp, "x");
        write_session_event_index(&session, 1, &[0u8; 32], 1).unwrap();
        let raw = std::fs::read_to_string(crate::store::session_event_index(&session)).unwrap();
        assert!(raw.ends_with("}\n"), "{raw:?}");
        assert_eq!(raw.lines().count(), 9, "{raw}");
        assert!(
            raw.starts_with(
                "{\n  \"schema_version\": 1,\n  \"log_size\": 1,\n  \"message_count\": 1,\n  \"revision\": 1,\n  \"content_digest\": \""
            ),
            "{raw}"
        );
        // No stray temp files remain next to the index.
        let leftovers: Vec<_> = std::fs::read_dir(
            crate::store::session_event_index(&session)
                .parent()
                .unwrap(),
        )
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(".session-event-index.") && n.ends_with(".tmp")
        })
        .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn missing_log_removes_stale_index() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let index = crate::store::session_event_index(&session);
        std::fs::write(&index, "stale").unwrap();
        write_session_event_index(&session, 1, &[0u8; 32], 1).unwrap();
        assert!(!index.exists());
    }

    #[test]
    fn empty_session_path_is_noop() {
        write_session_event_index(Path::new(""), 1, &[0u8; 32], 1).unwrap();
        assert!(read_session_event_index(Path::new("")).unwrap().is_none());
    }

    #[test]
    fn creates_missing_index_parent_dir() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("nested/dir/s.jsonl");
        let log = crate::store::session_event_log(&session);
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "abc").unwrap();
        write_session_event_index(&session, 2, &[1u8; 32], 3).unwrap();
        assert!(crate::store::session_event_index(&session).exists());
    }

    #[test]
    fn read_index_missing_file_is_error() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let err = read_session_event_index(&session).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_index_rejects_unsupported_schema() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let index = crate::store::session_event_index(&session);
        std::fs::write(&index, r#"{"schema_version":2,"log_size":10}"#).unwrap();
        let err = read_session_event_index(&session).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "unsupported session event index schema 2");
    }

    #[test]
    fn read_index_tolerates_missing_fields() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let index = crate::store::session_event_index(&session);
        std::fs::write(&index, r#"{"schema_version":1}"#).unwrap();
        let idx = read_session_event_index(&session).unwrap().unwrap();
        assert_eq!(idx.schema_version, 1);
        assert_eq!(idx.log_size, 0);
        assert_eq!(idx.message_count, 0);
        assert_eq!(idx.revision, 0);
        assert_eq!(idx.updated_at.get(), CreatedAt::zero().get());
    }
}
