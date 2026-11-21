//! The display-index sidecar (`<stem>.display-index.json`, Go:
//! `session_display_index.go`).
//!
//! Every entry carries one message line's byte range in the canonical
//! transcript plus the metadata a listing needs; bodies are never copied.
//! Offset semantics are file-exact: `offset`/`length` describe exactly the
//! bytes `write_session_messages` writes for the same slice (`json.Marshal`
//! plus a newline, with the real `createdAt` — not the zeroed identity form
//! the content digest hashes).

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use reasonix_contract::go_struct;
use reasonix_contract::gojson::{go_vec, null_default};
use reasonix_contract::provider::{Message, Role};
use sha2::{Digest as _, Sha256};

use super::eventlog::record::{CreatedAt, deserialize_created_at};
use super::eventlog::writer_id::digest_string;
use super::session::message_for_session_identity;
use super::turn::classify_turn;
use crate::store::naming::session_display_index;

/// On-disk schema of the display index. Loaders reject any other version so
/// a future layout change fails closed into the rescan fallback instead of
/// misreading offsets (Go: `SessionDisplayIndexSchemaVersion`).
pub const SESSION_DISPLAY_INDEX_SCHEMA_VERSION: i64 = 1;

/// Hard per-line cap for recovery scans of a damaged transcript with no
/// newline for an arbitrarily large payload (Go:
/// `sessionDisplayIndexMaxLineBytes`).
pub const SESSION_DISPLAY_INDEX_MAX_LINE_BYTES: usize = 16 << 20;

/// Buffer size the scanner reads the transcript with; lines can be megabytes
/// when a message carries images, so the file is never loaded whole.
const SCAN_BUFFER_BYTES: usize = 1 << 20;

go_struct! {
    /// One message's metadata (Go: `DisplayIndexEntry`).
    pub struct DisplayIndexEntry : DisplayIndexEntryWire {
        #[serde(rename = "index", default, deserialize_with = "null_default")]
        pub index: i64,
        #[serde(rename = "offset", default, deserialize_with = "null_default")]
        pub offset: i64,
        #[serde(rename = "length", default, deserialize_with = "null_default")]
        pub length: i64,
        #[serde(rename = "role", default, deserialize_with = "null_default")]
        pub role: Role,
        #[serde(rename = "authored_turn", default, deserialize_with = "null_default")]
        pub authored_turn: i64,
        #[serde(default, skip_serializing_if = "is_false")]
        pub starts_turn: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub has_images: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub has_tool_calls: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub local_only: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub tool_result: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub synthetic: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        pub steer: bool,
    }
}

/// `omitempty` for the entry flags.
fn is_false(value: &bool) -> bool {
    !*value
}

impl PartialEq for DisplayIndexEntry {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
            && self.offset == other.offset
            && self.length == other.length
            && self.role == other.role
            && self.authored_turn == other.authored_turn
            && self.starts_turn == other.starts_turn
            && self.has_images == other.has_images
            && self.has_tool_calls == other.has_tool_calls
            && self.local_only == other.local_only
            && self.tool_result == other.tool_result
            && self.synthetic == other.synthetic
            && self.steer == other.steer
    }
}

go_struct! {
    /// The per-session paging sidecar (Go: `SessionDisplayIndex`).
    pub struct SessionDisplayIndex : SessionDisplayIndexWire {
        #[serde(rename = "schema_version", default, deserialize_with = "null_default")]
        pub schema_version: i64,
        #[serde(rename = "revision", default, deserialize_with = "null_default")]
        pub revision: i64,
        #[serde(rename = "revision_known", default, deserialize_with = "null_default")]
        pub revision_known: bool,
        #[serde(rename = "content_digest", default, deserialize_with = "null_default")]
        pub content_digest: String,
        #[serde(rename = "transcript_size", default, deserialize_with = "null_default")]
        pub transcript_size: i64,
        #[serde(rename = "message_count", default, deserialize_with = "null_default")]
        pub message_count: i64,
        #[serde(rename = "authored_turns", default, deserialize_with = "null_default")]
        pub authored_turns: i64,
        #[serde(rename = "entries", default, deserialize_with = "go_vec")]
        pub entries: Vec<DisplayIndexEntry>,
        #[serde(rename = "updated_at", default, deserialize_with = "deserialize_created_at")]
        pub updated_at: CreatedAt,
    }
}

