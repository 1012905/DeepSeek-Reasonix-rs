//! Event-log tail repair and damaged-tail salvage
//! (session_events.go 452-553, 677-694).
//!
//! repairSessionEventLogTail truncates undecodable bytes left by a crash or
//! disk-full append so the next append cannot bury them mid-log where replay
//! would stop forever. The event index's LogSize doubles as a cheap intact
//! check so the common case never re-reads the log. Everything the
//! truncation discards is first appended to the .damaged salvage sidecar: a
//! torn tail is usually one partial record, but replay also stops at a
//! buried undecodable or out-of-order record, and then everything past it
//! (including intact turns) would be silently and permanently lost (#6607).
//! Preservation is best-effort: it must not block the repair, and its most
//! likely failure (a full disk) is the same condition that tears tails.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::index::read_session_event_index;
use super::replay::{ReplayError, replay_session_event_log};

/// Errors from repairing an event-log tail.
#[derive(Debug, thiserror::Error)]
pub enum RepairError {
    #[error(transparent)]
    Replay(#[from] ReplayError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Truncates undecodable bytes left by a crash or disk-full append so the
/// next append cannot bury them mid-log where replay would stop forever
/// (session_events.go 457). Callers must hold the session file lock.
pub fn repair_session_event_log_tail(session_path: &Path) -> Result<(), RepairError> {
    let log_path = crate::store::session_event_log(session_path);
    if log_path.as_os_str().is_empty() {
        return Ok(());
    }
    let info = match std::fs::metadata(&log_path) {
        Ok(info) => info,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    if info.is_dir() || info.len() == 0 {
        return Ok(());
    }
    // Cheap intact check: the index records the log size it was written
    // against. Any index read error just skips the check and replays.
    if let Ok(Some(idx)) = read_session_event_index(session_path)
        && idx.log_size == info.len() as i64
    {
        return Ok(());
    }
    let replay = replay_session_event_log(&log_path)?;
    if replay.last_good_end >= replay.size {
        return Ok(());
    }
    if let Err(preserve_err) =
        preserve_damaged_event_log_tail(session_path, &log_path, replay.last_good_end, replay.size)
    {
        tracing::warn!(
            path = %log_path.display(),
            from = replay.last_good_end,
            size = replay.size,
            err = %preserve_err,
            "session: could not preserve damaged event log tail; truncating anyway"
        );
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open(&log_path)?
        .set_len(replay.last_good_end as u64)?;
    if replay.last_good_end == 0 {
        return Ok(());
    }
    // The truncation point sits exactly at the end of a JSON value; restore
    // the trailing newline so the file stays line-oriented for external
    // tools (session_events.go 500-514).
    let mut file = std::fs::OpenOptions::new().append(true).open(&log_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    // Go also calls Chmod(0o600) on Windows, but there Chmod only toggles
    // the read-only bit, and a file just opened for append is already
    // writable, so there is nothing to do.
    file.write_all(b"\n")?;
    Ok(())
}

/// Appends the about-to-be-truncated byte range of the event log to the
/// .damaged salvage sidecar, prefixed with a one-line JSON header recording
/// when and where the bytes came from (session_events.go 522). The sidecar
/// is a forensic artifact for recovery, never replayed by the loader, and
/// is removed with the session's other sidecars on delete.
pub fn preserve_damaged_event_log_tail(
    session_path: &Path,
    log_path: &Path,
    from: i64,
    to: i64,
) -> io::Result<()> {
    if to <= from {
        return Ok(());
    }
    let mut src = File::open(log_path)?;
    src.seek(SeekFrom::Start(from as u64))?;
    let damaged_path = crate::store::session_event_log_damaged(session_path);
    #[cfg(unix)]
    let mut dst = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&damaged_path)?
    };
    #[cfg(not(unix))]
    let mut dst = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&damaged_path)?;
    let preserved_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let bytes = to - from;
    let header = format!(
        "{{\"damaged_tail\":true,\"preserved_at\":\"{preserved_at}\",\"log_offset\":{from},\"bytes\":{bytes}}}\n"
    );
    dst.write_all(header.as_bytes())?;
    let mut remaining = bytes as u64;
    let mut buf = vec![0u8; 8192];
    while remaining > 0 {
        let want = std::cmp::min(buf.len() as u64, remaining) as usize;
        let n = src.read(&mut buf[..want])?;
        if n == 0 {
            // Go tolerates CopyN hitting EOF early: the log may have
            // shrunk between replay and here.
            break;
        }
        dst.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    dst.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support;
    use std::path::PathBuf;

    fn record_line(schema: i64, event_type: &str, messages_json: &str) -> String {
        format!(
            r#"{{"schema_version":{schema},"type":"{event_type}","messages":{messages_json},"created_at":"2026-01-01T00:00:00Z"}}"#
        )
    }

    fn append_line(index: i64, messages_json: &str) -> String {
        format!(
            r#"{{"schema_version":1,"type":"append","message_index":{index},"messages":{messages_json},"created_at":"2026-01-01T00:00:00Z"}}"#
        )
    }

    fn write_log(dir: &tempfile::TempDir, body: &str) -> (PathBuf, PathBuf) {
        let session = dir.path().join("s.jsonl");
        let log = crate::store::session_event_log(&session);
        std::fs::write(&log, body).unwrap();
        (session, log)
    }

    fn damaged_path(session: &Path) -> PathBuf {
        crate::store::session_event_log_damaged(session)
    }

    #[test]
    fn empty_session_path_is_noop() {
        repair_session_event_log_tail(Path::new("")).unwrap();
        assert!(read_session_event_index(Path::new("")).unwrap().is_none());
    }

    #[test]
    fn missing_log_is_noop() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        repair_session_event_log_tail(&session).unwrap();
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn zero_byte_log_is_noop() {
        let tmp = test_support::tempdir();
        let (session, log) = write_log(&tmp, "");
        repair_session_event_log_tail(&session).unwrap();
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        assert!(!damaged_path(&session).exists());
    }

    #[test]
    fn dir_as_log_is_noop() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let log = crate::store::session_event_log(&session);
        std::fs::create_dir(&log).unwrap();
        repair_session_event_log_tail(&session).unwrap();
        assert!(log.is_dir());
    }

    #[test]
    fn intact_log_without_trailing_newline_is_noop() {
        let tmp = test_support::tempdir();
        let record = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let (session, log) = write_log(&tmp, &record);
        repair_session_event_log_tail(&session).unwrap();
        assert_eq!(std::fs::read(&log).unwrap(), record.as_bytes());
        assert!(!damaged_path(&session).exists());
    }

    #[test]
    fn torn_tail_is_truncated_and_preserved() {
        let tmp = test_support::tempdir();
        let record = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let torn = "{torn";
        let body = format!("{record}\n{torn}");
        let (session, log) = write_log(&tmp, &body);
        repair_session_event_log_tail(&session).unwrap();
        // The log now ends exactly at the intact record plus the restored
        // newline.
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!("{record}\n")
        );
        let sidecar = std::fs::read_to_string(damaged_path(&session)).unwrap();
        let header_end = sidecar.find('\n').unwrap();
        let header: serde_json::Value = serde_json::from_str(&sidecar[..header_end]).unwrap();
        assert_eq!(header["damaged_tail"], true);
        assert_eq!(header["log_offset"].as_i64(), Some(record.len() as i64));
        assert_eq!(header["bytes"].as_i64(), Some((1 + torn.len()) as i64));
        let preserved_at = header["preserved_at"].as_str().unwrap();
        chrono::DateTime::parse_from_rfc3339(preserved_at).unwrap();
        // The preserved range is the newline plus the torn fragment.
        let rest = &sidecar[header_end + 1..];
        assert_eq!(rest, format!("\n{torn}\n"));
        // The repaired log replays clean.
        let replay = replay_session_event_log(&log).unwrap();
        assert_eq!(replay.records, 1);
        assert!(!replay.damaged);
        assert_eq!(replay.msgs.len(), 1);
        assert_eq!(replay.msgs[0].content, "a");
    }

    #[test]
    fn buried_out_of_order_record_is_salvaged_with_later_records() {
        let tmp = test_support::tempdir();
        let record = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let buried = append_line(99, r#"[{"role":"user","content":"b"}]"#);
        let intact = append_line(1, r#"[{"role":"user","content":"c"}]"#);
        let body = format!("{record}\n{buried}\n{intact}\n");
        let (session, log) = write_log(&tmp, &body);
        repair_session_event_log_tail(&session).unwrap();
        // The repair keeps only the intact prefix.
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!("{record}\n")
        );
        // Everything past the buried record, including intact turns, is in
        // the salvage sidecar rather than silently lost.
        let sidecar = std::fs::read_to_string(damaged_path(&session)).unwrap();
        assert!(sidecar.contains(r#""message_index":99"#));
        assert!(sidecar.contains(r#""content":"c""#));
        // And the repaired log replays clean.
        let replay = replay_session_event_log(&log).unwrap();
        assert_eq!(replay.records, 1);
        assert!(!replay.damaged);
        assert_eq!(replay.msgs.len(), 1);
        assert_eq!(replay.msgs[0].content, "a");
    }

    #[test]
    fn second_incident_appends_to_sidecar() {
        let tmp = test_support::tempdir();
        let record = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let (session, log) = write_log(&tmp, &format!("{record}\n{{torn1"));
        repair_session_event_log_tail(&session).unwrap();
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        file.write_all(b"{torn2").unwrap();
        drop(file);
        repair_session_event_log_tail(&session).unwrap();
        let sidecar = std::fs::read_to_string(damaged_path(&session)).unwrap();
        assert_eq!(sidecar.matches(r#""damaged_tail":true"#).count(), 2);
        assert!(sidecar.contains("{torn1"));
        assert!(sidecar.contains("{torn2"));
    }

    #[test]
    fn index_log_size_short_circuits_repair() {
        let tmp = test_support::tempdir();
        let record = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let body = format!("{record}\n{{torn");
        let (session, log) = write_log(&tmp, &body);
        let index = crate::store::session_event_index(&session);
        std::fs::write(
            &index,
            format!(r#"{{"schema_version":1,"log_size":{}}}"#, body.len()),
        )
        .unwrap();
        repair_session_event_log_tail(&session).unwrap();
        // The intact check trusted the index: the torn log is untouched.
        assert_eq!(std::fs::read_to_string(&log).unwrap(), body);
        assert!(!damaged_path(&session).exists());
    }

    #[test]
    fn corrupt_index_does_not_block_repair() {
        let tmp = test_support::tempdir();
        let record = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let (session, log) = write_log(&tmp, &format!("{record}\n{{torn"));
        let index = crate::store::session_event_index(&session);
        std::fs::write(&index, "not json").unwrap();
        repair_session_event_log_tail(&session).unwrap();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!("{record}\n")
        );
    }

    #[test]
    fn newline_terminated_intact_log_keeps_content() {
        let tmp = test_support::tempdir();
        let record = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let body = format!("{record}\n");
        let (session, log) = write_log(&tmp, &body);
        repair_session_event_log_tail(&session).unwrap();
        // Go's lastGoodEnd sits before the trailing newline, so repair
        // salvages that one byte and re-appends it: the content is
        // unchanged.
        assert_eq!(std::fs::read_to_string(&log).unwrap(), body);
        let sidecar = std::fs::read_to_string(damaged_path(&session)).unwrap();
        assert!(sidecar.contains(r#""bytes":1"#));
        assert!(sidecar.ends_with("\n\n"));
    }

    #[test]
    fn preserve_noop_when_range_empty() {
        let tmp = test_support::tempdir();
        let (session, log) = write_log(&tmp, "abcdef");
        preserve_damaged_event_log_tail(&session, &log, 3, 3).unwrap();
        preserve_damaged_event_log_tail(&session, &log, 4, 2).unwrap();
        assert!(!damaged_path(&session).exists());
    }
}
