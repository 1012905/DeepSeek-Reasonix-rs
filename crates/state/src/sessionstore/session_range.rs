//! Bounded range reads through the display index (Go:
//! `session_display_range.go`).
//!
//! The transcript is a random-read model of the authoritative event log, so
//! the index's offsets are file-exact. Callers must still compare the index
//! revision and digest with the authoritative content ledger before trusting
//! a range — this module proves only that the bytes it read decode.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use reasonix_contract::provider::Message;

use super::display_index::{SessionDisplayIndex, load_session_display_index};
use crate::store::naming::session_display_index;

/// Decodes messages `[start, end)` through the display index (Go:
/// `LoadSessionDisplayMessageRange`). Returns the messages plus the index the
/// ranges were read through.
pub fn load_session_display_message_range(
    session_path: &Path,
    start: i64,
    end: i64,
) -> io::Result<(Vec<Message>, SessionDisplayIndex)> {
    let index = load_session_display_index(&session_display_index(session_path))?;
    if start < 0 || end < start || end > index.message_count {
        return Err(io::Error::other(format!(
            "invalid display message range [{start},{end})"
        )));
    }
    let mut file = std::fs::File::open(session_path)?;
    let info = file.metadata()?;
    if info.len() as i64 != index.transcript_size {
        return Err(io::Error::other("display index transcript size changed"));
    }
    let count = usize::try_from(end - start).map_err(|_| io::Error::other("range too large"))?;
    let mut out: Vec<Message> = Vec::with_capacity(count);
    for entry in &index.entries[start as usize..end as usize] {
        let length = usize::try_from(entry.length)
            .map_err(|_| io::Error::other("display index entry length out of range"))?;
        let mut line = vec![0u8; length];
        file.seek(SeekFrom::Start(entry.offset as u64))?;
        file.read_exact(&mut line)?;
        if line.is_empty() || line[line.len() - 1] != b'\n' {
            return Err(io::Error::other(format!(
                "display message {} is not newline terminated",
                entry.index
            )));
        }
        let message: Message = serde_json::from_slice(&line[..line.len() - 1]).map_err(|err| {
            io::Error::other(format!("decode display message {}: {err}", entry.index))
        })?;
        out.push(message);
    }
    Ok((out, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::display_index::refresh_session_display_index;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::session::{digest_session_messages, write_session_messages};
    use reasonix_contract::provider::Role;

    fn user(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Vec<Message>) {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        let msgs = vec![
            Message {
                role: Role::SYSTEM,
                content: "system prompt".to_string(),
                ..Message::default()
            },
            user("first question"),
            Message {
                role: Role::ASSISTANT,
                content: "first answer".to_string(),
                ..Message::default()
            },
            user("second question"),
            Message {
                role: Role::ASSISTANT,
                content: "second answer".to_string(),
                ..Message::default()
            },
        ];
        write_session_messages(&path, &msgs).expect("write transcript");
        let digest = digest_session_messages(&msgs).expect("digest");
        refresh_session_display_index(&path, &msgs, &digest, 1, -1).expect("refresh index");
        (dir, path, msgs)
    }

    #[test]
    fn reads_a_bounded_range() {
        let (_dir, path, msgs) = fixture();
        let (got, index) = load_session_display_message_range(&path, 1, 3).expect("range");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].content, "first question");
        assert_eq!(got[1].content, "first answer");
        assert_eq!(index.message_count, msgs.len() as i64);
    }

    #[test]
    fn empty_range_reads_nothing() {
        let (_dir, path, _msgs) = fixture();
        let (got, _index) = load_session_display_message_range(&path, 2, 2).expect("empty range");
        assert!(got.is_empty());
    }

    #[test]
    fn whole_transcript_matches_the_source() {
        let (_dir, path, msgs) = fixture();
        let (got, _index) =
            load_session_display_message_range(&path, 0, msgs.len() as i64).expect("range");
        assert_eq!(got.len(), msgs.len());
        for (a, b) in got.iter().zip(msgs.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.content, b.content);
        }
    }

    #[test]
    fn out_of_bounds_ranges_are_refused() {
        let (_dir, path, msgs) = fixture();
        let count = msgs.len() as i64;
        for (start, end) in [(-1, 1), (2, 1), (0, count + 1)] {
            let err =
                load_session_display_message_range(&path, start, end).expect_err("range refused");
            assert!(err.to_string().contains("invalid display message range"));
        }
    }

    #[test]
    fn a_resized_transcript_is_refused() {
        let (_dir, path, _msgs) = fixture();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open");
        use std::io::Write as _;
        file.write_all(b"{}\n").expect("append");
        let err = load_session_display_message_range(&path, 0, 1).expect_err("size refused");
        assert_eq!(err.to_string(), "display index transcript size changed");
    }

    #[test]
    fn a_missing_index_is_an_error() {
        let dir = tempdir();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, b"{}\n").expect("write");
        assert!(load_session_display_message_range(&path, 0, 1).is_err());
    }
}