impl PartialEq for SessionDisplayIndex {
    fn eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.revision == other.revision
            && self.revision_known == other.revision_known
            && self.content_digest == other.content_digest
            && self.transcript_size == other.transcript_size
            && self.message_count == other.message_count
            && self.authored_turns == other.authored_turns
            && self.entries == other.entries
            && self.updated_at == other.updated_at
    }
}

/// Derives the index from an in-memory message slice (Go:/// `BuildSessionDisplayIndex`). A pure function: same messages, same entries.
/// `None` means a message failed to marshal — impossible for a slice that
/// already survived the digest — so save-path callers treat it as warn-only.
pub fn build_session_display_index(
    messages: &[Message],
    revision: i64,
    revision_known: bool,
    digest: &[u8; 32],
) -> Option<SessionDisplayIndex> {
    let mut idx = SessionDisplayIndex {
        schema_version: SESSION_DISPLAY_INDEX_SCHEMA_VERSION,
        revision,
        revision_known,
        content_digest: digest_string(digest),
        message_count: messages.len() as i64,
        entries: Vec::with_capacity(messages.len()),
        updated_at: CreatedAt::now(),
        ..SessionDisplayIndex::default()
    };
    encode_display_index_entries(&mut idx, messages, 0, 0, 0).ok()?;
    Some(idx)
}

/// Appends the entries for `msgs[start_index..]` at byte `offset` and
/// authored-turn count `turn`, and returns the new `(offset, turn)` (Go:
/// `encodeDisplayIndexEntries`).
fn encode_display_index_entries(
    idx: &mut SessionDisplayIndex,
    msgs: &[Message],
    start_index: usize,
    mut offset: i64,
    mut turn: i64,
) -> Result<(i64, i64), serde_json::Error> {
    for (i, msg) in msgs.iter().enumerate().skip(start_index) {
        let line = reasonix_contract::gojson::marshal(msg)?;
        let length = line.len() as i64 + 1;
        let (entry, next_turn) = classify_display_index_message(msg, i, offset, length, turn);
        turn = next_turn;
        idx.entries.push(entry);
        offset += length;
    }
    idx.authored_turns = turn;
    idx.transcript_size = offset;
    Ok((offset, turn))
}

/// Fills one entry from the message alone — no body retention, no state
/// beyond the running turn counter, and no turn rule of its own:
/// [`classify_turn`] owns that (Go: `classifyDisplayIndexMessage`).
fn classify_display_index_message(
    msg: &Message,
    index: usize,
    offset: i64,
    length: i64,
    turn: i64,
) -> (DisplayIndexEntry, i64) {
    let class = classify_turn(msg, turn);
    let entry = DisplayIndexEntry {
        index: index as i64,
        offset,
        length,
        role: msg.role.clone(),
        authored_turn: class.authored_turn,
        starts_turn: class.starts_turn,
        has_images: !msg.images.is_empty(),
        has_tool_calls: !msg.tool_calls.is_empty(),
        local_only: msg.local_only,
        tool_result: msg.role == Role::TOOL,
        synthetic: class.synthetic,
        steer: class.steer,
    };
    (entry, class.authored_turn)
}

/// Incrementally extends the previous index when the save was append-only:
/// the previous index describes exactly the prefix this save built on, so its
/// entries stay valid (the canonical encoding is prefix-stable) and only the
/// tail needs encoding (Go: `extendSessionDisplayIndex`). Any doubt returns
/// `None` and the caller rebuilds from the full slice.
pub fn extend_session_display_index(
    index_path: &Path,
    msgs: &[Message],
    digest: &[u8; 32],
    revision: i64,
    append_from: i64,
) -> Option<SessionDisplayIndex> {
    let prev = load_session_display_index(index_path).ok()?;
    if prev.message_count != append_from || !prev.revision_known || prev.revision != revision - 1 {
        return None;
    }
    let mut idx = SessionDisplayIndex {
        schema_version: SESSION_DISPLAY_INDEX_SCHEMA_VERSION,
        revision,
        revision_known: true,
        content_digest: digest_string(digest),
        message_count: msgs.len() as i64,
        entries: prev.entries.clone(),
        updated_at: CreatedAt::now(),
        ..SessionDisplayIndex::default()
    };
    let start = usize::try_from(append_from).ok()?;
    encode_display_index_entries(
        &mut idx,
        msgs,
        start,
        prev.transcript_size,
        prev.authored_turns,
    )
    .ok()?;
    Some(idx)
}

