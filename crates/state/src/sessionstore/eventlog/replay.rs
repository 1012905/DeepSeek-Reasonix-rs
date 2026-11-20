//! Event-log replay: turns a JSONL event log back into the live
//! message list (session_events.go, replay half).
//!
//! Three failure classes stay distinct, matching Go:
//! - hard errors (unsupported schema version / event type) abort the
//!   load with an error;
//! - budget refusals abort with SessionReplayLimitError and leave the
//!   files untouched;
//! - damaged logs (torn records, undecodable payloads, mismatched
//!   append indices) stop the replay at the last good record and
//!   return what decoded so far with the damaged flag set.

use std::fs::File;
use std::path::Path;

use serde_json::value::RawValue;

use super::images::resolve_session_event_images;
use super::preflight::{preflight_session_event_messages, trim_ascii};
use super::reader::{NextError, SessionRecordReader};
use super::record::{CreatedAt, SessionEventWireRecord, native_session_event_schema};
use super::{
    DecodeError, SESSION_EVENT_TYPE_APPEND, SESSION_EVENT_TYPE_REPLACE, SessionReplayLimitError,
    SessionReplayLimits,
};
use reasonix_contract::gojson;
use reasonix_contract::provider::Message;

/// Hard replay failures (Go: the fmt.Errorf paths in
/// replaySessionEventRecords).
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("decode session event log {path}: unsupported schema version {version}")]
    UnsupportedSchemaVersion { path: String, version: i64 },
    #[error("decode session event log {path}: unsupported event type {event_type:?}")]
    UnsupportedEventType { path: String, event_type: String },
    #[error("{0}")]
    Limit(#[from] SessionReplayLimitError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Outcome of a replay: the reconstructed messages plus bookkeeping
/// (Go: sessionEventReplay).
#[derive(Debug, Default)]
pub struct SessionEventReplay {
    /// Messages after replay (and any tail trimming).
    pub msgs: Vec<Message>,
    /// Sum of encoded message sizes still held.
    pub payload_bytes: i64,
    /// Messages dropped by tail trimming.
    pub dropped: usize,
    /// Per-message encoded sizes (Go: sizes).
    pub(crate) sizes: Vec<i64>,
    /// Collection items counted across all records.
    pub collection_items: usize,
    /// Creation time of every surviving message.
    pub times: Vec<CreatedAt>,
    /// Number of records replayed.
    pub records: usize,
    /// Offset just past the last fully replayed record.
    pub last_good_end: i64,
    /// Total log size in bytes.
    pub size: i64,
    /// True when the replay stopped early on damaged input.
    pub damaged: bool,
}

impl SessionEventReplay {
    /// Drops the oldest messages until the tail fits the budget (Go:
    /// trimToTail). Never removes the final message; prefers cutting
    /// at a user boundary and backs off over trailing tool results so
    /// the tail never starts mid-turn.
    pub(crate) fn trim_to_tail(&mut self, tail_budget: i64) {
        if tail_budget == 0 {
            return;
        }
        if self.sizes.len() > self.msgs.len() {
            self.sizes.clear();
        }
        for msg in &self.msgs[self.sizes.len()..] {
            let size = gojson::marshal(msg)
                .map(|s| s.len() as i64)
                .unwrap_or_default();
            self.sizes.push(size);
        }
        let mut total: i64 = self.sizes.iter().sum();
        let start = if !self.msgs.is_empty() && self.msgs[0].role == "system" {
            1
        } else {
            0
        };
        let mut cut = start;
        while cut + 1 < self.msgs.len() && total > tail_budget {
            total -= self.sizes[cut];
            cut += 1;
        }
        if cut > start {
            // Prefer cutting at the next user message so a turn stays
            // together; when none remains, back off trailing tool
            // results so the final pair is not orphaned.
            let mut next = cut;
            while next < self.msgs.len() && self.msgs[next].role != "user" {
                next += 1;
            }
            let end = if next < self.msgs.len() {
                next
            } else {
                let mut end = cut;
                while end > start && self.msgs[end].role == "tool" {
                    end -= 1;
                }
                end
            };
            self.dropped += end - start;
            self.msgs.drain(start..end);
            self.sizes.drain(start..end);
            self.times.drain(start..end);
        }
        self.payload_bytes = self.sizes.iter().sum();
        self.collection_items = 0;
    }
}

/// Replays an event log with default limits (Go:
/// replaySessionEventLog).
pub fn replay_session_event_log(log_path: &Path) -> Result<SessionEventReplay, ReplayError> {
    replay_session_event_log_with_limits(log_path, &SessionReplayLimits::default())
}

/// Replays with explicit limits (Go: replaySessionEventLogWithLimits).
///
/// Not ported yet: Go appends backfillUnavailableImages here, which
/// rescues unavailable image slots from raw checkpoint payloads; that
/// needs the checkpoint reader and lands with the checkpoint slice.
pub fn replay_session_event_log_with_limits(
    log_path: &Path,
    limits: &SessionReplayLimits,
) -> Result<SessionEventReplay, ReplayError> {
    replay_session_event_records(log_path, limits)
}

/// Core replay loop (Go: replaySessionEventRecords).
pub fn replay_session_event_records(
    log_path: &Path,
    limits: &SessionReplayLimits,
) -> Result<SessionEventReplay, ReplayError> {
    let path = log_path.to_string_lossy().into_owned();
    let mut file = File::open(log_path)?;
    let size = file.metadata()?.len() as i64;
    if size > limits.max_bytes {
        return Err(ReplayError::Limit(super::session_replay_limit_error(
            &path,
            "encoded_bytes",
            size,
            limits.max_bytes,
        )));
    }
    let mut replay = SessionEventReplay {
        size,
        ..Default::default()
    };
    let mut reader = SessionRecordReader::new(&mut file, &path, limits.max_bytes);
    loop {
        let (raw, _start) = match reader.next() {
            Ok(frame) => frame,
            Err(NextError::Eof) => return Ok(replay),
            Err(NextError::Limit(err)) => return Err(ReplayError::Limit(err)),
            Err(NextError::Damaged(_err)) => {
                replay.damaged = true;
                return Ok(replay);
            }
        };
        // Go json.Unmarshal(null, &record) succeeds and leaves the
        // zero record behind: schema 0, which then fails the
        // native-schema check as a hard error.
        let record: SessionEventWireRecord =
            match serde_json::from_slice::<Option<SessionEventWireRecord>>(&raw) {
                Ok(Some(record)) => record,
                Ok(None) => SessionEventWireRecord::default(),
                Err(_err) => {
                    replay.damaged = true;
                    return Ok(replay);
                }
            };
        if !native_session_event_schema(record.schema_version) {
            return Err(ReplayError::UnsupportedSchemaVersion {
                path: path.clone(),
                version: record.schema_version,
            });
        }
        if replay.records >= limits.max_records {
            return Err(ReplayError::Limit(super::session_replay_limit_error(
                &path,
                "event_records",
                replay.records as i64 + 1,
                limits.max_records as i64,
            )));
        }
        let raw_len = record
            .messages
            .as_deref()
            .map(|raw| raw.get().len() as i64)
            .unwrap_or_default();
        match record.event_type.as_str() {
            SESSION_EVENT_TYPE_REPLACE => {
                if limits.tail_budget == 0
                    && let Some(err) = limits.payload_err(&path, raw_len)
                {
                    return Err(ReplayError::Limit(err));
                }
                replay.dropped = 0;
                replay.sizes.clear();
                let (mut msgs, items) = match decode_session_event_messages(
                    &path,
                    record.messages.as_deref(),
                    0,
                    0,
                    limits,
                ) {
                    Ok(decoded) => decoded,
                    Err(err) => return finish_decode_err(replay, err),
                };
                let items = match resolve_session_event_images(
                    &path,
                    &mut msgs,
                    &record.image_blobs,
                    items,
                    limits,
                ) {
                    Ok(items) => items,
                    Err(err) => return finish_decode_err(replay, err),
                };
                replay.times = vec![record.created_at; msgs.len()];
                replay.msgs = msgs;
                replay.payload_bytes = raw_len;
                replay.collection_items = items;
                replay.trim_to_tail(limits.tail_budget);
            }
            SESSION_EVENT_TYPE_APPEND => {
                if record.message_index < 0
                    || record.message_index as usize != replay.dropped + replay.msgs.len()
                {
                    replay.damaged = true;
                    return Ok(replay);
                }
                if limits.tail_budget == 0
                    && let Some(err) = limits.payload_err(&path, replay.payload_bytes + raw_len)
                {
                    return Err(ReplayError::Limit(err));
                }
                let (mut msgs, items) = match decode_session_event_messages(
                    &path,
                    record.messages.as_deref(),
                    replay.msgs.len(),
                    replay.collection_items,
                    limits,
                ) {
                    Ok(decoded) => decoded,
                    Err(err) => return finish_decode_err(replay, err),
                };
                let items = match resolve_session_event_images(
                    &path,
                    &mut msgs,
                    &record.image_blobs,
                    items,
                    limits,
                ) {
                    Ok(items) => items,
                    Err(err) => return finish_decode_err(replay, err),
                };
                replay.payload_bytes += raw_len;
                replay.collection_items = items;
                for _ in 0..msgs.len() {
                    replay.times.push(record.created_at);
                }
                replay.msgs.extend(msgs);
                replay.trim_to_tail(limits.tail_budget);
            }
            other => {
                return Err(ReplayError::UnsupportedEventType {
                    path: path.clone(),
                    event_type: other.to_owned(),
                });
            }
        }
        replay.records += 1;
        replay.last_good_end = reader.offset();
    }
}

/// Shared tail of the decode/resolve error paths: budget refusals
/// propagate, everything else marks the log damaged and keeps the
/// records applied so far (Go: the errors.Is split in
/// replaySessionEventRecords).
fn finish_decode_err(
    mut replay: SessionEventReplay,
    err: DecodeError,
) -> Result<SessionEventReplay, ReplayError> {
    match err {
        DecodeError::Limit(err) => Err(ReplayError::Limit(err)),
        DecodeError::Other(_detail) => {
            replay.damaged = true;
            Ok(replay)
        }
    }
}

/// Decodes one record's messages payload (Go:
/// decodeSessionEventMessages). Returns the decoded messages and the
/// updated cumulative collection-item count. An absent, empty, or
/// null payload decodes to no messages.
pub(crate) fn decode_session_event_messages(
    path: &str,
    raw: Option<&RawValue>,
    existing_messages: usize,
    existing_items: usize,
    limits: &SessionReplayLimits,
) -> Result<(Vec<Message>, usize), DecodeError> {
    let Some(raw) = raw else {
        return Ok((Vec::new(), existing_items));
    };
    let bytes = raw.get().as_bytes();
    let trimmed = trim_ascii(bytes);
    if trimmed.is_empty() || trimmed == b"null" {
        return Ok((Vec::new(), existing_items));
    }
    let counts =
        preflight_session_event_messages(path, bytes, existing_messages, existing_items, limits)?;
    let msgs: Vec<Message> =
        serde_json::from_slice(trimmed).map_err(|err| DecodeError::Other(err.to_string()))?;
    // The preflight walk and the typed decode must agree on the
    // message count; a mismatch is a bug in one of the two walkers.
    assert_eq!(
        counts.message_count,
        msgs.len(),
        "preflight/decode message count mismatch"
    );
    Ok((msgs, counts.collection_items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support;
    use reasonix_contract::provider::Role;
    use std::io::Write;

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::from("user"),
            content: content.to_owned(),
            ..Default::default()
        }
    }

    fn write_log(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
        let log = dir.path().join("s.events.jsonl");
        let mut f = File::create(&log).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        log
    }

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

    #[test]
    fn empty_log_replays_empty() {
        let tmp = test_support::tempdir();
        let log = write_log(&tmp, "");
        let replay = replay_session_event_log(&log).unwrap();
        assert!(replay.msgs.is_empty());
        assert!(!replay.damaged);
        assert_eq!(replay.records, 0);
    }

    #[test]
    fn replace_then_append_chain() {
        let tmp = test_support::tempdir();
        let body = format!(
            "{}\n{}\n{}\n",
            record_line(1, "replace", r#"[{"role":"system","content":"sys"}]"#),
            append_line(1, r#"[{"role":"user","content":"one"}]"#),
            append_line(2, r#"[{"role":"user","content":"two"}]"#),
        );
        let log = write_log(&tmp, &body);
        let replay = replay_session_event_log(&log).unwrap();
        assert_eq!(replay.records, 3);
        assert_eq!(replay.msgs.len(), 3);
        assert_eq!(replay.msgs[0].role, "system");
        assert_eq!(replay.msgs[1].content, "one");
        assert_eq!(replay.msgs[2].content, "two");
        assert_eq!(replay.times.len(), 3);
        assert!(!replay.damaged);
    }

    #[test]
    fn append_index_mismatch_marks_damaged() {
        let tmp = test_support::tempdir();
        let body = format!(
            "{}\n{}\n",
            record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#),
            record_line(1, "append", r#"[{"role":"user","content":"b"}]"#)
                .replace(r#","created_at""#, r#","message_index":5,"created_at""#),
        );
        let log = write_log(&tmp, &body);
        let replay = replay_session_event_log(&log).unwrap();
        assert!(replay.damaged);
        assert_eq!(replay.records, 1);
        assert_eq!(replay.msgs.len(), 1);
    }

    #[test]
    fn torn_tail_marks_damaged_but_keeps_prefix() {
        let tmp = test_support::tempdir();
        let body = format!(
            "{}\n{{\"schema_version\":1,\"type\":\"app",
            record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#)
        );
        let log = write_log(&tmp, &body);
        let replay = replay_session_event_log(&log).unwrap();
        assert!(replay.damaged);
        assert_eq!(replay.msgs.len(), 1);
        assert_eq!(replay.records, 1);
    }

    #[test]
    fn schema_two_is_a_hard_error() {
        let tmp = test_support::tempdir();
        let log = write_log(&tmp, &record_line(2, "replace", "[]"));
        let err = replay_session_event_log(&log).unwrap_err();
        match &err {
            ReplayError::UnsupportedSchemaVersion { path, version } => {
                assert!(path.ends_with("s.events.jsonl"));
                assert_eq!(*version, 2);
                assert_eq!(
                    err.to_string(),
                    format!("decode session event log {path}: unsupported schema version 2")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_event_type_is_a_hard_error() {
        let tmp = test_support::tempdir();
        let log = write_log(&tmp, &record_line(1, "mutate", "[]"));
        let err = replay_session_event_log(&log).unwrap_err();
        match &err {
            ReplayError::UnsupportedEventType { path, event_type } => {
                assert_eq!(event_type, "mutate");
                assert_eq!(
                    err.to_string(),
                    format!("decode session event log {path}: unsupported event type \"mutate\"")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn null_line_is_a_hard_schema_zero_error() {
        let tmp = test_support::tempdir();
        let body = format!("null\n{}", record_line(1, "replace", "[]"));
        let log = write_log(&tmp, &body);
        let err = replay_session_event_log(&log).unwrap_err();
        match err {
            ReplayError::UnsupportedSchemaVersion { version, .. } => assert_eq!(version, 0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn undecodable_payload_marks_damaged() {
        let tmp = test_support::tempdir();
        let body = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#)
            + "\n"
            + &append_line(1, r#"[{"role":123}]"#);
        let log = write_log(&tmp, &body);
        let replay = replay_session_event_log(&log).unwrap();
        assert!(replay.damaged);
        assert_eq!(replay.records, 1);
        assert_eq!(replay.msgs.len(), 1);
    }

    #[test]
    fn message_budget_refusal_leaves_files_untouched() {
        let tmp = test_support::tempdir();
        let body = format!(
            "{}\n{}\n",
            record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#),
            append_line(
                1,
                r#"[{"role":"user","content":"b"},{"role":"user","content":"c"}]"#,
            ),
        );
        let log = write_log(&tmp, &body);
        let limits = SessionReplayLimits {
            max_messages: 2,
            ..Default::default()
        };
        let err = replay_session_event_log_with_limits(&log, &limits).unwrap_err();
        match err {
            ReplayError::Limit(err) => {
                assert_eq!(err.resource, "messages");
                assert_eq!(err.value, 3);
                assert_eq!(err.limit, 2);
                assert!(
                    err.to_string()
                        .starts_with(super::super::ERR_SESSION_REPLAY_LIMIT_EXCEEDED)
                );
                assert!(
                    err.to_string()
                        .ends_with("; session files were left unchanged")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn encoded_bytes_budget_refusal() {
        let tmp = test_support::tempdir();
        let body = record_line(1, "replace", r#"[{"role":"user","content":"a"}]"#);
        let log = write_log(&tmp, &body);
        let limits = SessionReplayLimits {
            max_bytes: 10,
            ..Default::default()
        };
        let err = replay_session_event_log_with_limits(&log, &limits).unwrap_err();
        match err {
            ReplayError::Limit(err) => {
                assert_eq!(err.resource, "encoded_bytes");
                assert_eq!(err.value, body.len() as i64);
                assert_eq!(err.limit, 10);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn payload_budget_refusal_counts_running_total() {
        let tmp = test_support::tempdir();
        let first = record_line(
            1,
            "replace",
            r#"[{"role":"user","content":"aaaaaaaaaaaaaaaaaaaa"}]"#,
        );
        let second = append_line(1, r#"[{"role":"user","content":"bbbbbbbbbbbbbbbbbbbb"}]"#);
        let body = format!("{first}\n{second}\n");
        let log = write_log(&tmp, &body);
        let limits = SessionReplayLimits {
            max_payload_bytes: 80,
            ..Default::default()
        };
        let err = replay_session_event_log_with_limits(&log, &limits).unwrap_err();
        match err {
            ReplayError::Limit(err) => {
                assert_eq!(err.resource, "message_payload_bytes");
                // The running total is the raw JSON length of both
                // payloads, not the decoded message size.
                let first_raw = first.matches(r#"[{"role"#).count();
                assert_eq!(first_raw, 1);
                assert!(err.value > 80);
                assert_eq!(err.limit, 80);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tail_budget_drops_oldest_after_system() {
        let tmp = test_support::tempdir();
        let body = record_line(
            1,
            "replace",
            r#"[{"role":"system","content":"sys"},{"role":"user","content":"one"},{"role":"assistant","content":"mid"},{"role":"user","content":"two"},{"role":"assistant","content":"end"}]"#,
        );
        let log = write_log(&tmp, &body);
        // Budget forces dropping the first user message but keeps the
        // tail; the system message is protected.
        let limits = SessionReplayLimits {
            tail_budget: 90,
            ..Default::default()
        };
        let replay = replay_session_event_log_with_limits(&log, &limits).unwrap();
        assert!(replay.dropped >= 1);
        assert_eq!(replay.msgs[0].role, "system");
        assert_eq!(replay.msgs.last().unwrap().content, "end");
        assert_eq!(replay.times.len(), replay.msgs.len());
        assert_eq!(replay.collection_items, 0);
        assert!(replay.payload_bytes <= 90 || replay.msgs.len() <= 2);
    }

    #[test]
    fn images_resolve_through_replay() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let log = crate::store::session_event_log(&session);
        let blobs_dir = crate::store::session_blobs_dir_for_event_log(&log);
        std::fs::create_dir_all(&blobs_dir).unwrap();
        let url = "data:image/png;base64,QQ==";
        let key = super::super::images::session_image_key(url);
        std::fs::write(blobs_dir.join(&key["sha256:".len()..]), url).unwrap();
        let body = format!(
            r#"{{"schema_version":1,"type":"replace","messages":[{{"role":"user","content":"pic"}}],"image_blobs":[{{"message":0,"slots":[{{"blob":"{key}"}}]}}],"created_at":"2026-01-01T00:00:00Z"}}"#
        );
        let mut f = File::create(&log).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        let replay = replay_session_event_log(&log).unwrap();
        assert_eq!(replay.msgs.len(), 1);
        assert_eq!(replay.msgs[0].images, vec![url.to_owned()]);
        assert!(replay.msgs[0].unavailable_images.is_empty());
    }

    #[test]
    fn empty_and_null_messages_decode_to_nothing() {
        let tmp = test_support::tempdir();
        let body = format!(
            "{}\n{}\n{}\n",
            record_line(1, "replace", "[]"),
            append_line(0, "null"),
            r#"{"schema_version":1,"type":"append","created_at":"2026-01-01T00:00:00Z"}"#,
        );
        let log = write_log(&tmp, &body);
        let replay = replay_session_event_log(&log).unwrap();
        assert!(replay.msgs.is_empty());
        assert_eq!(replay.records, 3);
        assert!(!replay.damaged);
    }

    #[test]
    fn message_index_matches_after_tail_drop() {
        let tmp = test_support::tempdir();
        let body = format!(
            "{}\n{}\n{}\n",
            record_line(1, "replace", r#"[{"role":"user","content":"one"}]"#),
            append_line(1, r#"[{"role":"user","content":"two"}]"#),
            append_line(2, r#"[{"role":"user","content":"three"}]"#),
        );
        let log = write_log(&tmp, &body);
        let mut limits = SessionReplayLimits::default();
        // Without a tail budget the append index is the plain running
        // message count.
        let replay = replay_session_event_log_with_limits(&log, &limits).unwrap();
        assert_eq!(replay.msgs.len(), 3);
        assert_eq!(replay.dropped, 0);
        // With trimming, Go expects message_index == dropped + len(msgs);
        // every record trims in turn and the append index still lines
        // up after each drop.
        limits.tail_budget = 42;
        let replay = replay_session_event_log_with_limits(&log, &limits).unwrap();
        assert_eq!(replay.dropped, 2);
        assert_eq!(replay.msgs.len(), 1);
        assert_eq!(replay.msgs[0].content, "three");
        assert!(!replay.damaged);
    }

    #[test]
    fn trim_never_drops_the_last_message() {
        let mut replay = SessionEventReplay {
            msgs: vec![user_msg(&"x".repeat(200))],
            ..Default::default()
        };
        replay.trim_to_tail(10);
        assert_eq!(replay.msgs.len(), 1);
        assert_eq!(replay.dropped, 0);
    }

    #[test]
    fn trim_backs_off_trailing_tool_results() {
        let assistant_msg = Message {
            role: Role::from("assistant"),
            content: "m".repeat(100),
            ..Default::default()
        };
        let tool_msg = Message {
            role: Role::from("tool"),
            content: "t".repeat(100),
            ..Default::default()
        };
        let mut replay = SessionEventReplay {
            msgs: vec![user_msg(&"a".repeat(100)), assistant_msg, tool_msg],
            times: vec![CreatedAt::zero(); 3],
            ..Default::default()
        };
        replay.trim_to_tail(10);
        // The cut walks past the last user message and finds no later
        // user message to advance to, so it backs off the trailing tool
        // result and stops at the assistant message.
        assert_eq!(replay.msgs.len(), 2);
        assert_eq!(replay.msgs[0].role, "assistant");
        assert_eq!(replay.msgs[1].role, "tool");
        assert_eq!(replay.dropped, 1);
    }
}