/// Republishes the display index after a successful save (Go:
/// `refreshSessionDisplayIndex`). `append_from > 0` hints that the previous
/// on-disk revision is a strict prefix of `msgs`, allowing an incremental
/// extension; any other shape rebuilds. Rewind, compaction, and other rewrites
/// change revision and digest, so they land here as rebuilds naturally.
pub fn refresh_session_display_index(
    session_path: &Path,
    msgs: &[Message],
    digest: &[u8; 32],
    revision: i64,
    append_from: i64,
) -> io::Result<()> {
    let index_path = session_display_index(session_path);
    if index_path.as_os_str().is_empty() {
        return Ok(());
    }
    let extended = if append_from > 0 {
        extend_session_display_index(&index_path, msgs, digest, revision, append_from)
    } else {
        None
    };
    let idx = match extended {
        Some(idx) => idx,
        None => build_session_display_index(msgs, revision, true, digest)
            .ok_or_else(|| io::Error::other("encode session display index"))?,
    };
    write_session_display_index(&index_path, &idx)
}

/// Publishes the index atomically (tmp + fsync + rename) with 0600
/// permissions, matching the other session sidecars (Go:
/// `WriteSessionDisplayIndex`).
pub fn write_session_display_index(path: &Path, idx: &SessionDisplayIndex) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::other("empty session display index path"));
    }
    let json = reasonix_contract::gojson::marshal(idx)
        .map_err(|err| io::Error::other(format!("encode session display index: {err}")))?;
    let data = format!("{json}\n");
    reasonix_core::fileutil::atomic_write_file(path, data.as_bytes(), 0o600)
        .map_err(|err| io::Error::other(format!("write session display index: {err}")))
}

/// Reads the sidecar, rejecting truncated JSON, unsupported schema versions,
/// and header/entry-count disagreements so a corrupt index fails closed into
/// the rescan fallback (Go: `LoadSessionDisplayIndex`).
pub fn load_session_display_index(path: &Path) -> io::Result<SessionDisplayIndex> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::other("empty session display index path"));
    }
    let bytes = reasonix_core::fileencoding::read_file_utf8(path)?;
    let idx: SessionDisplayIndex = serde_json::from_slice(&bytes)
        .map_err(|err| io::Error::other(format!("decode session display index: {err}")))?;
    if idx.schema_version != SESSION_DISPLAY_INDEX_SCHEMA_VERSION {
        return Err(io::Error::other(format!(
            "unsupported session display index schema {}",
            idx.schema_version
        )));
    }
    if idx.message_count != idx.entries.len() as i64 {
        return Err(io::Error::other(format!(
            "session display index message_count {} does not match {} entries",
            idx.message_count,
            idx.entries.len()
        )));
    }
    let mut expected_offset: i64 = 0;
    for (i, entry) in idx.entries.iter().enumerate() {
        if entry.index != i as i64 {
            return Err(io::Error::other(format!(
                "session display index entry {i} has index {}",
                entry.index
            )));
        }
        if entry.offset != expected_offset
            || entry.length <= 0
            || entry.length > idx.transcript_size - entry.offset
        {
            return Err(io::Error::other(format!(
                "session display index entry {i} has invalid range {}+{}",
                entry.offset, entry.length
            )));
        }
        expected_offset += entry.length;
    }
    if expected_offset != idx.transcript_size {
        return Err(io::Error::other(format!(
            "session display index covers {expected_offset} bytes, transcript_size is {}",
            idx.transcript_size
        )));
    }
    Ok(idx)
}

/// Whether the index still describes the transcript identified by
/// revision/digest/`transcript_size` (Go: `ValidateSessionDisplayIndex`). The
/// caller picks `transcript_size`: the canonical encoding size when checking
/// in-memory state, the anchor's actual file size when checking the on-disk
/// transcript.
pub fn validate_session_display_index(
    idx: &SessionDisplayIndex,
    revision: i64,
    revision_known: bool,
    digest: &[u8; 32],
    transcript_size: i64,
) -> bool {
    if idx.schema_version != SESSION_DISPLAY_INDEX_SCHEMA_VERSION {
        return false;
    }
    if idx.revision_known != revision_known || (revision_known && idx.revision != revision) {
        return false;
    }
    if idx.content_digest != digest_string(digest) {
        return false;
    }
    if idx.transcript_size != transcript_size {
        return false;
    }
    idx.message_count == idx.entries.len() as i64
}

/// The recovery path for a missing, corrupt, or stale index (Go:
/// `ScanSessionDisplayIndex`): streams the transcript line by line, decoding
/// each line only as far as classification requires and recording the real
/// byte offsets. The content digest is rebuilt alongside so the scanned index
/// validates exactly like a built one; the revision is not recoverable from
/// the transcript alone and stays unknown. A line that does not decode fails
/// the scan — the caller falls back to a full parse.
pub fn scan_session_display_index(transcript_path: &Path) -> io::Result<SessionDisplayIndex> {
    let mut file = std::fs::File::open(transcript_path)?;
    let mut idx = SessionDisplayIndex {
        schema_version: SESSION_DISPLAY_INDEX_SCHEMA_VERSION,
        entries: Vec::new(),
        updated_at: CreatedAt::now(),
        ..SessionDisplayIndex::default()
    };
    let mut hasher = Sha256::new();
    let mut offset: i64 = 0;
    let mut turn: i64 = 0;
    let mut reader = LineReader::new(SCAN_BUFFER_BYTES);
    loop {
        let (line, read_err) = reader.read_line(&mut file)?;
        if !line.is_empty() {
            let message: Message = serde_json::from_slice(&line).map_err(|err| {
                io::Error::other(format!(
                    "decode session transcript line {}: {err}",
                    idx.entries.len()
                ))
            })?;
            let identity = message_for_session_identity(&message);
            let identity_json = reasonix_contract::gojson::marshal(&identity).map_err(|err| {
                io::Error::other(format!(
                    "re-encode session transcript line {}: {err}",
                    idx.entries.len()
                ))
            })?;
            hasher.update(identity_json.as_bytes());
            hasher.update(b"\n");
            let (entry, next_turn) = classify_display_index_message(
                &message,
                idx.entries.len(),
                offset,
                line.len() as i64,
                turn,
            );
            turn = next_turn;
            idx.entries.push(entry);
            offset += line.len() as i64;
        }
        if read_err {
            break;
        }
    }
    idx.message_count = idx.entries.len() as i64;
    idx.authored_turns = turn;
    idx.transcript_size = offset;
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&hasher.finalize());
    idx.content_digest = digest_string(&digest);
    Ok(idx)
}

/// Reads one newline-terminated record while preserving its exact bytes and
/// enforcing the per-record cap (Go: `readSessionDisplayIndexLine`). The bool
/// reports end-of-file, mirroring the `io.EOF` the Go caller breaks on.
struct LineReader {
    buffer: Vec<u8>,
}

impl LineReader {
    fn new(buffer_bytes: usize) -> Self {
        Self {
            buffer: vec![0u8; buffer_bytes],
        }
    }

    /// Returns the record bytes (including a trailing newline when one was
    /// found) and whether the input ended.
    fn read_line(&mut self, file: &mut std::fs::File) -> io::Result<(Vec<u8>, bool)> {
        let mut line: Vec<u8> = Vec::new();
        loop {
            let read = file.read(&mut self.buffer)?;
            if read == 0 {
                return Ok((line, true));
            }
            let chunk = &self.buffer[..read];
            match chunk.iter().position(|&b| b == b'\n') {
                Some(end) => {
                    if end + 1 > SESSION_DISPLAY_INDEX_MAX_LINE_BYTES - line.len() {
                        return Err(line_too_long());
                    }
                    line.extend_from_slice(&chunk[..=end]);
                    let consumed = end as i64 + 1;
                    file.seek(SeekFrom::Current(-(read as i64) + consumed))?;
                    return Ok((line, false));
                }
                None => {
                    if read > SESSION_DISPLAY_INDEX_MAX_LINE_BYTES - line.len() {
                        return Err(line_too_long());
                    }
                    line.extend_from_slice(chunk);
                }
            }
        }
    }
}

fn line_too_long() -> io::Error {
    io::Error::other(format!(
        "session transcript line exceeds {SESSION_DISPLAY_INDEX_MAX_LINE_BYTES} bytes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::preview::mid_turn_steer_message;
    use crate::sessionstore::session::{digest_session_messages, write_session_messages};

    fn user(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    /// A transcript exercising every classification the index records: plain
    /// turns, a tool call plus result, an image attachment, a local-only
    /// message, a steer, and a synthetic user message (the Go test fixture).
    fn display_index_test_messages() -> Vec<Message> {
        vec![
            Message {
                role: Role::SYSTEM,
                content: "system prompt".to_string(),
                ..Message::default()
            },
            user("first question"),
            Message {
                role: Role::ASSISTANT,
                content: "calling a tool".to_string(),
                tool_calls: vec![reasonix_contract::provider::ToolCall {
                    id: "call_1".to_string(),
                    name: "shell".to_string(),
                    arguments: r#"{"cmd":"ls"}"#.to_string(),
                    ..Default::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                tool_call_id: "call_1".to_string(),
                name: "shell".to_string(),
                content: "file.go".to_string(),
                ..Message::default()
            },
            Message {
                role: Role::ASSISTANT,
                content: "interrupted partial".to_string(),
                local_only: true,
                ..Message::default()
            },
            user(&mid_turn_steer_message("hurry up", false)),
            user("Plan approved — plan mode is off. Implement the plan now."),
            Message {
                images: vec!["data:image/png;base64,iVBORw0KGgo=".to_string()],
                ..user("second question")
            },
            Message {
                role: Role::ASSISTANT,
                content: "second answer".to_string(),
                ..Message::default()
            },
        ]
    }

    fn transcript_size(msgs: &[Message]) -> i64 {
        msgs.iter()
            .map(|m| reasonix_contract::gojson::marshal(m).unwrap().len() as i64 + 1)
            .sum()
    }

    #[test]
    fn build_round_trips_through_the_sidecar() {
        let msgs = display_index_test_messages();
        let digest = digest_session_messages(&msgs).expect("digest");
        let idx = build_session_display_index(&msgs, 7, true, &digest).expect("build");
        assert_eq!(idx.message_count, msgs.len() as i64);
        assert_eq!(idx.entries.len(), msgs.len());
        // Steer and synthetic messages open no authored turn.
        assert_eq!(idx.authored_turns, 2);
        assert_eq!(idx.transcript_size, transcript_size(&msgs));

        let dir = tempdir();
        let path = dir.path().join("session.display-index.json");
        write_session_display_index(&path, &idx).expect("write");
        let loaded = load_session_display_index(&path).expect("load");
        assert_eq!(loaded, idx);
        assert!(validate_session_display_index(
            &loaded,
            7,
            true,
            &digest,
            idx.transcript_size
        ));
    }

    #[test]
    fn offsets_match_the_written_transcript() {
        let msgs = display_index_test_messages();
        let dir = tempdir();
        let path = dir.path().join("session.jsonl");
        write_session_messages(&path, &msgs).expect("write transcript");
        let raw = std::fs::read(&path).expect("read transcript");
        let digest = digest_session_messages(&msgs).expect("digest");
        let idx = build_session_display_index(&msgs, 1, true, &digest).expect("build");
        assert_eq!(raw.len() as i64, idx.transcript_size);
        for entry in &idx.entries {
            let start = entry.offset as usize;
            let end = (entry.offset + entry.length) as usize;
            assert!(end <= raw.len(), "entry {} exceeds the file", entry.index);
            let line = &raw[start..end];
            assert_eq!(line.last(), Some(&b'\n'));
            let decoded: Message = serde_json::from_slice(&line[..line.len() - 1]).expect("decode");
            assert_eq!(decoded.role, msgs[entry.index as usize].role);
        }
        let entries = &idx.entries;
        assert_eq!(
            (
                entries[1].role.clone(),
                entries[1].authored_turn,
                entries[1].starts_turn
            ),
            (Role::USER, 1, true)
        );
        assert!(entries[2].has_tool_calls);
        assert!(entries[3].tool_result);
        assert!(entries[4].local_only);
        assert!(entries[5].steer);
        assert!(entries[6].synthetic);
        assert!(entries[7].starts_turn && entries[7].has_images);
        assert_eq!(entries[7].authored_turn, 2);
        assert_eq!(entries[8].authored_turn, 2);
    }

    #[test]
    fn extension_reuses_the_prefix_and_encodes_only_the_tail() {
        let msgs = display_index_test_messages();
        let dir = tempdir();
        let index_path = dir.path().join("session.display-index.json");
        let base = &msgs[..5];
        let base_digest = digest_session_messages(base).expect("digest base");
        let before = build_session_display_index(base, 3, true, &base_digest).expect("build base");
        write_session_display_index(&index_path, &before).expect("write base");

        let digest = digest_session_messages(&msgs).expect("digest");
        let after =
            extend_session_display_index(&index_path, &msgs, &digest, 4, 5).expect("extend");
        assert_eq!(after.message_count, msgs.len() as i64);
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(after.entries[..5], before.entries[..]);
        assert_eq!(after.transcript_size, transcript_size(&msgs));
        assert_eq!(after.authored_turns, 2);
    }

    #[test]
    fn extension_declines_when_the_prefix_is_unproven() {
        let msgs = display_index_test_messages();
        let dir = tempdir();
        let index_path = dir.path().join("session.display-index.json");
        let digest = digest_session_messages(&msgs).expect("digest");
        // No previous index at all.
        assert!(extend_session_display_index(&index_path, &msgs, &digest, 2, 3).is_none());

        let base = &msgs[..4];
        let base_digest = digest_session_messages(base).expect("digest base");
        let prev = build_session_display_index(base, 3, true, &base_digest).expect("build");
        write_session_display_index(&index_path, &prev).expect("write");
        // Entry count disagrees with the append boundary.
        assert!(extend_session_display_index(&index_path, &msgs, &digest, 4, 5).is_none());
        // Revision is not the immediate predecessor.
        assert!(extend_session_display_index(&index_path, &msgs, &digest, 9, 4).is_none());
    }

    #[test]
    fn refresh_rebuilds_without_an_appendix_hint() {
        let msgs = display_index_test_messages();
        let digest = digest_session_messages(&msgs).expect("digest");
        let dir = tempdir();
        let session = dir.path().join("s.jsonl");
        write_session_messages(&session, &msgs).expect("write transcript");
        refresh_session_display_index(&session, &msgs, &digest, 5, -1).expect("refresh");
        let idx = load_session_display_index(&session_display_index(&session)).expect("load");
        assert_eq!(idx.message_count, msgs.len() as i64);
        assert!(idx.revision_known);
        assert_eq!(idx.revision, 5);
    }

    #[test]
    fn scan_agrees_with_build() {
        let msgs = display_index_test_messages();
        let dir = tempdir();
        let session = dir.path().join("s.jsonl");
        write_session_messages(&session, &msgs).expect("write transcript");
        let digest = digest_session_messages(&msgs).expect("digest");
        let built = build_session_display_index(&msgs, 3, true, &digest).expect("build");
        let scanned = scan_session_display_index(&session).expect("scan");
        assert_eq!(scanned.entries, built.entries);
        assert_eq!(scanned.message_count, built.message_count);
        assert_eq!(scanned.authored_turns, built.authored_turns);
        assert_eq!(scanned.transcript_size, built.transcript_size);
        assert_eq!(scanned.content_digest, built.content_digest);
        // The transcript carries no revision, so the scan cannot claim one.
        assert!(!scanned.revision_known);
        assert!(validate_session_display_index(
            &scanned,
            0,
            false,
            &digest,
            built.transcript_size
        ));
    }

    #[test]
    fn scan_reads_multiple_buffered_lines_and_a_final_unterminated_line() {
        let dir = tempdir();
        let session = dir.path().join("s.jsonl");
        let body = format!(
            "{}\n{}\n{}",
            reasonix_contract::gojson::marshal(&user("one")).unwrap(),
            reasonix_contract::gojson::marshal(&user("two")).unwrap(),
            reasonix_contract::gojson::marshal(&user("three")).unwrap(),
        );
        std::fs::write(&session, body.as_bytes()).expect("write");
        let scanned = scan_session_display_index(&session).expect("scan");
        assert_eq!(scanned.entries.len(), 3);
        assert_eq!(scanned.transcript_size, body.len() as i64);
        assert_eq!(
            scanned.entries[2].offset + scanned.entries[2].length,
            body.len() as i64
        );
    }

    #[test]
    fn scan_rejects_a_line_over_the_cap() {
        let dir = tempdir();
        let session = dir.path().join("oversized.jsonl");
        let mut body = vec![b' '; SESSION_DISPLAY_INDEX_MAX_LINE_BYTES + 1];
        body.push(b'\n');
        std::fs::write(&session, body).expect("write");
        let err = scan_session_display_index(&session).expect_err("oversized line refused");
        assert!(err.to_string().contains("exceeds"), "{err}");
    }

    #[test]
    fn load_rejects_corrupt_indices() {
        let dir = tempdir();
        let truncated = dir.path().join("truncated.display-index.json");
        std::fs::write(&truncated, br#"{"schema_version":1,"revision":"#).expect("write");
        assert!(load_session_display_index(&truncated).is_err());

        let wrong_schema = dir.path().join("schema.display-index.json");
        std::fs::write(
            &wrong_schema,
            br#"{"schema_version":999,"message_count":0,"entries":[]}"#,
        )
        .expect("write");
        let err = load_session_display_index(&wrong_schema).expect_err("schema refused");
        assert!(
            err.to_string()
                .contains("unsupported session display index schema 999"),
            "{err}"
        );

        let count = dir.path().join("count.display-index.json");
        std::fs::write(
            &count,
            br#"{"schema_version":1,"message_count":2,"entries":[]}"#,
        )
        .expect("write");
        assert!(load_session_display_index(&count).is_err());

        let range = dir.path().join("range.display-index.json");
        std::fs::write(
            &range,
            br#"{"schema_version":1,"transcript_size":10,"message_count":1,"entries":[{"index":0,"offset":1,"length":9}]}"#,
        )
        .expect("write");
        let err = load_session_display_index(&range).expect_err("range refused");
        assert!(err.to_string().contains("invalid range"), "{err}");

        let cover = dir.path().join("cover.display-index.json");
        std::fs::write(
            &cover,
            br#"{"schema_version":1,"transcript_size":10,"message_count":1,"entries":[{"index":0,"offset":0,"length":9}]}"#,
        )
        .expect("write");
        let err = load_session_display_index(&cover).expect_err("coverage refused");
        assert!(err.to_string().contains("covers 9 bytes"), "{err}");
    }

    #[test]
    fn validate_rejects_every_mismatch() {
        let msgs = display_index_test_messages();
        let digest = digest_session_messages(&msgs).expect("digest");
        let size = transcript_size(&msgs);
        let idx = build_session_display_index(&msgs, 4, true, &digest).expect("build");
        assert!(validate_session_display_index(&idx, 4, true, &digest, size));
        let other = digest_session_messages(&msgs[..2]).expect("digest prefix");
        assert!(!validate_session_display_index(&idx, 4, true, &other, size));
        assert!(!validate_session_display_index(
            &idx, 5, true, &digest, size
        ));
        assert!(!validate_session_display_index(
            &idx,
            4,
            true,
            &digest,
            size - 1
        ));
        assert!(!validate_session_display_index(
            &idx, 0, false, &digest, size
        ));
    }
}
